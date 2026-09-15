use std::ops::Range;

pub const PLACEHOLDER: &str = "...";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub range: Range<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Toggle {
    pub edit: Range<usize>,
    pub text: String,
    pub cursor: usize,
    pub folded: bool,
}

enum Change {
    Fold(Range<usize>),
    Unfold(usize),
}

pub fn projected(source: &str, folds: &[Fold]) -> String {
    let mut result = String::with_capacity(source.len());
    let mut from = 0;
    for fold in folds {
        if fold.range.start < from
            || fold.range.end > source.len()
            || !source.is_char_boundary(fold.range.start)
            || !source.is_char_boundary(fold.range.end)
        {
            continue;
        }
        result.push_str(&source[from..fold.range.start]);
        result.push_str(PLACEHOLDER);
        from = fold.range.end;
    }
    result.push_str(&source[from..]);
    result
}

pub fn toggle(source: &str, folds: &mut Vec<Fold>, display_cursor: usize) -> Option<Toggle> {
    let change = unfold_at(folds, display_cursor)
        .or_else(|| line_change(source, folds, display_cursor))
        .or_else(|| enclosing_fold(source, folds, display_cursor))?;
    Some(apply(source, folds, display_cursor, change))
}

pub fn toggle_line(source: &str, folds: &mut Vec<Fold>, display_offset: usize) -> Option<Toggle> {
    let change = line_change(source, folds, display_offset)?;
    Some(apply(source, folds, display_offset, change))
}

pub fn apply_edit(source: &str, folds: &mut Vec<Fold>, displayed: &str) -> Option<String> {
    let previous = projected(source, folds);
    if previous == displayed {
        return None;
    }

    let (old_range, replacement) = difference(&previous, displayed);
    let source_range = display_to_source(folds, old_range.start, false)
        ..display_to_source(folds, old_range.end, true);
    if replacement == PLACEHOLDER && foldable_ranges(source).any(|range| range == source_range) {
        insert(folds, source_range);
        return None;
    }

    let mut updated = source.to_string();
    updated.replace_range(source_range.clone(), replacement);
    let delta = replacement.len() as isize - source_range.len() as isize;
    folds.retain_mut(|fold| {
        if source_range.end <= fold.range.start {
            fold.range.start = fold.range.start.saturating_add_signed(delta);
            fold.range.end = fold.range.end.saturating_add_signed(delta);
            true
        } else {
            source_range.start >= fold.range.end
        }
    });
    Some(updated)
}

fn unfold_at(folds: &[Fold], display_cursor: usize) -> Option<Change> {
    display_spans(folds)
        .iter()
        .position(|(_, display)| display.start <= display_cursor && display_cursor <= display.end)
        .map(Change::Unfold)
}

fn line_change(source: &str, folds: &[Fold], display_offset: usize) -> Option<Change> {
    let line = line_span(&projected(source, folds), display_offset);
    if let Some(ix) = display_spans(folds)
        .iter()
        .position(|(_, display)| line.contains(&display.start))
    {
        return Some(Change::Unfold(ix));
    }

    let source_line =
        display_to_source(folds, line.start, false)..display_to_source(folds, line.end, false);
    foldable_ranges(source)
        .filter(|range| source_line.contains(&(range.start - 1)))
        .min_by_key(|range| range.start)
        .map(Change::Fold)
}

fn enclosing_fold(source: &str, folds: &[Fold], display_cursor: usize) -> Option<Change> {
    let cursor = display_to_source(folds, display_cursor, false);
    foldable_ranges(source)
        .filter(|range| range.start <= cursor && cursor <= range.end)
        .min_by_key(Range::len)
        .map(Change::Fold)
}

fn apply(source: &str, folds: &mut Vec<Fold>, display_cursor: usize, change: Change) -> Toggle {
    let before = projected(source, folds);
    let cursor = display_to_source(folds, display_cursor, false);
    let folded = match change {
        Change::Fold(range) => {
            insert(folds, range);
            true
        }
        Change::Unfold(ix) => {
            folds.remove(ix);
            false
        }
    };
    let after = projected(source, folds);
    let (edit, text) = difference(&before, &after);
    Toggle {
        edit,
        text: text.to_string(),
        cursor: source_to_display(folds, cursor),
        folded,
    }
}

fn insert(folds: &mut Vec<Fold>, range: Range<usize>) {
    folds.retain(|fold| fold.range.end <= range.start || fold.range.start >= range.end);
    folds.push(Fold { range });
    folds.sort_by_key(|fold| fold.range.start);
}

fn line_span(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |ix| offset + ix);
    start..end
}

fn display_spans(folds: &[Fold]) -> Vec<(Range<usize>, Range<usize>)> {
    let mut removed = 0isize;
    folds
        .iter()
        .map(|fold| {
            let start = fold.range.start.saturating_add_signed(-removed);
            let display = start..start + PLACEHOLDER.len();
            removed += fold.range.len() as isize - PLACEHOLDER.len() as isize;
            (fold.range.clone(), display)
        })
        .collect()
}

fn display_to_source(folds: &[Fold], offset: usize, end_bias: bool) -> usize {
    let mut adjustment = 0isize;
    for (source, display) in display_spans(folds) {
        if offset <= display.start {
            break;
        }
        if offset < display.end {
            return if end_bias { source.end } else { source.start };
        }
        adjustment += source.len() as isize - display.len() as isize;
    }
    offset.saturating_add_signed(adjustment)
}

fn source_to_display(folds: &[Fold], offset: usize) -> usize {
    let mut adjustment = 0isize;
    for fold in folds {
        if offset <= fold.range.start {
            break;
        }
        if offset < fold.range.end {
            return fold.range.start.saturating_add_signed(-adjustment);
        }
        adjustment += fold.range.len() as isize - PLACEHOLDER.len() as isize;
    }
    offset.saturating_add_signed(-adjustment)
}

fn difference<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(&old[prefix..], &new[prefix..]);
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.char_indices())
        .take_while(|((_, a), (_, b))| a == b)
        .map(|((ix, ch), _)| ix + ch.len_utf8())
        .last()
        .unwrap_or(0)
}

fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum()
}

pub fn active_pair(source: &str, cursor: usize) -> Option<(usize, usize)> {
    brace_pairs(source)
        .into_iter()
        .filter(|&(open, close)| open <= cursor && cursor <= close + 1)
        .min_by_key(|&(open, close)| close - open)
}

fn foldable_ranges(source: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    brace_pairs(source)
        .into_iter()
        .map(|(open, close)| open + 1..close)
        .filter(|range| source[range.clone()].contains('\n'))
}

fn brace_pairs(source: &str) -> Vec<(usize, usize)> {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        String { quote: u8, escaped: bool },
        LineComment,
        BlockComment,
    }

    let bytes = source.as_bytes();
    let mut state = State::Code;
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    let mut ix = 0;
    while ix < bytes.len() {
        match state {
            State::Code => match bytes[ix] {
                b'/' if bytes.get(ix + 1) == Some(&b'/') => {
                    state = State::LineComment;
                    ix += 1;
                }
                b'/' if bytes.get(ix + 1) == Some(&b'*') => {
                    state = State::BlockComment;
                    ix += 1;
                }
                b'"' => {
                    state = State::String {
                        quote: b'"',
                        escaped: false,
                    }
                }
                b'\''
                    if source[ix + 1..].find('\'').is_some_and(|distance| {
                        !source[ix + 1..ix + 1 + distance].contains('\n')
                    }) =>
                {
                    state = State::String {
                        quote: b'\'',
                        escaped: false,
                    }
                }
                b'{' => stack.push(ix),
                b'}' => {
                    if let Some(open) = stack.pop() {
                        pairs.push((open, ix));
                    }
                }
                _ => {}
            },
            State::String { quote, escaped } => {
                if escaped {
                    state = State::String {
                        quote,
                        escaped: false,
                    };
                } else if bytes[ix] == b'\\' {
                    state = State::String {
                        quote,
                        escaped: true,
                    };
                } else if bytes[ix] == quote {
                    state = State::Code;
                }
            }
            State::LineComment if bytes[ix] == b'\n' => state = State::Code,
            State::BlockComment if bytes[ix] == b'*' && bytes.get(ix + 1) == Some(&b'/') => {
                state = State::Code;
                ix += 1;
            }
            _ => {}
        }
        ix += 1;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn main() {\n  if ready {\n    run();\n  }\n  let s = S { a: 1 };\n}\n";

    fn line_start(text: &str, line: usize) -> usize {
        text.split_inclusive('\n').take(line).map(str::len).sum()
    }

    fn edited(before: &str, toggle: &Toggle) -> String {
        let mut text = before.to_string();
        text.replace_range(toggle.edit.clone(), &toggle.text);
        text
    }

    #[test]
    fn line_toggle_folds_the_block_that_opens_on_that_line() {
        let mut folds = Vec::new();
        let toggle = toggle_line(SOURCE, &mut folds, line_start(SOURCE, 1)).unwrap();
        assert!(toggle.folded);
        let displayed = projected(SOURCE, &folds);
        assert_eq!(
            displayed,
            "fn main() {\n  if ready {...}\n  let s = S { a: 1 };\n}\n"
        );
        assert_eq!(edited(SOURCE, &toggle), displayed);
        assert_eq!(toggle.cursor, line_start(&displayed, 1));

        let toggle = toggle_line(SOURCE, &mut folds, line_start(&displayed, 1)).unwrap();
        assert!(!toggle.folded);
        assert_eq!(edited(&displayed, &toggle), SOURCE);
        assert!(folds.is_empty());
    }

    #[test]
    fn line_toggle_needs_a_multi_line_block_on_that_line() {
        let mut folds = Vec::new();
        assert!(toggle_line(SOURCE, &mut folds, line_start(SOURCE, 2)).is_none());
        assert!(toggle_line(SOURCE, &mut folds, line_start(SOURCE, 4)).is_none());
        assert!(folds.is_empty());
    }

    #[test]
    fn folding_an_outer_block_replaces_nested_folds() {
        let mut folds = Vec::new();
        toggle_line(SOURCE, &mut folds, line_start(SOURCE, 1)).unwrap();
        toggle_line(SOURCE, &mut folds, 0).unwrap();
        assert_eq!(projected(SOURCE, &folds), "fn main() {...}\n");
        toggle_line(SOURCE, &mut folds, 0).unwrap();
        assert_eq!(projected(SOURCE, &folds), SOURCE);
    }

    #[test]
    fn cursor_toggle_folds_the_innermost_enclosing_block() {
        let mut folds = Vec::new();
        let cursor = SOURCE.find("run").unwrap();
        let folded = toggle(SOURCE, &mut folds, cursor).unwrap();
        assert!(folded.folded);
        let displayed = projected(SOURCE, &folds);
        assert_eq!(
            displayed,
            "fn main() {\n  if ready {...}\n  let s = S { a: 1 };\n}\n"
        );
        assert_eq!(folded.cursor, displayed.find(PLACEHOLDER).unwrap());

        let unfolded = toggle(SOURCE, &mut folds, folded.cursor + 1).unwrap();
        assert!(!unfolded.folded);
        assert_eq!(projected(SOURCE, &folds), SOURCE);
    }

    #[test]
    fn cursor_toggle_prefers_the_block_opening_on_the_cursor_line() {
        let mut folds = Vec::new();
        toggle(SOURCE, &mut folds, SOURCE.find("ready").unwrap()).unwrap();
        assert_eq!(
            projected(SOURCE, &folds),
            "fn main() {\n  if ready {...}\n  let s = S { a: 1 };\n}\n"
        );
    }

    #[test]
    fn ignores_braces_in_strings_and_comments() {
        let source = "fn f() {\n  let text = \"}\"; /* { */\n  work();\n}";
        let mut folds = Vec::new();
        toggle(source, &mut folds, source.find("work").unwrap()).unwrap();
        assert_eq!(projected(source, &folds), "fn f() {...}");
    }

    #[test]
    fn edits_outside_a_fold_preserve_hidden_source() {
        let source = "before { hidden(); } after";
        let mut folds = vec![Fold {
            range: source.find(" hidden").unwrap()..source.find("}").unwrap(),
        }];
        let displayed = projected(source, &folds).replace("before", "BEFORE!");
        let updated = apply_edit(source, &mut folds, &displayed).unwrap();
        assert_eq!(updated, "BEFORE! { hidden(); } after");
        assert_eq!(projected(&updated, &folds), "BEFORE! {...} after");
    }

    #[test]
    fn typing_next_to_a_placeholder_keeps_the_hidden_source() {
        let source = "a { hidden } z";
        let mut folds = vec![Fold { range: 3..11 }];
        let updated = apply_edit(source, &mut folds, "a {...!} z").unwrap();
        assert_eq!(updated, "a { hidden !} z");
        assert_eq!(projected(&updated, &folds), "a {...!} z");

        let updated = apply_edit(&updated, &mut folds, "a {?...!} z").unwrap();
        assert_eq!(updated, "a {? hidden !} z");
        assert_eq!(projected(&updated, &folds), "a {?...!} z");
    }

    #[test]
    fn an_edit_through_a_placeholder_unfolds_that_range() {
        let source = "a { hidden } z";
        let mut folds = vec![Fold { range: 3..11 }];
        let updated = apply_edit(source, &mut folds, "a {new} z").unwrap();
        assert_eq!(updated, "a {new} z");
        assert!(folds.is_empty());
    }

    #[test]
    fn undoing_and_redoing_a_fold_keeps_the_source() {
        let mut folds = Vec::new();
        let toggle = toggle_line(SOURCE, &mut folds, 0).unwrap();
        let folded = edited(SOURCE, &toggle);

        assert_eq!(
            apply_edit(SOURCE, &mut folds, SOURCE).as_deref(),
            Some(SOURCE)
        );
        assert!(folds.is_empty());
        assert_eq!(apply_edit(SOURCE, &mut folds, &folded), None);
        assert_eq!(projected(SOURCE, &folds), folded);
    }

    fn pair_text(source: &str, cursor: usize) -> Option<&str> {
        active_pair(source, cursor).map(|(open, close)| &source[open..=close])
    }

    #[test]
    fn active_pair_is_the_innermost_block_around_the_cursor() {
        let cursor = SOURCE.find("run").unwrap();
        assert_eq!(pair_text(SOURCE, cursor), Some("{\n    run();\n  }"));
        let cursor = SOURCE.find("let").unwrap();
        assert_eq!(
            pair_text(SOURCE, cursor),
            Some(&SOURCE[10..SOURCE.len() - 1])
        );
        assert_eq!(pair_text(SOURCE, 0), None);
    }

    #[test]
    fn active_pair_includes_braces_touching_the_cursor() {
        let open = SOURCE.find("{ a").unwrap();
        let close = SOURCE.find("1 }").unwrap() + 2;
        assert_eq!(pair_text(SOURCE, open), Some("{ a: 1 }"));
        assert_eq!(pair_text(SOURCE, open + 1), Some("{ a: 1 }"));
        assert_eq!(pair_text(SOURCE, close), Some("{ a: 1 }"));
        assert_eq!(pair_text(SOURCE, close + 1), Some("{ a: 1 }"));
        assert_eq!(
            pair_text(SOURCE, SOURCE.len() - 1),
            Some(&SOURCE[10..SOURCE.len() - 1])
        );
        assert_eq!(pair_text(SOURCE, SOURCE.len()), None);
    }

    #[test]
    fn active_pair_ignores_braces_in_strings_and_comments() {
        let source = "fn f() {\n  let text = \"{\"; // }\n  work();\n}";
        let cursor = source.find("work").unwrap();
        assert_eq!(pair_text(source, cursor), Some(&source[7..]));
        assert_eq!(
            pair_text(source, source.find("\"{").unwrap() + 1),
            Some(&source[7..])
        );
    }
}
