//! The Testing view (IDE-018).
//!
//! Tests are discovered per package and target (Cargo provider) in the background and
//! shown grouped by target. Runs execute the exact test binary with `--exact` filters in a
//! piped terminal (its output is visible in the Run panel) and results stream in as
//! libtest reports them. Results are keyed by test identity, so they survive re-discovery
//! and editing; discovery failures are shown separately from failing tests. Saving a Rust
//! source or manifest refreshes discovery while the view is open.

use std::{collections::VecDeque, time::Instant};

use ide_core::testing::{self, Executable, LibtestParser, TestId, TestResult, TestState};

use super::*;

pub(super) struct TestEntry {
    pub id: TestId,
    pub executable: Executable,
    pub location: Option<(PathBuf, usize)>,
    pub result: TestResult,
}

pub(super) struct TestRun {
    executable: Executable,
    tests: Vec<TestId>,
    view: Entity<TerminalView>,
    parser: LibtestParser,
    started: Instant,
    _subscription: Subscription,
}

#[derive(Default)]
pub(super) struct TestingState {
    pub entries: Vec<TestEntry>,
    pub discovering: bool,
    pub error: Option<String>,
    pub selected: Option<usize>,
    pub(super) run: Option<TestRun>,
    queue: VecDeque<(Executable, Vec<TestId>)>,
    task: Option<Task<()>>,
}

impl IdeShell {
    pub(super) fn show_testing(&mut self, _: &ShowTesting, _: &mut Window, cx: &mut Context<Self>) {
        self.active_sidebar = SidebarPanel::Testing;
        self.show_sidebar = true;
        if self.testing.entries.is_empty() && !self.testing.discovering {
            self.discover_tests(cx);
        }
        cx.notify();
    }

    pub(super) fn refresh_tests(
        &mut self,
        _: &RefreshTests,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.discover_tests(cx);
    }

    pub(super) fn discover_tests(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.settings_root() else {
            self.testing.error = Some("Open a folder to discover tests".into());
            cx.notify();
            return;
        };
        if !root.join("Cargo.toml").is_file() {
            self.testing.error = Some(
                "No Cargo.toml in the workspace: only the Cargo test provider is available".into(),
            );
            cx.notify();
            return;
        }
        self.testing.discovering = true;
        self.testing.error = None;
        cx.notify();
        let executor = cx.background_executor().clone();
        self.testing.task = Some(cx.spawn(async move |this, cx| {
            let found = executor
                .spawn(async move {
                    testing::discover(&root).map(|found| {
                        found
                            .into_iter()
                            .flat_map(|(executable, tests)| {
                                tests.into_iter().map(move |path| {
                                    let id = TestId {
                                        target: executable.target.clone(),
                                        path,
                                    };
                                    let location =
                                        testing::locate(&executable.root, &executable.source, &id);
                                    (id, executable.clone(), location)
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.testing.discovering = false;
                match found {
                    Ok(found) => {
                        // Keep results of tests that still exist.
                        let mut previous: HashMap<TestId, TestResult> = this
                            .testing
                            .entries
                            .drain(..)
                            .map(|entry| (entry.id, entry.result))
                            .collect();
                        this.testing.entries = found
                            .into_iter()
                            .map(|(id, executable, location)| TestEntry {
                                result: previous.remove(&id).unwrap_or_default(),
                                id,
                                executable,
                                location,
                            })
                            .collect();
                        this.testing.entries.sort_by(|a, b| a.id.cmp(&b.id));
                        this.status = format!("Discovered {} tests", this.testing.entries.len());
                    }
                    Err(error) => this.testing.error = Some(error),
                }
                cx.notify();
            });
        }));
    }

    /// Queue tests (grouped per binary) and start running.
    pub(super) fn run_tests(&mut self, ids: Vec<TestId>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            self.status = "No tests to run".into();
            cx.notify();
            return;
        }
        self.cancel_test_run(cx);
        let mut batches: Vec<(Executable, Vec<TestId>)> = Vec::new();
        for id in ids {
            let Some(entry) = self.testing.entries.iter_mut().find(|entry| entry.id == id) else {
                continue;
            };
            entry.result = TestResult {
                state: TestState::Queued,
                ..Default::default()
            };
            match batches
                .iter_mut()
                .find(|(executable, _)| executable.path == entry.executable.path)
            {
                Some((_, tests)) => tests.push(id),
                None => batches.push((entry.executable.clone(), vec![id])),
            }
        }
        self.testing.queue = batches.into();
        self.start_next_test_batch(cx);
    }

    fn start_next_test_batch(&mut self, cx: &mut Context<Self>) {
        let Some((executable, tests)) = self.testing.queue.pop_front() else {
            self.status = self.test_summary();
            cx.notify();
            return;
        };
        let all_of_target = self
            .testing
            .entries
            .iter()
            .filter(|entry| entry.executable.path == executable.path)
            .count()
            == tests.len();
        let mut args: Vec<String> = Vec::new();
        if !all_of_target {
            args.extend(tests.iter().map(|id| id.path.clone()));
            args.push("--exact".into());
        }
        args.extend([
            "--test-threads".into(),
            "1".into(),
            "--include-ignored".into(),
        ]);
        // `--include-ignored` would run ignored tests; only pass it for explicit selections.
        if all_of_target {
            args.retain(|arg| arg != "--include-ignored");
        }
        for entry in &mut self.testing.entries {
            if tests.contains(&entry.id) {
                entry.result.state = TestState::Running;
            }
        }
        let view = cx.new(TerminalView::new);
        let subscription = cx.subscribe(&view, |this, _, event, cx| this.test_output(event, cx));
        let result = view.update(cx, |view, cx| {
            view.run_piped(
                terminal::Options {
                    shell: Some((executable.path.to_string_lossy().into_owned(), args)),
                    working_directory: Some(executable.root.clone()),
                    ..Default::default()
                },
                cx,
            )
        });
        if let Err(error) = result {
            self.status = format!("Could not run {}: {error}", executable.target.label());
            return;
        }
        self.testing.run = Some(TestRun {
            executable,
            tests,
            view,
            parser: LibtestParser::default(),
            started: Instant::now(),
            _subscription: subscription,
        });
        cx.notify();
    }

    fn test_output(&mut self, event: &TerminalEvent, cx: &mut Context<Self>) {
        let Some(run) = &mut self.testing.run else {
            return;
        };
        match event {
            TerminalEvent::Output(bytes) => {
                run.parser.feed(&String::from_utf8_lossy(bytes));
                self.apply_test_results(false);
            }
            TerminalEvent::Finished(code) => {
                run.parser.finish();
                self.apply_test_results(true);
                let failed_to_run = *code != Some(0)
                    && self
                        .testing
                        .run
                        .as_ref()
                        .is_some_and(|run| run.parser.results.is_empty());
                if let Some(run) = self.testing.run.take() {
                    for entry in &mut self.testing.entries {
                        if run.tests.contains(&entry.id) && entry.result.state == TestState::Running
                        {
                            entry.result.state = if failed_to_run {
                                TestState::Failed
                            } else {
                                TestState::Unknown
                            };
                            if failed_to_run {
                                entry.result.output =
                                    "The test binary failed before reporting results".into();
                            }
                        }
                    }
                }
                self.start_next_test_batch(cx);
            }
            TerminalEvent::Exited(_) => {}
        }
        cx.notify();
    }

    fn apply_test_results(&mut self, finished: bool) {
        let Some(run) = &mut self.testing.run else {
            return;
        };
        let elapsed = run.started.elapsed();
        let single = run.tests.len() == 1;
        for (path, state) in run.parser.results.drain(..) {
            let Some(entry) = self.testing.entries.iter_mut().find(|entry| {
                entry.executable.path == run.executable.path && entry.id.path == path
            }) else {
                continue;
            };
            entry.result.state = state;
            // Per-test durations are exact for single-test runs (libtest's stable output
            // does not report per-test times).
            entry.result.duration = single.then_some(elapsed);
        }
        if finished {
            for (path, output) in run.parser.outputs.drain() {
                if let Some(entry) = self.testing.entries.iter_mut().find(|entry| {
                    entry.executable.path == run.executable.path && entry.id.path == path
                }) {
                    entry.result.failure = testing::failure_location(&output, &run.executable.root);
                    entry.result.output = output;
                }
            }
        }
    }

    pub(super) fn cancel_test_run(&mut self, cx: &mut Context<Self>) {
        self.testing.queue.clear();
        if let Some(run) = self.testing.run.take() {
            run.view.update(cx, |view, cx| view.terminate(cx));
        }
        for entry in &mut self.testing.entries {
            if matches!(entry.result.state, TestState::Running | TestState::Queued) {
                entry.result.state = TestState::Unknown;
            }
        }
        cx.notify();
    }

    fn test_summary(&self) -> String {
        let count = |state: TestState| {
            self.testing
                .entries
                .iter()
                .filter(|entry| entry.result.state == state)
                .count()
        };
        format!(
            "Tests: {} passed, {} failed, {} skipped",
            count(TestState::Passed),
            count(TestState::Failed),
            count(TestState::Skipped)
        )
    }

    pub(super) fn run_all_tests(
        &mut self,
        _: &RunAllTests,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self
            .testing
            .entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        self.run_tests(ids, cx);
    }

    pub(super) fn rerun_failed_tests(
        &mut self,
        _: &RerunFailedTests,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = self
            .testing
            .entries
            .iter()
            .filter(|entry| entry.result.state == TestState::Failed)
            .map(|entry| entry.id.clone())
            .collect();
        self.run_tests(ids, cx);
    }

    pub(super) fn cancel_tests(&mut self, _: &CancelTests, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_test_run(cx);
        self.status = "Cancelled the test run".into();
    }

    pub(super) fn render_testing(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut last: Option<&testing::Target> = None;
        for (ix, entry) in self.testing.entries.iter().enumerate() {
            if last != Some(&entry.id.target) {
                let target = entry.id.target.clone();
                let ids: Vec<TestId> = self
                    .testing
                    .entries
                    .iter()
                    .filter(|other| other.id.target == target)
                    .map(|other| other.id.clone())
                    .collect();
                rows.push(
                    div()
                        .id(("test-target", ix))
                        .px_2()
                        .pt_1()
                        .flex()
                        .gap_1()
                        .items_center()
                        .font_weight(FontWeight::MEDIUM)
                        .child(div().flex_1().truncate().child(target.label()))
                        .child(ui::count_badge(ids.len()))
                        .child(
                            Button::new(("test-run-target", ix))
                                .ghost()
                                .xsmall()
                                .icon(Icon::new(AppIcon::Play))
                                .tooltip("Run these tests")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_tests(ids.clone(), cx)
                                })),
                        )
                        .into_any_element(),
                );
                last = Some(&entry.id.target);
            }
            let color = match entry.result.state {
                TestState::Passed => theme::git_change(Change::Added),
                TestState::Failed => theme::git_change(Change::Deleted),
                TestState::Running | TestState::Queued => theme::accent(),
                _ => theme::muted(),
            };
            let id = entry.id.clone();
            let location = entry.location.clone();
            rows.push(
                div()
                    .id(("test", ix))
                    .group(ui::ROW_GROUP)
                    .pl_4()
                    .pr_2()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::hover()))
                    .when(self.testing.selected == Some(ix), |row| {
                        row.bg(theme::active_row())
                    })
                    .child(
                        div()
                            .w(px(52.))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(color)
                            .child(entry.result.state.label()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(entry.id.path.clone()),
                    )
                    .when_some(entry.result.duration, |row, duration| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme::subtle())
                                .child(format!("{:.2}s", duration.as_secs_f32())),
                        )
                    })
                    .child(
                        div()
                            .invisible()
                            .group_hover(ui::ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new(("test-run", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(AppIcon::Play))
                                    .tooltip("Run")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.run_tests(vec![id.clone()], cx)
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.testing.selected = Some(ix);
                        let target = this.testing.entries[ix]
                            .result
                            .failure
                            .clone()
                            .or_else(|| location.clone().map(|(path, line)| (path, line, 1)));
                        if let Some((path, line, column)) = target {
                            let hit = SearchMatch {
                                path,
                                line,
                                column,
                                preview: String::new(),
                            };
                            this.record_location(&hit, cx);
                            this.open_at(hit, window, cx);
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        let selected_output = self
            .testing
            .selected
            .and_then(|ix| self.testing.entries.get(ix))
            .filter(|entry| !entry.result.output.is_empty())
            .map(|entry| entry.result.output.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .child(
                ui::panel_header("TESTING")
                    .child(
                        Button::new("tests-run-all")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(AppIcon::Play))
                            .tooltip("Run All Tests")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.run_all_tests(&RunAllTests, window, cx)
                            })),
                    )
                    .child(
                        Button::new("tests-rerun-failed")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Redo)
                            .tooltip("Rerun Failed Tests")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.rerun_failed_tests(&RerunFailedTests, window, cx)
                            })),
                    )
                    .child(
                        Button::new("tests-cancel")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("Cancel Test Run")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_tests(&CancelTests, window, cx)
                            })),
                    )
                    .child(
                        Button::new("tests-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Loader)
                            .tooltip("Refresh Tests")
                            .loading(self.testing.discovering)
                            .on_click(cx.listener(|this, _, _, cx| this.discover_tests(cx))),
                    ),
            )
            .when_some(self.testing.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme::git_change(Change::Deleted))
                        .child(error),
                )
            })
            .when(self.testing.discovering, |panel| {
                panel.child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(theme::muted())
                        .child("Discovering tests…"),
                )
            })
            .child(
                div()
                    .id("tests")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .text_xs()
                    .children(rows),
            )
            .when_some(selected_output, |panel, output| {
                panel.child(
                    div()
                        .id("test-output")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .p_2()
                        .border_t_1()
                        .border_color(theme::border())
                        .font_family(theme::mono_family())
                        .text_xs()
                        .child(output),
                )
            })
    }
}
