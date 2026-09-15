//! Small building blocks shared by the shell and its panels.

use gpui::{Div, FontWeight, SharedString, div, prelude::*, px};

use crate::theme;

/// Height of every panel header, so neighbouring panels line up.
pub const HEADER_HEIGHT: f32 = 34.;

/// Group name for rows whose actions appear on hover.
pub const ROW_GROUP: &str = "row";

/// A panel header strip: caption on the left, actions on the right.
pub fn panel_header(caption_text: impl Into<SharedString>) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .pl_4()
        .pr_2()
        .child(caption(caption_text).flex_1().min_w_0().truncate())
}

/// Small uppercase label for panels and list sections.
pub fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::muted())
        .child(text.into())
}

/// Rounded pill showing a count next to a section title.
pub fn count_badge(count: usize) -> Div {
    div()
        .h(px(16.))
        .min_w(px(18.))
        .px_1()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::hover())
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::muted())
        .child(count.to_string())
}

/// Centred placeholder for panels with nothing to show yet.
pub fn empty_state(icon: impl IntoElement, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .px_6()
        .pt_10()
        .text_center()
        .child(div().text_color(theme::subtle()).child(icon))
        .child(div().text_color(theme::muted()).child(title.into()))
}
