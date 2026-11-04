//! Code actions (quick fixes, refactors, source actions) and server commands.

use super::*;

impl IdeShell {
    pub(in crate::shell) fn request_code_actions(
        &mut self,
        only: Option<Vec<lsp_types::CodeActionKind>>,
        what: &'static str,
        auto_apply_single: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.code_actions {
            return self.unsupported(what, &target, cx);
        }
        let Some(document) = Servers::text_document(&target.path) else {
            return;
        };
        let range = self.source_range(target.ix, cx);
        let diagnostics: Vec<lsp_types::Diagnostic> = self
            .lsp_diagnostics
            .get(&target.path)
            .map(|list| {
                list.iter()
                    .filter(|diagnostic| {
                        diagnostic.range.start <= range.end && range.start <= diagnostic.range.end
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let Some(request) = self.servers.read(cx).send::<CodeActionRequest>(
            target.language,
            CodeActionParams {
                text_document: document,
                range,
                context: CodeActionContext {
                    diagnostics,
                    only,
                    trigger_kind: Some(lsp_types::CodeActionTriggerKind::INVOKED),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        ) else {
            return;
        };
        self.status = format!("{what}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, what, cx) || this.buffer().id != target.id {
                    return;
                }
                let actions = match response {
                    Ok(actions) => actions.unwrap_or_default(),
                    Err(error) => {
                        this.status = format!("{what} failed: {error}");
                        cx.notify();
                        return;
                    }
                };
                if actions.is_empty() {
                    this.status = format!("No {} available here", what.to_lowercase());
                    cx.notify();
                    return;
                }
                let enabled = actions
                    .iter()
                    .filter(|action| !matches!(action, CodeActionOrCommand::CodeAction(action) if action.disabled.is_some()))
                    .count();
                this.code_actions = Some((target, actions));
                if auto_apply_single && enabled == 1 {
                    let index = this
                        .code_actions
                        .as_ref()
                        .and_then(|(_, actions)| {
                            actions.iter().position(|action| {
                                !matches!(action, CodeActionOrCommand::CodeAction(action) if action.disabled.is_some())
                            })
                        })
                        .unwrap_or(0);
                    return this.run_code_action(index, window, cx);
                }
                let items = this
                    .code_actions
                    .as_ref()
                    .map(|(_, actions)| {
                        actions
                            .iter()
                            .enumerate()
                            .map(|(ix, action)| {
                                let (title, detail) = match action {
                                    CodeActionOrCommand::Command(command) => (command.title.clone(), String::new()),
                                    CodeActionOrCommand::CodeAction(action) => (
                                        format!(
                                            "{}{}",
                                            if action.is_preferred == Some(true) { "★ " } else { "" },
                                            action.title
                                        ),
                                        match &action.disabled {
                                            Some(disabled) => format!("disabled: {}", disabled.reason),
                                            None => action
                                                .kind
                                                .as_ref()
                                                .map(|kind| kind.as_str().to_string())
                                                .unwrap_or_default(),
                                        },
                                    ),
                                };
                                PickerItem {
                                    filter: title.clone(),
                                    label: title,
                                    detail,
                                    depth: 0,
                                    action: PickerAction::CodeAction(ix),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                this.show_picker(PickerKind::CodeActions, what.to_string(), items, window, cx);
            });
        });
    }

    pub(in crate::shell) fn quick_fix(
        &mut self,
        _: &QuickFix,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(None, "Quick Fix", false, window, cx);
    }

    pub(in crate::shell) fn refactor(
        &mut self,
        _: &Refactor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::REFACTOR]),
            "Refactor",
            false,
            window,
            cx,
        );
    }

    pub(in crate::shell) fn source_action(
        &mut self,
        _: &SourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::SOURCE]),
            "Source Action",
            false,
            window,
            cx,
        );
    }

    pub(in crate::shell) fn organize_imports(
        &mut self,
        _: &OrganizeImports,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
            "Organize Imports",
            true,
            window,
            cx,
        );
    }

    /// Run the chosen code action: resolve it lazily when it has no edit, apply its edit
    /// through the shared applier, then execute its command.
    pub(in crate::shell) fn run_code_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, actions)) = self.code_actions.take() else {
            return;
        };
        let Some(action) = actions.get(index).cloned() else {
            return;
        };
        if !self.servers.read(cx).is_current(&target.origin) {
            self.status = "The document changed; run the code action request again".into();
            cx.notify();
            return;
        }
        match action {
            CodeActionOrCommand::Command(command) => {
                self.run_command_for(&target, command, window, cx)
            }
            CodeActionOrCommand::CodeAction(action) => {
                if let Some(disabled) = &action.disabled {
                    self.status = format!("{} is unavailable: {}", action.title, disabled.reason);
                    cx.notify();
                    return;
                }
                let needs_resolve = action.edit.is_none() && action.data.is_some();
                if needs_resolve && target.features.resolve_code_actions {
                    let Some(request) = self
                        .servers
                        .read(cx)
                        .send::<CodeActionResolveRequest>(target.language, action.clone())
                    else {
                        return;
                    };
                    self.lsp_task = cx.spawn_in(window, async move |this, cx| {
                        let resolved = request.await;
                        let _ = this.update_in(cx, |this, window, cx| match resolved {
                            Ok(resolved) => this.finish_code_action(&target, resolved, window, cx),
                            Err(error) => {
                                this.status =
                                    format!("Could not resolve {}: {error}", action.title);
                                cx.notify();
                            }
                        });
                    });
                } else {
                    self.finish_code_action(&target, action, window, cx);
                }
            }
        }
    }

    pub(in crate::shell) fn finish_code_action(
        &mut self,
        target: &Target,
        action: lsp_types::CodeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.still_current(target, &action.title, cx) {
            return;
        }
        if let Some(edit) = action.edit.clone() {
            let plan = self.plan_workspace_edit(&edit, cx);
            match plan {
                Ok(plan) => match self.apply_plan(&plan, &action.title, window, cx) {
                    Ok(summary) => self.status = summary,
                    Err(error) => {
                        self.status = error;
                        cx.notify();
                        return;
                    }
                },
                Err(error) => {
                    self.status = format!("{} not applied: {error}", action.title);
                    cx.notify();
                    return;
                }
            }
        }
        if let Some(command) = action.command {
            self.run_command_for(target, command, window, cx);
        } else if action.edit.is_none() {
            self.status = format!("{} has nothing to apply", action.title);
        }
        cx.notify();
    }

    pub(in crate::shell) fn run_command_for(
        &mut self,
        target: &Target,
        command: lsp_types::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !target.features.execute_command {
            self.status = format!("The server cannot run {}", command.title);
            cx.notify();
            return;
        }
        let Some(request) = self.servers.read(cx).send::<ExecuteCommand>(
            target.language,
            lsp_types::ExecuteCommandParams {
                command: command.command.clone(),
                arguments: command.arguments.clone().unwrap_or_default(),
                work_done_progress_params: Default::default(),
            },
        ) else {
            return;
        };
        let title = command.title.clone();
        self.status = format!("Running {title}…");
        cx.notify();
        // Edits produced by the command arrive as workspace/applyEdit requests.
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.status = format!("{title} failed: {error}");
                } else if this.status.starts_with("Running ") {
                    this.status = format!("Ran {title}");
                }
                cx.notify();
            });
        });
    }
}
