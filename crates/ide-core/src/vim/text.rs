use std::ops::Range;

use ropey::{LineType, Rope};

const LF: LineType = LineType::LF;

pub(super) fn char_at(text: &Rope, pos: usize) -> Option<char> {
    (pos < text.len()).then(|| text.char(pos))
}

pub(super) fn next(text: &Rope, pos: usize) -> usize {
    char_at(text, pos).map_or(text.len(), |ch| pos + ch.len_utf8())
}

pub(super) fn prev(text: &Rope, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    text.chars_at(pos)
        .prev()
        .map_or(0, |ch| pos - ch.len_utf8())
}

pub(super) fn line_count(text: &Rope) -> usize {
    text.len_lines(LF)
}

pub(super) fn last_line(text: &Rope) -> usize {
    line_count(text) - 1
}

pub(super) fn line_of(text: &Rope, pos: usize) -> usize {
    text.byte_to_line_idx(pos.min(text.len()), LF)
}

pub(super) fn line_start(text: &Rope, line: usize) -> usize {
    text.line_to_byte_idx(line.min(last_line(text)), LF)
}

pub(super) fn line_end(text: &Rope, line: usize) -> usize {
    if line < last_line(text) {
        line_start(text, line + 1) - 1
    } else {
        text.len()
    }
}

pub(super) fn is_line_start(text: &Rope, pos: usize) -> bool {
    pos == 0 || text.byte(pos - 1) == b'\n'
}

pub(super) fn is_empty_line(text: &Rope, line: usize) -> bool {
    line_start(text, line) == line_end(text, line)
}

pub(super) fn lines_range(text: &Rope, first: usize, last: usize) -> Range<usize> {
    let end = if last < last_line(text) {
        line_start(text, last + 1)
    } else {
        text.len()
    };
    line_start(text, first)..end
}

pub(super) fn column(text: &Rope, pos: usize) -> usize {
    let start = line_start(text, line_of(text, pos));
    text.slice(start..pos).chars().count()
}

pub(super) fn at_column(text: &Rope, line: usize, column: usize) -> usize {
    let mut pos = line_start(text, line);
    let end = line_end(text, line);
    for _ in 0..column {
        if pos >= end {
            break;
        }
        pos = next(text, pos);
    }
    pos
}

pub(super) fn first_non_blank(text: &Rope, line: usize) -> usize {
    let mut pos = line_start(text, line);
    let end = line_end(text, line);
    while pos < end && matches!(char_at(text, pos), Some(' ' | '\t')) {
        pos = next(text, pos);
    }
    pos
}

pub(super) fn indentation(text: &Rope, line: usize) -> String {
    text.slice(line_start(text, line)..first_non_blank(text, line))
        .to_string()
}

pub(super) fn clamp_normal(text: &Rope, pos: usize) -> usize {
    let line = line_of(text, pos);
    let (start, end) = (line_start(text, line), line_end(text, line));
    if pos >= end && end > start {
        prev(text, end)
    } else {
        pos.min(end)
    }
}

pub(super) fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn class(ch: char, big: bool) -> u8 {
    if ch.is_whitespace() {
        0
    } else if big || is_word_char(ch) {
        1
    } else {
        2
    }
}

fn class_at(text: &Rope, pos: usize, big: bool) -> u8 {
    char_at(text, pos).map_or(0, |ch| class(ch, big))
}

pub(super) fn next_word_start(text: &Rope, pos: usize, big: bool) -> usize {
    let Some(ch) = char_at(text, pos) else {
        return pos;
    };
    let start_class = class(ch, big);
    let mut pos = next(text, pos);
    if start_class != 0 {
        while pos < text.len() && class_at(text, pos, big) == start_class {
            pos = next(text, pos);
        }
    }
    while let Some(ch) = char_at(text, pos) {
        if class(ch, big) != 0 || (ch == '\n' && is_line_start(text, pos)) {
            break;
        }
        pos = next(text, pos);
    }
    pos
}

pub(super) fn prev_word_start(text: &Rope, pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut pos = prev(text, pos);
    while pos > 0 && class_at(text, pos, big) == 0 {
        if is_line_start(text, pos) && char_at(text, pos) == Some('\n') {
            return pos;
        }
        pos = prev(text, pos);
    }
    let start_class = class_at(text, pos, big);
    if start_class == 0 {
        return pos;
    }
    while pos > 0 {
        let before = prev(text, pos);
        if class_at(text, before, big) != start_class {
            break;
        }
        pos = before;
    }
    pos
}

pub(super) fn word_end(text: &Rope, pos: usize, big: bool) -> usize {
    let mut end = next(text, pos);
    while end < text.len() && class_at(text, end, big) == 0 {
        end = next(text, end);
    }
    if end >= text.len() {
        return pos;
    }
    let end_class = class_at(text, end, big);
    loop {
        let after = next(text, end);
        if after >= text.len() || class_at(text, after, big) != end_class {
            return end;
        }
        end = after;
    }
}

pub(super) fn prev_word_end(text: &Rope, pos: usize, big: bool) -> usize {
    let start_class = class_at(text, pos, big);
    let mut pos = pos;
    if start_class != 0 {
        while pos > 0 && class_at(text, prev(text, pos), big) == start_class {
            pos = prev(text, pos);
        }
    }
    if pos == 0 {
        return 0;
    }
    pos = prev(text, pos);
    while pos > 0 && class_at(text, pos, big) == 0 {
        if is_line_start(text, pos) && char_at(text, pos) == Some('\n') {
            return pos;
        }
        pos = prev(text, pos);
    }
    pos
}

pub(super) fn find_in_line(
    text: &Rope,
    pos: usize,
    target: char,
    forward: bool,
    count: usize,
) -> Option<usize> {
    let line = line_of(text, pos);
    let (start, end) = (line_start(text, line), line_end(text, line));
    let mut found = 0;
    let mut pos = pos;
    loop {
        if forward {
            pos = next(text, pos);
            if pos >= end {
                return None;
            }
        } else {
            if pos <= start {
                return None;
            }
            pos = prev(text, pos);
        }
        if char_at(text, pos) == Some(target) {
            found += 1;
            if found == count {
                return Some(pos);
            }
        }
    }
}

const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

pub(super) fn matching_bracket(text: &Rope, pos: usize) -> Option<usize> {
    let end = line_end(text, line_of(text, pos));
    let mut pos = pos;
    while pos < end {
        let ch = char_at(text, pos)?;
        for (open, close) in PAIRS {
            if ch == open {
                return scan_to_close(text, next(text, pos), open, close);
            }
            if ch == close {
                return scan_to_open(text, pos, open, close);
            }
        }
        pos = next(text, pos);
    }
    None
}

fn scan_to_close(text: &Rope, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut pos = from;
    for ch in text.chars_at(from) {
        if ch == close {
            if depth == 0 {
                return Some(pos);
            }
            depth -= 1;
        } else if ch == open {
            depth += 1;
        }
        pos += ch.len_utf8();
    }
    None
}

fn scan_to_open(text: &Rope, before: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut pos = before;
    let mut chars = text.chars_at(before);
    while let Some(ch) = chars.prev() {
        pos -= ch.len_utf8();
        if ch == open {
            if depth == 0 {
                return Some(pos);
            }
            depth -= 1;
        } else if ch == close {
            depth += 1;
        }
    }
    None
}

pub(super) fn next_paragraph(text: &Rope, pos: usize) -> usize {
    let last = last_line(text);
    let mut line = line_of(text, pos);
    while line < last && is_empty_line(text, line) {
        line += 1;
    }
    while line < last && !is_empty_line(text, line) {
        line += 1;
    }
    if is_empty_line(text, line) {
        line_start(text, line)
    } else {
        text.len()
    }
}

pub(super) fn prev_paragraph(text: &Rope, pos: usize) -> usize {
    let mut line = line_of(text, pos);
    while line > 0 && is_empty_line(text, line) {
        line -= 1;
    }
    while line > 0 && !is_empty_line(text, line) {
        line -= 1;
    }
    line_start(text, line)
}

pub(super) fn word_object(
    text: &Rope,
    pos: usize,
    big: bool,
    around: bool,
) -> Option<Range<usize>> {
    let ch = char_at(text, pos).filter(|&ch| ch != '\n')?;
    let line = line_of(text, pos);
    let (line_start, line_end) = (line_start(text, line), line_end(text, line));
    let target = class(ch, big);
    let same =
        |pos: usize| char_at(text, pos).is_some_and(|ch| ch != '\n' && class(ch, big) == target);
    let mut start = pos;
    while start > line_start && same(prev(text, start)) {
        start = prev(text, start);
    }
    let mut end = next(text, pos);
    while end < line_end && same(end) {
        end = next(text, end);
    }
    if !around {
        return Some(start..end);
    }
    let blank = |pos: usize| matches!(char_at(text, pos), Some(' ' | '\t'));
    if target == 0 {
        if end < line_end {
            let word = class_at(text, end, big);
            while end < line_end && class_at(text, end, big) == word {
                end = next(text, end);
            }
        }
    } else if blank(end) {
        while end < line_end && blank(end) {
            end = next(text, end);
        }
    } else {
        while start > line_start && blank(prev(text, start)) {
            start = prev(text, start);
        }
    }
    Some(start..end)
}

pub(super) fn quote_object(
    text: &Rope,
    pos: usize,
    quote: char,
    around: bool,
) -> Option<Range<usize>> {
    let line = line_of(text, pos);
    let (mut at, end) = (line_start(text, line), line_end(text, line));
    let mut quotes = Vec::new();
    let mut escaped = false;
    while at < end {
        let ch = char_at(text, at)?;
        if ch == quote && !escaped {
            quotes.push(at);
        }
        escaped = ch == '\\' && !escaped;
        at = next(text, at);
    }
    let &[open, close] = quotes
        .as_chunks::<2>()
        .0
        .iter()
        .find(|&&[_, close]| close >= pos)?;
    Some(if around {
        open..next(text, close)
    } else {
        next(text, open)..close
    })
}

pub(super) fn bracket_object(
    text: &Rope,
    pos: usize,
    open: char,
    close: char,
    around: bool,
) -> Option<Range<usize>> {
    let open_at = if char_at(text, pos) == Some(open) {
        pos
    } else {
        scan_to_open(text, pos, open, close)?
    };
    let close_at = scan_to_close(text, next(text, open_at), open, close)?;
    if around {
        return Some(open_at..next(text, close_at));
    }
    let mut start = next(text, open_at);
    let mut end = close_at;
    let close_line = line_of(text, close_at);
    if char_at(text, start) == Some('\n') && close_line > line_of(text, open_at) {
        start += 1;
        if first_non_blank(text, close_line) == close_at {
            end = line_start(text, close_line).max(start);
        }
    }
    Some(start..end)
}
