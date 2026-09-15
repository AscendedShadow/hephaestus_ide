use gpui::{
    Action, App, ClipboardItem, Entity, EntityInputHandler as _, Focusable as _, Keystroke, Window,
};
use gpui_component::{
    Rope, RopeExt as _,
    input::{
        InputState, Redo, SelectToEnd, SelectToEndOfLine, SelectToStart, SelectToStartOfLine, Undo,
    },
};
use ide_core::vim::{Clipboard, Command, Key, Mode, Vim};

pub const NORMAL_CONTEXT: &str = "VimNormal";
pub const INSERT_CONTEXT: &str = "VimInsert";

pub struct Response {
    pub commands: Vec<Command>,
    pub message: Option<String>,
}

pub struct VimInput {
    vim: Vim,
    applied: Option<(usize, usize)>,
}

impl VimInput {
    pub fn new(shift_width: usize) -> Self {
        Self {
            vim: Vim::new(shift_width),
            applied: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.vim.mode()
    }

    pub fn key_context(&self) -> &'static str {
        if self.vim.mode() == Mode::Insert {
            INSERT_CONTEXT
        } else {
            NORMAL_CONTEXT
        }
    }

    pub fn prompt(&self) -> Option<String> {
        self.vim.prompt()
    }

    pub fn pending(&self) -> String {
        self.vim.pending()
    }

    pub fn reset(&mut self, editor: &Entity<InputState>, cx: &App) {
        let editor = editor.read(cx);
        self.vim.reset(editor.text(), editor.cursor());
        self.applied = None;
    }

    pub fn key_down(
        &mut self,
        keystroke: &Keystroke,
        editor: &Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Response> {
        let key = key(keystroke)?;
        if self.vim.mode() == Mode::Insert && !matches!(key, Key::Escape | Key::Ctrl('[')) {
            return None;
        }
        let current = selection(editor, window, cx);
        if self.applied != Some(current) {
            self.vim.sync(editor.read(cx).text(), current.0, current.1);
        }

        let text = editor.read(cx).text().clone();
        let outcome = self.vim.key(key, &text, &mut SystemClipboard(cx));
        if !outcome.edits.is_empty() {
            editor.update(cx, |editor, cx| {
                for edit in &outcome.edits {
                    let text = editor.text();
                    let range = text.byte_to_utf16_idx(edit.range.start)
                        ..text.byte_to_utf16_idx(edit.range.end);
                    editor.replace_text_in_range(Some(range), &edit.text, window, cx);
                }
            });
        }

        let mut commands = Vec::new();
        let mut rewound = false;
        let before = editor.read(cx).text().clone();
        let focus = editor.focus_handle(cx);
        for command in outcome.commands {
            match command {
                Command::Undo => focus.dispatch_action(&Undo, window, cx),
                Command::Redo => focus.dispatch_action(&Redo, window, cx),
                command => {
                    commands.push(command);
                    continue;
                }
            }
            rewound = true;
        }
        let text = editor.read(cx).text().clone();
        if rewound && let Some(changed) = first_difference(&before, &text) {
            self.vim.sync(&text, changed, changed);
        }

        let (anchor, head) = self.vim.selection(&text);
        select(editor, anchor, head, window, cx);
        self.applied = Some(selection(editor, window, cx));
        Some(Response {
            commands,
            message: outcome.message,
        })
    }
}

fn key(keystroke: &Keystroke) -> Option<Key> {
    let modifiers = &keystroke.modifiers;
    let typed = || {
        let mut chars = keystroke.key_char.as_deref()?.chars();
        match (chars.next(), chars.next()) {
            (Some(ch), None) if !ch.is_control() => Some(Key::Char(ch)),
            (Some(_), _) => Some(Key::Other),
            (None, _) => None,
        }
    };
    if modifiers.platform || modifiers.function {
        return None;
    }
    if modifiers.control && modifiers.alt {
        return typed();
    }
    if modifiers.alt {
        return None;
    }
    if modifiers.control {
        return match keystroke.key.as_str() {
            "r" | "d" | "u" | "[" => keystroke.key.chars().next().map(Key::Ctrl),
            _ => None,
        };
    }
    Some(match keystroke.key.as_str() {
        "escape" => Key::Escape,
        "enter" => Key::Enter,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "tab" => Key::Tab,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "space" => Key::Char(' '),
        _ => return typed(),
    })
}

struct SystemClipboard<'a>(&'a mut App);

impl Clipboard for SystemClipboard<'_> {
    fn read(&mut self) -> Option<String> {
        self.0.read_from_clipboard().and_then(|item| item.text())
    }

    fn write(&mut self, text: String) {
        self.0.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

fn selection(editor: &Entity<InputState>, window: &mut Window, cx: &mut App) -> (usize, usize) {
    editor.update(cx, |editor, cx| {
        let head = editor.cursor();
        let range = editor
            .selected_text_range(false, window, cx)
            .map(|selection| selection.range)
            .unwrap_or_default();
        let text = editor.text();
        let (start, end) = (
            text.utf16_to_byte_idx(range.start),
            text.utf16_to_byte_idx(range.end),
        );
        (if head == start { end } else { start }, head)
    })
}

fn select(
    editor: &Entity<InputState>,
    anchor: usize,
    head: usize,
    window: &mut Window,
    cx: &mut App,
) {
    let current = selection(editor, window, cx);
    if current == (anchor, head) {
        return;
    }
    let text = editor.read(cx).text().clone();
    if current.0 != anchor || anchor == head {
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(text.offset_to_position(anchor), window, cx)
        });
    }
    if anchor == head {
        return;
    }
    let (Ok(left), Ok(right)) = (
        cx.build_action("ui::SelectLeft", None),
        cx.build_action("ui::SelectRight", None),
    ) else {
        return;
    };
    let focus = editor.focus_handle(cx);
    let row = |pos: usize| text.offset_to_point(pos).row;
    loop {
        let cursor = editor.read(cx).cursor();
        if cursor == head {
            break;
        }
        let steps: &[&dyn Action] = if head > cursor {
            if head == text.len() {
                &[&SelectToEnd]
            } else if row(head) > row(cursor) {
                &[&SelectToEndOfLine, right.as_ref()]
            } else {
                &[right.as_ref()]
            }
        } else if head == 0 {
            &[&SelectToStart]
        } else if row(head) < row(cursor) {
            &[&SelectToStartOfLine, left.as_ref()]
        } else {
            &[left.as_ref()]
        };
        for step in steps {
            focus.dispatch_action(*step, window, cx);
        }
        if editor.read(cx).cursor() == cursor {
            break;
        }
    }
}

fn first_difference(before: &Rope, after: &Rope) -> Option<usize> {
    let mut position = 0;
    let (mut a, mut b) = (before.bytes(), after.bytes());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return None,
            (Some(x), Some(y)) if x == y => position += 1,
            _ => return Some(after.floor_char_boundary(position)),
        }
    }
}
