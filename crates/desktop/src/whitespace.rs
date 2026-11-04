//! Render whitespace markers (`·` for spaces, `→` for tabs) over the code editor.
//!
//! Markers are positioned by shaping each visible line with the editor font, so they
//! line up with the editor's own layout. With word wrap enabled only the first visual
//! row of each line is marked (documented limitation).

use gpui::{App, Bounds, ContentMask, Font, Hsla, Pixels, TextRun, Window, point};
use gpui_component::{Rope, RopeExt as _, input::TextGeometry};

use crate::theme;

/// Byte offsets and kinds of whitespace characters in `line`.
pub fn markers(line: &str) -> Vec<(usize, char)> {
    line.char_indices()
        .filter_map(|(ix, character)| match character {
            ' ' => Some((ix, '·')),
            '\t' => Some((ix, '→')),
            '\u{00a0}' => Some((ix, '°')),
            _ => None,
        })
        .collect()
}

/// Paint markers for the visible rows of `text` (only those lines are read).
pub fn paint(
    text: &Rope,
    geometry: TextGeometry,
    bounds: Bounds<Pixels>,
    font: Font,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let line_height = geometry.line_height;
    if line_height <= Pixels::ZERO {
        return;
    }
    let first = ((geometry.viewport.top() - geometry.origin.y) / line_height)
        .floor()
        .max(0.) as usize;
    let last = ((geometry.viewport.bottom() - geometry.origin.y) / line_height).ceil() as usize;
    let color: Hsla = Hsla::from(theme::subtle()).opacity(0.7);
    let run = |len: usize| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mask = Bounds::from_corners(
        point(geometry.viewport.left().max(bounds.left()), bounds.top()),
        bounds.bottom_right(),
    );
    window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
        for row in first..=last.min(text.lines_len().saturating_sub(1)) {
            let line = text.slice_line(row).to_string();
            let line = line.as_str();
            let found = markers(line);
            if found.is_empty() {
                continue;
            }
            let shaped = window.text_system().shape_line(
                line.to_string().into(),
                font_size,
                &[run(line.len())],
                None,
            );
            let y = geometry.origin.y + line_height * row as f32;
            for (offset, marker) in found {
                let x = geometry.origin.x + shaped.x_for_index(offset);
                let glyph = marker.to_string();
                let symbol = window.text_system().shape_line(
                    glyph.clone().into(),
                    font_size,
                    &[run(glyph.len())],
                    None,
                );
                let next = offset + line[offset..].chars().next().map_or(1, char::len_utf8);
                let width = shaped.x_for_index(next) - shaped.x_for_index(offset);
                let x = x + (width - symbol.width).max(Pixels::ZERO) / 2.;
                let _ = symbol.paint(point(x, y), line_height, window, cx);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_spaces_tabs_and_non_breaking_spaces() {
        assert_eq!(
            markers("\tlet x = 1;\u{a0}"),
            [(0, '→'), (4, '·'), (6, '·'), (8, '·'), (11, '°')]
        );
        assert!(markers("abc").is_empty());
    }
}
