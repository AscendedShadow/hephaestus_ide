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

#[derive(Clone)]
struct Listener(async_channel::Sender<PtyMessage>);

impl EventListener for Listener {
    fn send_event(&self, event: PtyMessage) {
        let _ = self.0.try_send(event);
    }
}

pub struct PtyEvent(PtyMessage);

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
    notifier: Option<Notifier>,
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
                size,
                palette: options.palette,
                exited: false,
                exit_code: None,
            },
            Events(receiver),
        ))
    }

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
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(notifier) = &self.notifier {
            let _ = notifier.0.send(Msg::Shutdown);
        }
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
