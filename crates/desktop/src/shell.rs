use std::path::PathBuf;

use gpui::{
    Context, Entity, IntoElement, PathPromptOptions, Render, Subscription, Window, div, prelude::*,
    px,
};
use gpui_component::{
    Disableable as _, TitleBar,
    button::Button,
    input::{Input, InputEvent, InputState},
};
use ide_core::document::Document;

use crate::{commands::*, theme};

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum ToolPanel {
    #[default]
    Terminal,
    Git,
    Debug,
}

impl ToolPanel {
    const ALL: [Self; 3] = [Self::Terminal, Self::Git, Self::Debug];

    fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Git => "Git",
            Self::Debug => "Debug",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Terminal => "PTY terminal integration is coming next.",
            Self::Git => "Repository status and diffs are not implemented yet.",
            Self::Debug => "Debug adapter integration is not implemented yet.",
        }
    }
}

#[derive(Clone, Copy)]
enum PendingAction {
    New,
    Open,
    Close,
}

pub struct IdeShell {
    document: Document,
    editor: Entity<InputState>,
    editor_subscription: Option<Subscription>,
    active_panel: ToolPanel,
    pending: Option<PendingAction>,
    busy: bool,
    dirty: bool,
    status: String,
}

impl IdeShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = Self::make_editor(&Document::default(), window, cx);
        let mut shell = Self {
            document: Document::default(),
            editor,
            editor_subscription: None,
            active_panel: ToolPanel::default(),
            pending: None,
            busy: false,
            dirty: false,
            status: "Ready — open a file or start typing".into(),
        };
        shell.subscribe_editor(window, cx);
        shell
    }

    fn make_editor(
        document: &Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            let editor = InputState::new(window, cx)
                .code_editor("plain_text")
                .line_number(true)
                .soft_wrap(false)
                .default_value(document.text().to_string());
            editor.focus(window, cx);
            editor
        })
    }

    fn subscribe_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_subscription =
            Some(
                cx.subscribe_in(&self.editor, window, |this, editor, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.document.set_text(editor.read(cx).text().clone());
                        this.dirty = this.document.is_dirty();
                        cx.notify();
                    }
                }),
            );
    }

    fn install_document(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A fresh input entity gives each document its own undo history and selection.
        self.editor = Self::make_editor(&document, window, cx);
        self.document = document;
        self.dirty = false;
        self.subscribe_editor(window, cx);
        cx.notify();
    }

    pub fn can_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if self.dirty {
            self.request(PendingAction::Close, window, cx);
            false
        } else {
            true
        }
    }

    fn request(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.pending.is_some() {
            return;
        }
        if self.dirty {
            self.pending = Some(action);
            cx.notify();
        } else {
            self.perform(action, window, cx);
        }
    }

    fn perform(&mut self, action: PendingAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            PendingAction::New => {
                self.install_document(Document::default(), window, cx);
                self.status = "New document".into();
            }
            PendingAction::Open => self.open(window, cx),
            PendingAction::Close => window.remove_window(),
        }
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        self.status = "Choose a UTF-8 text file…".into();
        cx.notify();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open file".into()),
        });
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let selected = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = selected.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        Document::open(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(document)) => {
                        this.status = format!("Opened {}", document.name());
                        this.install_document(document, window, cx);
                    }
                    Ok(None) => this.status = "Open cancelled".into(),
                    Err(error) => this.status = format!("Open failed: {error}"),
                }
                this.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Saving…".into();
        cx.notify();
        let existing_path = if save_as {
            None
        } else {
            self.document.path().map(PathBuf::from)
        };
        let picker = if existing_path.is_none() {
            let directory = self
                .document
                .path()
                .and_then(|path| path.parent())
                .unwrap_or(std::path::Path::new("."));
            Some(cx.prompt_for_new_path(directory, Some(&self.document.name())))
        } else {
            None
        };
        let snapshot = self.document.clone();
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<Option<Document>, String> = async {
                let path = if let Some(picker) = picker {
                    picker
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| e.to_string())?
                } else {
                    existing_path
                };
                let Some(path) = path else {
                    return Ok(None);
                };
                executor
                    .spawn(async move {
                        snapshot
                            .save_to(&path)
                            .map(Some)
                            .map_err(|error| format!("{}: {error}", path.display()))
                    })
                    .await
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some(saved)) => {
                        this.document.accept_saved(saved);
                        this.dirty = this.document.is_dirty();
                        this.status = format!("Saved {}", this.document.name());
                        if !this.dirty
                            && let Some(action) = this.pending.take()
                        {
                            this.perform(action, window, cx);
                        }
                    }
                    Ok(None) => this.status = "Save cancelled".into(),
                    Err(error) => this.status = format!("Save failed: {error}"),
                }
                this.editor
                    .update(cx, |editor, cx| editor.focus(window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::New, window, cx);
    }

    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::Open, window, cx);
    }

    fn save_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        self.save(false, window, cx);
    }

    fn save_file_as(&mut self, _: &SaveFileAs, window: &mut Window, cx: &mut Context<Self>) {
        self.save(true, window, cx);
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.request(PendingAction::Close, window, cx);
    }
}

impl Render for IdeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = format!(
            "{}{}",
            self.document.name(),
            if self.dirty { " •" } else { "" }
        );
        window.set_window_title(&format!("{name} — Hephaestus"));
        let blocked = self.busy || self.pending.is_some();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_color(theme::text())
            .text_sm()
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::save_file))
            .on_action(cx.listener(Self::save_file_as))
            .on_action(cx.listener(Self::close_window))
            .child(
                TitleBar::new()
                    .bg(theme::panel())
                    .text_color(theme::text())
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .child("Hephaestus")
                            .child(div().text_color(theme::muted()).child(name.clone())),
                    )
                    .on_close_window(cx.listener(|this, _, window, cx| {
                        this.request(PendingAction::Close, window, cx)
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        Button::new("new-file")
                            .label("New")
                            .disabled(blocked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.request(PendingAction::New, window, cx)
                            })),
                    )
                    .child(
                        Button::new("open-file")
                            .label("Open…")
                            .disabled(blocked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.request(PendingAction::Open, window, cx)
                            })),
                    )
                    .child(
                        Button::new("save-file")
                            .label("Save")
                            .disabled(blocked)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("save-file-as")
                            .label("Save As…")
                            .disabled(blocked)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save(true, window, cx)),
                            ),
                    ),
            )
            .when(self.pending.is_some(), |view| {
                view.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .bg(theme::panel())
                        .border_b_1()
                        .border_color(theme::accent())
                        .child(div().flex_1().child("Save changes before continuing?"))
                        .child(
                            Button::new("confirm-save")
                                .label("Save")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                                ),
                        )
                        .child(
                            Button::new("discard")
                                .label("Discard")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(action) = this.pending.take() {
                                        this.perform(action, window, cx);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("cancel")
                                .label("Cancel")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.pending = None;
                                    this.editor
                                        .update(cx, |editor, cx| editor.focus(window, cx));
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(px(200.))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .p_4()
                            .bg(theme::panel())
                            .border_r_1()
                            .border_color(theme::border())
                            .child("DOCUMENT")
                            .child(name.clone())
                            .child(
                                div().text_color(theme::muted()).child(
                                    self.document
                                        .path()
                                        .map(|path| path.display().to_string())
                                        .unwrap_or_else(|| "Not saved to disk".into()),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                div()
                                    .h(px(34.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .px_4()
                                    .border_b_1()
                                    .border_color(theme::border())
                                    .child(name),
                            )
                            .child(
                                div().flex_1().min_h_0().overflow_hidden().child(
                                    Input::new(&self.editor)
                                        .h_full()
                                        .w_full()
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .appearance(false)
                                        .disabled(blocked)
                                        .font_family(if cfg!(target_os = "windows") {
                                            "Consolas"
                                        } else {
                                            "monospace"
                                        })
                                        .text_size(px(14.))
                                        .bg(theme::background())
                                        .text_color(theme::text()),
                                ),
                            )
                            .child(
                                div()
                                    .h(px(110.))
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .border_t_1()
                                    .border_color(theme::border())
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .px_3()
                                            .py_1()
                                            .bg(theme::panel())
                                            .children(ToolPanel::ALL.into_iter().map(|panel| {
                                                div()
                                                    .id(panel.label())
                                                    .px_3()
                                                    .py_1()
                                                    .cursor_pointer()
                                                    .text_color(if self.active_panel == panel {
                                                        theme::accent()
                                                    } else {
                                                        theme::muted()
                                                    })
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.active_panel = panel;
                                                        cx.notify();
                                                    }))
                                                    .child(panel.label())
                                            })),
                                    )
                                    .child(
                                        div()
                                            .p_3()
                                            .text_color(theme::muted())
                                            .child(self.active_panel.placeholder()),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .min_h(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .bg(theme::panel())
                    .border_t_1()
                    .border_color(theme::border())
                    .text_color(theme::muted())
                    .child(div().flex_1().min_w_0().child(self.status.clone()))
                    .child(format!(
                        "{} | UTF-8 | {}",
                        if self.dirty { "Modified" } else { "Saved" },
                        self.document.line_ending().label()
                    )),
            )
    }
}
