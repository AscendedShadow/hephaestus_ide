//! Jumping to locations and Go Back / Go Forward history.

use super::*;

impl IdeShell {
    pub(super) fn jump_to_hit(
        &mut self,
        hit: &SearchMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.editor().clone();
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position::new((hit.line - 1) as u32, (hit.column - 1) as u32),
                window,
                cx,
            );
            editor.focus(window, cx);
        });
    }

    pub(super) fn current_location(&self, cx: &App) -> Option<SearchMatch> {
        let path = self.document().path()?.to_path_buf();
        let cursor = self.editor().read(cx).cursor_position();
        Some(SearchMatch {
            path,
            line: cursor.line as usize + 1,
            column: cursor.character as usize + 1,
            preview: String::new(),
        })
    }

    pub(super) fn record_location(&mut self, destination: &SearchMatch, cx: &App) {
        let Some(origin) = self.current_location(cx) else {
            return;
        };
        self.locations
            .truncate(self.location_index.saturating_add(1));
        if self.locations.last() != Some(&origin) {
            self.locations.push(origin);
        }
        if self.locations.last() != Some(destination) {
            self.locations.push(destination.clone());
        }
        self.location_index = self.locations.len().saturating_sub(1);
        if self.locations.len() > 100 {
            self.locations.remove(0);
            self.location_index = self.location_index.saturating_sub(1);
        }
    }

    pub(super) fn navigate_history(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self.location_index as isize + direction;
        if index < 0 || index as usize >= self.locations.len() {
            return;
        }
        self.location_index = index as usize;
        let hit = self.locations[self.location_index].clone();
        self.open_at(hit, window, cx);
    }

    pub(super) fn go_back(&mut self, _: &GoBack, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_history(-1, window, cx);
    }

    pub(super) fn go_forward(
        &mut self,
        _: &GoForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_history(1, window, cx);
    }

    pub(super) fn open_at(
        &mut self,
        hit: SearchMatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.buffer_for_path(&hit.path) {
            self.activate(ix, window, cx);
            self.jump_to_hit(&hit, window, cx);
        } else {
            self.open(Some(hit.path.clone()), window, cx);
            // The asynchronous open completes later; location is applied in its completion handler.
            self.pending_hit = Some(hit);
        }
    }
}
