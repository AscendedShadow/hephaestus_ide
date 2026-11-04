//! Caret-anchored completion (IDE-010): an LSP completion provider for the editor's
//! completion menu.
//!
//! The provider asks the shell (through [`Hooks`]) for the buffer's path, language and
//! *source* position, so folded projections never shift positions. Complete lists are
//! filtered and sorted locally while typing (by `filterText`/`sortText`); incomplete lists
//! and trigger characters re-query the server. Accepting an item is delegated to the shell,
//! which applies the exact text edit, additional edits (auto-imports) and snippets as one
//! undo step.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::Result;
use gpui::{App, Context, Entity, Task, WeakEntity, Window};
use gpui_component::input::{CompletionProvider, InputState, Rope};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionList, CompletionParams, CompletionResponse,
    CompletionTriggerKind, request::Completion,
};

use crate::language_server::Servers;

const MAX_ITEMS: usize = 200;

type SourceHook = Box<dyn Fn(usize, &App) -> Option<(PathBuf, &'static str, usize, String)>>;
type AcceptHook = Box<dyn Fn(CompletionItem, &mut Window, &mut App)>;

/// What the provider needs from the shell for one buffer.
pub struct Hooks {
    /// `(path, language, source offset, source text)` for a display offset.
    pub source: SourceHook,
    /// Accept an item; called outside the editor's update.
    pub accept: AcceptHook,
}

#[derive(Default)]
struct Cache {
    path: PathBuf,
    /// Source offset where the completed word starts.
    word_start: usize,
    items: Vec<CompletionItem>,
    incomplete: bool,
}

pub struct Provider {
    servers: WeakEntity<Servers>,
    hooks: Rc<Hooks>,
    cache: Rc<RefCell<Option<Cache>>>,
}

pub fn attach(editor: &Entity<InputState>, servers: &Entity<Servers>, hooks: Hooks, cx: &mut App) {
    let provider = Rc::new(Provider {
        servers: servers.downgrade(),
        hooks: Rc::new(hooks),
        cache: Rc::default(),
    });
    editor.update(cx, |editor, _| {
        editor.lsp.completion_provider = Some(provider)
    });
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// Start of the identifier ending at `offset`.
pub fn word_start(text: &str, offset: usize) -> usize {
    text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, character)| is_word(*character))
        .last()
        .map_or(offset, |(ix, _)| ix)
}

/// Filter by `filterText` (or label) against `prefix` and sort by match quality, then
/// `sortText` (or label), as the protocol recommends.
pub fn filter_and_sort(items: &[CompletionItem], prefix: &str) -> Vec<CompletionItem> {
    let mut ranked: Vec<_> = items
        .iter()
        .filter_map(|item| {
            let filter = item.filter_text.as_deref().unwrap_or(&item.label);
            let score = if prefix.is_empty() {
                Some(0)
            } else {
                ide_core::search::rank(std::path::Path::new(filter), prefix).map(|score| {
                    score
                        + usize::from(!filter.to_lowercase().starts_with(&prefix.to_lowercase()))
                            * 100
                })
            }?;
            Some((score, item))
        })
        .collect();
    ranked.sort_by(|(a_score, a), (b_score, b)| {
        let preselect = |item: &CompletionItem| !item.preselect.unwrap_or(false);
        preselect(a)
            .cmp(&preselect(b))
            .then(a_score.cmp(b_score))
            .then_with(|| {
                a.sort_text
                    .as_deref()
                    .unwrap_or(&a.label)
                    .cmp(b.sort_text.as_deref().unwrap_or(&b.label))
            })
    });
    ranked
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(_, item)| item.clone())
        .collect()
}

impl CompletionProvider for Provider {
    fn completions(
        &self,
        _text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        // Called while the editor (and possibly the shell) is being updated: resolve the
        // source position on the next tick, once the edit has reached the document.
        let hooks = self.hooks.clone();
        let servers = self.servers.clone();
        let cache = self.cache.clone();
        cx.spawn(async move |_, cx| {
            let empty = Ok(CompletionResponse::Array(Vec::new()));
            let Some((path, language, source_offset, source)) =
                cx.update(|cx| (hooks.source)(offset, cx)).ok().flatten()
            else {
                return empty;
            };
            let source_offset = source_offset.min(source.len());
            let start = word_start(&source, source_offset);
            let prefix = source[start..source_offset].to_string();
            let character = source[..source_offset].chars().next_back();
            let is_trigger_character = character.is_some_and(|character| !is_word(character));
            let invoked = trigger.trigger_kind == CompletionTriggerKind::INVOKED;
            if !is_trigger_character
                && !invoked
                && let Some(cached) = cache.borrow().as_ref()
                && cached.path == path
                && cached.word_start == start
                && !cached.incomplete
            {
                return Ok(CompletionResponse::Array(filter_and_sort(
                    &cached.items,
                    &prefix,
                )));
            }
            let incomplete_refresh = cache.borrow().as_ref().is_some_and(|cached| {
                cached.incomplete && cached.word_start == start && cached.path == path
            });
            let context = CompletionContext {
                trigger_kind: if is_trigger_character && !invoked {
                    CompletionTriggerKind::TRIGGER_CHARACTER
                } else if incomplete_refresh {
                    CompletionTriggerKind::TRIGGER_FOR_INCOMPLETE_COMPLETIONS
                } else {
                    CompletionTriggerKind::INVOKED
                },
                trigger_character: (is_trigger_character && !invoked)
                    .then(|| character.unwrap().to_string()),
            };
            let (line, column) = ide_core::text_edit::position(&source, source_offset);
            let Some(position) =
                Servers::position_params(&path, lsp_types::Position::new(line, column))
            else {
                return empty;
            };
            let request = cx
                .update(|cx| {
                    let servers = servers.upgrade()?;
                    servers.read(cx).send::<Completion>(
                        language,
                        CompletionParams {
                            text_document_position: position,
                            work_done_progress_params: Default::default(),
                            partial_result_params: Default::default(),
                            context: Some(context),
                        },
                    )
                })
                .ok()
                .flatten();
            let Some(request) = request else {
                return empty;
            };
            let (items, incomplete) = match request.await {
                Ok(Some(CompletionResponse::Array(items))) => (items, false),
                Ok(Some(CompletionResponse::List(CompletionList {
                    is_incomplete,
                    items,
                }))) => (items, is_incomplete),
                Ok(None) => (Vec::new(), false),
                Err(error) => return Err(anyhow::anyhow!(error.to_string())),
            };
            let filtered = filter_and_sort(&items, &prefix);
            *cache.borrow_mut() = Some(Cache {
                path,
                word_start: start,
                items,
                incomplete,
            });
            Ok(CompletionResponse::Array(filtered))
        })
    }

    fn apply_completion(
        &self,
        item: &CompletionItem,
        _trigger_start: usize,
        _cursor: usize,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> bool {
        let hooks = self.hooks.clone();
        let item = item.clone();
        *self.cache.borrow_mut() = None;
        window.defer(cx, move |window, cx| (hooks.accept)(item, window, cx));
        true
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        cx: &mut Context<InputState>,
    ) -> bool {
        let Some(character) = new_text.chars().next_back() else {
            return false;
        };
        if new_text.chars().count() > 1 {
            // Pastes and programmatic edits do not open the menu.
            return false;
        }
        if is_word(character) {
            return true;
        }
        // The shell may be mid-update here, so only the servers are consulted.
        self.servers
            .upgrade()
            .is_some_and(|servers| servers.read(cx).is_trigger_character(character))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, sort: Option<&str>, filter: Option<&str>) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            sort_text: sort.map(String::from),
            filter_text: filter.map(String::from),
            ..Default::default()
        }
    }

    #[test]
    fn filters_by_filter_text_and_sorts_by_quality_then_sort_text() {
        let items = vec![
            item("to_string()", Some("2"), None),
            item("trim", Some("1"), None),
            item("as_str", Some("0"), Some("str")),
            item("contains", Some("0"), None),
        ];
        let labels = |prefix: &str| {
            filter_and_sort(&items, prefix)
                .into_iter()
                .map(|item| item.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(""), ["as_str", "contains", "trim", "to_string()"]);
        assert_eq!(labels("tr"), ["trim", "as_str", "to_string()"]);
        assert_eq!(labels("str"), ["as_str", "to_string()"]);
        assert_eq!(word_start("let ab_c", 8), 4);
        assert_eq!(word_start("x.", 2), 2);
    }
}
