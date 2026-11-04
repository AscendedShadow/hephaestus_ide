//! Breakpoint options and exception filters (IDE-022), watch expressions (IDE-023),
//! attach sessions (IDE-021) and per-workspace persistence of debugger state.

use std::{collections::BTreeMap, path::Path};

use dap::{Evaluation, SourceBreakpoint};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;

/// Options of one source breakpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakpointOptions {
    pub enabled: bool,
    pub condition: Option<String>,
    pub hit_condition: Option<String>,
    pub log_message: Option<String>,
}

impl Default for BreakpointOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            condition: None,
            hit_condition: None,
            log_message: None,
        }
    }
}

impl BreakpointOptions {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(condition) = &self.condition {
            parts.push(format!("if {condition}"));
        }
        if let Some(hit) = &self.hit_condition {
            parts.push(format!("hits {hit}"));
        }
        if let Some(message) = &self.log_message {
            parts.push(format!("log \"{message}\""));
        }
        parts.join(" · ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionFilter {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WatchValue {
    Pending,
    /// Not evaluated in the current stop (running, or a previous stop's value).
    Stale(Option<Evaluation>),
    Value(Evaluation),
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Watch {
    pub expression: String,
    pub value: WatchValue,
}

/// Debugger state persisted per workspace (paths relative to the workspace root).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedState {
    pub breakpoints: Vec<PersistedBreakpoint>,
    pub exception_filters: BTreeMap<String, bool>,
    pub watches: Vec<String>,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PersistedBreakpoint {
    pub path: PathBuf,
    pub line: usize,
    #[serde(flatten)]
    pub options: BreakpointOptions,
}

/// Where the debugger state of a workspace is stored (outside the project).
pub fn state_path(settings_path: &Path, root: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hasher);
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    settings_path
        .with_file_name("workspaces")
        .join(format!("{name}-{:016x}.json", hasher.finish()))
}

impl Debugger {
    /// Breakpoints to send for `path`: enabled ones, with only the options the adapter
    /// supports. Returns the lines sent (in order) and the request entries.
    pub(super) fn source_breakpoints(
        &mut self,
        path: &Path,
    ) -> (Vec<usize>, Vec<SourceBreakpoint>) {
        let supports =
            |name: &str| self.capabilities.get(name).and_then(Value::as_bool) == Some(true);
        let (conditions, hits, logs) = (
            supports("supportsConditionalBreakpoints"),
            supports("supportsHitConditionalBreakpoints"),
            supports("supportsLogPoints"),
        );
        let known = !self.capabilities.is_null();
        let mut lines = Vec::new();
        let mut breakpoints = Vec::new();
        let mut unsupported = Vec::new();
        for line in self.breakpoints.get(path).into_iter().flatten().copied() {
            let options = self.options_at(path, line);
            if !options.enabled {
                continue;
            }
            let mut send = SourceBreakpoint {
                line,
                ..Default::default()
            };
            for (value, supported, label, slot) in [
                (
                    &options.condition,
                    conditions,
                    "conditions",
                    &mut send.condition,
                ),
                (
                    &options.hit_condition,
                    hits,
                    "hit counts",
                    &mut send.hit_condition,
                ),
                (
                    &options.log_message,
                    logs,
                    "log messages",
                    &mut send.log_message,
                ),
            ] {
                if value.is_some() {
                    if supported || !known {
                        *slot = value.clone();
                    } else {
                        unsupported
                            .push(format!("line {line}: the adapter does not support {label}"));
                    }
                }
            }
            lines.push(line);
            breakpoints.push(send);
        }
        for message in unsupported {
            self.log(LineKind::Error, format!("{}: {message}", path.display()));
        }
        (lines, breakpoints)
    }

    pub fn options_at(&self, path: &Path, line: usize) -> BreakpointOptions {
        self.options
            .get(&(path.to_path_buf(), line))
            .cloned()
            .unwrap_or_default()
    }

    /// Set a breakpoint's options (adding the breakpoint when needed).
    pub fn set_breakpoint_options(
        &mut self,
        path: PathBuf,
        line: usize,
        options: BreakpointOptions,
        cx: &mut Context<Self>,
    ) {
        self.breakpoints
            .entry(path.clone())
            .or_default()
            .insert(line);
        if options == BreakpointOptions::default() {
            self.options.remove(&(path.clone(), line));
        } else {
            self.options.insert((path.clone(), line), options);
        }
        self.sync_breakpoints(&path, cx);
        cx.emit(DebuggerEvent::StateChanged);
        cx.notify();
    }

    pub fn toggle_breakpoint_enabled(
        &mut self,
        path: PathBuf,
        line: usize,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if !self
            .breakpoints
            .get(&path)
            .is_some_and(|lines| lines.contains(&line))
        {
            return None;
        }
        let mut options = self.options_at(&path, line);
        options.enabled = !options.enabled;
        let enabled = options.enabled;
        self.set_breakpoint_options(path, line, options, cx);
        Some(enabled)
    }

    pub fn remove_all_breakpoints(&mut self, cx: &mut Context<Self>) {
        self.options.clear();
        self.remove_breakpoints(None, cx);
        cx.emit(DebuggerEvent::StateChanged);
    }

    /// Keep breakpoints on their source lines while editing: lines after an edit that
    /// starts at `line` (1-based) move by `delta`; breakpoints inside deleted lines move
    /// to the edit line.
    pub fn shift_breakpoints(
        &mut self,
        path: &Path,
        line: usize,
        removed: usize,
        added: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(lines) = self.breakpoints.get(path).cloned() else {
            return;
        };
        if removed == added {
            return;
        }
        let mut moved = BTreeSet::new();
        let mut options = Vec::new();
        for old in lines {
            let new = if old <= line {
                old
            } else if old <= line + removed {
                line + added.min(old - line)
            } else {
                (old as isize + added as isize - removed as isize).max(1) as usize
            };
            if let Some(option) = self.options.remove(&(path.to_path_buf(), old)) {
                options.push((new, option));
            }
            moved.insert(new);
        }
        for (new, option) in options {
            self.options.insert((path.to_path_buf(), new), option);
        }
        self.breakpoints.insert(path.to_path_buf(), moved);
        self.rejected.retain(|(rejected, _)| rejected != path);
        self.sync_breakpoints(path, cx);
        cx.emit(DebuggerEvent::StateChanged);
        cx.notify();
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn exception_filters(&self) -> &[ExceptionFilter] {
        &self.exception_filters
    }

    pub fn set_exception_filter(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        self.exception_choices.insert(id.to_string(), enabled);
        if let Some(filter) = self
            .exception_filters
            .iter_mut()
            .find(|filter| filter.id == id)
        {
            filter.enabled = enabled;
        }
        if let Some(client) = self
            .session
            .as_ref()
            .filter(|session| session.configured)
            .map(|session| session.client.clone())
        {
            let request = client.request(
                "setExceptionBreakpoints",
                json!({"filters": self.enabled_filters()}),
            );
            cx.spawn(async move |this, cx| {
                if let Err(error) = request.await {
                    let _ = this.update(cx, |this, cx| {
                        this.log(LineKind::Error, format!("Exception breakpoints: {error}"));
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        cx.emit(DebuggerEvent::StateChanged);
        cx.notify();
    }

    pub(super) fn enabled_filters(&self) -> Vec<String> {
        self.exception_filters
            .iter()
            .filter(|filter| filter.enabled)
            .map(|filter| filter.id.clone())
            .collect()
    }

    /// Read exception filters from the adapter's capabilities, applying saved choices.
    pub(super) fn adopt_capabilities(&mut self, capabilities: Value) {
        self.exception_filters = capabilities
            .get("exceptionBreakpointFilters")
            .and_then(Value::as_array)
            .map(|filters| {
                filters
                    .iter()
                    .filter_map(|filter| {
                        let id = filter.get("filter")?.as_str()?.to_string();
                        let default = filter
                            .get("default")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        Some(ExceptionFilter {
                            label: filter
                                .get("label")
                                .and_then(Value::as_str)
                                .unwrap_or(&id)
                                .to_string(),
                            enabled: self.exception_choices.get(&id).copied().unwrap_or(default),
                            id,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.capabilities = capabilities;
    }

    // ----- watches -----

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn watches(&self) -> &[Watch] {
        &self.watches
    }

    pub fn add_watch(&mut self, expression: String, cx: &mut Context<Self>) {
        let expression = expression.trim().to_string();
        if expression.is_empty() {
            return;
        }
        self.watches.push(Watch {
            expression,
            value: WatchValue::Stale(None),
        });
        self.refresh_watches(cx);
        cx.emit(DebuggerEvent::StateChanged);
    }

    pub fn remove_watch(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.watches.len() {
            self.watches.remove(ix);
            cx.emit(DebuggerEvent::StateChanged);
            cx.notify();
        }
    }

    pub fn move_watch(&mut self, ix: usize, up: bool, cx: &mut Context<Self>) {
        let target = if up { ix.checked_sub(1) } else { Some(ix + 1) };
        if let Some(target) = target.filter(|target| *target < self.watches.len()) {
            self.watches.swap(ix, target);
            cx.emit(DebuggerEvent::StateChanged);
            cx.notify();
        }
    }

    /// Mark watch values stale (the program is running, or a new session started).
    pub(super) fn mark_watches_stale(&mut self) {
        self.watch_generation += 1;
        for watch in &mut self.watches {
            let previous = match &watch.value {
                WatchValue::Value(value) => Some(value.clone()),
                WatchValue::Stale(value) => value.clone(),
                _ => None,
            };
            watch.value = WatchValue::Stale(previous);
        }
    }

    /// Evaluate every watch in the selected frame (DAP `watch` context). Responses from
    /// an older stop or frame are ignored.
    pub(super) fn refresh_watches(&mut self, cx: &mut Context<Self>) {
        self.watch_generation += 1;
        let generation = self.watch_generation;
        let Some(client) = self.client().cloned().filter(|_| self.is_paused()) else {
            cx.notify();
            return;
        };
        let frame = self.frame_id();
        for watch in &mut self.watches {
            watch.value = WatchValue::Pending;
        }
        let requests: Vec<_> = self
            .watches
            .iter()
            .map(|watch| client.evaluate(&watch.expression, frame, "watch"))
            .collect();
        cx.spawn(async move |this, cx| {
            let mut results = Vec::new();
            for request in requests {
                results.push(request.await);
            }
            let _ = this.update(cx, |this, cx| {
                if this.watch_generation != generation {
                    return;
                }
                for (watch, result) in this.watches.iter_mut().zip(results) {
                    watch.value = match result {
                        Ok(evaluation) => WatchValue::Value(evaluation),
                        Err(error) => WatchValue::Error(error.to_string()),
                    };
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ----- persistence -----

    pub fn export_state(&self, root: &Path, profile: Option<String>) -> PersistedState {
        let mut breakpoints: Vec<PersistedBreakpoint> = self
            .breakpoints
            .iter()
            .flat_map(|(path, lines)| lines.iter().map(move |line| (path, *line)))
            .filter_map(|(path, line)| {
                Some(PersistedBreakpoint {
                    path: path.strip_prefix(root).ok()?.to_path_buf(),
                    line,
                    options: self.options_at(path, line),
                })
            })
            .collect();
        breakpoints.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        PersistedState {
            breakpoints,
            exception_filters: self
                .exception_choices
                .iter()
                .map(|(id, enabled)| (id.clone(), *enabled))
                .collect(),
            watches: self
                .watches
                .iter()
                .map(|watch| watch.expression.clone())
                .collect(),
            profile,
        }
    }

    /// Replace breakpoints, exception choices and watches with a workspace's state.
    pub fn import_state(&mut self, root: &Path, state: &PersistedState, cx: &mut Context<Self>) {
        let old: Vec<PathBuf> = self.breakpoints.keys().cloned().collect();
        self.breakpoints.clear();
        self.options.clear();
        self.rejected.clear();
        for breakpoint in &state.breakpoints {
            let path = root.join(&breakpoint.path);
            self.breakpoints
                .entry(path.clone())
                .or_default()
                .insert(breakpoint.line);
            if breakpoint.options != BreakpointOptions::default() {
                self.options
                    .insert((path, breakpoint.line), breakpoint.options.clone());
            }
        }
        self.exception_choices = state.exception_filters.clone().into_iter().collect();
        for filter in &mut self.exception_filters {
            if let Some(enabled) = self.exception_choices.get(&filter.id) {
                filter.enabled = *enabled;
            }
        }
        self.watches = state
            .watches
            .iter()
            .map(|expression| Watch {
                expression: expression.clone(),
                value: WatchValue::Stale(None),
            })
            .collect();
        let paths: Vec<PathBuf> = old
            .into_iter()
            .chain(self.breakpoints.keys().cloned())
            .collect();
        for path in paths {
            self.sync_breakpoints(&path, cx);
        }
        cx.notify();
    }
}

// ----- panel sections -----

impl Debugger {
    pub fn focus_watch_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn render_exception_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .pb_2()
            .when(!self.exception_filters.is_empty(), |section| {
                section.child(section_header("EXCEPTIONS", None)).children(
                    self.exception_filters
                        .iter()
                        .enumerate()
                        .map(|(ix, filter)| {
                            let id = filter.id.clone();
                            let enabled = filter.enabled;
                            row(("debug-exception", ix), false)
                                .child(
                                    gpui_component::checkbox::Checkbox::new((
                                        "debug-exception-check",
                                        ix,
                                    ))
                                    .checked(enabled)
                                    .on_click(cx.listener(
                                        move |this, checked: &bool, _, cx| {
                                            this.set_exception_filter(&id, *checked, cx)
                                        },
                                    )),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(filter.label.clone()),
                                )
                        }),
                )
            })
    }

    pub(super) fn render_watches(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut key = 10_000;
        for (ix, watch) in self.watches.iter().enumerate() {
            let (value, color, reference) = match &watch.value {
                WatchValue::Pending => ("…".to_string(), theme::subtle(), 0),
                WatchValue::Stale(Some(previous)) => {
                    (format!("{} (stale)", previous.result), theme::subtle(), 0)
                }
                WatchValue::Stale(None) => ("not available".to_string(), theme::subtle(), 0),
                WatchValue::Value(evaluation) => (
                    match evaluation
                        .type_name
                        .as_deref()
                        .filter(|name| !name.is_empty())
                    {
                        Some(type_name) => format!("{}  : {type_name}", evaluation.result),
                        None => evaluation.result.clone(),
                    },
                    theme::text(),
                    evaluation.variables_reference,
                ),
                WatchValue::Error(error) => (
                    error.clone(),
                    theme::git_change(ide_core::git::Change::Deleted),
                    0,
                ),
            };
            let expanded = reference > 0 && self.expanded.contains(&reference);
            rows.push(
                row(("debug-watch", ix), false)
                    .child(
                        div()
                            .w(px(12.))
                            .flex_shrink_0()
                            .text_color(theme::subtle())
                            .child(if reference > 0 {
                                if expanded { "▾" } else { "▸" }
                            } else {
                                ""
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::mono_family())
                            .text_color(theme::accent())
                            .child(watch.expression.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::mono_family())
                            .text_color(color)
                            .child(value),
                    )
                    .child(
                        div()
                            .flex()
                            .invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new(("debug-watch-up", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronUp)
                                    .tooltip("Move up")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.move_watch(ix, true, cx)
                                    })),
                            )
                            .child(
                                Button::new(("debug-watch-down", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronDown)
                                    .tooltip("Move down")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.move_watch(ix, false, cx)
                                    })),
                            )
                            .child(
                                Button::new(("debug-watch-edit", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Replace)
                                    .tooltip("Edit (loads it into the input; Enter adds it back)")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        if let Some(watch) = this.watches.get(ix) {
                                            let expression = watch.expression.clone();
                                            this.watch_input.update(cx, |input, cx| {
                                                input.set_value(expression, window, cx);
                                                input.focus(window, cx);
                                            });
                                            this.remove_watch(ix, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new(("debug-watch-remove", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip("Remove")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.remove_watch(ix, cx)
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if reference > 0 {
                            this.toggle_variable(reference, cx);
                        }
                    }))
                    .into_any_element(),
            );
            if expanded && let Some(children) = self.children.get(&reference) {
                for child in children.clone() {
                    self.render_variable(&child, 1, &mut key, &mut rows, cx);
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .pb_2()
            .child(section_header("WATCH", Some(self.watches.len())))
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .child(gpui_component::input::Input::new(&self.watch_input).small()),
            )
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_state_round_trips_with_relative_paths() {
        let state = PersistedState {
            breakpoints: vec![PersistedBreakpoint {
                path: "src/main.rs".into(),
                line: 4,
                options: BreakpointOptions {
                    enabled: false,
                    condition: Some("x > 1".into()),
                    hit_condition: None,
                    log_message: Some("x = {x}".into()),
                },
            }],
            exception_filters: [("panic".to_string(), true)].into_iter().collect(),
            watches: vec!["self.len()".into()],
            profile: Some("Attach".into()),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"condition\":\"x > 1\""), "{json}");
        assert_eq!(
            serde_json::from_str::<PersistedState>(&json).unwrap(),
            state
        );
        let path = state_path(
            Path::new("/config/hephaestus/settings.json"),
            Path::new("/work/app"),
        );
        assert!(path.starts_with("/config/hephaestus/workspaces"));
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("app-")
        );
    }
}
