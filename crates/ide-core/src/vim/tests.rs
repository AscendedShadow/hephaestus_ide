use super::*;

#[derive(Default)]
struct TestClipboard(Option<String>);

impl Clipboard for TestClipboard {
    fn read(&mut self) -> Option<String> {
        self.0.clone()
    }

    fn write(&mut self, text: String) {
        self.0 = Some(text);
    }
}

fn keys(notation: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = notation;
    while let Some(ch) = rest.chars().next() {
        if ch == '<'
            && let Some(end) = rest.find('>')
        {
            let key = match &rest[1..end] {
                "Esc" => Some(Key::Escape),
                "CR" => Some(Key::Enter),
                "BS" => Some(Key::Backspace),
                name if name.starts_with("C-") => name.chars().nth(2).map(Key::Ctrl),
                _ => None,
            };
            if let Some(key) = key {
                keys.push(key);
                rest = &rest[end + 1..];
                continue;
            }
        }
        keys.push(Key::Char(ch));
        rest = &rest[ch.len_utf8()..];
    }
    keys
}

struct Editor {
    vim: Vim,
    text: Rope,
    clipboard: TestClipboard,
    commands: Vec<Command>,
    message: Option<String>,
}

impl Editor {
    fn new(marked: &str) -> Self {
        let cursor = marked.find('|').expect("cursor marker");
        let text = Rope::from(marked.replacen('|', "", 1).as_str());
        let mut vim = Vim::new(4);
        vim.reset(&text, cursor);
        Self {
            vim,
            text,
            clipboard: TestClipboard::default(),
            commands: Vec::new(),
            message: None,
        }
    }

    fn cursor(&self) -> usize {
        self.vim.selection(&self.text).1
    }

    fn press(&mut self, notation: &str) -> &mut Self {
        for key in keys(notation) {
            if self.vim.mode() == Mode::Insert && key != Key::Escape {
                self.type_key(key);
                continue;
            }
            let outcome = self.vim.key(key, &self.text, &mut self.clipboard);
            for edit in outcome.edits {
                self.text.remove(edit.range.clone());
                self.text.insert(edit.range.start, &edit.text);
            }
            self.commands.extend(outcome.commands);
            if outcome.message.is_some() {
                self.message = outcome.message;
            }
        }
        self
    }

    fn type_key(&mut self, key: Key) {
        let cursor = self.cursor();
        let cursor = match key {
            Key::Char(ch) => {
                self.text.insert(cursor, &ch.to_string());
                cursor + ch.len_utf8()
            }
            Key::Enter => {
                self.text.insert(cursor, "\n");
                cursor + 1
            }
            Key::Backspace => {
                let start = prev(&self.text, cursor);
                self.text.remove(start..cursor);
                start
            }
            _ => cursor,
        };
        self.vim.sync(&self.text, cursor, cursor);
    }

    fn marked(&self) -> String {
        let mut text = self.text.to_string();
        text.insert(self.cursor(), '|');
        text
    }
}

#[track_caller]
fn check(before: &str, notation: &str, after: &str) {
    let mut editor = Editor::new(before);
    editor.press(notation);
    assert_eq!(editor.marked(), after, "{before:?} then {notation:?}");
}

#[test]
fn word_motions() {
    check("|foo bar", "w", "foo |bar");
    check("|foo.bar baz", "w", "foo|.bar baz");
    check("|foo.bar baz", "W", "foo.bar |baz");
    check("|a b c d", "3w", "a b c |d");
    check("foo.bar |baz", "b", "foo.|bar baz");
    check("foo.bar |baz", "B", "|foo.bar baz");
    check("f|oo bar", "e", "fo|o bar");
    check("fo|o bar", "e", "foo ba|r");
    check("foo ba|r", "ge", "fo|o bar");
    check("|foo\n\nbar", "w", "foo\n|\nbar");
    check("|foo\n\nbar", "ww", "foo\n\n|bar");
    check("foo\n\n|bar", "b", "foo\n|\nbar");
    check("foo |bar", "w", "foo ba|r");
}

#[test]
fn line_motions_keep_the_column() {
    check("  |foo bar", "0", "|  foo bar");
    check("|  foo", "^", "  |foo");
    check("f|oo bar", "$", "foo ba|r");
    check("one\n|two\nthree", "gg", "|one\ntwo\nthree");
    check("|one\ntwo\nthree", "G", "one\ntwo\n|three");
    check("|one\ntwo\nthree", "2G", "one\n|two\nthree");
    check("abc|d\nx\nabcdef", "jj", "abcd\nx\nabc|def");
    check("abcdef\nx\nabc|d", "kk", "abc|def\nx\nabcd");
    check("a|bc\nabcdef", "$j", "abc\nabcde|f");
    check("|a\n  b", "<CR>", "a\n  |b");
    check("a\n  |b", "-", "|a\n  b");
    check("a|b", "k", "a|b");
    check("|ab", "h", "|ab");
    check("a|b", "l", "a|b");
}

#[test]
fn find_and_bracket_motions() {
    check("|a,b,c", "f,", "a|,b,c");
    check("|a,b,c", "f,;", "a,b|,c");
    check("|a,b,c", "f,;,", "a|,b,c");
    check("|a,b,c", "2f,", "a,b|,c");
    check("a,b,|c", "F,", "a,b|,c");
    check("|a,b,c", "t,", "|a,b,c");
    check("|a,b,c", "t,;", "a,|b,c");
    check("a,b,|c", "T,", "a,b,|c");
    check("|a,b,c", "fz", "|a,b,c");
    check("|(a [b] c)", "%", "(a [b] c|)");
    check("(a [b] c|)", "%", "|(a [b] c)");
    check("fo|o(bar)", "%", "foo(bar|)");
    check("|a\nb\n\nc\n", "}", "a\nb\n|\nc\n");
    check("|a\nb\n\nc\n", "}}", "a\nb\n\nc\n|");
    check("a\nb\n\n|c\n", "{", "a\nb\n|\nc\n");
}

#[test]
fn delete_change_and_yank() {
    check("foo |bar baz", "dw", "foo |baz");
    check("foo ba|r\nnext", "dw", "foo b|a\nnext");
    check("|foo\nbar", "dw", "|\nbar");
    check("|a b c d e", "d3w", "|d e");
    check("|a b c d e", "2d2w", "|e");
    check("a\n|b\nc\nd", "dd", "a\n|c\nd");
    check("a\n|b\nc\nd", "2dd", "a\n|d");
    check("a\nb\n|c", "dd", "a\n|b");
    check("a\n|b\nc", "dG", "|a");
    check("a\n|b\nc", "dgg", "|c");
    check("|a\nb\nc", "dj", "|c");
    check("  f|oo bar", "D", "  |f");
    check("a|bc", "dfc", "|a");
    check("a|bcd", "dtd", "a|d");
    check("f|oo", "x", "f|o");
    check("fo|o", "x", "f|o");
    check("|abc", "3x", "|");
    check("ab|c", "X", "a|c");
    check("|a\nb\n\nc", "d}", "|\nc");
    check("|foo bar", "cwxyz<Esc>", "xy|z bar");
    check("fo|o bar", "cwX<Esc>", "fo|X bar");
    check("  f|oo\nbar", "ccx<Esc>", "  |x\nbar");
    check("f|oo bar", "Cx<Esc>", "f|x");
    check("f|oo", "sx<Esc>", "f|xo");
    check("|foo bar", "ywP", "foo| foo bar");
    check("|foo bar", "yeP", "fo|ofoo bar");
    check("one\n|two", "yyp", "one\ntwo\n|two");
    check("|one\ntwo", "yyp", "one\n|one\ntwo");
    check("one\n|two", "yyP", "one\n|two\ntwo");
    check("|one\ntwo", "Yp", "one\n|one\ntwo");
    check("|a\nb", "ddp", "b\n|a");
    check("|abc", "xp", "b|ac");
    check("|ab", "yl3p", "aaa|ab");
    check("a\n|b\nc", "yk", "|a\nb\nc");
}

#[test]
fn text_objects() {
    check("foo b|ar baz", "diw", "foo | baz");
    check("foo b|ar baz", "daw", "foo |baz");
    check("foo b|ar", "daw", "fo|o");
    check("foo b|ar baz", "ciwx<Esc>", "foo |x baz");
    check("foo.b|ar baz", "diW", "| baz");
    check("call(a, |b)", "di(", "call(|)");
    check("call(a, |b)", "dib", "call(|)");
    check("call(a, |b)", "da(", "cal|l");
    check("f(|a(b)c)", "di(", "f(|)");
    check("say \"hi |there\" now", "di\"", "say \"|\" now");
    check("say \"hi |there\" now", "da\"", "say | now");
    check("|say 'hi'", "ci'yo<Esc>", "say 'y|o'");
    check("fn x() {\n    |body\n}", "di{", "fn x() {\n|}");
    check("fn x() {\n    |body\n}", "da{", "fn x()| ");
    check("a[1, |2]", "ci]0<Esc>", "a[|0]");
}

#[test]
fn other_edits() {
    check("|foo\n  bar", "J", "foo| bar");
    check("|foo \nbar", "J", "foo |bar");
    check("|a\nb\nc", "3J", "a b| c");
    check("|foo(\n)", "J", "foo(|)");
    check("|abc", "rx", "|xbc");
    check("|abc", "3rx", "xx|x");
    check("|abc", "4rx", "|abc");
    check("a|bc", "r<CR>", "a\n|c");
    check("|abC", "~~~", "AB|c");
    check("|abc", "2~", "AB|c");
    check("|foo\nbar", ">>", "    |foo\nbar");
    check("|foo\nbar", "2>>", "    |foo\n    bar");
    check("|foo\n\nbar", ">2j", "    |foo\n\n    bar");
    check("    |foo", "<<", "|foo");
    check("\t|foo", "<<", "|foo");
    check("  |foo", "<<", "|foo");
    check("|foo bar", "gUiw", "|FOO bar");
    check("|ABC def", "guu", "|abc def");
    check("|foo bar", "g~w", "|FOO bar");
    check("|foo bar", "gUU", "|FOO BAR");
}

#[test]
fn insert_commands() {
    check("|foo", "ix<Esc>", "|xfoo");
    check("|foo", "i<Esc>", "|foo");
    check("|abc", "ax<Esc>", "a|xbc");
    check("|foo", "Ax<Esc>", "foo|x");
    check("  f|oo", "Ix<Esc>", "  |xfoo");
    check("  |foo", "obar<Esc>", "  foo\n  ba|r");
    check("  |foo", "Obar<Esc>", "  ba|r\n  foo");
    check("|x", "3ia<Esc>", "aa|ax");
    check("|x", "ia<CR>b<Esc>", "a\n|bx");
    check("|x", "iab<BS><Esc>", "|ax");
}

#[test]
fn dot_repeats_the_last_change() {
    check("|a b c d", "dw..", "|d");
    check("|abcd", "x..", "|d");
    check("|a b c d e", "dw3.", "|e");
    check("|foo foo foo", "ciwbar<Esc>w.w.", "bar bar ba|r");
    check("|a\nb", "A;<Esc>j.", "a;\nb|;");
    check("|x\ny", "2ia<Esc>j.", "aax\na|ay");
    check("|foo\nbar", ">>j.", "    foo\n    |bar");
    check("|a b c", "dwwyw.", "b| ");
}

#[test]
fn visual_mode() {
    check("|foo bar baz", "vwd", "|ar baz");
    check("a\n|b\nc\nd", "Vjd", "a\n|d");
    check("|foo bar", "veyP", "fo|ofoo bar");
    check("|foo bar", "viwU", "|FOO bar");
    check("|a\nb\nc", "VjJ", "a| b\nc");
    check("|foo\nbar", "v$d", "|bar");
    check("|foo bar", "yiwwviwp", "foo |foo");
    check("|foo bar", "vex", "| bar");
    check("|ab\ncd", "vjc<Esc>", "|d");
    check("|a\nb\nc", "VjcX<Esc>", "|X\nc");
    check("|abc", "vlrx", "|xxc");

    let mut editor = Editor::new("|foo bar");
    editor.press("vl");
    assert_eq!(editor.vim.mode(), Mode::Visual);
    assert_eq!(editor.vim.selection(&editor.text), (0, 2));
    editor.press("o");
    assert_eq!(editor.vim.selection(&editor.text), (2, 0));
    editor.press("<Esc>");
    assert_eq!(editor.vim.mode(), Mode::Normal);
    assert_eq!(editor.cursor(), 0);

    let mut editor = Editor::new("foo |bar");
    editor.press("vh");
    assert_eq!(editor.vim.selection(&editor.text), (5, 3));

    let mut editor = Editor::new("a\n|b\nc");
    editor.press("Vk");
    assert_eq!(editor.vim.mode(), Mode::VisualLine);
    assert_eq!(editor.vim.selection(&editor.text), (3, 0));
    editor.press("V");
    assert_eq!(editor.vim.mode(), Mode::Normal);
}

#[test]
fn registers_follow_the_system_clipboard() {
    let mut editor = Editor::new("|foo bar");
    editor.press("yw");
    assert_eq!(editor.clipboard.0.as_deref(), Some("foo "));

    let mut editor = Editor::new("|ab");
    editor.clipboard.0 = Some("XY".into());
    editor.press("p");
    assert_eq!(editor.marked(), "aX|Yb");
    let mut editor = Editor::new("|ab");
    editor.clipboard.0 = Some("line\r\n".into());
    editor.press("p");
    assert_eq!(editor.marked(), "ab\n|line");

    let mut editor = Editor::new("|foo bar");
    editor.press("\"ayw");
    editor.clipboard.0 = Some("zz".into());
    editor.press("\"aP");
    assert_eq!(editor.marked(), "foo| foo bar");
    editor.press("\"_dd");
    assert_eq!(editor.clipboard.0.as_deref(), Some("zz"));
    assert_eq!(editor.marked(), "|");
    let mut editor = Editor::new("|a b");
    editor.press("\"ayw\"Ayw\"ap");
    assert_eq!(editor.marked(), "aa a|  b");
}

#[test]
fn search() {
    let mut editor = Editor::new("|foo bar foo");
    editor.press("/foo<CR>");
    assert_eq!(editor.marked(), "foo bar |foo");
    editor.press("n");
    assert_eq!(editor.marked(), "|foo bar foo");
    assert_eq!(
        editor.message.as_deref(),
        Some("Search hit BOTTOM, continuing at TOP")
    );
    editor.press("N");
    assert_eq!(editor.marked(), "foo bar |foo");
    editor.press("/zzz<CR>");
    assert_eq!(editor.message.as_deref(), Some("Pattern not found: zzz"));
    assert_eq!(editor.marked(), "foo bar |foo");
    editor.press("?bar<CR>");
    assert_eq!(editor.marked(), "foo |bar foo");

    check("|foo bar foobar foo", "*", "foo bar foobar |foo");
    check("foo bar foobar |foo", "#", "|foo bar foobar foo");
    check("|foo bar foo", "*D", "foo bar| ");
}

#[test]
fn command_line() {
    let mut editor = Editor::new("|a\nb\nc\nd");
    editor.press(":wq");
    assert_eq!(editor.vim.prompt().as_deref(), Some(":wq"));
    editor.press("<CR>");
    assert_eq!(editor.vim.prompt(), None);
    assert_eq!(
        editor.commands,
        [Command::Write, Command::Close { force: false }]
    );

    let commands = |notation: &str| Editor::new("|a").press(notation).commands.clone();
    assert_eq!(commands(":w<CR>"), [Command::Write]);
    assert_eq!(
        commands(":w out.txt<CR>"),
        [Command::WriteAs("out.txt".into())]
    );
    assert_eq!(commands(":q!<CR>"), [Command::Close { force: true }]);
    assert_eq!(commands(":qa<CR>"), [Command::CloseAll { force: false }]);
    assert_eq!(
        commands(":e src/main.rs<CR>"),
        [Command::Open("src/main.rs".into())]
    );
    assert_eq!(
        commands(":bn<CR>:bp<CR>"),
        [Command::NextBuffer, Command::PreviousBuffer]
    );
    assert_eq!(commands(":enew<CR>"), [Command::NewBuffer]);
    assert_eq!(
        commands("ZZ"),
        [Command::Write, Command::Close { force: false }]
    );
    assert_eq!(commands("ZQ"), [Command::Close { force: true }]);
    assert_eq!(
        commands("gtgT"),
        [Command::NextBuffer, Command::PreviousBuffer]
    );
    assert_eq!(
        commands("3u"),
        [Command::Undo, Command::Undo, Command::Undo]
    );
    assert_eq!(commands("<C-r>"), [Command::Redo]);
    assert!(commands(":w<BS><BS>x").is_empty());

    check("|a\nb\nc\nd", ":3<CR>", "a\nb\n|c\nd");
    check("|a\nb\nc\nd", ":$<CR>", "a\nb\nc\n|d");
    check("|a", ":q<Esc>x", "|");
    let mut editor = Editor::new("|a");
    editor.press(":bogus<CR>");
    assert_eq!(
        editor.message.as_deref(),
        Some("Not an editor command: bogus")
    );
}

#[test]
fn pending_keys_counts_and_cancelling() {
    let mut editor = Editor::new("|a b c");
    editor.press("2d");
    assert_eq!(editor.vim.pending(), "2d");
    editor.press("<Esc>");
    assert_eq!(editor.vim.pending(), "");
    assert_eq!(editor.marked(), "|a b c");
    editor.press("dzx");
    assert_eq!(editor.marked(), "| b c");
}

#[test]
fn sync_adopts_outside_cursor_and_selection_changes() {
    let text = Rope::from("foo bar");
    let mut vim = Vim::new(4);
    vim.sync(&text, 2, 5);
    assert_eq!(vim.mode(), Mode::Visual);
    assert_eq!(vim.selection(&text), (2, 5));
    vim.sync(&text, 5, 2);
    assert_eq!(vim.selection(&text), (5, 2));
    vim.sync(&text, 7, 7);
    assert_eq!(vim.mode(), Mode::Normal);
    assert_eq!(vim.selection(&text), (6, 6));
}
