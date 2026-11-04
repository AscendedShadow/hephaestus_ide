//! Document change notifications, full or incremental as each server asks.
//!
//! Incremental changes cover the span that differs between the text last sent and the
//! new text (one event; several edits in one keystroke, e.g. with multiple cursors,
//! become one wider event). Positions are LSP positions: zero-based lines and UTF-16
//! columns, computed in the text *before* the change.

use gpui_component::Rope;
use ide_core::text_edit::{rope_difference, rope_position};
use lsp_types::{
    Position, Range, ServerCapabilities, TextDocumentContentChangeEvent,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};

/// How the server wants document changes. Servers that do not declare synchronization
/// are sent the full text (the lenient choice; declaring `None` turns changes off).
pub(super) fn change_kind(capabilities: Option<&ServerCapabilities>) -> TextDocumentSyncKind {
    match capabilities.and_then(|capabilities| capabilities.text_document_sync.as_ref()) {
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind,
        Some(TextDocumentSyncCapability::Options(options)) => {
            options.change.unwrap_or(TextDocumentSyncKind::NONE)
        }
        None => TextDocumentSyncKind::FULL,
    }
}

/// The change events taking the server from `old` to `new`, or `None` when the server
/// does not want changes. `may_contain_cr` forces full text: LSP counts a lone `\r` as a
/// line break, rope line indexing does not, so ranges could point at the wrong line.
pub(super) fn content_changes(
    old: &Rope,
    new: &Rope,
    kind: TextDocumentSyncKind,
    may_contain_cr: bool,
) -> Option<Vec<TextDocumentContentChangeEvent>> {
    if kind == TextDocumentSyncKind::NONE {
        return None;
    }
    if kind != TextDocumentSyncKind::INCREMENTAL || may_contain_cr {
        return Some(vec![TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: new.to_string(),
        }]);
    }
    let (removed, inserted) = rope_difference(old, new);
    let position = |offset: usize| {
        let (line, character) = rope_position(old, offset);
        Position::new(line, character)
    };
    Some(vec![TextDocumentContentChangeEvent {
        range: Some(Range::new(position(removed.start), position(removed.end))),
        range_length: None,
        text: new.slice(inserted).to_string(),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ide_core::text_edit;
    use lsp_types::TextDocumentSyncOptions;

    /// Apply change events the way a server does (LSP positions on the previous text).
    fn replay(text: &str, changes: &[TextDocumentContentChangeEvent]) -> String {
        let mut text = text.to_string();
        for change in changes {
            text = match change.range {
                None => change.text.clone(),
                Some(range) => {
                    let start =
                        text_edit::offset(&text, range.start.line, range.start.character).unwrap();
                    let end =
                        text_edit::offset(&text, range.end.line, range.end.character).unwrap();
                    text_edit::apply(&text, &[(start..end, change.text.clone())]).unwrap()
                }
            };
        }
        text
    }

    fn incremental(old: &str, new: &str) -> Vec<TextDocumentContentChangeEvent> {
        content_changes(
            &Rope::from(old),
            &Rope::from(new),
            TextDocumentSyncKind::INCREMENTAL,
            false,
        )
        .unwrap()
    }

    #[test]
    fn incremental_changes_carry_only_the_edited_span() {
        let changes = incremental(
            "fn main() {\n    run();\n}\n",
            "fn main() {\n    run(1);\n}\n",
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].range,
            Some(Range::new(Position::new(1, 8), Position::new(1, 8)))
        );
        assert_eq!(changes[0].text, "1");

        // Deleting across lines, after a non-BMP character (two UTF-16 units).
        let changes = incremental("a😀bc\nde\nf", "a😀f");
        assert_eq!(
            changes[0].range,
            Some(Range::new(Position::new(0, 3), Position::new(2, 0)))
        );
        assert_eq!(changes[0].text, "");
        assert_eq!(replay("a😀bc\nde\nf", &changes), "a😀f");
    }

    #[test]
    fn replaying_changes_reproduces_the_new_text() {
        let samples = [
            ("", "hello"),
            ("hello", ""),
            ("same", "same"),
            ("café\n", "cafè\n"),
            ("x😀y", "x😁y"),
            ("line one\nline two\n", "line one\ninserted\nline two\n"),
            ("αβγ\nδε", "αβ\nγδε"),
            ("tab\there", "tab\t\there"),
            ("end without newline", "end without newline\n"),
        ];
        for (old, new) in samples {
            assert_eq!(
                replay(old, &incremental(old, new)),
                new,
                "{old:?} -> {new:?}"
            );
        }
        // Pseudo-random edits over mixed-width text.
        let pieces = ["a", "é", "😀", "\n", "中", " ", "z"];
        let mut seed = 0x2545_f491_u32;
        let mut next = move |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize % bound
        };
        let mut text: Vec<&str> = (0..40).map(|_| pieces[next(pieces.len())]).collect();
        for _ in 0..300 {
            let old = text.concat();
            let start = next(text.len() + 1);
            let end = (start + next(4)).min(text.len());
            let inserted: Vec<&str> = (0..next(4)).map(|_| pieces[next(pieces.len())]).collect();
            text.splice(start..end, inserted);
            let new = text.concat();
            assert_eq!(
                replay(&old, &incremental(&old, &new)),
                new,
                "{old:?} -> {new:?}"
            );
        }
    }

    #[test]
    fn sync_kind_follows_the_server() {
        let with = |sync: Option<TextDocumentSyncCapability>| ServerCapabilities {
            text_document_sync: sync,
            ..Default::default()
        };
        assert_eq!(change_kind(None), TextDocumentSyncKind::FULL);
        assert_eq!(change_kind(Some(&with(None))), TextDocumentSyncKind::FULL);
        let kind = TextDocumentSyncCapability::Kind(TextDocumentSyncKind::INCREMENTAL);
        assert_eq!(
            change_kind(Some(&with(Some(kind)))),
            TextDocumentSyncKind::INCREMENTAL
        );
        let options = TextDocumentSyncCapability::Options(TextDocumentSyncOptions {
            open_close: Some(true),
            change: None,
            ..Default::default()
        });
        assert_eq!(
            change_kind(Some(&with(Some(options)))),
            TextDocumentSyncKind::NONE
        );

        let (old, new) = (Rope::from("a\r\nb"), Rope::from("a\r\nbc"));
        assert!(content_changes(&old, &new, TextDocumentSyncKind::NONE, false).is_none());
        let full = content_changes(&old, &new, TextDocumentSyncKind::FULL, false).unwrap();
        assert_eq!((full[0].range, full[0].text.as_str()), (None, "a\r\nbc"));
        let fallback =
            content_changes(&old, &new, TextDocumentSyncKind::INCREMENTAL, true).unwrap();
        assert_eq!(fallback[0].range, None, "text with \\r is sent whole");
    }
}
