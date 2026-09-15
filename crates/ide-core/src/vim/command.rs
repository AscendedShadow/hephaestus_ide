#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Escape,
    Enter,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lowercase,
    Uppercase,
    ToggleCase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Find {
    pub forward: bool,
    pub till: bool,
    pub target: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    FirstNonBlank,
    LineEnd,
    NextWordStart { big: bool },
    PrevWordStart { big: bool },
    WordEnd { big: bool },
    PrevWordEnd { big: bool },
    FirstLine,
    LastLine,
    Find(Find),
    RepeatFind { reverse: bool },
    MatchingBracket,
    NextParagraph,
    PrevParagraph,
    NextLine,
    PrevLine,
    CurrentLine,
    SearchNext { reverse: bool },
    SearchWord { forward: bool },
    HalfPageDown,
    HalfPageUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Object {
    Word { big: bool },
    Quote(char),
    Bracket(char, char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Motion(Motion),
    Object { object: Object, around: bool },
    Lines,
    Selection,
    SelectionLines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InsertAt {
    Cursor,
    AfterCursor,
    LineStart,
    LineEnd,
    LineBelow,
    LineAbove,
    SelectionStart,
    SelectionEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    Command,
    Search { forward: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Move(Motion),
    Operate(Operator, Target),
    Insert(InsertAt),
    Paste { before: bool },
    Join,
    Replace(char),
    ToggleCaseUnderCursor,
    Undo,
    Redo,
    Repeat,
    Visual { linewise: bool },
    SwapSelectionEnds,
    SelectObject { object: Object, around: bool },
    Prompt(PromptKind),
    NextBuffer,
    PreviousBuffer,
    WriteClose,
    ForceClose,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cmd {
    pub register: Option<char>,
    pub count: Option<usize>,
    pub action: Action,
}

impl Cmd {
    pub fn count_or_one(&self) -> usize {
        self.count.unwrap_or(1)
    }

    pub fn is_change(&self) -> bool {
        match self.action {
            Action::Operate(operator, _) => operator != Operator::Yank,
            Action::Insert(_)
            | Action::Paste { .. }
            | Action::Join
            | Action::Replace(_)
            | Action::ToggleCaseUnderCursor => true,
            _ => false,
        }
    }
}

pub(super) enum Stop {
    Incomplete,
    Invalid,
}

const MAX_COUNT: usize = 99_999;

struct Keys<'a> {
    keys: &'a [Key],
    pos: usize,
}

impl Keys<'_> {
    fn next(&mut self) -> Result<Key, Stop> {
        let key = *self.keys.get(self.pos).ok_or(Stop::Incomplete)?;
        self.pos += 1;
        if key == Key::Escape && self.pos > 1 {
            return Err(Stop::Invalid);
        }
        Ok(key)
    }

    fn count(&mut self, mut key: Key) -> Result<(Option<usize>, Key), Stop> {
        let mut count: Option<usize> = None;
        loop {
            match key {
                Key::Char(digit @ '1'..='9') | Key::Char(digit @ '0')
                    if count.is_some() || digit != '0' =>
                {
                    let value = count.unwrap_or(0) * 10 + digit.to_digit(10).unwrap_or(0) as usize;
                    count = Some(value.min(MAX_COUNT));
                    key = self.next()?;
                }
                _ => return Ok((count, key)),
            }
        }
    }

    fn char(&mut self) -> Result<char, Stop> {
        match self.next()? {
            Key::Char(ch) => Ok(ch),
            Key::Enter => Ok('\n'),
            Key::Tab => Ok('\t'),
            _ => Err(Stop::Invalid),
        }
    }
}

fn multiply(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(a), Some(b)) => Some((a * b).min(MAX_COUNT)),
        (a, b) => a.or(b),
    }
}

fn is_register(name: char) -> bool {
    name.is_ascii_alphabetic() || matches!(name, '"' | '+' | '*' | '_')
}

pub(super) fn parse(keys: &[Key], visual: bool) -> Result<Cmd, Stop> {
    let mut keys = Keys { keys, pos: 0 };
    let mut key = keys.next()?;
    let mut register = None;
    if key == Key::Char('"') {
        let name = keys.char()?;
        if !is_register(name) {
            return Err(Stop::Invalid);
        }
        register = Some(name);
        key = keys.next()?;
    }
    let (count, key) = keys.count(key)?;
    let command = |action| -> Result<Cmd, Stop> {
        Ok(Cmd {
            register,
            count,
            action,
        })
    };

    if let Some(operator) = operator(key, &mut keys)? {
        if visual {
            return command(Action::Operate(operator, Target::Selection));
        }
        let first = keys.next()?;
        let (motion_count, key) = keys.count(first)?;
        let target = if key == doubled(operator) {
            Target::Lines
        } else if let Key::Char(side @ ('i' | 'a')) = key {
            Target::Object {
                object: object(keys.char()?)?,
                around: side == 'a',
            }
        } else {
            Target::Motion(motion(key, &mut keys)?.ok_or(Stop::Invalid)?)
        };
        return Ok(Cmd {
            register,
            count: multiply(count, motion_count),
            action: Action::Operate(operator, target),
        });
    }
    if let Some(motion) = motion(key, &mut keys)? {
        return command(Action::Move(motion));
    }
    let action = if visual {
        visual_command(key, &mut keys)?
    } else {
        normal_command(key, &mut keys)?
    };
    command(action)
}

fn operator(key: Key, keys: &mut Keys) -> Result<Option<Operator>, Stop> {
    Ok(Some(match key {
        Key::Char('d') => Operator::Delete,
        Key::Char('c') => Operator::Change,
        Key::Char('y') => Operator::Yank,
        Key::Char('>') => Operator::Indent,
        Key::Char('<') => Operator::Outdent,
        Key::Char('g') => {
            let operator = match keys.keys.get(keys.pos) {
                None => return Err(Stop::Incomplete),
                Some(Key::Char('~')) => Operator::ToggleCase,
                Some(Key::Char('u')) => Operator::Lowercase,
                Some(Key::Char('U')) => Operator::Uppercase,
                Some(_) => return Ok(None),
            };
            keys.pos += 1;
            operator
        }
        _ => return Ok(None),
    }))
}

fn doubled(operator: Operator) -> Key {
    Key::Char(match operator {
        Operator::Delete => 'd',
        Operator::Change => 'c',
        Operator::Yank => 'y',
        Operator::Indent => '>',
        Operator::Outdent => '<',
        Operator::ToggleCase => '~',
        Operator::Lowercase => 'u',
        Operator::Uppercase => 'U',
    })
}

fn object(key: char) -> Result<Object, Stop> {
    Ok(match key {
        'w' => Object::Word { big: false },
        'W' => Object::Word { big: true },
        '"' | '\'' | '`' => Object::Quote(key),
        '(' | ')' | 'b' => Object::Bracket('(', ')'),
        '{' | '}' | 'B' => Object::Bracket('{', '}'),
        '[' | ']' => Object::Bracket('[', ']'),
        '<' | '>' => Object::Bracket('<', '>'),
        _ => return Err(Stop::Invalid),
    })
}

fn motion(key: Key, keys: &mut Keys) -> Result<Option<Motion>, Stop> {
    let find = |forward, till, keys: &mut Keys| -> Result<Option<Motion>, Stop> {
        Ok(Some(Motion::Find(Find {
            forward,
            till,
            target: keys.char()?,
        })))
    };
    Ok(Some(match key {
        Key::Char('h') | Key::Left | Key::Backspace => Motion::Left,
        Key::Char('l') | Key::Right | Key::Char(' ') => Motion::Right,
        Key::Char('j') | Key::Down | Key::Ctrl('n') => Motion::Down,
        Key::Char('k') | Key::Up | Key::Ctrl('p') => Motion::Up,
        Key::Char('0') | Key::Home => Motion::LineStart,
        Key::Char('^') => Motion::FirstNonBlank,
        Key::Char('$') | Key::End => Motion::LineEnd,
        Key::Char('w') => Motion::NextWordStart { big: false },
        Key::Char('W') => Motion::NextWordStart { big: true },
        Key::Char('b') => Motion::PrevWordStart { big: false },
        Key::Char('B') => Motion::PrevWordStart { big: true },
        Key::Char('e') => Motion::WordEnd { big: false },
        Key::Char('E') => Motion::WordEnd { big: true },
        Key::Char('G') => Motion::LastLine,
        Key::Char('f') => return find(true, false, keys),
        Key::Char('t') => return find(true, true, keys),
        Key::Char('F') => return find(false, false, keys),
        Key::Char('T') => return find(false, true, keys),
        Key::Char(';') => Motion::RepeatFind { reverse: false },
        Key::Char(',') => Motion::RepeatFind { reverse: true },
        Key::Char('%') => Motion::MatchingBracket,
        Key::Char('}') => Motion::NextParagraph,
        Key::Char('{') => Motion::PrevParagraph,
        Key::Enter | Key::Char('+') => Motion::NextLine,
        Key::Char('-') => Motion::PrevLine,
        Key::Char('_') => Motion::CurrentLine,
        Key::Char('n') => Motion::SearchNext { reverse: false },
        Key::Char('N') => Motion::SearchNext { reverse: true },
        Key::Char('*') => Motion::SearchWord { forward: true },
        Key::Char('#') => Motion::SearchWord { forward: false },
        Key::Ctrl('d') => Motion::HalfPageDown,
        Key::Ctrl('u') => Motion::HalfPageUp,
        Key::Char('g') => {
            let motion = match keys.keys.get(keys.pos) {
                None => return Err(Stop::Incomplete),
                Some(Key::Char('g')) => Motion::FirstLine,
                Some(Key::Char('e')) => Motion::PrevWordEnd { big: false },
                Some(Key::Char('E')) => Motion::PrevWordEnd { big: true },
                Some(_) => return Ok(None),
            };
            keys.pos += 1;
            motion
        }
        _ => return Ok(None),
    }))
}

fn normal_command(key: Key, keys: &mut Keys) -> Result<Action, Stop> {
    let delete = |motion| Action::Operate(Operator::Delete, Target::Motion(motion));
    let change = |motion| Action::Operate(Operator::Change, Target::Motion(motion));
    Ok(match key {
        Key::Char('x') | Key::Delete => delete(Motion::Right),
        Key::Char('X') => delete(Motion::Left),
        Key::Char('D') => delete(Motion::LineEnd),
        Key::Char('C') => change(Motion::LineEnd),
        Key::Char('s') => change(Motion::Right),
        Key::Char('S') => Action::Operate(Operator::Change, Target::Lines),
        Key::Char('Y') => Action::Operate(Operator::Yank, Target::Lines),
        Key::Char('i') => Action::Insert(InsertAt::Cursor),
        Key::Char('a') => Action::Insert(InsertAt::AfterCursor),
        Key::Char('I') => Action::Insert(InsertAt::LineStart),
        Key::Char('A') => Action::Insert(InsertAt::LineEnd),
        Key::Char('o') => Action::Insert(InsertAt::LineBelow),
        Key::Char('O') => Action::Insert(InsertAt::LineAbove),
        Key::Char('p') => Action::Paste { before: false },
        Key::Char('P') => Action::Paste { before: true },
        Key::Char('J') => Action::Join,
        Key::Char('r') => Action::Replace(keys.char()?),
        Key::Char('~') => Action::ToggleCaseUnderCursor,
        Key::Char('u') => Action::Undo,
        Key::Ctrl('r') => Action::Redo,
        Key::Char('.') => Action::Repeat,
        Key::Char('v') => Action::Visual { linewise: false },
        Key::Char('V') => Action::Visual { linewise: true },
        Key::Char(':') => Action::Prompt(PromptKind::Command),
        Key::Char('/') => Action::Prompt(PromptKind::Search { forward: true }),
        Key::Char('?') => Action::Prompt(PromptKind::Search { forward: false }),
        Key::Char('g') => match keys.next()? {
            Key::Char('t') => Action::NextBuffer,
            Key::Char('T') => Action::PreviousBuffer,
            _ => return Err(Stop::Invalid),
        },
        Key::Char('Z') => match keys.next()? {
            Key::Char('Z') => Action::WriteClose,
            Key::Char('Q') => Action::ForceClose,
            _ => return Err(Stop::Invalid),
        },
        Key::Escape | Key::Ctrl('[') => Action::Escape,
        _ => return Err(Stop::Invalid),
    })
}

fn visual_command(key: Key, keys: &mut Keys) -> Result<Action, Stop> {
    let selection = |operator| Action::Operate(operator, Target::Selection);
    let lines = |operator| Action::Operate(operator, Target::SelectionLines);
    Ok(match key {
        Key::Char('x') | Key::Delete => selection(Operator::Delete),
        Key::Char('s') => selection(Operator::Change),
        Key::Char('~') => selection(Operator::ToggleCase),
        Key::Char('u') => selection(Operator::Lowercase),
        Key::Char('U') => selection(Operator::Uppercase),
        Key::Char('X') | Key::Char('D') => lines(Operator::Delete),
        Key::Char('C') | Key::Char('S') | Key::Char('R') => lines(Operator::Change),
        Key::Char('Y') => lines(Operator::Yank),
        Key::Char('I') => Action::Insert(InsertAt::SelectionStart),
        Key::Char('A') => Action::Insert(InsertAt::SelectionEnd),
        Key::Char('p') | Key::Char('P') => Action::Paste { before: false },
        Key::Char('J') => Action::Join,
        Key::Char('r') => Action::Replace(keys.char()?),
        Key::Char('o') | Key::Char('O') => Action::SwapSelectionEnds,
        Key::Char(side @ ('i' | 'a')) => Action::SelectObject {
            object: object(keys.char()?)?,
            around: side == 'a',
        },
        Key::Char('v') => Action::Visual { linewise: false },
        Key::Char('V') => Action::Visual { linewise: true },
        Key::Char(':') => Action::Prompt(PromptKind::Command),
        Key::Char('/') => Action::Prompt(PromptKind::Search { forward: true }),
        Key::Char('?') => Action::Prompt(PromptKind::Search { forward: false }),
        Key::Escape | Key::Ctrl('[') => Action::Escape,
        _ => return Err(Stop::Invalid),
    })
}
