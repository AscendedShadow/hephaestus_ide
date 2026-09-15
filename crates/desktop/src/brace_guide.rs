use std::ops::Range;

use gpui::{
    BorderStyle, Bounds, ContentMask, Hsla, PathBuilder, Pixels, TextRun, Window, point, px, quad,
    size,
};
use gpui_component::input::TextGeometry;

use crate::{folding, theme};

const GUIDE_OPACITY: f32 = 0.85;
const BRACE_FILL_OPACITY: f32 = 0.08;
const BRACE_BORDER_OPACITY: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BraceGuide {
    open: Brace,
    close: Brace,
    indent: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Brace {
    line: usize,
    prefix: String,
}

impl Brace {
    fn at(text: &str, offset: usize) -> Self {
        let before = &text[..offset];
        let line_start = before.rfind('\n').map_or(0, |ix| ix + 1);
        Self {
            line: before.bytes().filter(|byte| *byte == b'\n').count(),
            prefix: before[line_start..].to_string(),
        }
    }

    fn indent(&self) -> &str {
        let trimmed = self.prefix.trim_start_matches([' ', '\t']);
        &self.prefix[..self.prefix.len() - trimmed.len()]
    }
}

impl BraceGuide {
    pub fn find(text: &str, cursor: usize) -> Option<Self> {
        let (open, close) = folding::active_pair(text, cursor)?;
        let open = Brace::at(text, open);
        let close = Brace::at(text, close);
        let indent = [open.indent(), close.indent()]
            .into_iter()
            .min_by_key(|indent| indent.len())
            .unwrap_or_default()
            .to_string();
        Some(Self {
            open,
            close,
            indent,
        })
    }

    fn guide_lines(&self) -> Range<usize> {
        self.open.line + 1..self.close.line
    }

    pub fn paint(
        &self,
        geometry: TextGeometry,
        bounds: Bounds<Pixels>,
        font_size: Pixels,
        window: &mut Window,
    ) {
        let width = |text: &str, window: &Window| {
            let run = TextRun {
                len: text.len(),
                font: theme::monospace_font(),
                color: theme::text().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.to_string().into(), font_size, &[run], None)
                .width
        };
        let top = |line: usize| geometry.origin.y + geometry.line_height * line as f32;
        let mask = Bounds::from_corners(
            point(geometry.viewport.left().max(bounds.left()), bounds.top()),
            bounds.bottom_right(),
        );
        let color = Hsla::from(theme::text());

        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            let lines = self.guide_lines();
            let start = top(lines.start).max(mask.top());
            let end = top(lines.end).min(mask.bottom());
            if !lines.is_empty() && start < end {
                let x = geometry.origin.x + width(&self.indent, window);
                let mut builder = PathBuilder::stroke(px(1.));
                builder.move_to(point(x, start));
                builder.line_to(point(x, end));
                if let Ok(path) = builder.build() {
                    window.paint_path(path, color.opacity(GUIDE_OPACITY));
                }
            }

            let brace_width = width("{", window);
            for brace in [&self.open, &self.close] {
                let origin = point(
                    geometry.origin.x + width(&brace.prefix, window),
                    top(brace.line),
                );
                window.paint_quad(quad(
                    Bounds::new(origin, size(brace_width, geometry.line_height)),
                    px(2.),
                    color.opacity(BRACE_FILL_OPACITY),
                    px(1.),
                    color.opacity(BRACE_BORDER_OPACITY),
                    BorderStyle::Solid,
                ));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "fn main() {\n    if ready {\n        run();\n    }\n}\n";

    #[test]
    fn guide_spans_the_lines_between_the_braces_at_the_block_indent() {
        let guide = BraceGuide::find(SOURCE, SOURCE.find("run").unwrap()).unwrap();
        assert_eq!(guide.open.line, 1);
        assert_eq!(guide.open.prefix, "    if ready ");
        assert_eq!(guide.close.line, 3);
        assert_eq!(guide.close.prefix, "    ");
        assert_eq!(guide.indent, "    ");
        assert_eq!(guide.guide_lines(), 2..3);
    }

    #[test]
    fn outer_block_guide_sits_at_column_zero() {
        let guide = BraceGuide::find(SOURCE, SOURCE.find("if").unwrap()).unwrap();
        assert_eq!(guide.open.prefix, "fn main() ");
        assert_eq!(guide.indent, "");
        assert_eq!(guide.guide_lines(), 1..4);
    }

    #[test]
    fn single_line_blocks_only_mark_the_braces() {
        let source = "let s = S { a: 1 };";
        let guide = BraceGuide::find(source, source.find('a').unwrap()).unwrap();
        assert_eq!(guide.open.prefix, "let s = S ");
        assert_eq!(guide.close.prefix, "let s = S { a: 1 ");
        assert!(guide.guide_lines().is_empty());
    }

    #[test]
    fn closing_line_with_less_indent_moves_the_guide_left() {
        let source = "    call(|| {\n        work();\n  });";
        let guide = BraceGuide::find(source, source.find("work").unwrap()).unwrap();
        assert_eq!(guide.indent, "  ");
    }

    #[test]
    fn no_guide_outside_braces() {
        assert_eq!(BraceGuide::find("let x = 1;", 4), None);
    }
}
