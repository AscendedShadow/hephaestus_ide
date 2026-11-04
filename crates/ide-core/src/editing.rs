//! Line-oriented editing commands (comment toggling, move/duplicate/delete/insert line).
//!
//! Every command is a pure transformation of the text and the selection (UTF-8 byte
//! offsets) into a list of non-overlapping edits and the selection to apply afterwards,
//! so the editor can apply it as a single undo step.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transform {
    /// Non-overlapping edits against the original text.
    pub edits: Vec<(Range<usize>, String)>,
    /// Selection in the edited text.
    pub selection: Range<usize>,
    /// The cursor is at the start of the selection.
    pub reversed: bool,
}

impl Transform {
    /// Apply the edits to `text` (used by tests and non-editor callers).
    pub fn apply(&self, text: &str) -> String {
        let mut edits = self.edits.clone();
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut result = text.to_string();
        for (range, replacement) in edits {
            result.replace_range(range, &replacement);
        }
        result
    }
}

/// Byte ranges of the lines touched by a selection: `(first line start, end of last line
/// excluding its newline, end including the newline when present)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lines {
    start: usize,
    end: usize,
    end_with_newline: usize,
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
}

fn line_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .find('\n')
        .map_or(text.len(), |ix| offset + ix)
}

fn clamp(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn selected_lines(text: &str, selection: &Range<usize>) -> Lines {
    let start = clamp(text, selection.start.min(selection.end));
    let mut end = clamp(text, selection.start.max(selection.end));
    // A selection ending at the start of a line does not include that line.
    if end > start && line_start(text, end) == end {
        end -= 1;
    }
    let first = line_start(text, start);
    let last_end = line_end(text, end);
    let with_newline = if last_end < text.len() {
        last_end + 1
    } else {
        last_end
    };
    Lines {
        start: first,
        end: last_end,
        end_with_newline: with_newline,
    }
}

fn line_ranges(text: &str, lines: Lines) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = lines.start;
    loop {
        let end = line_end(text, start);
        ranges.push(start..end);
        if end >= lines.end {
            break;
        }
        start = end + 1;
    }
    ranges
}

fn indentation(line: &str) -> &str {
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// Shift a byte offset that lies at or after `at` by `delta`.
fn shift(offset: usize, at: usize, delta: isize) -> usize {
    if offset >= at {
        offset.saturating_add_signed(delta)
    } else {
        offset
    }
}

/// Toggle a line comment (`//`, `#`, …) on every line touched by the selection.
///
/// When every non-blank line is commented, the comments are removed; otherwise every
/// non-blank line is commented at the selection's minimum indentation.
pub fn toggle_line_comment(text: &str, selection: Range<usize>, token: &str) -> Transform {
    let lines = selected_lines(text, &selection);
    let ranges = line_ranges(text, lines);
    let non_blank: Vec<_> = ranges
        .iter()
        .filter(|range| !text[(*range).clone()].trim().is_empty())
        .cloned()
        .collect();
    let commented = |range: &Range<usize>| text[range.clone()].trim_start().starts_with(token);
    let uncomment = !non_blank.is_empty() && non_blank.iter().all(commented);
    let mut edits = Vec::new();
    if uncomment {
        for range in &non_blank {
            let line = &text[range.clone()];
            let at = range.start + indentation(line).len();
            let mut len = token.len();
            if text[at + len..range.end].starts_with(' ') {
                len += 1;
            }
            edits.push((at..at + len, String::new()));
        }
    } else {
        let targets: Vec<_> = if non_blank.is_empty() {
            ranges.clone()
        } else {
            non_blank
        };
        let indent = targets
            .iter()
            .map(|range| indentation(&text[range.clone()]).len())
            .min()
            .unwrap_or(0);
        for range in &targets {
            let at = range.start + indent.min(range.len());
            edits.push((at..at, format!("{token} ")));
        }
    }
    let selection = map_selection(&edits, selection);
    Transform {
        edits,
        selection,
        reversed: false,
    }
}

/// Toggle a block comment around the selection (or the current line when empty).
pub fn toggle_block_comment(
    text: &str,
    selection: Range<usize>,
    (open, close): (&str, &str),
) -> Transform {
    let (mut start, mut end) = (
        clamp(text, selection.start.min(selection.end)),
        clamp(text, selection.start.max(selection.end)),
    );
    if start == end {
        let lines = selected_lines(text, &selection);
        let line = &text[lines.start..lines.end];
        start = lines.start + indentation(line).len();
        end = lines.start + line.trim_end().len();
    }
    let inner = &text[start..end];
    let trimmed = inner.trim();
    let lead = inner.len() - inner.trim_start().len();
    if trimmed.starts_with(open)
        && trimmed.ends_with(close)
        && trimmed.len() >= open.len() + close.len()
    {
        let open_at = start + lead;
        let close_at = open_at + trimmed.len() - close.len();
        let mut open_len = open.len();
        if text[open_at + open_len..close_at].starts_with(' ') {
            open_len += 1;
        }
        let mut close_start = close_at;
        if close_start > open_at + open_len && text[..close_start].ends_with(' ') {
            close_start -= 1;
        }
        let edits = vec![
            (open_at..open_at + open_len, String::new()),
            (close_start..close_at + close.len(), String::new()),
        ];
        let selection = start..end - open_len - (close_at + close.len() - close_start);
        return Transform {
            edits,
            selection,
            reversed: false,
        };
    }
    // Also detect a selection that sits just inside an existing block comment.
    if text[..start].ends_with(&format!("{open} ")) && text[end..].starts_with(&format!(" {close}"))
    {
        let edits = vec![
            (start - open.len() - 1..start, String::new()),
            (end..end + close.len() + 1, String::new()),
        ];
        let removed = open.len() + 1;
        return Transform {
            edits,
            selection: start - removed..end - removed,
            reversed: false,
        };
    }
    let edits = vec![
        (start..start, format!("{open} ")),
        (end..end, format!(" {close}")),
    ];
    let shift_by = open.len() + 1;
    let selection = if selection.start == selection.end {
        let caret = clamp(text, selection.start) + shift_by;
        caret..caret
    } else {
        start + shift_by..end + shift_by
    };
    Transform {
        edits,
        selection,
        reversed: false,
    }
}

fn map_selection(edits: &[(Range<usize>, String)], selection: Range<usize>) -> Range<usize> {
    let map = |offset: usize, is_end: bool| {
        let mut result = offset as isize;
        for (range, replacement) in edits {
            let delta = replacement.len() as isize - range.len() as isize;
            if range.end < offset || (range.end == offset && is_end) {
                if range.start < offset && range.end > offset {
                    result -= (offset - range.start) as isize;
                } else {
                    result += delta;
                }
            } else if range.start < offset {
                // The offset was inside a removed range; snap to its start.
                result -= (offset - range.start) as isize;
            }
        }
        result.max(0) as usize
    };
    let (start, end) = (
        selection.start.min(selection.end),
        selection.start.max(selection.end),
    );
    if start == end {
        let mapped = map(start, true);
        return mapped..mapped;
    }
    map(start, false)..map(end, true)
}

/// Move the selected lines up (`up == true`) or down by one line.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<Transform> {
    let lines = selected_lines(text, &selection);
    let block = &text[lines.start..lines.end];
    let relocate = |block_start: usize, limit: usize| {
        let map = |offset: usize| (offset.max(lines.start) - lines.start + block_start).min(limit);
        map(selection.start)..map(selection.end)
    };
    if up {
        if lines.start == 0 {
            return None;
        }
        let previous_start = line_start(text, lines.start - 1);
        let previous = &text[previous_start..lines.start - 1];
        let replacement = format!("{block}\n{previous}");
        Some(Transform {
            edits: vec![(previous_start..lines.end, replacement)],
            selection: relocate(previous_start, previous_start + block.len() + 1),
            reversed: false,
        })
    } else {
        if lines.end >= text.len() {
            return None;
        }
        let next_end = line_end(text, lines.end + 1);
        let next = &text[lines.end + 1..next_end];
        let replacement = format!("{next}\n{block}");
        let block_start = lines.start + next.len() + 1;
        let limit = if next_end < text.len() {
            next_end + 1
        } else {
            next_end
        };
        Some(Transform {
            edits: vec![(lines.start..next_end, replacement)],
            selection: relocate(block_start, limit),
            reversed: false,
        })
    }
}

/// Duplicate the selected lines above (`up == true`, the selection stays on the upper
/// copy) or below (the selection moves to the lower copy).
pub fn duplicate_lines(text: &str, selection: Range<usize>, up: bool) -> Transform {
    let lines = selected_lines(text, &selection);
    let block = &text[lines.start..lines.end];
    let insertion = format!("{block}\n");
    if up {
        Transform {
            edits: vec![(lines.start..lines.start, insertion)],
            selection,
            reversed: false,
        }
    } else if lines.end == text.len() {
        let delta = (block.len() + 1) as isize;
        Transform {
            edits: vec![(lines.end..lines.end, format!("\n{block}"))],
            selection: shift(selection.start, 0, delta)..shift(selection.end, 0, delta),
            reversed: false,
        }
    } else {
        let delta = insertion.len() as isize;
        Transform {
            edits: vec![(lines.end_with_newline..lines.end_with_newline, insertion)],
            selection: shift(selection.start, 0, delta)..shift(selection.end, 0, delta),
            reversed: false,
        }
    }
}

/// Delete the selected lines; the cursor lands on the following line (or the previous
/// line at the end of the file), keeping its column when possible.
pub fn delete_lines(text: &str, selection: Range<usize>) -> Transform {
    let lines = selected_lines(text, &selection);
    let cursor = clamp(text, selection.end);
    let column = text[line_start(text, cursor)..cursor].chars().count();
    let (range, cursor_line_start) = if lines.end_with_newline > lines.end {
        (lines.start..lines.end_with_newline, lines.start)
    } else if lines.start > 0 {
        // Last line without a trailing newline: also remove the preceding newline.
        (
            lines.start - 1..lines.end,
            line_start(text, lines.start - 1),
        )
    } else {
        (lines.start..lines.end, 0)
    };
    let mut remaining = text.to_string();
    remaining.replace_range(range.clone(), "");
    let line_end_offset = line_end(&remaining, cursor_line_start.min(remaining.len()));
    let line = &remaining[cursor_line_start.min(remaining.len())..line_end_offset];
    let offset = cursor_line_start
        + line
            .char_indices()
            .nth(column)
            .map_or(line.len(), |(ix, _)| ix);
    Transform {
        edits: vec![(range, String::new())],
        selection: offset..offset,
        reversed: false,
    }
}

/// Insert an empty line above or below the current line, keeping its indentation, and
/// place the cursor on it.
pub fn insert_line(text: &str, selection: Range<usize>, above: bool) -> Transform {
    let lines = selected_lines(text, &selection);
    let anchor = if above {
        lines.start
    } else {
        line_start(text, lines.end)
    };
    let indent = indentation(&text[anchor..line_end(text, anchor)]).to_string();
    if above {
        let insertion = format!("{indent}\n");
        let cursor = lines.start + indent.len();
        Transform {
            edits: vec![(lines.start..lines.start, insertion)],
            selection: cursor..cursor,
            reversed: false,
        }
    } else {
        let insertion = format!("\n{indent}");
        let cursor = lines.end + insertion.len();
        Transform {
            edits: vec![(lines.end..lines.end, insertion)],
            selection: cursor..cursor,
            reversed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply a transform and render the selection with `[` `]` markers (or `|` for a caret).
    fn run(text: &str, transform: Transform) -> String {
        let mut result = transform.apply(text);
        let range = transform.selection;
        if range.is_empty() {
            result.insert(range.start, '|');
        } else {
            result.insert(range.end, ']');
            result.insert(range.start, '[');
        }
        result
    }

    /// Parse `[`/`]` or `|` markers into a selection.
    fn parse(marked: &str) -> (String, Range<usize>) {
        if let Some(caret) = marked.find('|') {
            return (marked.replacen('|', "", 1), caret..caret);
        }
        let start = marked.find('[').unwrap();
        let end = marked.find(']').unwrap() - 1;
        (marked.replacen('[', "", 1).replacen(']', "", 1), start..end)
    }

    fn check(marked: &str, f: impl FnOnce(&str, Range<usize>) -> Transform) -> String {
        let (text, selection) = parse(marked);
        run(&text, f(&text, selection))
    }

    #[test]
    fn line_comments_toggle_at_common_indentation() {
        let comment = |text: &str, sel| toggle_line_comment(text, sel, "//");
        assert_eq!(
            check("fn a() {\n    [let x = 1;\n\n  let y = 2;]\n}", comment),
            "fn a() {\n  //   [let x = 1;\n\n  // let y = 2;]\n}"
        );
        assert_eq!(check("  // a|\n  //b\n", comment), "  a|\n  //b\n");
        assert_eq!(check("[  // a\n  //b\n]", comment), "[  a\n  b\n]");
        assert_eq!(check("|", comment), "// |");
        assert_eq!(
            check("x = \"é\"|", |t, s| toggle_line_comment(t, s, "#")),
            "# x = \"é\"|"
        );
        // A mixed selection comments everything.
        assert_eq!(check("[// a\nb]", comment), "[// // a\n// b]");
    }

    #[test]
    fn block_comments_wrap_and_unwrap() {
        let toggle = |text: &str, sel| toggle_block_comment(text, sel, ("/*", "*/"));
        assert_eq!(check("let [x] = 1;", toggle), "let /* [x] */ = 1;");
        assert_eq!(check("let /* [x] */ = 1;", toggle), "let [x] = 1;");
        assert_eq!(check("let [/* x */] = 1;", toggle), "let [x] = 1;");
        assert_eq!(check("  call(|);\n", toggle), "  /* call(|); */\n");
        let html = |text: &str, sel| toggle_block_comment(text, sel, ("<!--", "-->"));
        assert_eq!(check("<p>[hé]</p>", html), "<p><!-- [hé] --></p>");
    }

    #[test]
    fn move_lines_swaps_with_neighbours() {
        let up = |text: &str, sel| move_lines(text, sel, true).unwrap();
        let down = |text: &str, sel| move_lines(text, sel, false).unwrap();
        assert_eq!(check("a\nb|b\nc", up), "b|b\na\nc");
        assert_eq!(check("a\nb|b\nc", down), "a\nc\nb|b");
        assert_eq!(check("a\n[b\nc]\nd", up), "[b\nc]\na\nd");
        assert_eq!(check("a\n[b\nc\n]d", down), "a\nd\n[b\nc]");
        assert_eq!(check("🦀\nx|", up), "x|\n🦀");
        let (text, sel) = parse("|a\nb");
        assert!(move_lines(&text, sel, true).is_none());
        let (text, sel) = parse("a\nb|");
        assert!(move_lines(&text, sel, false).is_none());
    }

    #[test]
    fn duplicate_delete_and_insert() {
        assert_eq!(
            check("a|b\nc", |t, s| duplicate_lines(t, s, false)),
            "ab\na|b\nc"
        );
        assert_eq!(
            check("a|b\nc", |t, s| duplicate_lines(t, s, true)),
            "a|b\nab\nc"
        );
        assert_eq!(
            check("x\na|b", |t, s| duplicate_lines(t, s, false)),
            "x\nab\na|b"
        );
        assert_eq!(check("a\nb|b\nc", delete_lines), "a\nc|");
        assert_eq!(check("a\nbb\ncc|c", delete_lines), "a\nbb|");
        assert_eq!(check("on|ly", delete_lines), "|");
        assert_eq!(check("|", delete_lines), "|");
        assert_eq!(check("[a\nb]\nc", delete_lines), "c|");
        assert_eq!(check("ünï|code\nnext", delete_lines), "nex|t");
        assert_eq!(
            check("    a|b\nc", |t, s| insert_line(t, s, false)),
            "    ab\n    |\nc"
        );
        assert_eq!(
            check("x\n  a|b", |t, s| insert_line(t, s, true)),
            "x\n  |\n  ab"
        );
        assert_eq!(check("|", |t, s| insert_line(t, s, false)), "\n|");
    }
}
