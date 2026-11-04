//! The Outline sidebar view (IDE-009): document symbols of the active buffer with the
//! symbol under the cursor highlighted. Refreshes after edits (debounced) and when the
//! active buffer changes; responses for other documents or older versions are ignored.

use std::time::Duration;

use super::lsp_features::{Symbol, kind_label};
use super::*;

const REFRESH_DELAY: Duration = Duration::from_millis(400);

impl IdeShell {
    pub(super) fn show_outline(
        &mut self,
        _: &ShowOutline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_sidebar = SidebarPanel::Outline;
        self.show_sidebar = true;
        self.refresh_outline(window, cx);
        cx.notify();
    }

    fn outline_visible(&self) -> bool {
        self.show_sidebar && self.active_sidebar == SidebarPanel::Outline
    }

    pub(super) fn refresh_outline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_document_symbols(true, |_, _, _, cx| cx.notify(), window, cx);
    }

    /// Schedule an outline refresh after an edit (only while the outline is visible).
    pub(super) fn schedule_outline_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.outline_visible() {
            return;
        }
        self.outline_timer = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(REFRESH_DELAY).await;
            let _ = this.update_in(cx, |this, window, cx| this.refresh_outline(window, cx));
        });
    }

    /// Symbols for the active buffer, if the latest outline belongs to it.
    pub(super) fn active_outline(&self) -> Option<&[Symbol]> {
        let (id, _, symbols) = self.outline.as_ref()?;
        (*id == self.buffer().id).then_some(symbols.as_slice())
    }

    /// The deepest symbol containing the cursor.
    pub(super) fn current_symbol(&self, cx: &App) -> Option<usize> {
        let symbols = self.active_outline()?;
        let (selection, _) = self.source_selection(self.active, cx);
        let cursor = selection.start;
        symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.range.start <= cursor && cursor <= symbol.range.end)
            .max_by_key(|(_, symbol)| symbol.depth)
            .map(|(ix, _)| ix)
    }

    fn reveal_symbol(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(symbol) = self
            .active_outline()
            .and_then(|symbols| symbols.get(ix))
            .cloned()
        else {
            return;
        };
        let Some(path) = self.document().path().map(Path::to_path_buf) else {
            return;
        };
        let text = self.document().text().to_string();
        let (line, character) = ide_core::text_edit::position(&text, symbol.selection.start);
        let location = SearchMatch {
            path,
            line: line as usize + 1,
            column: character as usize + 1,
            preview: String::new(),
        };
        self.record_location(&location, cx);
        self.reveal_declaration(self.active, symbol.selection.clone(), window, cx);
    }

    pub(super) fn render_outline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.buffer().language;
        let features = self.servers.read(cx).features(language);
        let has_server = self
            .servers
            .read(cx)
            .server_capabilities(language)
            .is_some();
        let current = self.current_symbol(cx);
        let body = match self.active_outline() {
            Some(symbols) if !symbols.is_empty() => div()
                .id("outline-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_1p5()
                .children(symbols.iter().enumerate().map(|(ix, symbol)| {
                    div()
                        .id(("outline-row", ix))
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pl(px(6. + 12. * symbol.depth as f32))
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::hover()))
                        .when(current == Some(ix), |row| row.bg(theme::active_row()))
                        .child(div().truncate().child(symbol.name.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::subtle())
                                .child(kind_label(symbol.kind)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reveal_symbol(ix, window, cx)
                        }))
                }))
                .into_any_element(),
            _ => {
                let message = if self.document().path().is_none() {
                    "Save the file to see its outline".to_string()
                } else if !has_server {
                    format!(
                        "Outline unavailable: no language server is running for {}",
                        ide_core::language::get(language).name
                    )
                } else if !features.document_symbols {
                    "Outline unavailable: the language server does not provide document symbols"
                        .to_string()
                } else {
                    "No symbols".to_string()
                };
                ui::empty_state(Icon::new(IconName::Inbox).size(px(24.)), message)
                    .into_any_element()
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                ui::panel_header("OUTLINE").child(
                    Button::new("outline-refresh")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Loader)
                        .tooltip("Refresh")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.refresh_outline(window, cx)),
                        ),
                ),
            )
            .child(body)
    }
}
