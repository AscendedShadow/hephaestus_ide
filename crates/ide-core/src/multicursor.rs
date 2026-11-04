//! Multiple cursors and rectangular selections (IDE-013).
//!
//! The editor keeps one primary selection; secondary selections are byte ranges kept
//! alongside it. After the primary selection is edited (typing, deleting, pasting,
//! indenting), [`replicate`] derives the equivalent edits at every secondary selection so
//! they apply as part of the same undo step.

use std::ops::Range;

use unicode_width::UnicodeWidthChar;

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .find('\n')
        .map_or(text.len(), |ix| offset + ix)
}

/// The visual column of `offset` (tabs expand to `tab_width` stops, wide characters take
/// two columns, combining marks none).
pub fn visual_column(text: &str, offset: usize, tab_width: usize) -> usize {
    let start = line_start(text, offset);
    let mut column = 0;
    for character in text[start..offset].chars() {
        column += match character {
            '\t' => tab_width.max(1) - column % tab_width.max(1),
            character => character.width().unwrap_or(0),
        };
    }
    column
}

/// The offset on the line containing `line_offset` at visual `column`, or `None` when the
/// line is shorter (rectangular selections skip such lines unless `clamp`).
pub fn offset_at_column(
    text: &str,
    line_offset: usize,
    column: usize,
    tab_width: usize,
    clamp: bool,
) -> Option<usize> {
    let start = line_start(text, line_offset);
    let end = line_end(text, start);
    let mut current = 0;
    for (ix, character) in text[start..end].char_indices() {
        if current >= column {
            return Some(start + ix);
        }
        current += match character {
            '\t' => tab_width.max(1) - current % tab_width.max(1),
            character => character.width().unwrap_or(0),
        };
        if current > column {
            // Inside a tab or wide character: snap to its start.
            return Some(start + ix);
        }
    }
    (current >= column || clamp).then_some(end)
}

/// Offset of the line `delta` lines away at the same visual column (for Add Cursor
/// Above/Below), clamped to the target line's end.
pub fn vertical(text: &str, offset: usize, delta: isize, tab_width: usize) -> Option<usize> {
    let column = visual_column(text, offset, tab_width);
    let mut start = line_start(text, offset);
    if delta < 0 {
        for _ in 0..delta.unsigned_abs() {
            if start == 0 {
                return None;
            }
            start = line_start(text, start - 1);
        }
    } else {
        for _ in 0..delta {
            let end = line_end(text, start);
            if end == text.len() {
                return None;
            }
            start = end + 1;
        }
    }
    offset_at_column(text, start, column, tab_width, true)
}

/// Selections of a rectangle between two points (rows of `anchor` and `head`, visual
/// columns between theirs). Lines shorter than the left column are skipped.
pub fn rectangle(text: &str, anchor: usize, head: usize, tab_width: usize) -> Vec<Range<usize>> {
    let (left, right) = {
        let a = visual_column(text, anchor, tab_width);
        let b = visual_column(text, head, tab_width);
        (a.min(b), a.max(b))
    };
    let (first, last) = (anchor.min(head), anchor.max(head));
    let mut selections = Vec::new();
    let mut start = line_start(text, first);
    loop {
        if let Some(from) = offset_at_column(text, start, left, tab_width, false) {
            let to = offset_at_column(text, start, right, tab_width, true).unwrap_or(from);
            selections.push(from..to.max(from));
        }
        let end = line_end(text, start);
        if end >= last || end == text.len() {
            break;
        }
        start = end + 1;
    }
    selections
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The word around `offset`.
pub fn word_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(offset, |(ix, _)| ix);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(ix, _)| offset + ix);
    (start < end).then_some(start..end)
}

/// The next occurrence of `needle` starting at or after `from`, wrapping around, that is
/// not already selected.
pub fn next_occurrence(
    text: &str,
    needle: &str,
    from: usize,
    taken: &[Range<usize>],
) -> Option<Range<usize>> {
    if needle.is_empty() {
        return None;
    }
    let candidates = text[from..]
        .match_indices(needle)
        .map(|(ix, _)| from + ix)
        .chain(text[..from].match_indices(needle).map(|(ix, _)| ix));
    candidates
        .map(|start| start..start + needle.len())
        .find(|range| !taken.contains(range))
}

pub fn all_occurrences(text: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    text.match_indices(needle)
        .map(|(ix, _)| ix..ix + needle.len())
        .collect()
}

/// Sort and merge overlapping selections (touching carets merge too).
pub fn merge(mut selections: Vec<Range<usize>>) -> Vec<Range<usize>> {
    selections.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in selections {
        match merged.last_mut() {
            Some(last) if range.start < last.end || range.start == last.start => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// Edits for the secondary selections and their carets afterwards (see [`replicate`]).
pub type Replicated = (Vec<(Range<usize>, String)>, Vec<Range<usize>>);

/// Given the primary selection before an edit, the primary caret after it and the text
/// before/after the edit, compute the edits (offsets in the edited text, non-overlapping)
/// that apply the same change at each secondary selection, and the secondary carets
/// afterwards (offsets in the text after those edits, in `secondary` order). Returns
/// `None` for edits that do not map to a per-cursor change (the caller then drops the
/// secondary cursors).
///
/// Supported shapes: replacing the primary selection (typing, paste, auto-closed pairs),
/// replacing characters just before an empty primary caret (backspace, completions),
/// deleting after it (forward delete), and whitespace changes at the start of the
/// primary line (indent/outdent).
pub fn replicate(
    old: &str,
    new: &str,
    primary: Range<usize>,
    primary_after: usize,
    secondary: &[Range<usize>],
) -> Option<Replicated> {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !(old.is_char_boundary(prefix) && new.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    let mut suffix = old[prefix..]
        .bytes()
        .rev()
        .zip(new[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !(old.is_char_boundary(old.len() - suffix) && new.is_char_boundary(new.len() - suffix)) {
        suffix -= 1;
    }
    let mut changed = prefix..old.len() - suffix;
    let mut inserted = &new[prefix..new.len() - suffix];
    // The minimal diff is ambiguous when the typed text repeats its neighbours ("a" typed
    // inside "aa"); prefer the interpretation anchored at the primary selection.
    let anchored = primary.start;
    let length = new.len() as isize - old.len() as isize + primary.len() as isize;
    if (changed.start != anchored || changed.end != primary.end)
        && length >= 0
        && primary.end <= old.len()
        && new.get(..anchored) == old.get(..anchored)
        && new.get(anchored + length as usize..) == old.get(primary.end..)
    {
        changed = primary.clone();
        inserted = &new[anchored..anchored + length as usize];
    }
    let deleted = &old[changed.clone()];
    enum Kind {
        Replace,
        Before(usize),
        After(usize),
        LineStart,
    }
    let caret = primary.start;
    let kind = if changed == primary {
        Kind::Replace
    } else if primary.is_empty() && changed.end == caret {
        Kind::Before(deleted.chars().count())
    } else if primary.is_empty() && changed.start == caret && inserted.is_empty() {
        Kind::After(deleted.chars().count())
    } else if changed.start == line_start(old, caret)
        && deleted
            .chars()
            .chain(inserted.chars())
            .all(|c| c == ' ' || c == '\t')
    {
        Kind::LineStart
    } else {
        return None;
    };
    let delta = inserted.len() as isize - deleted.len() as isize;
    let map = |offset: usize| -> usize {
        if offset >= changed.end {
            offset.saturating_add_signed(delta)
        } else {
            offset
        }
    };
    // Where the primary caret ended up relative to the inserted text.
    let caret_in_insert = primary_after
        .checked_sub(changed.start)
        .filter(|ix| *ix <= inserted.len() && inserted.is_char_boundary(*ix))
        .unwrap_or(inserted.len());
    let mut edits = Vec::new();
    // (edit index, caret offset relative to the start of the edit's replacement, or the
    // offset in `new` for carets outside the edit)
    let mut carets = Vec::new();
    for selection in secondary {
        if selection.start < changed.end.max(changed.start + 1)
            && changed.start < selection.end.max(selection.start + 1)
        {
            return None;
        }
        let (start, end) = (map(selection.start), map(selection.end));
        match kind {
            Kind::Replace => {
                edits.push((start..end, inserted.to_string()));
                carets.push(caret_in_insert);
            }
            Kind::Before(count) if start == end => {
                let from = new[..start]
                    .char_indices()
                    .rev()
                    .nth(count.saturating_sub(1))
                    .map_or(0, |(ix, _)| ix);
                let from = if count == 0 { start } else { from };
                edits.push((from..start, inserted.to_string()));
                carets.push(caret_in_insert);
            }
            Kind::After(count) if start == end => {
                let to = new[start..]
                    .char_indices()
                    .nth(count)
                    .map_or(new.len(), |(ix, _)| start + ix);
                edits.push((start..to, String::new()));
                carets.push(0);
            }
            Kind::Before(_) | Kind::After(_) => {
                edits.push((start..end, String::new()));
                carets.push(0);
            }
            Kind::LineStart => {
                let line = line_start(new, start);
                let existing =
                    new[line..].len() - new[line..].trim_start_matches([' ', '\t']).len();
                let removed = deleted.len().min(existing);
                edits.push((line..line + removed, inserted.to_string()));
                // The caret keeps its position relative to the line's text.
                let relative = end.saturating_sub(line + removed);
                carets.push(inserted.len() + relative);
            }
        }
    }
    // Reject overlapping edits (e.g. two carets deleting the same character).
    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|ix| (edits[*ix].0.start, edits[*ix].0.end));
    if order
        .windows(2)
        .any(|pair| edits[pair[0]].0.end > edits[pair[1]].0.start)
    {
        return None;
    }
    let mut after = vec![0..0; edits.len()];
    let mut shift = 0isize;
    for ix in order {
        let (range, text) = &edits[ix];
        let caret = (range.start as isize + shift) as usize + carets[ix];
        after[ix] = caret..caret;
        shift += text.len() as isize - range.len() as isize;
    }
    Some((edits, after))
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn apply(text: &str, edits: &[(Range<usize>, String)]) -> String {
        crate::text_edit::apply(text, edits).unwrap()
    }

    #[test]
    fn visual_columns_handle_tabs_wide_and_combining_characters() {
        let text = "\tx\n漢字y\ne\u{301}z\nshort\n";
        assert_eq!(visual_column(text, 1, 4), 4);
        let y = text.find('y').unwrap();
        assert_eq!(visual_column(text, y, 4), 4);
        let z = text.find('z').unwrap();
        assert_eq!(visual_column(text, z, 4), 1);
        // Add Cursor Below keeps the visual column: x (col 4) -> y (col 4).
        assert_eq!(vertical(text, 1, 1, 4), Some(y));
        assert_eq!(
            vertical(text, y, 1, 4),
            Some(text.find("\nshort").unwrap()),
            "clamped to a short line"
        );
        assert_eq!(vertical(text, 0, -1, 4), None);
    }

    #[test]
    fn rectangles_skip_short_lines() {
        let text = "abcdef\na\nab\nabcdef\n";
        let selections = rectangle(text, 2, text.rfind('e').unwrap(), 4);
        let pieces: Vec<&str> = selections
            .iter()
            .map(|range| &text[range.clone()])
            .collect();
        // "a" ends before the left column and is skipped; "ab" gets a caret at its end.
        assert_eq!(pieces, ["cd", "", "cd"]);
        assert_eq!(selections[1], 11..11);
    }

    #[test]
    fn occurrences_and_merging() {
        let text = "foo bar foo baz foo";
        assert_eq!(word_at(text, 1), Some(0..3));
        assert_eq!(next_occurrence(text, "foo", 3, &[0..3]), Some(8..11));
        assert_eq!(
            next_occurrence(text, "foo", 17, &[16..19, 8..11]),
            Some(0..3),
            "wraps around"
        );
        assert_eq!(all_occurrences(text, "foo").len(), 3);
        assert_eq!(
            merge(vec![5..8, 0..3, 2..4, 8..8, 8..8]),
            vec![0..4, 5..8, 8..8]
        );
    }

    #[test]
    fn replicates_typing_deleting_and_indenting() {
        // Typing at the primary caret (0) and a secondary caret (4).
        let (edits, after) = replicate("ab\ncd", "Xab\ncd", 0..0, 1, &[3..3]).unwrap();
        assert_eq!(apply("Xab\ncd", &edits), "Xab\nXcd");
        assert_eq!(after, vec![5..5]);
        // Replacing selections.
        let (edits, _) = replicate("foo foo", "bar foo", 0..3, 3, &[4..7]).unwrap();
        assert_eq!(apply("bar foo", &edits), "bar bar");
        // Backspace over a wide character and a combining sequence.
        let old = "a漢\nbé";
        let new = "a\nbé";
        let (edits, _) = replicate(old, new, 4..4, 1, &[old.len()..old.len()]).unwrap();
        assert_eq!(apply(new, &edits), "a\nb");
        // Forward delete.
        let (edits, _) = replicate("xab\nxcd", "ab\nxcd", 0..0, 0, &[4..4]).unwrap();
        assert_eq!(apply("ab\nxcd", &edits), "ab\ncd");
        // Indent (inserted at the start of the line, caret mid-line).
        let (edits, after) = replicate("ab\ncd", "  ab\ncd", 1..1, 3, &[4..4]).unwrap();
        assert_eq!(apply("  ab\ncd", &edits), "  ab\n  cd");
        assert_eq!(after, vec![8..8]);
        // Typing a character equal to its neighbour stays anchored at the caret.
        let (edits, _) = replicate("aa\naa", "aaa\naa", 1..1, 2, &[4..4]).unwrap();
        assert_eq!(apply("aaa\naa", &edits), "aaa\naaa");
        // A change elsewhere cannot be replicated.
        assert!(replicate("ab\ncd", "ab\ncX", 0..0, 0, &[1..1]).is_none());
        // Auto-closed pairs keep the caret between the brackets.
        let (edits, after) = replicate("a\nb", "a()\nb", 1..1, 2, &[3..3]).unwrap();
        assert_eq!(apply("a()\nb", &edits), "a()\nb()");
        assert_eq!(after, vec![6..6]);
        // A completion replacing the word before the caret.
        let (edits, after) = replicate("pr x\npr", "println x\npr", 2..2, 7, &[7..7]).unwrap();
        assert_eq!(apply("println x\npr", &edits), "println x\nprintln");
        assert_eq!(after, vec![17..17]);
        // Non-BMP characters are deleted whole.
        let old = "😀\n😀";
        let (edits, _) = replicate(old, "\n😀", 4..4, 0, &[old.len()..old.len()]).unwrap();
        assert_eq!(apply("\n😀", &edits), "\n");
        // Overlapping deletions are refused.
        assert!(replicate("abcd", "acd", 2..2, 1, &[4..4, 4..4]).is_none());
    }
}
