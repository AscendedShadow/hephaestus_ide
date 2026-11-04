//! Manual language-mode selection (IDE-030).

use ide_core::language;

use super::*;

impl IdeShell {
    pub(super) fn change_language_mode(
        &mut self,
        _: &ChangeLanguageMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff {
            return;
        }
        self.show_search(SearchMode::Languages, window, cx);
    }

    /// Language modes matching `query`, the active buffer's mode first when unfiltered.
    pub(super) fn ranked_languages(&self, query: &str) -> Vec<&'static language::Language> {
        let mut ranked: Vec<_> = language::all()
            .iter()
            .filter_map(|language| {
                let name = search::rank(Path::new(language.name), query);
                let id = search::rank(Path::new(language.id), query);
                name.into_iter()
                    .chain(id)
                    .min()
                    .map(|score| (score, language))
            })
            .collect();
        let current = self.buffer().language;
        ranked.sort_by_key(|(score, language)| (language.id != current, *score, language.name));
        ranked.into_iter().map(|(_, language)| language).collect()
    }

    pub(super) fn language_label(language: &language::Language) -> String {
        let support = if syntax::has_grammar(language.id) {
            language.support.label()
        } else {
            language::Support::Recognized.label()
        };
        format!("{}  ({})  {support}", language.name, language.id)
    }

    /// Switch a buffer to another language mode, updating highlighting, editor support and
    /// the language-server document lifecycle (close under the old id, open under the new).
    pub(super) fn set_buffer_language(
        &mut self,
        ix: usize,
        language: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(buffer) = self.buffers.get_mut(ix) else {
            return;
        };
        buffer.language_overridden = true;
        if buffer.language == language {
            return;
        }
        let path = buffer.document.path().map(Path::to_path_buf);
        if let Some(path) = &path {
            self.diagnostics.remove(path);
            self.servers.update(cx, |servers, _| servers.close(path));
        }
        let buffer = &mut self.buffers[ix];
        buffer.language = language;
        let editor = buffer.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.set_highlighter(language, cx);
            if let Some(diagnostics) = editor.diagnostics_mut() {
                diagnostics.clear();
            }
        });
        language_server::attach(&editor, &self.servers, path, language, cx);
        self.sync_servers(cx);
        self.sync_lsp_buffer(ix, cx);
        self.status = format!("Language mode: {}", language::get(language).name);
        cx.notify();
    }
}
