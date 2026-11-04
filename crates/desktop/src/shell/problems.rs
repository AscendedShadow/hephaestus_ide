//! The Problems view (IDE-020): language-server diagnostics (including files that are
//! not open, when servers publish them) and problems parsed from task output while it
//! runs. Problems are grouped by file, filterable by severity, source and active file,
//! and navigable with Next/Previous Problem. Each task run replaces that task's
//! problems; each server publication replaces that file's diagnostics.

use gpui::AnyElement;
use gpui_component::Selectable as _;
use ide_core::problems::{Problem, Severity};

use super::*;

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub problem: Problem,
    /// Related locations (path, line, message), e.g. from LSP related information.
    pub related: Vec<(PathBuf, usize, String)>,
}

pub(super) struct ProblemsState {
    pub build: Vec<Problem>,
    pub show: [bool; 4],
    pub active_file_only: bool,
    pub source: Option<String>,
    pub current: Option<usize>,
}

impl Default for ProblemsState {
    fn default() -> Self {
        Self {
            build: Vec::new(),
            show: [true; 4],
            active_file_only: false,
            source: None,
            current: None,
        }
    }
}

fn severity_index(severity: Severity) -> usize {
    match severity {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Information => 2,
        Severity::Hint => 3,
    }
}

fn from_lsp(path: &Path, diagnostic: &lsp_types::Diagnostic) -> Entry {
    let severity = match diagnostic.severity {
        Some(lsp_types::DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(lsp_types::DiagnosticSeverity::INFORMATION) => Severity::Information,
        Some(lsp_types::DiagnosticSeverity::HINT) => Severity::Hint,
        _ => Severity::Error,
    };
    let code = diagnostic.code.as_ref().map(|code| match code {
        lsp_types::NumberOrString::Number(number) => number.to_string(),
        lsp_types::NumberOrString::String(text) => text.clone(),
    });
    Entry {
        problem: Problem {
            path: path.to_path_buf(),
            line: diagnostic.range.start.line as usize + 1,
            column: diagnostic.range.start.character as usize + 1,
            severity,
            code,
            message: diagnostic.message.clone(),
            source: diagnostic
                .source
                .clone()
                .unwrap_or_else(|| "language server".into()),
        },
        related: diagnostic
            .related_information
            .iter()
            .flatten()
            .filter_map(|related| {
                Some((
                    lsp::uri_to_path(&related.location.uri)?,
                    related.location.range.start.line as usize + 1,
                    related.message.clone(),
                ))
            })
            .collect(),
    }
}

impl IdeShell {
    pub(super) fn clear_build_problems(&mut self, source: &str) {
        self.problems
            .build
            .retain(|problem| problem.source != source);
        self.problems.current = None;
    }

    pub(super) fn add_build_problems(&mut self, problems: Vec<Problem>) {
        self.problems.build.extend(problems);
    }

    /// All problems after filters, sorted by file, line and severity.
    pub(super) fn problem_entries(&self) -> Vec<Entry> {
        let active = self.document().path();
        let mut entries: Vec<Entry> = self
            .lsp_diagnostics
            .iter()
            .flat_map(|(path, diagnostics)| {
                diagnostics
                    .iter()
                    .map(move |diagnostic| from_lsp(path, diagnostic))
            })
            .chain(self.problems.build.iter().cloned().map(|problem| Entry {
                problem,
                related: Vec::new(),
            }))
            .filter(|entry| self.problems.show[severity_index(entry.problem.severity)])
            .filter(|entry| {
                !self.problems.active_file_only || active == Some(entry.problem.path.as_path())
            })
            .filter(|entry| {
                self.problems
                    .source
                    .as_ref()
                    .is_none_or(|source| entry.problem.source.starts_with(source.as_str()))
            })
            .collect();
        entries.sort_by(|a, b| {
            a.problem
                .path
                .cmp(&b.problem.path)
                .then(a.problem.line.cmp(&b.problem.line))
                .then(a.problem.column.cmp(&b.problem.column))
                .then(a.problem.severity.cmp(&b.problem.severity))
        });
        entries
    }

    pub(super) fn problem_counts(&self) -> [usize; 4] {
        let mut counts = [0; 4];
        for diagnostics in self.lsp_diagnostics.values() {
            for diagnostic in diagnostics {
                counts[severity_index(from_lsp(Path::new(""), diagnostic).problem.severity)] += 1;
            }
        }
        for problem in &self.problems.build {
            counts[severity_index(problem.severity)] += 1;
        }
        counts
    }

    pub(super) fn show_problems(
        &mut self,
        _: &ShowProblems,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_panel = ToolPanel::Problems;
        cx.notify();
    }

    fn go_to_problem(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.problem_entries();
        if entries.is_empty() {
            self.status = "No problems".into();
            cx.notify();
            return;
        }
        let next = match self.problems.current {
            Some(current) => (current as isize + step).rem_euclid(entries.len() as isize) as usize,
            None if step > 0 => 0,
            None => entries.len() - 1,
        };
        self.open_problem(next, &entries, window, cx);
    }

    fn open_problem(
        &mut self,
        ix: usize,
        entries: &[Entry],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = entries.get(ix) else { return };
        self.problems.current = Some(ix);
        let location = SearchMatch {
            path: entry.problem.path.clone(),
            line: entry.problem.line,
            column: entry.problem.column,
            preview: entry.problem.message.clone(),
        };
        self.status = format!(
            "{} {}/{}: {}",
            entry.problem.severity.label(),
            ix + 1,
            entries.len(),
            entry.problem.message.lines().next().unwrap_or_default()
        );
        self.record_location(&location, cx);
        self.open_at(location, window, cx);
    }

    pub(super) fn next_problem(
        &mut self,
        _: &NextProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(1, window, cx);
    }

    pub(super) fn previous_problem(
        &mut self,
        _: &PreviousProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.go_to_problem(-1, window, cx);
    }

    pub(super) fn render_problems(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.problem_entries();
        let counts = self.problem_counts();
        let filter = |id: &'static str, ix: usize, label: &'static str, cx: &mut Context<Self>| {
            Button::new(id)
                .ghost()
                .xsmall()
                .label(format!("{label} {}", counts[ix]))
                .selected(self.problems.show[ix])
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.problems.show[ix] = !this.problems.show[ix];
                    this.problems.current = None;
                    cx.notify();
                }))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut last: Option<PathBuf> = None;
        for (ix, entry) in entries.iter().enumerate() {
            if last.as_ref() != Some(&entry.problem.path) {
                let count = entries
                    .iter()
                    .filter(|other| other.problem.path == entry.problem.path)
                    .count();
                let relative = self
                    .workspace
                    .root()
                    .and_then(|root| entry.problem.path.strip_prefix(root).ok())
                    .unwrap_or(&entry.problem.path)
                    .display()
                    .to_string();
                rows.push(
                    div()
                        .px_2()
                        .pt_1()
                        .flex()
                        .gap_2()
                        .font_weight(FontWeight::MEDIUM)
                        .child(relative)
                        .child(ui::count_badge(count))
                        .into_any_element(),
                );
                last = Some(entry.problem.path.clone());
            }
            let color = match entry.problem.severity {
                Severity::Error => theme::git_change(Change::Deleted),
                Severity::Warning => theme::accent(),
                _ => theme::muted(),
            };
            let entries_for_click = entries.clone();
            rows.push(
                div()
                    .id(("problem", ix))
                    .pl_6()
                    .pr_2()
                    .py_0p5()
                    .flex()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::hover()))
                    .when(self.problems.current == Some(ix), |row| {
                        row.bg(theme::active_row())
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(color)
                            .child(entry.problem.severity.label()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(entry.problem.message.clone())
                            .children(entry.related.iter().map(|(path, line, message)| {
                                div().text_color(theme::subtle()).child(format!(
                                    "↳ {}:{line} {message}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ))
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::subtle())
                            .child(format!(
                                "{}{} [{}:{}]",
                                entry
                                    .problem
                                    .code
                                    .as_deref()
                                    .map(|code| format!("{code} · "))
                                    .unwrap_or_default(),
                                entry.problem.source,
                                entry.problem.line,
                                entry.problem.column
                            )),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_problem(ix, &entries_for_click, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        div()
            .debug_selector(|| "problems".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(filter("problems-errors", 0, "Errors", cx))
                    .child(filter("problems-warnings", 1, "Warnings", cx))
                    .child(filter("problems-info", 2, "Info", cx))
                    .child(filter("problems-hints", 3, "Hints", cx))
                    .child(
                        Button::new("problems-active-file")
                            .ghost()
                            .xsmall()
                            .label("Active file")
                            .selected(self.problems.active_file_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.problems.active_file_only = !this.problems.active_file_only;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("problems-build-only")
                            .ghost()
                            .xsmall()
                            .label("Build output only")
                            .selected(self.problems.source.is_some())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.problems.source = match this.problems.source {
                                    Some(_) => None,
                                    None => Some("task:".into()),
                                };
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("problems-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(rows.is_empty(), |list| {
                        list.child(div().p_3().text_color(theme::subtle()).child("No problems"))
                    })
                    .children(rows),
            )
    }
}
