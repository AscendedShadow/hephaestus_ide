use alacritty_terminal::{
    Term,
    event::EventListener,
    grid::Dimensions,
    term::{
        cell::Flags,
        color::{COUNT, Colors},
    },
    vte::ansi::{Color, CursorShape as TermCursorShape, NamedColor, Rgb},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub ansi: [u32; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: 0xdfe1e5,
            background: 0x1e1f22,
            cursor: 0xdfe1e5,
            ansi: [
                0x3f4451, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd7dae0,
                0x5c6370, 0xf0838c, 0xb5e08f, 0xf0d08f, 0x80c4ff, 0xdb9ff0, 0x70d4df, 0xffffff,
            ],
        }
    }
}

const BACKGROUND: usize = NamedColor::Background as usize;
const CURSOR: usize = NamedColor::Cursor as usize;
const DIM_BLACK: usize = NamedColor::DimBlack as usize;
const DIM_WHITE: usize = NamedColor::DimWhite as usize;
const DIM_FOREGROUND: usize = NamedColor::DimForeground as usize;

impl Palette {
    pub(crate) fn color(&self, index: usize, overrides: &Colors) -> u32 {
        if let Some(rgb) = (index < COUNT).then(|| overrides[index]).flatten() {
            return from_rgb(rgb);
        }
        match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let level = |value: usize| {
                    if value == 0 {
                        0
                    } else {
                        55 + 40 * value as u32
                    }
                };
                let cube = index - 16;
                level(cube / 36) << 16 | level(cube / 6 % 6) << 8 | level(cube % 6)
            }
            232..=255 => (8 + 10 * (index - 232) as u32) * 0x01_01_01,
            BACKGROUND => self.background,
            CURSOR => self.cursor,
            DIM_BLACK..=DIM_WHITE => dim(self.ansi[index - DIM_BLACK]),
            DIM_FOREGROUND => dim(self.foreground),
            _ => self.foreground,
        }
    }

    fn resolve(&self, color: Color, overrides: &Colors) -> u32 {
        match color {
            Color::Spec(rgb) => from_rgb(rgb),
            Color::Named(named) => self.color(named as usize, overrides),
            Color::Indexed(index) => self.color(index.into(), overrides),
        }
    }
}

fn from_rgb(rgb: Rgb) -> u32 {
    u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b)
}

pub(crate) fn to_rgb(color: u32) -> Rgb {
    let [_, r, g, b] = color.to_be_bytes();
    Rgb { r, g, b }
}

fn dim(color: u32) -> u32 {
    let [_, r, g, b] = color.to_be_bytes();
    let dim = |channel: u8| u32::from(channel) * 2 / 3;
    dim(r) << 16 | dim(g) << 8 | dim(b)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub combining: Vec<char>,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub width: u8,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
    pub wide: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: Vec<Vec<Cell>>,
    pub cursor: Option<Cursor>,
}

impl Snapshot {
    pub(crate) fn new<T: EventListener>(term: &Term<T>, palette: &Palette) -> Self {
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let mut rows: Vec<Vec<Cell>> = (0..term.screen_lines())
            .map(|_| Vec::with_capacity(term.columns()))
            .collect();
        for indexed in content.display_iter {
            let Some(row) = rows.get_mut((indexed.point.line.0 + offset) as usize) else {
                continue;
            };
            let flags = indexed.cell.flags;
            let mut foreground = palette.resolve(indexed.cell.fg, content.colors);
            let mut background = palette.resolve(indexed.cell.bg, content.colors);
            if flags.contains(Flags::DIM) {
                foreground = dim(foreground);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut foreground, &mut background);
            }
            if flags.contains(Flags::HIDDEN) {
                foreground = background;
            }
            row.push(Cell {
                ch: indexed.cell.c,
                combining: indexed.cell.zerowidth().unwrap_or_default().to_vec(),
                foreground,
                background,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strikethrough: flags.contains(Flags::STRIKEOUT),
                width: if flags.contains(Flags::WIDE_CHAR) {
                    2
                } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
                    0
                } else {
                    1
                },
                selected: content
                    .selection
                    .is_some_and(|selection| selection.contains(indexed.point)),
            });
        }

        let point = content.cursor.point;
        let shape = match content.cursor.shape {
            TermCursorShape::Block | TermCursorShape::HollowBlock => Some(CursorShape::Block),
            TermCursorShape::Underline => Some(CursorShape::Underline),
            TermCursorShape::Beam => Some(CursorShape::Beam),
            TermCursorShape::Hidden => None,
        };
        let cursor = shape.and_then(|shape| {
            let row = usize::try_from(point.line.0 + offset).ok()?;
            (row < rows.len()).then(|| Cursor {
                row,
                column: point.column.0,
                shape,
                wide: term.grid()[point].flags.contains(Flags::WIDE_CHAR),
            })
        });
        Self { rows, cursor }
    }

    pub fn lines(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
                row.iter()
                    .filter(|cell| cell.width > 0)
                    .flat_map(|cell| std::iter::once(cell.ch).chain(cell.combining.iter().copied()))
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::{event::VoidListener, term::Config, vte::ansi::Processor};

    use super::*;
    use crate::GridSize;

    fn screen(bytes: &[u8]) -> Snapshot {
        let size = GridSize {
            columns: 10,
            rows: 3,
            ..Default::default()
        };
        let mut term = Term::new(Config::default(), &size, VoidListener);
        Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut term, bytes);
        Snapshot::new(&term, &Palette::default())
    }

    #[test]
    fn resolves_colors_attributes_and_wide_characters() {
        let palette = Palette::default();
        let snapshot = screen("\x1b[1;31mre\x1b[0;7md\x1b[0m 世\r\n\x1b[38;2;1;2;3mx".as_bytes());
        let row = &snapshot.rows[0];
        assert_eq!(row.len(), 10);
        assert_eq!(snapshot.lines()[0].trim_end(), "red 世");
        assert!(row[0].bold);
        assert_eq!(row[0].foreground, palette.ansi[1]);
        assert_eq!(row[2].foreground, palette.background);
        assert_eq!(row[2].background, palette.foreground);
        assert_eq!((row[4].ch, row[4].width, row[5].width), ('世', 2, 0));
        assert_eq!(snapshot.rows[1][0].foreground, 0x010203);
        assert_eq!(
            snapshot.cursor,
            Some(Cursor {
                row: 1,
                column: 1,
                shape: CursorShape::Block,
                wide: false,
            })
        );
        assert_eq!(screen(b"\x1b[?25l").cursor, None);
    }

    #[test]
    fn indexed_colors_cover_the_cube_and_grayscale_ramp() {
        let palette = Palette::default();
        let overrides = Colors::default();
        assert_eq!(palette.color(9, &overrides), palette.ansi[9]);
        assert_eq!(palette.color(196, &overrides), 0xff0000);
        assert_eq!(palette.color(21, &overrides), 0x0000ff);
        assert_eq!(palette.color(232, &overrides), 0x080808);
        assert_eq!(palette.color(255, &overrides), 0xeeeeee);
        assert_eq!(palette.color(BACKGROUND, &overrides), palette.background);
        let mut overrides = overrides;
        overrides[1] = Some(to_rgb(0x123456));
        assert_eq!(palette.color(1, &overrides), 0x123456);
    }
}
