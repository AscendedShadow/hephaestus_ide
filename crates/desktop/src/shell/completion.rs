//! Completion acceptance and snippet tab stops (IDE-010).

use gpui::KeyDownEvent;
use lsp_types::{
    CompletionItem, CompletionTextEdit, InsertTextFormat, request::ResolveCompletionItem,
};

use super::*;
use crate::completion::{self, Hooks};

/// An active snippet: tab stops (source byte ranges) in one buffer.
pub(super) struct SnippetSession {
    buffer: BufferId,
    stops: Vec<Range<usize>>,
    current: usize,
    /// Source text after the last known edit, to track stop positions while typing.
    text: String,
}

impl IdeShell {
    /// Install the completion provider on a buffer's editor.
    pub(super) fn attach_completion(
        &self,
        id: BufferId,
        editor: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) {
        let source_shell = cx.entity().downgrade();
        let accept_shell = cx.entity().downgrade();
        let hooks = Hooks {
            source: Box::new(move |display, cx| {
                let shell = source_shell.upgrade()?;
                let shell = shell.read(cx);
                let buffer = &shell.buffers[shell.buffer_index(id)?];
                let path = buffer.document.path()?.to_path_buf();
                let offset = folding::display_to_source(&buffer.folds, display, false);
                Some((
                    path,
                    buffer.language,
                    offset,
                    buffer.document.text().to_string(),
                ))
            }),
            accept: Box::new(move |item, window, cx| {
                let _ = accept_shell.update(cx, |shell, cx| {
                    shell.accept_completion(id, item, window, cx)
                });
            }),
        };
        completion::attach(editor, &self.servers, hooks, cx);
    }

    pub(super) fn complete_code(
        &mut self,
        _: &CompleteCode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff || self.blocked() {
            return;
        }
        let language = self.buffer().language;
        if self
            .servers
            .read(cx)
            .server_capabilities(language)
            .is_none()
        {
            self.status = "No language server is running for this file".into();
            cx.notify();
            return;
        }
        self.editor()
            .clone()
            .update(cx, |editor, cx| editor.show_completions(window, cx));
    }

    /// Accept a completion item: resolve it first when the server supports lazy
    /// resolution, then apply it.
    pub(super) fn accept_completion(
        &mut self,
        id: BufferId,
        item: CompletionItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let language = self.buffers[ix].language;
        let features = self.servers.read(cx).features(language);
        let text = self.buffers[ix].document.text().to_string();
        let needs_resolve = features.resolve_completion
            && (item.additional_text_edits.is_none() || item.documentation.is_none());
        if !needs_resolve {
            return self.apply_completion_item(id, item, &text, window, cx);
        }
        let Some(request) = self
            .servers
            .read(cx)
            .send::<ResolveCompletionItem>(language, item.clone())
        else {
            return self.apply_completion_item(id, item, &text, window, cx);
        };
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let resolved = request.await.unwrap_or(item);
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_completion_item(id, resolved, &text, window, cx)
            });
        });
    }

    /// Apply an item's exact edit, additional edits and snippet as one undo step.
    /// `requested` is the source text when acceptance started; if the buffer changed
    /// since, other than by typing at the cursor, the item is stale and is not applied.
    fn apply_completion_item(
        &mut self,
        id: BufferId,
        item: CompletionItem,
        requested: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.buffer_index(id) else {
            return;
        };
        let text = self.buffers[ix].document.text().to_string();
        if text != requested {
            self.status = format!("Completion {} discarded: the document changed", item.label);
            cx.notify();
            return;
        }
        let (selection, _) = self.source_selection(ix, cx);
        let cursor = selection.end;
        let offset = |position: lsp_types::Position| {
            ide_core::text_edit::offset(&text, position.line, position.character).ok()
        };
        let (range, new_text) = match &item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (
                offset(edit.range.start).zip(offset(edit.range.end)),
                edit.new_text.clone(),
            ),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => (
                offset(edit.replace.start).zip(offset(edit.replace.end)),
                edit.new_text.clone(),
            ),
            None => (
                Some((completion::word_start(&text, cursor), cursor)),
                item.insert_text
                    .clone()
                    .unwrap_or_else(|| item.label.clone()),
            ),
        };
        let Some((start, mut end)) = range else {
            self.status = format!("Completion {} has an invalid range", item.label);
            cx.notify();
            return;
        };
        // Characters typed after the request extend the replaced range to the cursor.
        if end < cursor && !text[end..cursor].contains('\n') {
            end = cursor;
        }
        let snippet = item.insert_text_format == Some(InsertTextFormat::SNIPPET);
        let path = self.buffers[ix].document.path().map(Path::to_path_buf);
        let (insert, stops) = if snippet {
            let expanded = ide_core::snippet::expand(&new_text, &|name| match name {
                "TM_FILENAME" => path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned()),
                "TM_FILENAME_BASE" => path
                    .as_deref()
                    .and_then(Path::file_stem)
                    .map(|name| name.to_string_lossy().into_owned()),
                "TM_SELECTED_TEXT" => Some(String::new()),
                _ => None,
            });
            (expanded.text, expanded.stops)
        } else {
            let len = new_text.len();
            (new_text, std::iter::once(len..len).collect())
        };
        let mut edits = vec![(start..end, insert.clone())];
        for edit in item.additional_text_edits.iter().flatten() {
            let (Some(edit_start), Some(edit_end)) =
                (offset(edit.range.start), offset(edit.range.end))
            else {
                continue;
            };
            edits.push((edit_start..edit_end, edit.new_text.replace("\r\n", "\n")));
        }
        if let Err(error) = ide_core::text_edit::validate(&text, &edits) {
            self.status = format!("Completion {} not applied: {error}", item.label);
            cx.notify();
            return;
        }
        // Map snippet stops into the edited text: additional edits before the insertion
        // shift it.
        let shift: isize = edits[1..]
            .iter()
            .filter(|(range, _)| range.end <= start)
            .map(|(range, text)| text.len() as isize - range.len() as isize)
            .sum();
        let base = start.saturating_add_signed(shift);
        let absolute: Vec<Range<usize>> = stops
            .iter()
            .map(|stop| base + stop.start..base + stop.end)
            .collect();
        let first = absolute
            .first()
            .cloned()
            .unwrap_or(base + insert.len()..base + insert.len());
        self.apply_source_edits(ix, &edits, Some((first, false)), window, cx);
        self.snippet = (snippet && absolute.len() > 1).then(|| SnippetSession {
            buffer: id,
            stops: absolute,
            current: 0,
            text: self.buffers[ix].document.text().to_string(),
        });
        if let Some(command) = item.command {
            if command.command == "editor.action.triggerParameterHints" {
                self.trigger_signature_help(window, cx);
            } else if let Some(target) = self.lsp_target(cx) {
                self.run_command_for(&target, command, window, cx);
            }
        }
        cx.notify();
    }

    /// Keep snippet stops in place while the user types inside them.
    pub(super) fn track_snippet(&mut self, ix: usize) {
        let Some(session) = &mut self.snippet else {
            return;
        };
        let buffer = &self.buffers[ix];
        if buffer.id != session.buffer {
            return;
        }
        let new = buffer.document.text().to_string();
        let old = std::mem::replace(&mut session.text, new.clone());
        let prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = old[prefix..]
            .bytes()
            .rev()
            .zip(new[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let old_end = old.len() - suffix;
        let delta = new.len() as isize - old.len() as isize;
        for stop in &mut session.stops {
            if stop.start >= old_end && stop.start > prefix {
                stop.start = stop.start.saturating_add_signed(delta);
                stop.end = stop.end.saturating_add_signed(delta);
            } else if stop.end >= prefix {
                stop.end = stop.end.saturating_add_signed(delta).max(stop.start);
            }
        }
    }

    pub(super) fn snippet_active(&self) -> bool {
        self.snippet
            .as_ref()
            .is_some_and(|session| session.buffer == self.buffers[self.active].id)
    }

    fn move_tab_stop(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = &mut self.snippet else {
            return;
        };
        if self.buffers[self.active].id != session.buffer {
            self.snippet = None;
            return;
        }
        let next = session.current as isize + step;
        if next < 0 {
            return;
        }
        session.current = next as usize;
        let Some(stop) = session.stops.get(session.current).cloned() else {
            self.snippet = None;
            return;
        };
        if session.current + 1 >= session.stops.len() {
            self.snippet = None;
        }
        self.apply_source_edits(self.active, &[], Some((stop, false)), window, cx);
        cx.notify();
    }

    pub(super) fn next_tab_stop(
        &mut self,
        _: &NextTabStop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_tab_stop(1, window, cx);
    }

    pub(super) fn previous_tab_stop(
        &mut self,
        _: &PreviousTabStop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_tab_stop(-1, window, cx);
    }

    /// Escape ends the snippet session.
    pub(super) fn snippet_key_down(&mut self, event: &KeyDownEvent) {
        if event.keystroke.key == "escape" {
            self.snippet = None;
        }
    }
}
