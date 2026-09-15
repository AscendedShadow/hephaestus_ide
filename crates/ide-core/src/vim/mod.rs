mod command;
mod text;

#[cfg(test)]
mod tests;

use std::{collections::HashMap, ops::Range};

use ropey::Rope;

pub use command::Key;
use command::{Action, Cmd, Find, InsertAt, Motion, Object, Operator, PromptKind, Stop, Target};
use text::*;

const HALF_PAGE: usize = 15;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualLine => "VISUAL LINE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Redo,
    Write,
    WriteAs(String),
    WriteAll,
    Close { force: bool },
    CloseAll { force: bool },
    NextBuffer,
    PreviousBuffer,
    NewBuffer,
    Open(String),
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub edits: Vec<Edit>,
    pub commands: Vec<Command>,
    pub message: Option<String>,
}

pub trait Clipboard {
    fn read(&mut self) -> Option<String>;
    fn write(&mut self, text: String);
}

#[derive(Clone, Debug)]
struct Register {
    text: String,
    linewise: bool,
}

#[derive(Clone)]
struct Change {
    cmd: Cmd,
    inserted: Option<String>,
}

struct Insertion {
    cmd: Option<Cmd>,
    start: usize,
    len: usize,
    count: usize,
}

struct Prompt {
    kind: PromptKind,
    text: String,
}

struct Search {
    pattern: String,
    forward: bool,
    whole_word: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Exclusive,
    Inclusive,
    Linewise,
}

enum Span {
    Chars(Range<usize>),
    Lines(usize, usize),
}

struct Ctx<'a> {
    text: Rope,
    out: Outcome,
    clipboard: &'a mut dyn Clipboard,
}

impl Ctx<'_> {
    fn replace(&mut self, range: Range<usize>, text: &str) {
        if range.is_empty() && text.is_empty() {
            return;
        }
        self.text.remove(range.clone());
        self.text.insert(range.start, text);
        self.out.edits.push(Edit {
            range,
            text: text.to_owned(),
        });
    }
}

pub struct Vim {
    mode: Mode,
    cursor: usize,
    anchor: usize,
    column: Option<usize>,
    keys: Vec<Key>,
    prompt: Option<Prompt>,
    registers: HashMap<char, Register>,
    last_find: Option<Find>,
    search: Option<Search>,
    last_change: Option<Change>,
    insertion: Option<Insertion>,
    shift_width: usize,
}

impl Vim {
    pub fn new(shift_width: usize) -> Self {
        Self {
            mode: Mode::Normal,
            cursor: 0,
            anchor: 0,
            column: None,
            keys: Vec::new(),
            prompt: None,
            registers: HashMap::new(),
            last_find: None,
            search: None,
            last_change: None,
            insertion: None,
            shift_width,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn is_visual(&self) -> bool {
        matches!(self.mode, Mode::Visual | Mode::VisualLine)
    }

    pub fn pending(&self) -> String {
        self.keys
            .iter()
            .filter_map(|key| match key {
                Key::Char(ch) => Some(ch.to_string()),
                Key::Ctrl(ch) => Some(format!("^{}", ch.to_ascii_uppercase())),
                _ => None,
            })
            .collect()
    }

    pub fn prompt(&self) -> Option<String> {
        self.prompt.as_ref().map(|prompt| {
            let prefix = match prompt.kind {
                PromptKind::Command => ':',
                PromptKind::Search { forward: true } => '/',
                PromptKind::Search { forward: false } => '?',
            };
            format!("{prefix}{}", prompt.text)
        })
    }

    pub fn selection(&self, text: &Rope) -> (usize, usize) {
        let (anchor, cursor) = (self.anchor.min(text.len()), self.cursor.min(text.len()));
        match self.mode {
            Mode::Visual if cursor >= anchor => (anchor, next(text, cursor)),
            Mode::Visual => (next(text, anchor), cursor),
            Mode::VisualLine => {
                let (from, to) = (line_of(text, anchor), line_of(text, cursor));
                if to >= from {
                    (line_start(text, from), line_end(text, to))
                } else {
                    (line_end(text, from), line_start(text, to))
                }
            }
            Mode::Normal | Mode::Insert => (cursor, cursor),
        }
    }

    pub fn sync(&mut self, text: &Rope, anchor: usize, head: usize) {
        let (anchor, head) = (anchor.min(text.len()), head.min(text.len()));
        self.keys.clear();
        self.column = None;
        if self.mode == Mode::Insert {
            self.cursor = head;
        } else if anchor != head {
            if !self.is_visual() {
                self.mode = Mode::Visual;
            }
            if head > anchor {
                (self.anchor, self.cursor) = (anchor, prev(text, head));
            } else {
                (self.anchor, self.cursor) = (prev(text, anchor), head);
            }
        } else {
            self.mode = Mode::Normal;
            self.cursor = clamp_normal(text, head);
        }
    }

    pub fn reset(&mut self, text: &Rope, cursor: usize) {
        self.mode = Mode::Normal;
        self.keys.clear();
        self.prompt = None;
        self.insertion = None;
        self.column = None;
        self.cursor = clamp_normal(text, cursor.min(text.len()));
    }

    pub fn key(&mut self, key: Key, text: &Rope, clipboard: &mut dyn Clipboard) -> Outcome {
        let mut cx = Ctx {
            text: text.clone(),
            out: Outcome::default(),
            clipboard,
        };
        self.cursor = self.cursor.min(text.len());
        if self.prompt.is_some() {
            self.prompt_key(key, &mut cx);
        } else if self.mode == Mode::Insert {
            if matches!(key, Key::Escape | Key::Ctrl('[')) {
                self.finish_insert(&mut cx);
            }
        } else {
            self.keys.push(key);
            match command::parse(&self.keys, self.is_visual()) {
                Ok(cmd) => {
                    self.keys.clear();
                    self.run(cmd, &mut cx);
                }
                Err(Stop::Incomplete) => {}
                Err(Stop::Invalid) => self.keys.clear(),
            }
        }
        match self.mode {
            Mode::Normal => self.cursor = clamp_normal(&cx.text, self.cursor),
            _ => {
                self.cursor = self.cursor.min(cx.text.len());
                self.anchor = self.anchor.min(cx.text.len());
            }
        }
        cx.out
    }

    fn run(&mut self, cmd: Cmd, cx: &mut Ctx) {
        let vertical = matches!(
            cmd.action,
            Action::Move(Motion::Up | Motion::Down | Motion::HalfPageDown | Motion::HalfPageUp)
        );
        if !vertical {
            self.column = None;
        }
        let visual = self.is_visual();
        let done = self.execute(cmd, cx);
        if done && !visual && cmd.is_change() && self.mode != Mode::Insert {
            self.last_change = Some(Change {
                cmd,
                inserted: None,
            });
        }
    }

    fn execute(&mut self, cmd: Cmd, cx: &mut Ctx) -> bool {
        let count = cmd.count_or_one();
        match cmd.action {
            Action::Move(motion) => self.move_cursor(motion, cmd.count, cx),
            Action::Operate(operator, target) => self.operate(operator, target, cmd, cx),
            Action::Insert(at) => {
                self.insert(at, cmd, cx);
                true
            }
            Action::Paste { before } => self.paste(before, cmd, cx),
            Action::Join => self.join(count, cx),
            Action::Replace(ch) => self.replace_chars(ch, count, cx),
            Action::ToggleCaseUnderCursor => self.toggle_case_under_cursor(count, cx),
            Action::Undo | Action::Redo => {
                let command = if cmd.action == Action::Undo {
                    Command::Undo
                } else {
                    Command::Redo
                };
                cx.out.commands.extend(std::iter::repeat_n(command, count));
                true
            }
            Action::Repeat => {
                self.repeat(cmd.count, cx);
                true
            }
            Action::Visual { linewise } => {
                let mode = if linewise {
                    Mode::VisualLine
                } else {
                    Mode::Visual
                };
                if self.mode == mode {
                    self.mode = Mode::Normal;
                } else {
                    if !self.is_visual() {
                        self.anchor = self.cursor;
                    }
                    self.mode = mode;
                }
                true
            }
            Action::SwapSelectionEnds => {
                std::mem::swap(&mut self.anchor, &mut self.cursor);
                true
            }
            Action::SelectObject { object, around } => {
                let Some(range) = object_range(&cx.text, self.cursor, object, around)
                    .filter(|range| !range.is_empty())
                else {
                    return false;
                };
                self.anchor = range.start;
                self.cursor = prev(&cx.text, range.end);
                self.mode = Mode::Visual;
                true
            }
            Action::Prompt(kind) => {
                self.prompt = Some(Prompt {
                    kind,
                    text: String::new(),
                });
                true
            }
            Action::NextBuffer => {
                cx.out.commands.push(Command::NextBuffer);
                true
            }
            Action::PreviousBuffer => {
                cx.out.commands.push(Command::PreviousBuffer);
                true
            }
            Action::WriteClose => {
                cx.out
                    .commands
                    .extend([Command::Write, Command::Close { force: false }]);
                true
            }
            Action::ForceClose => {
                cx.out.commands.push(Command::Close { force: true });
                true
            }
            Action::Escape => {
                self.mode = Mode::Normal;
                true
            }
        }
    }

    fn move_cursor(&mut self, motion: Motion, count: Option<usize>, cx: &mut Ctx) -> bool {
        let text = cx.text.clone();
        let Some((pos, _)) = self.motion_target(motion, count, &text, false, &mut cx.out.message)
        else {
            return false;
        };
        self.cursor = if motion == Motion::LineEnd && self.is_visual() {
            pos
        } else {
            clamp_normal(&text, pos)
        };
        if motion == Motion::LineEnd {
            self.column = Some(usize::MAX);
        }
        true
    }

    fn motion_target(
        &mut self,
        motion: Motion,
        count: Option<usize>,
        text: &Rope,
        operator: bool,
        message: &mut Option<String>,
    ) -> Option<(usize, Kind)> {
        let from = self.cursor;
        let n = count.unwrap_or(1).max(1);
        let line = line_of(text, from);
        let last = last_line(text);
        let repeat = |step: &dyn Fn(usize) -> usize| (0..n).fold(from, |pos, _| step(pos));
        Some(match motion {
            Motion::Left => {
                let start = line_start(text, line);
                if from <= start {
                    return None;
                }
                (repeat(&|pos| prev(text, pos).max(start)), Kind::Exclusive)
            }
            Motion::Right => {
                let end = line_end(text, line);
                let limit = if operator {
                    end
                } else {
                    clamp_normal(text, end)
                };
                if from >= limit {
                    return None;
                }
                (repeat(&|pos| next(text, pos).min(limit)), Kind::Exclusive)
            }
            Motion::Up | Motion::Down | Motion::HalfPageUp | Motion::HalfPageDown => {
                let lines = match motion {
                    Motion::HalfPageUp | Motion::HalfPageDown => n * HALF_PAGE,
                    _ => n,
                };
                let target = if matches!(motion, Motion::Up | Motion::HalfPageUp) {
                    line.checked_sub(1)?;
                    line.saturating_sub(lines)
                } else {
                    if line >= last {
                        return None;
                    }
                    (line + lines).min(last)
                };
                let column = *self.column.get_or_insert_with(|| column(text, from));
                (at_column(text, target, column), Kind::Linewise)
            }
            Motion::LineStart => (line_start(text, line), Kind::Exclusive),
            Motion::FirstNonBlank => (first_non_blank(text, line), Kind::Exclusive),
            Motion::LineEnd => {
                let end = line_end(text, (line + n - 1).min(last));
                if operator {
                    (end, Kind::Exclusive)
                } else {
                    (end, Kind::Inclusive)
                }
            }
            Motion::NextWordStart { big } => {
                let mut pos = from;
                for step in 0..n {
                    let mut next_pos = next_word_start(text, pos, big);
                    if operator && step == n - 1 && line_of(text, next_pos) > line_of(text, pos) {
                        next_pos = line_end(text, line_of(text, pos)).max(pos);
                    }
                    pos = next_pos;
                }
                if pos == from {
                    return None;
                }
                (pos, Kind::Exclusive)
            }
            Motion::PrevWordStart { big } => (
                repeat(&|pos| prev_word_start(text, pos, big)),
                Kind::Exclusive,
            ),
            Motion::WordEnd { big } => (repeat(&|pos| word_end(text, pos, big)), Kind::Inclusive),
            Motion::PrevWordEnd { big } => (
                repeat(&|pos| prev_word_end(text, pos, big)),
                Kind::Inclusive,
            ),
            Motion::FirstLine | Motion::LastLine => {
                let default = if motion == Motion::FirstLine { 0 } else { last };
                let target = count
                    .map_or(default, |count| count.saturating_sub(1))
                    .min(last);
                (first_non_blank(text, target), Kind::Linewise)
            }
            Motion::Find(find) => {
                self.last_find = Some(find);
                find_char(text, from, find, n, false)?
            }
            Motion::RepeatFind { reverse } => {
                let mut find = self.last_find?;
                find.forward ^= reverse;
                find_char(text, from, find, n, true)?
            }
            Motion::MatchingBracket => (matching_bracket(text, from)?, Kind::Inclusive),
            Motion::NextParagraph => (repeat(&|pos| next_paragraph(text, pos)), Kind::Exclusive),
            Motion::PrevParagraph => (repeat(&|pos| prev_paragraph(text, pos)), Kind::Exclusive),
            Motion::NextLine => {
                if line >= last {
                    return None;
                }
                (first_non_blank(text, (line + n).min(last)), Kind::Linewise)
            }
            Motion::PrevLine => {
                line.checked_sub(1)?;
                (
                    first_non_blank(text, line.saturating_sub(n)),
                    Kind::Linewise,
                )
            }
            Motion::CurrentLine => (
                first_non_blank(text, (line + n - 1).min(last)),
                Kind::Linewise,
            ),
            Motion::SearchNext { reverse } => {
                let forward = self.search.as_ref()?.forward != reverse;
                let mut pos = from;
                for _ in 0..n {
                    pos = self.find_match(text, pos, forward, message)?;
                }
                (pos, Kind::Exclusive)
            }
            Motion::SearchWord { forward } => {
                let word = keyword_at(text, from)?;
                self.search = Some(Search {
                    pattern: text.slice(word.clone()).to_string(),
                    forward,
                    whole_word: true,
                });
                let mut pos = word.start;
                for _ in 0..n {
                    pos = self.find_match(text, pos, forward, message)?;
                }
                (pos, Kind::Exclusive)
            }
        })
    }

    fn find_match(
        &self,
        text: &Rope,
        from: usize,
        forward: bool,
        message: &mut Option<String>,
    ) -> Option<usize> {
        let search = self.search.as_ref()?;
        let pattern = search.pattern.as_str();
        let haystack = text.to_string();
        let whole = |start: usize| {
            !search.whole_word
                || (!haystack[..start]
                    .chars()
                    .next_back()
                    .is_some_and(is_word_char)
                    && !haystack[start + pattern.len()..]
                        .chars()
                        .next()
                        .is_some_and(is_word_char))
        };
        let (found, wrapped) = if forward {
            let after = next(text, from);
            let ahead = haystack[after..]
                .match_indices(pattern)
                .map(|(ix, _)| after + ix)
                .find(|&ix| whole(ix));
            match ahead {
                Some(ix) => (Some(ix), false),
                None => (
                    haystack
                        .match_indices(pattern)
                        .map(|(ix, _)| ix)
                        .find(|&ix| ix <= from && whole(ix)),
                    true,
                ),
            }
        } else {
            let behind = haystack[..from]
                .rmatch_indices(pattern)
                .map(|(ix, _)| ix)
                .find(|&ix| whole(ix));
            match behind {
                Some(ix) => (Some(ix), false),
                None => (
                    haystack
                        .rmatch_indices(pattern)
                        .map(|(ix, _)| ix)
                        .find(|&ix| ix >= from && whole(ix)),
                    true,
                ),
            }
        };
        *message = match (found, wrapped) {
            (None, _) => Some(format!("Pattern not found: {pattern}")),
            (Some(_), true) if forward => Some("Search hit BOTTOM, continuing at TOP".into()),
            (Some(_), true) => Some("Search hit TOP, continuing at BOTTOM".into()),
            (Some(_), false) => None,
        };
        found
    }

    fn operate(&mut self, operator: Operator, target: Target, cmd: Cmd, cx: &mut Ctx) -> bool {
        let text = cx.text.clone();
        let from = self.cursor;
        let count = cmd.count_or_one();
        let span = match target {
            Target::Motion(motion) => {
                let target = if operator == Operator::Change
                    && matches!(motion, Motion::NextWordStart { .. })
                    && char_at(&text, from).is_some_and(|ch| !ch.is_whitespace())
                {
                    let big = motion == Motion::NextWordStart { big: true };
                    let mut end = end_of_word_at(&text, from, big);
                    for _ in 1..count {
                        end = word_end(&text, end, big);
                    }
                    Some((end, Kind::Inclusive))
                } else {
                    self.motion_target(motion, cmd.count, &text, true, &mut cx.out.message)
                };
                let Some((pos, kind)) = target else {
                    return false;
                };
                let (start, end) = (from.min(pos), from.max(pos));
                match kind {
                    Kind::Linewise => Span::Lines(line_of(&text, start), line_of(&text, end)),
                    Kind::Inclusive => Span::Chars(start..next(&text, end)),
                    Kind::Exclusive => exclusive_span(&text, start, end),
                }
            }
            Target::Object { object, around } => match object_range(&text, from, object, around) {
                Some(range) => Span::Chars(range),
                None => return false,
            },
            Target::Lines => {
                let first = line_of(&text, from);
                Span::Lines(first, (first + count - 1).min(last_line(&text)))
            }
            Target::Selection => self.selection_span(&text),
            Target::SelectionLines => match self.selection_span(&text) {
                Span::Chars(range) => Span::Lines(
                    line_of(&text, range.start),
                    line_of(&text, prev(&text, range.end)),
                ),
                lines => lines,
            },
        };
        let visual = self.is_visual();
        if visual {
            self.mode = Mode::Normal;
        }
        let register = cmd.register;
        match operator {
            Operator::Yank => {
                let (yanked, linewise) = span_text(&text, &span);
                self.store(register, yanked, linewise, cx);
                self.cursor = match span {
                    Span::Chars(range) => range.start,
                    Span::Lines(first, _) if first < line_of(&text, from) || visual => {
                        at_column(&text, first, column(&text, from))
                    }
                    Span::Lines(..) => from,
                };
            }
            Operator::Delete => {
                let (deleted, linewise) = span_text(&text, &span);
                self.store(register, deleted, linewise, cx);
                match span {
                    Span::Chars(range) => {
                        cx.replace(range.clone(), "");
                        self.cursor = range.start;
                    }
                    Span::Lines(first, last) => {
                        let mut range = lines_range(&text, first, last);
                        if last == last_line(&text) && first > 0 {
                            range.start = line_end(&text, first - 1);
                        }
                        cx.replace(range, "");
                        self.cursor = first_non_blank(&cx.text, first.min(last_line(&cx.text)));
                    }
                }
            }
            Operator::Change => {
                let (changed, linewise) = span_text(&text, &span);
                self.store(register, changed, linewise, cx);
                match span {
                    Span::Chars(range) => {
                        cx.replace(range.clone(), "");
                        self.cursor = range.start;
                    }
                    Span::Lines(first, last) => {
                        let indent = indentation(&text, first);
                        let start = line_start(&text, first);
                        cx.replace(start..line_end(&text, last), &indent);
                        self.cursor = start + indent.len();
                    }
                }
                self.enter_insert((!visual).then_some(cmd), 1, cx);
            }
            Operator::Indent | Operator::Outdent => {
                let (first, last) = match span {
                    Span::Lines(first, last) => (first, last),
                    Span::Chars(range) => (
                        line_of(&text, range.start),
                        line_of(&text, prev(&text, range.end).max(range.start)),
                    ),
                };
                let range = line_start(&text, first)..line_end(&text, last);
                let shifted = text
                    .slice(range.clone())
                    .to_string()
                    .split('\n')
                    .map(|line| self.shift(line, operator == Operator::Indent))
                    .collect::<Vec<_>>()
                    .join("\n");
                cx.replace(range, &shifted);
                self.cursor = first_non_blank(&cx.text, first);
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                let range = match span {
                    Span::Chars(range) => range,
                    Span::Lines(first, last) => lines_range(&text, first, last),
                };
                let original = text.slice(range.clone()).to_string();
                let converted: String = match operator {
                    Operator::Lowercase => original.to_lowercase(),
                    Operator::Uppercase => original.to_uppercase(),
                    _ => toggle_case(&original),
                };
                cx.replace(range.clone(), &converted);
                self.cursor = range.start;
            }
        }
        true
    }

    fn selection_span(&self, text: &Rope) -> Span {
        let (start, end) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
        if self.mode == Mode::VisualLine {
            Span::Lines(line_of(text, start), line_of(text, end))
        } else {
            Span::Chars(start..next(text, end))
        }
    }

    fn shift(&self, line: &str, indent: bool) -> String {
        if line.is_empty() {
            return String::new();
        }
        if indent {
            return format!("{}{line}", " ".repeat(self.shift_width));
        }
        if let Some(rest) = line.strip_prefix('\t') {
            return rest.to_owned();
        }
        let spaces = line
            .bytes()
            .take(self.shift_width)
            .take_while(|&byte| byte == b' ')
            .count();
        line[spaces..].to_owned()
    }

    fn insert(&mut self, at: InsertAt, cmd: Cmd, cx: &mut Ctx) {
        let text = cx.text.clone();
        let cursor = self.cursor;
        let line = line_of(&text, cursor);
        let visual = self.is_visual();
        let selection = self.selection_span(&text);
        self.cursor = match at {
            InsertAt::Cursor => cursor,
            InsertAt::AfterCursor => match char_at(&text, cursor) {
                Some(ch) if ch != '\n' => next(&text, cursor),
                _ => cursor,
            },
            InsertAt::LineStart => first_non_blank(&text, line),
            InsertAt::LineEnd => line_end(&text, line),
            InsertAt::LineBelow => {
                let indent = indentation(&text, line);
                let end = line_end(&text, line);
                cx.replace(end..end, &format!("\n{indent}"));
                end + 1 + indent.len()
            }
            InsertAt::LineAbove => {
                let indent = indentation(&text, line);
                let start = line_start(&text, line);
                cx.replace(start..start, &format!("{indent}\n"));
                start + indent.len()
            }
            InsertAt::SelectionStart => match selection {
                Span::Chars(range) => range.start,
                Span::Lines(first, _) => line_start(&text, first),
            },
            InsertAt::SelectionEnd => match selection {
                Span::Chars(range) => range.end,
                Span::Lines(_, last) => line_end(&text, last),
            },
        };
        let count = match at {
            InsertAt::Cursor | InsertAt::AfterCursor | InsertAt::LineStart | InsertAt::LineEnd => {
                cmd.count_or_one()
            }
            _ => 1,
        };
        self.enter_insert((!visual).then_some(cmd), count, cx);
    }

    fn enter_insert(&mut self, cmd: Option<Cmd>, count: usize, cx: &Ctx) {
        self.mode = Mode::Insert;
        self.insertion = Some(Insertion {
            cmd,
            start: self.cursor,
            len: cx.text.len(),
            count,
        });
    }

    fn finish_insert(&mut self, cx: &mut Ctx) {
        if let Some(insertion) = self.insertion.take() {
            let text = cx.text.clone();
            let cursor = self.cursor;
            let inserted = (cursor >= insertion.start
                && text.len() >= insertion.len
                && text.len() - insertion.len == cursor - insertion.start)
                .then(|| text.slice(insertion.start..cursor).to_string());
            if let Some(inserted) = &inserted
                && insertion.count > 1
                && !inserted.is_empty()
            {
                let more = inserted.repeat(insertion.count - 1);
                cx.replace(cursor..cursor, &more);
                self.cursor += more.len();
            }
            if let (Some(cmd), Some(inserted)) = (insertion.cmd, inserted) {
                self.last_change = Some(Change {
                    cmd,
                    inserted: Some(inserted),
                });
            }
        }
        self.mode = Mode::Normal;
        if !is_line_start(&cx.text, self.cursor) {
            self.cursor = prev(&cx.text, self.cursor);
        }
    }

    fn repeat(&mut self, count: Option<usize>, cx: &mut Ctx) {
        let Some(change) = self.last_change.clone() else {
            return;
        };
        let mut cmd = change.cmd;
        if count.is_some() {
            cmd.count = count;
        }
        self.execute(cmd, cx);
        if self.mode == Mode::Insert {
            if let Some(inserted) = &change.inserted {
                let at = self.cursor;
                cx.replace(at..at, inserted);
                self.cursor = at + inserted.len();
            }
            self.finish_insert(cx);
        }
    }

    fn paste(&mut self, before: bool, cmd: Cmd, cx: &mut Ctx) -> bool {
        let Some(register) = self.load(cmd.register, cx).filter(|r| !r.text.is_empty()) else {
            return false;
        };
        let text = cx.text.clone();
        let count = cmd.count_or_one();
        if self.is_visual() {
            let span = self.selection_span(&text);
            self.mode = Mode::Normal;
            match span {
                Span::Chars(range) => {
                    let body = if register.linewise {
                        format!("\n{}", register.text)
                    } else {
                        register.text.repeat(count)
                    };
                    cx.replace(range.clone(), &body);
                    self.cursor = range.start;
                }
                Span::Lines(first, last) => {
                    let range = lines_range(&text, first, last);
                    let mut body = register.text.clone();
                    if !register.linewise {
                        body.push('\n');
                    }
                    if range.end > range.start && text.byte(range.end - 1) != b'\n' {
                        body.pop();
                    }
                    cx.replace(range, &body);
                    self.cursor = first_non_blank(&cx.text, first);
                }
            }
            return true;
        }
        let cursor = self.cursor;
        let line = line_of(&text, cursor);
        let body = register.text.repeat(count);
        if register.linewise {
            let first = if before { line } else { line + 1 };
            if before {
                let at = line_start(&text, line);
                cx.replace(at..at, &body);
            } else if line < last_line(&text) {
                let at = line_start(&text, line + 1);
                cx.replace(at..at, &body);
            } else {
                let at = text.len();
                let body = body.strip_suffix('\n').unwrap_or(&body);
                cx.replace(at..at, &format!("\n{body}"));
            }
            self.cursor = first_non_blank(&cx.text, first);
        } else {
            let at = match char_at(&text, cursor) {
                Some(ch) if !before && ch != '\n' => next(&text, cursor),
                _ => cursor,
            };
            cx.replace(at..at, &body);
            self.cursor = if body.contains('\n') {
                at
            } else {
                prev(&cx.text, at + body.len())
            };
        }
        true
    }

    fn join(&mut self, count: usize, cx: &mut Ctx) -> bool {
        let (first, joins) = match self.selection_span(&cx.text) {
            Span::Lines(first, last) if self.is_visual() => (first, (last - first).max(1)),
            Span::Chars(range) if self.is_visual() => {
                let (first, last) = (
                    line_of(&cx.text, range.start),
                    line_of(&cx.text, prev(&cx.text, range.end)),
                );
                (first, (last - first).max(1))
            }
            _ => (line_of(&cx.text, self.cursor), count.max(2) - 1),
        };
        self.mode = Mode::Normal;
        let mut joined = false;
        for _ in 0..joins {
            let text = cx.text.clone();
            if first >= last_line(&text) {
                break;
            }
            let end = line_end(&text, first);
            let next_line = line_end(&text, first + 1);
            let mut rest = end + 1;
            while rest < next_line && matches!(char_at(&text, rest), Some(' ' | '\t')) {
                rest += 1;
            }
            let bare = end == line_start(&text, first)
                || matches!(char_at(&text, prev(&text, end)), Some(' ' | '\t'));
            let separator = if bare || matches!(char_at(&text, rest), None | Some('\n' | ')')) {
                ""
            } else {
                " "
            };
            cx.replace(end..rest, separator);
            self.cursor = end;
            joined = true;
        }
        joined
    }

    fn replace_chars(&mut self, ch: char, count: usize, cx: &mut Ctx) -> bool {
        let text = cx.text.clone();
        if self.is_visual() {
            let range = match self.selection_span(&text) {
                Span::Chars(range) => range,
                Span::Lines(first, last) => lines_range(&text, first, last),
            };
            let replaced: String = text
                .slice(range.clone())
                .chars()
                .map(|original| if original == '\n' { original } else { ch })
                .collect();
            cx.replace(range.clone(), &replaced);
            self.cursor = range.start;
            self.mode = Mode::Normal;
            return true;
        }
        let end_of_line = line_end(&text, line_of(&text, self.cursor));
        let mut end = self.cursor;
        for _ in 0..count {
            if end >= end_of_line {
                return false;
            }
            end = next(&text, end);
        }
        let start = self.cursor;
        if ch == '\n' {
            cx.replace(start..end, "\n");
            self.cursor = start + 1;
        } else {
            cx.replace(start..end, &ch.to_string().repeat(count));
            self.cursor = start + ch.len_utf8() * (count - 1);
        }
        true
    }

    fn toggle_case_under_cursor(&mut self, count: usize, cx: &mut Ctx) -> bool {
        let text = cx.text.clone();
        let end_of_line = line_end(&text, line_of(&text, self.cursor));
        let start = self.cursor;
        let mut end = start;
        for _ in 0..count {
            if end >= end_of_line {
                break;
            }
            end = next(&text, end);
        }
        if end == start {
            return false;
        }
        let toggled = toggle_case(&text.slice(start..end).to_string());
        cx.replace(start..end, &toggled);
        self.cursor = start + toggled.len();
        true
    }

    fn store(&mut self, register: Option<char>, text: String, linewise: bool, cx: &mut Ctx) {
        let name = register.unwrap_or('"');
        if name == '_' {
            return;
        }
        let stored = if name.is_ascii_uppercase() {
            let entry = self
                .registers
                .entry(name.to_ascii_lowercase())
                .or_insert(Register {
                    text: String::new(),
                    linewise,
                });
            entry.text.push_str(&text);
            entry.linewise |= linewise;
            entry.clone()
        } else {
            let stored = Register { text, linewise };
            if name.is_ascii_lowercase() {
                self.registers.insert(name, stored.clone());
            }
            stored
        };
        cx.clipboard.write(stored.text.clone());
        self.registers.insert('"', stored);
    }

    fn load(&mut self, register: Option<char>, cx: &mut Ctx) -> Option<Register> {
        let mut clipboard = || {
            cx.clipboard.read().map(|text| {
                let text = text.replace("\r\n", "\n");
                Register {
                    linewise: text.ends_with('\n'),
                    text,
                }
            })
        };
        match register.unwrap_or('"') {
            '_' => None,
            '+' | '*' => clipboard(),
            name if name.is_ascii_alphabetic() => {
                self.registers.get(&name.to_ascii_lowercase()).cloned()
            }
            _ => {
                let unnamed = self.registers.get(&'"');
                match clipboard() {
                    Some(copied) if unnamed.is_none_or(|unnamed| unnamed.text != copied.text) => {
                        Some(copied)
                    }
                    _ => unnamed.cloned(),
                }
            }
        }
    }

    fn prompt_key(&mut self, key: Key, cx: &mut Ctx) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key {
            Key::Char(ch) => prompt.text.push(ch),
            Key::Backspace => {
                if prompt.text.pop().is_none() {
                    self.prompt = None;
                }
            }
            Key::Escape | Key::Ctrl('[') => self.prompt = None,
            Key::Enter => {
                let Some(prompt) = self.prompt.take() else {
                    return;
                };
                match prompt.kind {
                    PromptKind::Command => self.ex(&prompt.text, cx),
                    PromptKind::Search { forward } => self.search_for(prompt.text, forward, cx),
                }
            }
            _ => {}
        }
    }

    fn search_for(&mut self, pattern: String, forward: bool, cx: &mut Ctx) {
        if !pattern.is_empty() {
            self.search = Some(Search {
                pattern,
                forward,
                whole_word: false,
            });
        } else if let Some(search) = &mut self.search {
            search.forward = forward;
        } else {
            return;
        }
        let text = cx.text.clone();
        if let Some(pos) = self.find_match(&text, self.cursor, forward, &mut cx.out.message) {
            self.cursor = pos;
        }
    }

    fn ex(&mut self, line: &str, cx: &mut Ctx) {
        let line = line.trim();
        let (name, argument) = match line.split_once(char::is_whitespace) {
            Some((name, argument)) => (name, argument.trim()),
            None => (line, ""),
        };
        let commands = &mut cx.out.commands;
        match name {
            "" | "noh" | "nohlsearch" => {}
            "w" | "write" if argument.is_empty() => commands.push(Command::Write),
            "w" | "write" => commands.push(Command::WriteAs(argument.to_owned())),
            "wa" | "wall" => commands.push(Command::WriteAll),
            "q" | "quit" | "close" | "bd" | "bdelete" => {
                commands.push(Command::Close { force: false })
            }
            "q!" | "quit!" | "bd!" | "bdelete!" => commands.push(Command::Close { force: true }),
            "wq" | "x" | "xit" | "exit" => {
                commands.extend([Command::Write, Command::Close { force: false }])
            }
            "qa" | "qall" | "quitall" => commands.push(Command::CloseAll { force: false }),
            "qa!" | "qall!" | "quitall!" => commands.push(Command::CloseAll { force: true }),
            "wqa" | "wqall" | "xa" | "xall" => {
                commands.extend([Command::WriteAll, Command::CloseAll { force: false }])
            }
            "bn" | "bnext" | "tabn" | "tabnext" => commands.push(Command::NextBuffer),
            "bp" | "bprevious" | "bprev" | "bN" | "bNext" | "tabp" | "tabprevious" | "tabN"
            | "tabNext" => commands.push(Command::PreviousBuffer),
            "e" | "edit" | "tabe" | "tabedit" | "tabnew" | "enew" | "new"
                if !argument.is_empty() =>
            {
                commands.push(Command::Open(argument.to_owned()))
            }
            "enew" | "tabnew" | "new" => commands.push(Command::NewBuffer),
            "$" => {
                let text = &cx.text;
                self.cursor = first_non_blank(text, last_line(text));
            }
            number if number.parse::<usize>().is_ok() => {
                let text = &cx.text;
                let line = number.parse::<usize>().unwrap_or(1).saturating_sub(1);
                self.cursor = first_non_blank(text, line.min(last_line(text)));
            }
            _ => cx.out.message = Some(format!("Not an editor command: {line}")),
        }
    }
}

fn find_char(
    text: &Rope,
    from: usize,
    find: Find,
    count: usize,
    repeat: bool,
) -> Option<(usize, Kind)> {
    let start = match (repeat && find.till, find.forward) {
        (true, true) => next(text, from),
        (true, false) => prev(text, from),
        (false, _) => from,
    };
    let found = find_in_line(text, start, find.target, find.forward, count)?;
    let pos = match (find.till, find.forward) {
        (true, true) => prev(text, found),
        (true, false) => next(text, found),
        (false, _) => found,
    };
    Some((
        pos,
        if find.forward {
            Kind::Inclusive
        } else {
            Kind::Exclusive
        },
    ))
}

fn exclusive_span(text: &Rope, start: usize, end: usize) -> Span {
    let (first, last) = (line_of(text, start), line_of(text, end));
    if end > start && last > first && is_line_start(text, end) {
        if start <= first_non_blank(text, first) {
            return Span::Lines(first, last - 1);
        }
        return Span::Chars(start..line_end(text, last - 1));
    }
    Span::Chars(start..end)
}

fn span_text(text: &Rope, span: &Span) -> (String, bool) {
    match span {
        Span::Chars(range) => (text.slice(range.clone()).to_string(), false),
        Span::Lines(first, last) => {
            let mut lines = text.slice(lines_range(text, *first, *last)).to_string();
            if !lines.ends_with('\n') {
                lines.push('\n');
            }
            (lines, true)
        }
    }
}

fn object_range(text: &Rope, pos: usize, object: Object, around: bool) -> Option<Range<usize>> {
    match object {
        Object::Word { big } => word_object(text, pos, big, around),
        Object::Quote(quote) => quote_object(text, pos, quote, around),
        Object::Bracket(open, close) => bracket_object(text, pos, open, close, around),
    }
}

fn end_of_word_at(text: &Rope, pos: usize, big: bool) -> usize {
    match word_object(text, pos, big, false) {
        Some(range) if range.end > pos => prev(text, range.end),
        _ => pos,
    }
}

fn keyword_at(text: &Rope, pos: usize) -> Option<Range<usize>> {
    let end = line_end(text, line_of(text, pos));
    let mut pos = pos;
    while pos < end && !char_at(text, pos).is_some_and(is_word_char) {
        pos = next(text, pos);
    }
    (pos < end)
        .then(|| word_object(text, pos, false, false))
        .flatten()
}

fn toggle_case(text: &str) -> String {
    text.chars()
        .flat_map(|ch| {
            let toggled: Vec<char> = if ch.is_lowercase() {
                ch.to_uppercase().collect()
            } else if ch.is_uppercase() {
                ch.to_lowercase().collect()
            } else {
                vec![ch]
            };
            toggled
        })
        .collect()
}
