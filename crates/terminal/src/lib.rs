//! PTY-backed terminal sessions, independent of the UI.
//!
//! A [`Terminal`] owns a shell process and its emulated screen. A background thread reads
//! the PTY and parses its output; the UI drains [`Events`], passes each one to
//! [`Terminal::handle`], and renders [`Terminal::snapshot`].

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

/// Lines of scrollback kept above the screen.
const SCROLLBACK_LINES: usize = 10_000;

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Program and arguments to run; `None` runs the platform's default shell.
    pub shell: Option<(String, Vec<String>)>,
    pub working_directory: Option<PathBuf>,
    /// Added to the inherited environment.
    pub env: HashMap<String, String>,
    pub palette: Palette,
}

/// Screen size in cells, plus the cell size in pixels that some programs query.
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

/// What a [`Terminal`] needs the UI to do after handling an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The screen changed.
    Wakeup,
    /// The shell exited; the last screen remains readable.
    Exited,
    /// A program asked to put this text on the clipboard (OSC 52).
    Copy(String),
}

/// How far a mouse selection snaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Cells,
    Words,
    Lines,
}

/// A position on screen: `row` 0 is the top visible row, regardless of scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPoint {
    pub row: usize,
    pub column: usize,
    /// Whether the position is in the right half of the cell.
    pub right_half: bool,
}

/// Forwards emulator notifications from the PTY thread to [`Events`].
#[derive(Clone)]
struct Listener(async_channel::Sender<PtyMessage>);

impl EventListener for Listener {
    fn send_event(&self, event: PtyMessage) {
        let _ = self.0.try_send(event);
    }
}

/// An unprocessed notification from the PTY thread; pass it to [`Terminal::handle`].
pub struct PtyEvent(PtyMessage);

/// Notifications from a [`Terminal`]'s PTY thread. Ends when the terminal is dropped.
pub struct Events(async_channel::Receiver<PtyMessage>);

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
    notifier: Notifier,
    size: GridSize,
    palette: Palette,
    exited: bool,
    exit_code: Option<i32>,
}

impl Terminal {
    /// Start a shell on a new PTY. Output is processed on a background thread.
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
                notifier,
                size,
                palette: options.palette,
                exited: false,
                exit_code: None,
            },
            Events(receiver),
        ))
    }

    /// Apply one PTY notification. Replies that programs expect from the terminal, such as
    /// color and size queries, are written back here.
    pub fn handle(&mut self, event: PtyEvent) -> Option<Event> {
        match event.0 {
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

    pub fn exited(&self) -> bool {
        self.exited
    }

    /// The shell's exit code, when it exited and the platform reported one.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Colors for later snapshots and color queries, such as after a theme change.
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn resize(&mut self, size: GridSize) {
        let size = size.clamped();
        if size != self.size {
            self.size = size;
            self.notifier.on_resize(size.into());
            self.term.lock().resize(size);
        }
    }

    /// Send bytes to the shell as-is.
    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    /// Send user input: returns to the live screen and clears the selection first.
    pub fn input(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
        drop(term);
        self.write(bytes);
    }

    /// The bytes a key press sends, for keys that are not plain text input. `key` is a
    /// lowercase name such as `enter`, `up`, or `f5`, or the character on the key, and
    /// `text` is the character it would type. Returns `None` for keys the platform should
    /// deliver as text instead.
    pub fn key_sequence(
        &self,
        key: &str,
        text: Option<&str>,
        modifiers: Modifiers,
    ) -> Option<Vec<u8>> {
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        keys::sequence(key, text, modifiers, app_cursor)
    }

    /// Paste text, bracketed when the running program asked for that.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = if bracketed {
            // Strip ESC so pasted text cannot end the bracket early.
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.input(text.into_bytes());
    }

    /// Mouse-wheel scrolling by whole lines; positive values scroll towards older output.
    /// Full-screen programs that opt in receive arrow keys instead, as in other terminals.
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

    /// The visible screen with colors resolved against the palette.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(&self.term.lock(), &self.palette)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Ends the PTY thread, which closes the PTY and with it the shell.
        let _ = self.notifier.0.send(Msg::Shutdown);
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

/// PowerShell 7 when installed, otherwise Windows PowerShell.
#[cfg(target_os = "windows")]
fn default_shell() -> Option<tty::Shell> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join("pwsh.exe"))
        .find(|program| program.is_file())
        .map(|program| tty::Shell::new(program.to_string_lossy().into_owned(), Vec::new()))
}

/// The user's login shell, which alacritty resolves.
#[cfg(not(target_os = "windows"))]
fn default_shell() -> Option<tty::Shell> {
    None
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// Run `command` in the platform shell and collect the screen once it exits.
    fn run(command: &str) -> Snapshot {
        let shell = if cfg!(target_os = "windows") {
            ("cmd.exe".into(), vec!["/C".into(), command.into()])
        } else {
            ("/bin/sh".into(), vec!["-c".into(), command.into()])
        };
        let options = Options {
            shell: Some(shell),
            ..Default::default()
        };
        let (mut terminal, events) = Terminal::spawn(options, GridSize::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !terminal.exited() {
            assert!(Instant::now() < deadline, "shell did not exit");
            match events.try_recv() {
                Some(event) => {
                    terminal.handle(event);
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        terminal.snapshot()
    }

    #[test]
    fn runs_a_shell_command_and_reports_its_exit() {
        let snapshot = run("echo hephaestus-terminal");
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
