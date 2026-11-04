//! Validation and application of text edits, plus LSP-style position conversion.
//!
//! Positions follow the Language Server Protocol: zero-based lines and UTF-16 code-unit
//! columns. Edits use UTF-8 byte ranges once converted. Following LSP semantics, edits
//! must not overlap; several insertions at the same offset are applied in list order.

use std::{fmt, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    OutOfBounds { range: Range<usize>, len: usize },
    NotCharBoundary(usize),
    Overlap(Range<usize>, Range<usize>),
    InvalidPosition { line: u32, character: u32 },
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { range, len } => {
                write!(
                    formatter,
                    "edit {range:?} is outside the document ({len} bytes)"
                )
            }
            Self::NotCharBoundary(offset) => {
                write!(formatter, "edit offset {offset} splits a character")
            }
            Self::Overlap(a, b) => write!(formatter, "edits {a:?} and {b:?} overlap"),
            Self::InvalidPosition { line, character } => {
                write!(
                    formatter,
                    "position {}:{} is outside the document",
                    line + 1,
                    character + 1
                )
            }
        }
    }
}

impl std::error::Error for EditError {}

/// Check that edits are in bounds, on character boundaries and non-overlapping.
pub fn validate(text: &str, edits: &[(Range<usize>, String)]) -> Result<(), EditError> {
    for (range, _) in edits {
        if range.start > range.end || range.end > text.len() {
            return Err(EditError::OutOfBounds {
                range: range.clone(),
                len: text.len(),
            });
        }
        for offset in [range.start, range.end] {
            if !text.is_char_boundary(offset) {
                return Err(EditError::NotCharBoundary(offset));
            }
        }
    }
    let mut sorted: Vec<&Range<usize>> = edits.iter().map(|(range, _)| range).collect();
    sorted.sort_by_key(|range| (range.start, range.end));
    for pair in sorted.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let both_inserts_at_same_point = a.is_empty() && b.is_empty();
        let same_start_replacements =
            !both_inserts_at_same_point && a.start == b.start && !a.is_empty() && !b.is_empty();
        if b.start < a.end || same_start_replacements {
            return Err(EditError::Overlap(a.clone(), b.clone()));
        }
    }
    Ok(())
}

/// Apply validated edits, preserving the order of same-offset insertions.
pub fn apply(text: &str, edits: &[(Range<usize>, String)]) -> Result<String, EditError> {
    validate(text, edits)?;
    let mut ordered: Vec<(usize, &(Range<usize>, String))> = edits.iter().enumerate().collect();
    ordered.sort_by_key(|(ix, (range, _))| (range.start, range.end, *ix));
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;
    for (_, (range, replacement)) in ordered {
        result.push_str(&text[cursor..range.start]);
        result.push_str(replacement);
        cursor = cursor.max(range.end);
    }
    result.push_str(&text[cursor..]);
    Ok(result)
}

/// Map an offset in the original text through `edits` (after applying them).
pub fn map_offset(edits: &[(Range<usize>, String)], offset: usize) -> usize {
    let mut delta = 0isize;
    for (range, replacement) in edits {
        if range.end <= offset && !(range.is_empty() && range.start == offset) {
            delta += replacement.len() as isize - range.len() as isize;
        } else if range.start < offset && offset < range.end {
            // Inside a replaced range: move to the end of its replacement.
            delta += (range.start + replacement.len()) as isize - offset as isize;
        }
    }
    offset.saturating_add_signed(delta)
}

/// Convert an LSP position (line, UTF-16 column) to a byte offset. Columns past the end
/// of a line clamp to the line end, as the protocol requires; lines past the end of the
/// document are an error except for the position just after the last line.
pub fn offset(text: &str, line: u32, character: u32) -> Result<usize, EditError> {
    let mut start = 0;
    for _ in 0..line {
        match text[start..].find('\n') {
            Some(ix) => start += ix + 1,
            None => {
                return if character == 0 && start == text.len() {
                    Ok(text.len())
                } else {
                    Err(EditError::InvalidPosition { line, character })
                };
            }
        }
    }
    let end = text[start..].find('\n').map_or(text.len(), |ix| start + ix);
    let line_text = &text[start..end];
    let line_text = line_text.strip_suffix('\r').unwrap_or(line_text);
    let mut units = 0u32;
    for (ix, character_value) in line_text.char_indices() {
        if units >= character {
            return Ok(start + ix);
        }
        units += character_value.len_utf16() as u32;
        if units > character {
            // The position splits a surrogate pair; snap to the character start.
            return Ok(start + ix);
        }
    }
    Ok(start + line_text.len())
}

/// Convert a byte offset to an LSP position (line, UTF-16 column).
pub fn position(text: &str, offset: usize) -> (u32, u32) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map_or(0, |ix| ix + 1);
    (line, before[start..].encode_utf16().count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(range: Range<usize>, text: &str) -> (Range<usize>, String) {
        (range, text.to_string())
    }

    #[test]
    fn applies_unordered_edits_and_keeps_insert_order() {
        let text = "let a = b;";
        let edits = vec![
            edit(8..9, "c"),
            edit(4..5, "x"),
            edit(0..0, "/*1*/"),
            edit(0..0, "/*2*/"),
        ];
        assert_eq!(apply(text, &edits).unwrap(), "/*1*//*2*/let x = c;");
        assert!(matches!(
            apply(text, &[edit(0..5, ""), edit(3..6, "")]),
            Err(EditError::Overlap(..))
        ));
        assert!(matches!(
            apply(text, &[edit(5..50, "")]),
            Err(EditError::OutOfBounds { .. })
        ));
        assert!(matches!(
            apply("é", &[edit(1..2, "")]),
            Err(EditError::NotCharBoundary(1))
        ));
        // Adjacent replacements do not overlap.
        assert_eq!(
            apply("abcd", &[edit(0..2, "X"), edit(2..4, "Y")]).unwrap(),
            "XY"
        );
    }

    #[test]
    fn utf16_positions_round_trip() {
        let text = "a😀b\nsecond\r\nthird";
        assert_eq!(offset(text, 0, 0).unwrap(), 0);
        assert_eq!(offset(text, 0, 1).unwrap(), 1);
        assert_eq!(offset(text, 0, 3).unwrap(), 5);
        assert_eq!(
            offset(text, 0, 2).unwrap(),
            1,
            "inside a surrogate pair snaps back"
        );
        assert_eq!(offset(text, 0, 99).unwrap(), 6);
        assert_eq!(
            offset(text, 1, 99).unwrap(),
            13,
            "CR is not part of the line"
        );
        assert_eq!(offset(text, 2, 5).unwrap(), text.len());
        assert!(offset(text, 5, 0).is_err());
        assert_eq!(offset("x\n", 1, 0).unwrap(), 2);
        assert_eq!(position(text, 5), (0, 3));
        assert_eq!(position(text, 8), (1, 1));
    }

    #[test]
    fn maps_offsets_through_edits() {
        let edits = vec![edit(0..0, "ab"), edit(4..6, "x")];
        assert_eq!(map_offset(&edits, 0), 0);
        assert_eq!(map_offset(&edits, 2), 4);
        assert_eq!(map_offset(&edits, 5), 7);
        assert_eq!(map_offset(&edits, 8), 9);
    }
}
