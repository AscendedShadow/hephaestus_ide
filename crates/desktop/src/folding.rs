use std::ops::Range;

pub const PLACEHOLDER: &str = "...";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub range: Range<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Toggle {
    pub cursor: usize,
    pub folded: bool,
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
    let spans = display_spans(folds);
    if let Some((ix, _)) = spans
        .iter()
        .enumerate()
        .find(|(_, (_, display))| display.start <= display_cursor && display_cursor <= display.end)
    {
        let cursor = folds[ix].range.start;
        folds.remove(ix);
        return Some(Toggle {
            cursor: source_to_display(folds, cursor),
            folded: false,
        });
    }

    let source_cursor = display_to_source(folds, display_cursor, false);
    let range = brace_ranges(source)
        .into_iter()
        .filter(|range| {
            range.start.saturating_sub(1) <= source_cursor && source_cursor <= range.end
        })
        .min_by_key(Range::len)?;
    if range.is_empty() {
        return None;
    }

    folds.retain(|fold| fold.range.end <= range.start || fold.range.start >= range.end);
    folds.push(Fold {
        range: range.clone(),
    });
    folds.sort_by_key(|fold| fold.range.start);
    Some(Toggle {
        cursor: source_to_display(folds, range.start),
        folded: true,
    })
}

pub fn apply_edit(source: &str, folds: &mut Vec<Fold>, displayed: &str) -> Option<String> {
    let previous = projected(source, folds);
    if previous == displayed {
        return None;
    }

    let prefix = common_prefix(&previous, displayed);
    let suffix = common_suffix(&previous[prefix..], &displayed[prefix..]);
    let old_range = prefix..previous.len() - suffix;
    let replacement = &displayed[prefix..displayed.len() - suffix];
    let source_range = display_to_source(folds, old_range.start, false)
        ..display_to_source(folds, old_range.end, true);

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
        if offset < display.start {
            break;
        }
        if offset <= display.end {
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

fn brace_ranges(source: &str) -> Vec<Range<usize>> {
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
    let mut ranges = Vec::new();
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
                        ranges.push(open + 1..ix);
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
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_the_innermost_brace_body() {
        let source = "fn main() {\n  if ready { run(); }\n}\n";
        let mut folds = Vec::new();
        let cursor = source.find("run").unwrap();
        assert!(toggle(source, &mut folds, cursor).unwrap().folded);
        assert_eq!(
            projected(source, &folds),
            "fn main() {\n  if ready {...}\n}\n"
        );

        let dots = projected(source, &folds).find(PLACEHOLDER).unwrap() + 1;
        assert!(!toggle(source, &mut folds, dots).unwrap().folded);
        assert_eq!(projected(source, &folds), source);
    }

    #[test]
    fn ignores_braces_in_strings_and_comments() {
        let source = "fn f() { let text = \"}\"; /* { */ work(); }";
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
    fn an_edit_through_a_placeholder_unfolds_that_range() {
        let source = "a { hidden } z";
        let mut folds = vec![Fold { range: 3..11 }];
        let updated = apply_edit(source, &mut folds, "a {new} z").unwrap();
        assert_eq!(updated, "a {new} z");
        assert!(folds.is_empty());
    }
}
