//! The Terminal tool panel: draws a [`Terminal`] and forwards keyboard, mouse, and
//! clipboard input to it.

use std::{ops::Range, path::PathBuf};

use gpui::{
    App, BorderStyle, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler,
    FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render,
    ScrollWheelEvent, ShapedLine, Size, StrikethroughStyle, Task, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, canvas, div, fill, outline, point, prelude::*, px, rgb, size,
};
use terminal::{
    Cell, CursorShape, Event, GridPoint, GridSize, Modifiers, Options, Palette, PtyEvent,
    SelectionKind, Snapshot, Terminal,
};

use crate::theme;

#[cfg(test)]
#[path = "terminal_view_tests.rs"]
mod tests;

actions!(terminal, [Copy, Paste]);

/// Key context of a focused terminal; app shortcuts the shell needs are unbound in it.
pub const KEY_CONTEXT: &str = "Terminal";

const FONT_SIZE: f32 = 13.;
/// Line height as a multiple of the font size.
const LINE_HEIGHT: f32 = 1.3;

pub struct TerminalView {
    focus_handle: FocusHandle,
    session: Option<Session>,
    /// Why the last start failed.
    error: Option<String>,
    /// Geometry from the last layout, for mapping the mouse and IME to cells.
    layout: Option<Layout>,
    /// Uncommitted IME composition, drawn at the cursor.
    marked_text: Option<String>,
    /// Wheel movement not yet amounting to a whole line.
    scroll_remainder: f32,
    selecting: bool,
}

struct Session {
    terminal: Terminal,
    /// Reused to restart the shell after it exits.
    options: Options,
    _events: Task<()>,
}

#[derive(Clone, Copy)]
struct Layout {
    origin: Point<Pixels>,
    cell: Size<Pixels>,
    grid: GridSize,
    cursor: Option<(usize, usize)>,
}

impl Layout {
    fn cell_bounds(&self, row: usize, column: usize, columns: usize) -> Bounds<Pixels> {
        Bounds::new(
            point(
                self.origin.x + self.cell.width * column as f32,
                self.origin.y + self.cell.height * row as f32,
            ),
            size(self.cell.width * columns as f32, self.cell.height),
        )
    }

    fn grid_point(&self, position: Point<Pixels>) -> GridPoint {
        let column = ((position.x - self.origin.x) / self.cell.width).max(0.);
        let row = ((position.y - self.origin.y) / self.cell.height).max(0.);
        GridPoint {
            row: row as usize,
            column: column as usize,
            right_half: column.fract() >= 0.5,
        }
    }
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            session: None,
            error: None,
            layout: None,
            marked_text: None,
            scroll_remainder: 0.,
            selecting: false,
        }
    }

    fn is_running(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| !session.terminal.exited())
    }

    /// Start the default shell, in the home folder when no directory is given. Does nothing
    /// once a shell has started; after it exits, Enter restarts it.
    pub fn start(&mut self, working_directory: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.session.is_none() {
            self.spawn(
                Options {
                    working_directory: working_directory.or_else(std::env::home_dir),
                    ..Default::default()
                },
                cx,
            );
        }
    }

    fn spawn(&mut self, options: Options, cx: &mut Context<Self>) {
        // Close the previous shell before starting the next.
        self.session = None;
        let size = self.layout.map(|layout| layout.grid).unwrap_or_default();
        let spawn_options = Options {
            palette: theme::terminal_palette(),
            ..options.clone()
        };
        match Terminal::spawn(spawn_options, size) {
            Ok((terminal, events)) => {
                let events = cx.spawn(async move |this, cx| {
                    while let Some(event) = events.recv().await {
                        // Take everything already queued so a burst of output repaints once.
                        let mut batch = vec![event];
                        batch.extend(std::iter::from_fn(|| events.try_recv()));
                        if this
                            .update(cx, |this, cx| this.handle_events(batch, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                self.session = Some(Session {
                    terminal,
                    options,
                    _events: events,
                });
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Could not start a shell: {error}")),
        }
        cx.notify();
    }

    fn handle_events(&mut self, events: Vec<PtyEvent>, cx: &mut Context<Self>) {
        let Some(session) = &mut self.session else {
            return;
        };
        for event in events {
            if let Some(Event::Copy(text)) = session.terminal.handle(event) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let Some(session) = &self.session else {
            return;
        };
        if session.terminal.exited() {
            if keystroke.key == "enter" {
                let options = session.options.clone();
                self.spawn(options, cx);
                cx.stop_propagation();
            }
            return;
        }
        // Cmd / Windows-key chords stay app shortcuts.
        if keystroke.modifiers.platform {
            return;
        }
        let modifiers = Modifiers {
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
        };
        // Plain text is left to the platform, which delivers it through the input handler
        // so that IME composition and dead keys work.
        if let Some(bytes) =
            session
                .terminal
                .key_sequence(&keystroke.key, keystroke.key_char.as_deref(), modifiers)
        {
            session.terminal.input(bytes);
            cx.stop_propagation();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let (Some(session), Some(layout)) = (&self.session, self.layout) else {
            return;
        };
        let kind = match event.click_count {
            0 | 1 => SelectionKind::Cells,
            2 => SelectionKind::Words,
            _ => SelectionKind::Lines,
        };
        session
            .terminal
            .select(layout.grid_point(event.position), kind);
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let (Some(session), Some(layout)) = (&self.session, self.layout) {
            session
                .terminal
                .extend_selection(layout.grid_point(event.position));
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(session), Some(layout)) = (&self.session, self.layout) else {
            return;
        };
        let line_height = layout.cell.height;
        let lines = self.scroll_remainder + event.delta.pixel_delta(line_height).y / line_height;
        self.scroll_remainder = lines.fract();
        if lines.abs() >= 1. {
            session.terminal.scroll_wheel(lines.trunc() as i32);
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self
            .session
            .as_ref()
            .and_then(|session| session.terminal.selected_text())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_running()
            && let Some(session) = &self.session
            && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
        {
            session.terminal.paste(&text);
        }
    }

    /// Fit the terminal to `bounds` and lay out its screen for painting.
    fn layout_frame(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Option<Frame> {
        let font = theme::monospace_font();
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let cell_width = text_system
            .advance(text_system.resolve_font(&font), font_size, 'm')
            .map(|advance| advance.width)
            .unwrap_or(font_size * 0.6);
        let cell = size(cell_width, (font_size * LINE_HEIGHT).round());
        let grid = GridSize {
            columns: (bounds.size.width / cell.width) as u16,
            rows: (bounds.size.height / cell.height) as u16,
            cell_width: f32::from(cell.width) as u16,
            cell_height: f32::from(cell.height) as u16,
        };
        let mut layout = Layout {
            origin: bounds.origin,
            cell,
            grid,
            cursor: None,
        };

        let focused = self.focus_handle.is_focused(window);
        let frame = self.session.as_mut().map(|session| {
            let palette = theme::terminal_palette();
            session.terminal.set_palette(palette);
            session.terminal.resize(grid);
            let snapshot = session.terminal.snapshot();
            layout.cursor = snapshot.cursor.map(|cursor| (cursor.row, cursor.column));
            let marked_text = self.marked_text.as_deref();
            Frame::new(
                snapshot,
                &layout,
                &font,
                focused,
                marked_text,
                palette,
                window,
            )
        });
        self.layout = Some(layout);
        frame
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notice = |message: String| {
            div()
                .flex_shrink_0()
                .px_3()
                .py_1()
                .text_color(theme::muted())
                .child(message)
        };
        // Before a shell starts the notice leads; after one exits it follows the last screen.
        let intro = self.session.is_none().then(|| {
            self.error
                .clone()
                .unwrap_or_else(|| "Click here or press Ctrl+` to start a terminal.".into())
        });
        let exit = self
            .session
            .as_ref()
            .filter(|session| session.terminal.exited())
            .map(|session| match session.terminal.exit_code() {
                Some(code) => format!("Process exited with code {code}. Press Enter to restart."),
                None => "Process exited. Press Enter to restart.".into(),
            });
        let view = cx.entity();
        let focus_handle = self.focus_handle.clone();
        div()
            .id("terminal")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::background())
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .children(intro.map(notice))
            .child(
                div().flex_1().min_h_0().pl_2().pt_1().child(
                    canvas(
                        {
                            let view = view.clone();
                            move |bounds, window, cx| {
                                view.update(cx, |view, _| view.layout_frame(bounds, window))
                            }
                        },
                        move |bounds, frame, window, cx| {
                            if let Some(frame) = frame {
                                frame.paint(window, cx);
                            }
                            window.handle_input(
                                &focus_handle,
                                ElementInputHandler::new(bounds, view),
                                cx,
                            );
                        },
                    )
                    .size_full(),
                ),
            )
            .children(exit.map(notice))
    }
}

/// Typed text arrives here from the platform rather than as key events.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if self.is_running()
            && let Some(session) = &self.session
        {
            session.terminal.input(text.as_bytes().to_vec());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    /// Places the IME candidate window at the cursor.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout?;
        let (row, column) = layout.cursor?;
        Some(layout.cell_bounds(row, column, 1))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// A laid-out screen, ready to paint.
struct Frame {
    backgrounds: Vec<PaintQuad>,
    text: Vec<(Point<Pixels>, ShapedLine)>,
    /// Cursor shapes and the IME composition, drawn over the text.
    overlays: Vec<PaintQuad>,
    overlay_text: Option<(Point<Pixels>, ShapedLine)>,
    line_height: Pixels,
}

impl Frame {
    fn new(
        mut snapshot: Snapshot,
        layout: &Layout,
        font: &Font,
        focused: bool,
        marked_text: Option<&str>,
        palette: Palette,
        window: &mut Window,
    ) -> Self {
        let mut overlays = Vec::new();
        let mut overlay_text = None;
        if let Some(cursor) = snapshot.cursor {
            let columns = if cursor.wide { 2 } else { 1 };
            let bounds = layout.cell_bounds(cursor.row, cursor.column, columns);
            let color = hsla(palette.cursor);
            let bar = px(2.);
            match (focused, cursor.shape) {
                // Drawn by inverting the cell's colors.
                (true, CursorShape::Block) => {
                    for cell in snapshot.rows[cursor.row]
                        .iter_mut()
                        .skip(cursor.column)
                        .take(columns)
                    {
                        cell.background = palette.cursor;
                        cell.foreground = palette.background;
                        cell.selected = false;
                    }
                }
                (true, CursorShape::Beam) => overlays.push(fill(
                    Bounds::new(bounds.origin, size(bar, bounds.size.height)),
                    color,
                )),
                (true, CursorShape::Underline) => overlays.push(fill(
                    Bounds::new(
                        point(bounds.left(), bounds.bottom() - bar),
                        size(bounds.size.width, bar),
                    ),
                    color,
                )),
                (false, _) => overlays.push(outline(bounds, color, BorderStyle::Solid)),
            }
            if let Some(marked_text) = marked_text {
                let style = Cell {
                    ch: ' ',
                    combining: Vec::new(),
                    foreground: palette.foreground,
                    background: palette.background,
                    bold: false,
                    italic: false,
                    underline: true,
                    strikethrough: false,
                    width: 1,
                    selected: false,
                };
                let line = shape(marked_text, &style, font, window);
                overlays.push(fill(
                    Bounds::new(bounds.origin, size(line.width, bounds.size.height)),
                    hsla(palette.background),
                ));
                overlay_text = Some((bounds.origin, line));
            }
        }

        let selection = Hsla::from(theme::selection());
        let background = |cell: &Cell| {
            if cell.selected {
                Some(selection)
            } else {
                (cell.background != palette.background).then(|| hsla(cell.background))
            }
        };
        let mut backgrounds = Vec::new();
        let mut text = Vec::new();
        for (row_index, row) in snapshot.rows.iter().enumerate() {
            let mut column = 0;
            while column < row.len() {
                let start = column;
                let color = background(&row[start]);
                while column < row.len() && background(&row[column]) == color {
                    column += 1;
                }
                if let Some(color) = color {
                    let bounds = layout.cell_bounds(row_index, start, column - start);
                    backgrounds.push(fill(bounds, color));
                }
            }

            // Runs of same-styled text, each placed at its first column. Wide characters get
            // their own run so that a fallback font's advance cannot shift what follows.
            let mut column = 0;
            while column < row.len() {
                let first = &row[column];
                if first.width == 0 || is_blank(first) {
                    column += 1;
                    continue;
                }
                let start = column;
                let mut run = String::new();
                loop {
                    let cell = &row[column];
                    run.push(cell.ch);
                    run.extend(&cell.combining);
                    column += 1;
                    if first.width == 2
                        || column == row.len()
                        || row[column].width != 1
                        || !same_style(first, &row[column])
                    {
                        break;
                    }
                }
                let origin = layout.cell_bounds(row_index, start, 1).origin;
                text.push((origin, shape(&run, first, font, window)));
            }
        }

        Self {
            backgrounds,
            text,
            overlays,
            overlay_text,
            line_height: layout.cell.height,
        }
    }

    fn paint(self, window: &mut Window, cx: &mut App) {
        for quad in self.backgrounds {
            window.paint_quad(quad);
        }
        for (origin, line) in &self.text {
            line.paint(*origin, self.line_height, window, cx).ok();
        }
        for quad in self.overlays {
            window.paint_quad(quad);
        }
        if let Some((origin, line)) = &self.overlay_text {
            line.paint(*origin, self.line_height, window, cx).ok();
        }
    }
}

fn hsla(color: u32) -> Hsla {
    rgb(color).into()
}

fn is_blank(cell: &Cell) -> bool {
    cell.ch == ' ' && cell.combining.is_empty() && !cell.underline && !cell.strikethrough
}

fn same_style(a: &Cell, b: &Cell) -> bool {
    a.foreground == b.foreground
        && a.bold == b.bold
        && a.italic == b.italic
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
}

fn shape(text: &str, style: &Cell, font: &Font, window: &mut Window) -> ShapedLine {
    let color = hsla(style.foreground);
    let run = TextRun {
        len: text.len(),
        font: Font {
            weight: if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..font.clone()
        },
        color,
        background_color: None,
        underline: style.underline.then_some(UnderlineStyle {
            thickness: px(1.),
            color: Some(color),
            wavy: false,
        }),
        strikethrough: style.strikethrough.then_some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(color),
        }),
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), px(FONT_SIZE), &[run], None)
}
