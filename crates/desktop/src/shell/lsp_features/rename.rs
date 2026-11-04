//! Renaming symbols through the language server.

use super::*;

impl IdeShell {
    pub(in crate::shell) fn rename_symbol(
        &mut self,
        _: &RenameSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.rename {
            return self.unsupported("Rename Symbol", &target, cx);
        }
        let text = self.buffers[target.ix].document.text().to_string();
        let word_at =
            |text: &str, position: lsp_types::Position| -> Option<(Range<usize>, String)> {
                let offset =
                    ide_core::text_edit::offset(text, position.line, position.character).ok()?;
                let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
                let start = text[..offset]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map_or(offset, |(ix, _)| ix);
                let end = text[offset..]
                    .char_indices()
                    .find(|(_, c)| !is_word(*c))
                    .map_or(text.len(), |(ix, _)| offset + ix);
                (start < end).then(|| (start..end, text[start..end].to_string()))
            };
        if !target.features.prepare_rename {
            let Some((_, name)) = word_at(&text, target.position) else {
                self.status = "No symbol to rename here".into();
                cx.notify();
                return;
            };
            return self.prompt_rename(target, name, window, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let Some(request) = self
            .servers
            .read(cx)
            .send::<PrepareRenameRequest>(target.language, position)
        else {
            return;
        };
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let prepared = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Rename", cx) {
                    return;
                }
                let text = this.buffers[target.ix].document.text().to_string();
                let name = match prepared {
                    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                        placeholder, ..
                    })) => Some(placeholder),
                    Ok(Some(PrepareRenameResponse::Range(range))) => {
                        let start = ide_core::text_edit::offset(
                            &text,
                            range.start.line,
                            range.start.character,
                        )
                        .ok();
                        let end =
                            ide_core::text_edit::offset(&text, range.end.line, range.end.character)
                                .ok();
                        start
                            .zip(end)
                            .and_then(|(start, end)| text.get(start..end))
                            .map(str::to_string)
                    }
                    Ok(Some(PrepareRenameResponse::DefaultBehavior { .. })) => {
                        word_at(&text, target.position).map(|(_, name)| name)
                    }
                    Ok(None) => {
                        this.status = "This symbol cannot be renamed".into();
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        this.status = format!("Cannot rename: {error}");
                        cx.notify();
                        return;
                    }
                };
                match name {
                    Some(name) => this.prompt_rename(target, name, window, cx),
                    None => {
                        this.status = "No symbol to rename here".into();
                        cx.notify();
                    }
                }
            });
        });
    }

    pub(in crate::shell) fn prompt_rename(
        &mut self,
        target: Target,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current.clone())
                .validate(|text, _| !text.contains('\n'))
        });
        input.update(cx, |input, cx| {
            let len = current.len();
            input.select_byte_range(0..len, false, window, cx);
        });
        let shell = cx.entity().downgrade();
        let target = Rc::new(target);
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let (shell, input, target) = (shell.clone(), input.clone(), target.clone());
                dialog
                    .title(format!("Rename {current}"))
                    .w(px(400.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Rename"))
                    .on_ok(move |_, window, cx| {
                        let name = input.read(cx).value().trim().to_string();
                        if name.is_empty() {
                            return false;
                        }
                        let _ = shell.update(cx, |shell, cx| {
                            shell.focus_editor(window, cx);
                            shell.perform_rename(&target, name, window, cx)
                        });
                        true
                    })
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(in crate::shell) fn perform_rename(
        &mut self,
        target: &Target,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let Some(request) = self.servers.read(cx).send::<Rename>(
            target.language,
            RenameParams {
                text_document_position: position,
                new_name: name.clone(),
                work_done_progress_params: Default::default(),
            },
        ) else {
            return;
        };
        let target = Target {
            ix: target.ix,
            id: target.id,
            path: target.path.clone(),
            language: target.language,
            position: target.position,
            origin: target.origin.clone(),
            features: target.features,
        };
        self.status = format!("Renaming to {name}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let edit = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Rename", cx) {
                    return;
                }
                match edit {
                    Ok(Some(edit)) => this.request_workspace_edit(
                        edit,
                        format!("Rename to {name}"),
                        true,
                        window,
                        cx,
                    ),
                    Ok(None) => this.status = "The language server returned no rename edits".into(),
                    Err(error) => this.status = format!("Rename failed: {error}"),
                }
                cx.notify();
            });
        });
    }
}
