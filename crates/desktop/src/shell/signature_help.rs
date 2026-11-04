//! Signature help (IDE-011): a caret-anchored popover with the active signature and
//! parameter, overload navigation and documentation.
//!
//! Requests fire on the server's trigger characters (and retrigger characters while the
//! popover is open), after edits while it is open, and through Trigger Parameter Hints.
//! Each request carries a generation number; dismissing (Escape, leaving the call, or a
//! `null` response) bumps the generation so an older response can never reopen it.
//! Positions are source positions, so folds do not shift them.

use gpui::{anchored, deferred, point};
use lsp_types::{
    Documentation, ParameterLabel, SignatureHelp, SignatureHelpContext, SignatureHelpParams,
    SignatureHelpTriggerKind, request::SignatureHelpRequest,
};

use super::*;

#[derive(Default)]
pub(super) struct SignatureState {
    pub help: Option<SignatureHelp>,
    /// Overload chosen by the user (overrides the server's active signature).
    pub selected: Option<usize>,
    pub buffer: Option<BufferId>,
    generation: u64,
    task: Option<Task<()>>,
}

/// Split a signature label into (before, active parameter, after).
pub(super) fn split_label(
    help: &SignatureHelp,
    signature: usize,
) -> Option<(String, String, String)> {
    let information = help.signatures.get(signature)?;
    let label = &information.label;
    let active = information
        .active_parameter
        .or(help.active_parameter)
        .unwrap_or(0) as usize;
    let parameter = information.parameters.as_ref()?.get(active)?;
    let range = match &parameter.label {
        ParameterLabel::LabelOffsets([start, end]) => {
            // Offsets are UTF-16 code units.
            let to_byte = |units: u32| {
                let mut count = 0u32;
                for (ix, character) in label.char_indices() {
                    if count >= units {
                        return ix;
                    }
                    count += character.len_utf16() as u32;
                }
                label.len()
            };
            to_byte(*start)..to_byte(*end)
        }
        ParameterLabel::Simple(text) => {
            // Search after the opening parenthesis to avoid matching the function name.
            let from = label.find('(').map_or(0, |ix| ix + 1);
            let at = label[from..].find(text.as_str())? + from;
            at..at + text.len()
        }
    };
    Some((
        label[..range.start].to_string(),
        label[range.clone()].to_string(),
        label[range.end..].to_string(),
    ))
}

fn documentation_text(documentation: &Option<Documentation>) -> Option<String> {
    match documentation.as_ref()? {
        Documentation::String(text) => Some(text.clone()),
        Documentation::MarkupContent(markup) => Some(markup.value.clone()),
    }
    .filter(|text| !text.trim().is_empty())
}

impl IdeShell {
    /// Called after an edit to the active buffer.
    pub(super) fn signature_help_after_edit(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix != self.active {
            return;
        }
        let buffer = &self.buffers[ix];
        let Some(capabilities) = self.servers.read(cx).server_capabilities(buffer.language) else {
            return;
        };
        let Some(options) = capabilities.signature_help_provider else {
            return;
        };
        let (selection, _) = self.source_selection(ix, cx);
        let text = buffer.document.text();
        // Only the character before the caret matters; read it from the rope.
        let Some(character) = text.chars_at(selection.end.min(text.len())).prev() else {
            return;
        };
        let character = character.to_string();
        let open = self.signature.help.is_some();
        let trigger = options
            .trigger_characters
            .unwrap_or_default()
            .contains(&character);
        let retrigger = open
            && options
                .retrigger_characters
                .unwrap_or_default()
                .contains(&character);
        if trigger || retrigger {
            let kind = SignatureHelpTriggerKind::TRIGGER_CHARACTER;
            self.request_signature_help(kind, Some(character), window, cx);
        } else if open {
            self.request_signature_help(SignatureHelpTriggerKind::CONTENT_CHANGE, None, window, cx);
        }
    }

    pub(super) fn trigger_parameter_hints(
        &mut self,
        _: &TriggerParameterHints,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.trigger_signature_help(window, cx);
    }

    pub(super) fn trigger_signature_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.buffer().language;
        if !self.servers.read(cx).features(language).signature_help {
            self.status = "Parameter hints are not available for this file".into();
            cx.notify();
            return;
        }
        self.request_signature_help(SignatureHelpTriggerKind::INVOKED, None, window, cx);
    }

    fn request_signature_help(
        &mut self,
        kind: SignatureHelpTriggerKind,
        character: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.active;
        let buffer = &self.buffers[ix];
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            return;
        };
        let language = buffer.language;
        let id = buffer.id;
        let (selection, _) = self.source_selection(ix, cx);
        let text = buffer.document.text().to_string();
        let (line, column) = ide_core::text_edit::position(&text, selection.end);
        let Some(position) = language_server::Servers::position_params(
            &path,
            lsp_types::Position::new(line, column),
        ) else {
            return;
        };
        let request = self.servers.read(cx).send::<SignatureHelpRequest>(
            language,
            SignatureHelpParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
                context: Some(SignatureHelpContext {
                    trigger_kind: kind,
                    trigger_character: character,
                    is_retrigger: self.signature.help.is_some(),
                    active_signature_help: self.signature.help.clone(),
                }),
            },
        );
        let Some(request) = request else { return };
        self.signature.generation += 1;
        let generation = self.signature.generation;
        let origin = self.servers.read(cx).origin(&path);
        self.signature.task = Some(cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update(cx, |this, cx| {
                // Only the latest request for the current document version may update it.
                if this.signature.generation != generation
                    || !this.servers.read(cx).is_current(&origin)
                    || this.buffer().id != id
                {
                    return;
                }
                match response {
                    Ok(Some(help)) if !help.signatures.is_empty() => {
                        if this.signature.buffer != Some(id) {
                            this.signature.selected = None;
                        }
                        this.signature.help = Some(help);
                        this.signature.buffer = Some(id);
                    }
                    _ => this.dismiss_signature_help(),
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn dismiss_signature_help(&mut self) {
        self.signature.generation += 1;
        self.signature.help = None;
        self.signature.selected = None;
        self.signature.buffer = None;
        self.signature.task = None;
    }

    pub(super) fn active_signature(&self) -> Option<usize> {
        let help = self.signature.help.as_ref()?;
        let count = help.signatures.len();
        Some(
            self.signature
                .selected
                .unwrap_or(help.active_signature.unwrap_or(0) as usize)
                .min(count.saturating_sub(1)),
        )
    }

    pub(super) fn cycle_signature(&mut self, step: isize, cx: &mut Context<Self>) {
        let (Some(active), Some(help)) = (self.active_signature(), self.signature.help.as_ref())
        else {
            return;
        };
        let count = help.signatures.len() as isize;
        self.signature.selected = Some((active as isize + step).rem_euclid(count) as usize);
        cx.notify();
    }

    pub(super) fn next_signature(
        &mut self,
        _: &NextSignature,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_signature(1, cx);
    }

    pub(super) fn previous_signature(
        &mut self,
        _: &PreviousSignature,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_signature(-1, cx);
    }

    pub(super) fn render_signature_help(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let help = self.signature.help.as_ref()?;
        if self.signature.buffer != Some(self.buffer().id) {
            return None;
        }
        let active = self.active_signature()?;
        let bounds = self.editor().read(cx).cursor_bounds_in_window()?;
        let signature = &help.signatures[active];
        let (before, parameter, after) = split_label(help, active).unwrap_or((
            signature.label.clone(),
            String::new(),
            String::new(),
        ));
        let parameter_doc = signature
            .parameters
            .as_ref()
            .and_then(|parameters| {
                let index = signature
                    .active_parameter
                    .or(help.active_parameter)
                    .unwrap_or(0) as usize;
                parameters.get(index)
            })
            .and_then(|parameter| documentation_text(&parameter.documentation));
        let documentation = documentation_text(&signature.documentation);
        let count = help.signatures.len();
        let origin = point(bounds.origin.x, bounds.origin.y - px(6.));
        Some(deferred(
            anchored()
                .position(origin)
                .anchor(gpui::Corner::BottomLeft)
                .child(
                    div()
                        .debug_selector(|| "signature-help".into())
                        .max_w(px(560.))
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::panel())
                        .shadow_md()
                        .text_xs()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(count > 1, |row| {
                                    row.child(
                                        div()
                                            .id("signature-previous")
                                            .cursor_pointer()
                                            .text_color(theme::muted())
                                            .child("‹")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cycle_signature(-1, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_color(theme::muted())
                                            .child(format!("{}/{count}", active + 1)),
                                    )
                                    .child(
                                        div()
                                            .id("signature-next")
                                            .cursor_pointer()
                                            .text_color(theme::muted())
                                            .child("›")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cycle_signature(1, cx)
                                            })),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .font_family(theme::mono_family())
                                        .child(before)
                                        .child(
                                            div()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(theme::accent())
                                                .underline()
                                                .child(parameter),
                                        )
                                        .child(after),
                                ),
                        )
                        .when_some(parameter_doc, |popover, doc| {
                            popover.child(div().text_color(theme::text()).child(doc))
                        })
                        .when_some(documentation, |popover, doc| {
                            popover.child(div().text_color(theme::muted()).child(doc))
                        }),
                ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn help(json: serde_json::Value) -> SignatureHelp {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn labels_split_by_string_or_utf16_offsets() {
        let by_offsets = help(serde_json::json!({
            "signatures": [{"label": "fn größe(ä: u8, 😀: u8)", "parameters": [
                {"label": [9, 14]}, {"label": [16, 22]}]}],
            "activeParameter": 1
        }));
        assert_eq!(
            split_label(&by_offsets, 0).unwrap(),
            ("fn größe(ä: u8, ".into(), "😀: u8".into(), ")".into())
        );
        let by_text = help(serde_json::json!({
            "signatures": [{"label": "max(a, max)", "activeParameter": 1, "parameters": [
                {"label": "a"}, {"label": "max"}]}]
        }));
        assert_eq!(split_label(&by_text, 0).unwrap().1, "max");
        assert_eq!(split_label(&by_text, 0).unwrap().0, "max(a, ");
    }
}
