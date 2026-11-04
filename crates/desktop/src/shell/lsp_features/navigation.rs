//! Go to implementation/type definition and find references.

use super::*;

impl IdeShell {
    pub(in crate::shell) fn definition_like<R>(
        &mut self,
        what: &'static str,
        supported: fn(&Features) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        R: 'static
            + lsp_types::request::Request<
                Params = GotoDefinitionParams,
                Result = Option<GotoDefinitionResponse>,
            >,
    {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !supported(&target.features) {
            return self.unsupported(what, &target, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let request = self.servers.read(cx).send::<R>(
            target.language,
            GotoDefinitionParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        let Some(request) = request else { return };
        self.status = format!("Looking up {what}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, what, cx) || this.buffer().id != target.id {
                    return;
                }
                match response {
                    Ok(Some(response)) => {
                        this.show_locations(what, locations(response), false, window, cx)
                    }
                    Ok(None) => {
                        this.status = format!("No {what} found");
                        cx.notify();
                    }
                    Err(error) => {
                        this.status = format!("Could not find {what}: {error}");
                        cx.notify();
                    }
                }
            });
        });
    }

    pub(in crate::shell) fn go_to_implementation(
        &mut self,
        _: &GoToImplementation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.definition_like::<GotoImplementation>(
            "implementations",
            |f| f.implementation,
            window,
            cx,
        );
    }

    pub(in crate::shell) fn go_to_type_definition(
        &mut self,
        _: &GoToTypeDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.definition_like::<GotoTypeDefinition>(
            "type definitions",
            |f| f.type_definition,
            window,
            cx,
        );
    }

    pub(in crate::shell) fn references(
        &mut self,
        include_declaration: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.references {
            return self.unsupported("Find References", &target, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let request = self.servers.read(cx).send::<References>(
            target.language,
            ReferenceParams {
                text_document_position: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration,
                },
            },
        );
        let Some(request) = request else { return };
        self.status = "Finding references…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Find References", cx)
                    || this.buffer().id != target.id
                {
                    return;
                }
                match response {
                    Ok(list) => {
                        let targets = list
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|location| {
                                Some((lsp::uri_to_path(&location.uri)?, location.range))
                            })
                            .collect();
                        this.show_locations("references", targets, true, window, cx);
                    }
                    Err(error) => {
                        this.status = format!("Find References failed: {error}");
                        cx.notify();
                    }
                }
            });
        });
    }

    pub(in crate::shell) fn find_references(
        &mut self,
        _: &FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.references(true, window, cx);
    }

    pub(in crate::shell) fn find_references_without_declaration(
        &mut self,
        _: &FindReferencesWithoutDeclaration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.references(false, window, cx);
    }

    /// Present every definition target (instead of only the first) when there are
    /// several; used by Go to Definition.
    pub(in crate::shell) fn show_definition_targets(
        &mut self,
        response: GotoDefinitionResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let targets = locations(response);
        if targets.is_empty() {
            return false;
        }
        self.show_locations("definitions", targets, false, window, cx);
        true
    }
}
