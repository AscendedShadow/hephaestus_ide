mod keys;
mod snapshot;

use std::{borrow::Cow, collections::HashMap, io, path::PathBuf, sync::Arc};

use alacritty_terminal::{
    event::{Event as PtyMessage, EventListener, Notify, OnResize, WindowSize},
    event_loop::{EventLoop, Msg, Notifier},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{self, Term, TermMode},
    tty,
};

pub use keys::Modifiers;
pub use snapshot::{Cell, Cursor, CursorShape, Palette, Snapshot};

const SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub shell: Option<(String, Vec<String>)>,
    pub working_directory: Option<PathBuf>,
    pub env: HashMap<String, String>,
    pub palette: Palette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub columns: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl GridSize {
    fn clamped(self) -> Self {
        Self {
            columns: self.columns.max(2),
            rows: self.rows.max(1),
            ..self
        }
    }
}

impl Default for GridSize {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows.into()
    }

    fn columns(&self) -> usize {
        self.columns.into()
    }
}

impl From<GridSize> for WindowSize {
    fn from(size: GridSize) -> Self {
        Self {
            num_lines: size.rows,
            num_cols: size.columns,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Wakeup,
    Exited,
    Copy(String),
    /// Raw process output (piped sessions only), in arrival order.
    Output(Vec<u8>),
}

/// A match in the terminal grid: inclusive `(line, column)` cell positions. Lines are
/// relative to the live screen (negative lines are scrollback history).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: (i32, usize),
    pub end: (i32, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Cells,
    Words,
    Lines,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPoint {
    pub row: usize,
    pub column: usize,
    pub right_half: bool,
}

enum Message {
    Pty(PtyMessage),
    Output(Vec<u8>),
    Exit(Option<i32>),
}

#[derive(Clone)]
struct Listener(async_channel::Sender<Message>);

impl EventListener for Listener {
    fn send_event(&self, event: PtyMessage) {
        let _ = self.0.try_send(Message::Pty(event));
    }
}

pub struct PtyEvent(Message);

pub struct Events(async_channel::Receiver<Message>);

impl Events {
    pub async fn recv(&self) -> Option<PtyEvent> {
        self.0.recv().await.ok().map(PtyEvent)
    }

    pub fn try_recv(&self) -> Option<PtyEvent> {
        self.0.try_recv().ok().map(PtyEvent)
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Option<Notifier>,
    /// The process of a piped (non-PTY) session.
    child: Option<Arc<std::sync::Mutex<std::process::Child>>>,
    size: GridSize,
    palette: Palette,
    exited: bool,
    exit_code: Option<i32>,
}

impl Terminal {
    pub fn spawn(options: Options, size: GridSize) -> io::Result<(Self, Events)> {
        let size = size.clamped();
        let (sender, receiver) = async_channel::unbounded();
        let listener = Listener(sender);
        let config = term::Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));

        let mut env = options.env;
        for (key, value) in [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "hephaestus"),
        ] {
            env.entry(key.into()).or_insert_with(|| value.into());
        }
        let pty_options = tty::Options {
            shell: options
                .shell
                .map(|(program, args)| tty::Shell::new(program, args))
                .or_else(default_shell),
            working_directory: options.working_directory,
            drain_on_exit: true,
            env,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let pty = tty::new(&pty_options, size.into(), 0)?;
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();

        Ok((
            Self {
                term,
                notifier: Some(notifier),
                child: None,
                size,
                palette: options.palette,
                exited: false,
                exit_code: None,
            },
            Events(receiver),
        ))
    }

    /// Run a program with piped output (no PTY): output is rendered in the terminal and
    /// also delivered as [`Event::Output`], so callers can capture all of it
    /// independently of the scrollback size. The program runs in its own process group
    /// so [`Terminal::terminate`] can stop the whole process tree.
    pub fn spawn_piped(options: Options, size: GridSize) -> io::Result<(Self, Events)> {
        use std::{io::Read as _, process::Stdio};

        let size = size.clamped();
        let (sender, receiver) = async_channel::unbounded();
        let listener = Listener(sender.clone());
        let config = term::Config {
            scrolling_history: SCROLLBACK_LINES,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener)));
        let (program, args) = options
            .shell
            .ok_or_else(|| io::Error::other("A program is required"))?;
        let mut command = std::process::Command::new(&program);
        command
            .args(&args)
            .envs(&options.env)
            .env("CLICOLOR_FORCE", "1")
            .env("CARGO_TERM_COLOR", "always")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = &options.working_directory {
            command.current_dir(directory);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn()?;
        let streams: Vec<Box<dyn io::Read + Send>> = [
            child
                .stdout
                .take()
                .map(|stream| Box::new(stream) as Box<dyn io::Read + Send>),
            child
                .stderr
                .take()
                .map(|stream| Box::new(stream) as Box<dyn io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        let child = Arc::new(std::sync::Mutex::new(child));
        let (bytes_sender, bytes_receiver) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
        let readers = streams.len();
        for mut stream in streams {
            let bytes_sender = bytes_sender.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if bytes_sender.send(Some(buffer[..read].to_vec())).is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = bytes_sender.send(None);
            });
        }
        drop(bytes_sender);
        let feed_term = term.clone();
        let feed_child = child.clone();
        std::thread::spawn(move || {
            use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
            let mut processor = Processor::<StdSyncHandler>::new();
            let mut open = readers;
            while open > 0 {
                match bytes_receiver.recv() {
                    Ok(Some(bytes)) => {
                        // Pipes deliver bare LF; render it as CR LF like a terminal would.
                        let mut rendered = Vec::with_capacity(bytes.len() + 16);
                        for byte in &bytes {
                            if *byte == b'\n' {
                                rendered.push(b'\r');
                            }
                            rendered.push(*byte);
                        }
                        processor.advance(&mut *feed_term.lock(), &rendered);
                        let _ = sender.try_send(Message::Output(bytes));
                        let _ = sender.try_send(Message::Pty(PtyMessage::Wakeup));
                    }
                    Ok(None) => open -= 1,
                    Err(_) => break,
                }
            }
            // Poll instead of blocking in `wait` so `terminate` never waits on the lock.
            let code = loop {
                let status = match feed_child.lock() {
                    Ok(mut child) => child.try_wait(),
                    Err(_) => break None,
                };
                match status {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(_) => break None,
                }
            };
            let _ = sender.try_send(Message::Exit(code));
        });
        Ok((
            Self {
                term,
                notifier: None,
                child: Some(child),
                size,
                palette: options.palette,
                exited: false,
                exit_code: None,
            },
            Events(receiver),
        ))
    }

    /// Stop a piped session's process tree (no-op for PTY sessions, which end when the
    /// terminal is dropped).
    pub fn terminate(&self) {
        let Some(child) = &self.child else { return };
        let Ok(mut child) = child.lock() else { return };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        let pid = child.id();
        #[cfg(unix)]
        let _ = std::process::Command::new("kill")
            .args(["-s", "TERM", "--", &format!("-{pid}")])
            .status();
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .status();
        let _ = child.kill();
    }

    pub fn handle(&mut self, event: PtyEvent) -> Option<Event> {
        let message = match event.0 {
            Message::Pty(message) => message,
            Message::Output(bytes) => return Some(Event::Output(bytes)),
            Message::Exit(code) => {
                self.exit_code = code;
                self.exited = true;
                return Some(Event::Exited);
            }
        };
        match message {
            PtyMessage::Wakeup => Some(Event::Wakeup),
            PtyMessage::ChildExit(status) => {
                self.exit_code = status.code();
                self.exited = true;
                Some(Event::Exited)
            }
            PtyMessage::Exit => {
                self.exited = true;
                Some(Event::Exited)
            }
            PtyMessage::ClipboardStore(_, text) => Some(Event::Copy(text)),
            PtyMessage::PtyWrite(text) => {
                self.write(text.into_bytes());
                None
            }
            PtyMessage::ColorRequest(index, format) => {
                let color = self.palette.color(index, self.term.lock().colors());
                self.write(format(snapshot::to_rgb(color)).into_bytes());
                None
            }
            PtyMessage::TextAreaSizeRequest(format) => {
                self.write(format(self.size.into()).into_bytes());
                None
            }
            PtyMessage::Title(_)
            | PtyMessage::ResetTitle
            | PtyMessage::Bell
            | PtyMessage::ClipboardLoad(..)
            | PtyMessage::MouseCursorDirty
            | PtyMessage::CursorBlinkingChange => None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_test_output(output: &[u8], size: GridSize, exited: bool) -> Self {
        use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

        let size = size.clamped();
        let (sender, _) = async_channel::unbounded();
        let listener = Listener(sender);
        let mut term = Term::new(term::Config::default(), &size, listener);
        Processor::<StdSyncHandler>::new().advance(&mut term, output);
        Self {
            term: Arc::new(FairMutex::new(term)),
            notifier: None,
            child: None,
            size,
            palette: Palette::default(),
            exited,
            exit_code: None,
        }
    }

    pub fn exited(&self) -> bool {
        self.exited
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn resize(&mut self, size: GridSize) {
        let size = size.clamped();
        if size != self.size {
            self.size = size;
            if let Some(notifier) = &mut self.notifier {
                notifier.on_resize(size.into());
            }
            self.term.lock().resize(size);
        }
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(notifier) = &self.notifier {
            notifier.notify(bytes);
        }
    }

    pub fn input(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
        drop(term);
        self.write(bytes);
    }

    pub fn key_sequence(
        &self,
        key: &str,
        text: Option<&str>,
        modifiers: Modifiers,
    ) -> Option<Vec<u8>> {
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        keys::sequence(key, text, modifiers, app_cursor)
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = if bracketed {
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.input(text.into_bytes());
    }

    pub fn scroll_wheel(&self, lines: i32) {
        let mut term = self.term.lock();
        let mode = *term.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            drop(term);
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            term.scroll_display(Scroll::Delta(lines));
        }
    }

    pub fn select(&self, at: GridPoint, kind: SelectionKind) {
        let mut term = self.term.lock();
        let kind = match kind {
            SelectionKind::Cells => SelectionType::Simple,
            SelectionKind::Words => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        let (point, side) = grid_point(&term, at);
        term.selection = Some(Selection::new(kind, point, side));
    }

    pub fn extend_selection(&self, to: GridPoint) {
        let mut term = self.term.lock();
        let (point, side) = grid_point(&term, to);
        if let Some(selection) = &mut term.selection {
            selection.update(point, side);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(&self.term.lock(), &self.palette)
    }

    /// A snapshot with search matches highlighted.
    pub fn snapshot_with(&self, matches: &[SearchMatch]) -> Snapshot {
        Snapshot::with_matches(&self.term.lock(), &self.palette, matches)
    }

    /// Search the retained scrollback and the screen (top to bottom). Soft-wrapped rows
    /// are searched as one line; wide characters and combining marks map to their cells.
    /// In the alternate screen (full-screen programs) only the visible screen is searched,
    /// because it has no scrollback.
    pub fn search(&self, query: &str, case_sensitive: bool) -> Vec<SearchMatch> {
        let normalize = |character: char| {
            if case_sensitive {
                character
            } else {
                character.to_lowercase().next().unwrap_or(character)
            }
        };
        let needle: Vec<char> = query.chars().map(normalize).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        let term = self.term.lock();
        let grid = term.grid();
        let top = -(grid.history_size() as i32);
        let bottom = term.screen_lines() as i32;
        let columns = term.columns();
        let mut matches = Vec::new();
        let mut text: Vec<char> = Vec::new();
        let mut cells: Vec<(i32, usize)> = Vec::new();
        let mut flush = |text: &mut Vec<char>, cells: &mut Vec<(i32, usize)>| {
            if text.len() >= needle.len() {
                for start in 0..=text.len() - needle.len() {
                    if text[start..start + needle.len()] == needle[..] {
                        matches.push(SearchMatch {
                            start: cells[start],
                            end: cells[start + needle.len() - 1],
                        });
                    }
                }
            }
            text.clear();
            cells.clear();
        };
        for line in top..bottom {
            let row = &grid[Line(line)];
            let mut wrapped = false;
            for column in 0..columns {
                let cell = &row[Column(column)];
                if cell.flags.contains(term::cell::Flags::WIDE_CHAR_SPACER)
                    || cell
                        .flags
                        .contains(term::cell::Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(normalize(cell.c));
                cells.push((line, column));
                for combining in cell.zerowidth().unwrap_or_default() {
                    text.push(normalize(*combining));
                    cells.push((line, column));
                }
                if column == columns - 1 && cell.flags.contains(term::cell::Flags::WRAPLINE) {
                    wrapped = true;
                }
            }
            if !wrapped {
                // Trailing blanks of a line never match across lines.
                while text.last() == Some(&' ') {
                    text.pop();
                    cells.pop();
                }
                flush(&mut text, &mut cells);
            }
        }
        flush(&mut text, &mut cells);
        matches
    }

    /// Scroll so `found` is visible and select it (the current match).
    pub fn reveal(&self, found: &SearchMatch) {
        let mut term = self.term.lock();
        let screen = term.screen_lines() as i32;
        term.scroll_display(Scroll::Bottom);
        if found.start.0 < 0 {
            term.scroll_display(Scroll::Delta(-found.start.0 + screen / 2));
        }
        let start = Point::new(Line(found.start.0), Column(found.start.1));
        let end = Point::new(Line(found.end.0), Column(found.end.1));
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
    }

    pub fn clear_selection(&self) {
        self.term.lock().selection = None;
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(notifier) = &self.notifier {
            let _ = notifier.0.send(Msg::Shutdown);
        }
        self.terminate();
    }
}

fn grid_point<T>(term: &Term<T>, at: GridPoint) -> (Point, Side) {
    let row = at.row.min(term.screen_lines().saturating_sub(1));
    let line = Line(row as i32 - term.grid().display_offset() as i32);
    let column = Column(at.column.min(term.columns().saturating_sub(1)));
    let side = if at.right_half {
        Side::Right
    } else {
        Side::Left
    };
    (Point::new(line, column), side)
}

#[cfg(target_os = "windows")]
fn default_shell() -> Option<tty::Shell> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("pwsh.exe"))
        .find(|program| program.is_file())
        .map(|program| tty::Shell::new(program.to_string_lossy().into_owned(), Vec::new()))
}

#[cfg(not(target_os = "windows"))]
fn default_shell() -> Option<tty::Shell> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_completed_terminal_from_output() {
        let terminal =
            Terminal::from_test_output(b"hephaestus-terminal\r\n", GridSize::default(), true);
        let snapshot = terminal.snapshot();
        assert!(terminal.exited());
        assert!(
            snapshot
                .lines()
                .iter()
                .any(|line| line.trim_end() == "hephaestus-terminal"),
            "{:#?}",
            snapshot.lines()
        );
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn finds_matches_in_scrollback_wrapped_lines_and_wide_characters() {
        let mut output = Vec::new();
        for ix in 0..60 {
            output.extend(format!("row {ix} Needle\r\n").into_bytes());
        }
        // A line longer than the 20-column grid wraps; the match spans the wrap.
        output.extend("0123456789012345needle-wrapped\r\n".as_bytes());
        output.extend("漢字 needle 😀 e\u{301}x\r\n".as_bytes());
        let size = GridSize {
            columns: 20,
            rows: 6,
            cell_width: 8,
            cell_height: 16,
        };
        let terminal = Terminal::from_test_output(&output, size, false);
        let insensitive = terminal.search("needle", false);
        let sensitive = terminal.search("Needle", true);
        assert!(insensitive.len() > sensitive.len());
        assert!(
            insensitive[0].start.0 < 0,
            "scrollback is searched: {:?}",
            insensitive[0]
        );
        let wrapped = insensitive
            .iter()
            .find(|found| found.start.0 != found.end.0)
            .expect("a match across the soft wrap");
        assert_eq!(wrapped.start.1, 16);
        assert_eq!(wrapped.end, (wrapped.start.0 + 1, 1));
        let wide = insensitive.last().unwrap();
        assert_eq!(
            wide.start.1, 5,
            "two wide characters take four cells plus a space"
        );
        assert_eq!(
            terminal.search("e\u{301}x", true).len(),
            1,
            "combining marks stay with their cell"
        );
        terminal.reveal(&insensitive[0]);
        assert!(
            terminal
                .selected_text()
                .unwrap()
                .eq_ignore_ascii_case("needle")
        );
        let snapshot = terminal.snapshot_with(&insensitive);
        assert!(snapshot.rows.iter().flatten().any(|cell| cell.matched));
    }
}

#[cfg(all(test, unix))]
mod piped_tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn piped_sessions_capture_all_output_and_the_exit_code() {
        let (mut terminal, events) = Terminal::spawn_piped(
            Options {
                shell: Some((
                    "sh".into(),
                    vec![
                        "-c".into(),
                        "for i in $(seq 1 3000); do echo line$i; done; echo oops >&2; exit 3"
                            .into(),
                    ],
                )),
                ..Default::default()
            },
            GridSize::default(),
        )
        .unwrap();
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !terminal.exited() && Instant::now() < deadline {
            match events.try_recv() {
                Some(event) => {
                    if let Some(Event::Output(bytes)) = terminal.handle(event) {
                        output.extend(bytes);
                    }
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        let text = String::from_utf8(output).unwrap();
        assert!(text.starts_with("line1\n"), "{}", &text[..20]);
        assert!(
            text.contains("line3000\n"),
            "complete beyond the visible grid"
        );
        assert!(text.contains("oops"));
        assert_eq!(terminal.exit_code(), Some(3));
    }

    #[test]
    fn terminate_stops_the_process_tree() {
        let (mut terminal, events) = Terminal::spawn_piped(
            Options {
                shell: Some(("sh".into(), vec!["-c".into(), "sleep 30 & sleep 30".into()])),
                ..Default::default()
            },
            GridSize::default(),
        )
        .unwrap();
        terminal.terminate();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !terminal.exited() && Instant::now() < deadline {
            match events.try_recv() {
                Some(event) => {
                    terminal.handle(event);
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(terminal.exited(), "the process group was terminated");
    }
}
