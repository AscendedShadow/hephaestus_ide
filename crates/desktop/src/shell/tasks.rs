//! Task pipelines (IDE-019) feeding the Problems view (IDE-020).
//!
//! Running a task plans its dependency stages (`ide_core::tasks`), then starts each stage's
//! tasks in their own piped terminal (shown as tabs in the Run panel). A stage completes
//! when every task exits successfully, or, for background tasks, when their
//! `ready_pattern` matches. A failure or cancellation stops the pipeline and the launch
//! that was waiting for it (e.g. a debug profile's `pre_launch_task`). Cancelling, a
//! workspace switch, or rerunning terminates the running process trees.

use std::collections::{HashMap, VecDeque};

use ide_core::{problems::OutputParser, tasks as planner};
use regex::Regex;

use super::debugging::PendingLaunch;
use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;
use crate::settings::{DependsOrder, TaskConfig, TaskGroup};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RunState {
    Running,
    /// A background task signalled readiness and keeps running.
    Ready,
    Succeeded,
    Failed(Option<i32>),
    Cancelled,
}

impl RunState {
    pub fn label(&self) -> String {
        match self {
            Self::Running => "running".into(),
            Self::Ready => "ready".into(),
            Self::Succeeded => "done".into(),
            Self::Failed(Some(code)) => format!("failed ({code})"),
            Self::Failed(None) => "failed".into(),
            Self::Cancelled => "cancelled".into(),
        }
    }

    fn settled(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

pub(super) struct TaskRun {
    pub run: u64,
    pub task: String,
    pub name: String,
    pub view: Entity<TerminalView>,
    pub state: RunState,
    pub background: bool,
    ready: Option<Regex>,
    output: String,
    parser: OutputParser,
    _subscription: Subscription,
}

pub(super) struct Pipeline {
    pub target: String,
    stages: VecDeque<Vec<String>>,
    running: Vec<u64>,
    then: Option<PendingLaunch>,
}

#[derive(Default)]
pub(super) struct TaskState {
    pub runs: Vec<TaskRun>,
    pub selected: Option<u64>,
    pub pipeline: Option<Pipeline>,
    next_run: u64,
    last: Option<(String, Option<PendingLaunch>)>,
}

impl IdeShell {
    fn task_configs(&mut self, cx: &mut Context<Self>) -> Vec<TaskConfig> {
        self.load_settings(cx)
            .map(|settings| settings.tasks)
            .unwrap_or_default()
    }

    /// Values for `${…}` variables in task programs, arguments, directories and env.
    pub(super) fn task_variables(&self) -> HashMap<&'static str, String> {
        let mut variables = HashMap::new();
        let root = self.project_directory().map(Path::to_path_buf);
        if let Some(root) = &root {
            variables.insert("workspaceFolder", root.display().to_string());
            variables.insert(
                "workspaceFolderBasename",
                root.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        if let Some(file) = self.document().path() {
            variables.insert("file", file.display().to_string());
            variables.insert(
                "fileBasename",
                file.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            variables.insert(
                "fileBasenameNoExtension",
                file.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            );
            variables.insert(
                "fileDirname",
                file.parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default(),
            );
            if let Some(root) = &root
                && let Ok(relative) = file.strip_prefix(root)
            {
                variables.insert("relativeFile", relative.display().to_string());
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            variables.insert("cwd", cwd.display().to_string());
        }
        variables
    }

    pub(super) fn run_task_picker(
        &mut self,
        _: &RunTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tasks = self.task_configs(cx);
        if tasks.is_empty() {
            self.status = "Configure \"tasks\" in settings.json to run tasks".into();
            cx.notify();
            return;
        }
        let items = tasks
            .iter()
            .map(|task| PickerItem {
                label: task.name().to_string(),
                detail: format!(
                    "{} {}{}",
                    task.program,
                    task.args.join(" "),
                    if task.depends_on.is_empty() {
                        String::new()
                    } else {
                        format!("  (after {})", task.depends_on.join(", "))
                    }
                ),
                filter: format!("{} {}", task.id, task.name()),
                depth: 0,
                action: PickerAction::RunTask(task.id.clone()),
            })
            .collect();
        self.show_picker(PickerKind::Locations, "Run Task".into(), items, window, cx);
    }

    pub(super) fn run_build_task(
        &mut self,
        _: &RunBuildTask,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tasks = self.task_configs(cx);
        let build: Vec<&TaskConfig> = tasks
            .iter()
            .filter(|task| task.group == Some(TaskGroup::Build))
            .collect();
        let chosen = build
            .iter()
            .find(|task| task.default)
            .or(build.first())
            .map(|task| task.id.clone());
        match chosen {
            Some(id) => self.run_task_then(id, None, cx),
            None => {
                self.status = "No build task: set \"group\": \"build\" on a task".into();
                cx.notify();
            }
        }
    }

    pub(super) fn rerun_last_task(
        &mut self,
        _: &RerunLastTask,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.tasks.last.clone() {
            Some((id, then)) => self.run_task_then(id, then, cx),
            None => {
                self.status = "No task has run yet".into();
                cx.notify();
            }
        }
    }

    pub(super) fn cancel_task(&mut self, _: &CancelTask, _: &mut Window, cx: &mut Context<Self>) {
        self.cancel_tasks("Cancelled", cx);
    }

    /// Stop every running task and abandon the pipeline (and its pending launch).
    pub(super) fn cancel_tasks(&mut self, reason: &str, cx: &mut Context<Self>) {
        let had_pipeline = self.tasks.pipeline.take().is_some();
        let mut stopped = 0;
        for run in &mut self.tasks.runs {
            if matches!(run.state, RunState::Running | RunState::Ready) {
                run.view.update(cx, |view, cx| view.terminate(cx));
                run.state = RunState::Cancelled;
                stopped += 1;
            }
        }
        if had_pipeline || stopped > 0 {
            self.status = format!("{reason}: stopped {stopped} task(s)");
        }
        cx.notify();
    }

    /// Run a task (after its dependencies), then the pending launch if all succeed.
    pub(super) fn run_task_then(
        &mut self,
        id: String,
        then: Option<PendingLaunch>,
        cx: &mut Context<Self>,
    ) {
        let tasks = self.task_configs(cx);
        let nodes: Vec<planner::Node> = tasks
            .iter()
            .map(|task| planner::Node {
                id: task.id.clone(),
                depends_on: task.depends_on.clone(),
                parallel: task.depends_order == DependsOrder::Parallel,
            })
            .collect();
        let stages = match planner::plan(&nodes, &id) {
            Ok(stages) => stages,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        // A rerun replaces the previous pipeline.
        self.cancel_tasks("Restarted", cx);
        self.tasks.last = Some((id.clone(), then.clone()));
        self.tasks.pipeline = Some(Pipeline {
            target: id,
            stages: stages.into(),
            running: Vec::new(),
            then,
        });
        self.active_panel = ToolPanel::Run;
        self.start_next_stage(cx);
    }

    fn start_next_stage(&mut self, cx: &mut Context<Self>) {
        let Some(stage) = self
            .tasks
            .pipeline
            .as_mut()
            .and_then(|pipeline| pipeline.stages.pop_front())
        else {
            return self.finish_pipeline(cx);
        };
        let tasks = self.task_configs(cx);
        for id in stage {
            let Some(task) = tasks.iter().find(|task| task.id == id).cloned() else {
                continue;
            };
            match self.start_task(&task, cx) {
                Ok(run) => {
                    if let Some(pipeline) = &mut self.tasks.pipeline {
                        pipeline.running.push(run);
                    }
                }
                Err(error) => {
                    self.status = format!("Task \"{}\": {error}", task.name());
                    self.tasks.pipeline = None;
                    cx.notify();
                    return;
                }
            }
        }
        cx.notify();
    }

    fn finish_pipeline(&mut self, cx: &mut Context<Self>) {
        let Some(pipeline) = self.tasks.pipeline.take() else {
            return;
        };
        self.status = format!("Task \"{}\" finished", pipeline.target);
        match pipeline.then {
            Some(PendingLaunch::Debug(mut profile)) => {
                profile.pre_launch_task = None;
                self.launch_profile(profile, cx);
            }
            None => {}
        }
        cx.notify();
    }

    fn start_task(&mut self, task: &TaskConfig, cx: &mut Context<Self>) -> Result<u64, String> {
        let variables = self.task_variables();
        let expand = |text: &str| planner::substitute(text, &variables);
        let program = expand(&task.program)?;
        if program.trim().is_empty() {
            return Err("no program configured".into());
        }
        let args = task
            .args
            .iter()
            .map(|arg| expand(arg))
            .collect::<Result<Vec<_>, _>>()?;
        let env = task
            .env
            .iter()
            .map(|(key, value)| Ok((key.clone(), expand(value)?)))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let directory = match &task.working_directory {
            Some(directory) => Some(PathBuf::from(expand(&directory.to_string_lossy())?)),
            None => None,
        };
        let directory = self
            .config_directory(directory, cx)
            .ok_or("the working directory does not exist")?;
        let ready = match &task.ready_pattern {
            Some(pattern) => {
                Some(Regex::new(pattern).map_err(|error| format!("ready_pattern: {error}"))?)
            }
            None => None,
        };
        self.tasks.next_run += 1;
        let run = self.tasks.next_run;
        let view = cx.new(TerminalView::new);
        let subscription = cx.subscribe(&view, move |this, _, event, cx| {
            this.task_event(run, event, cx)
        });
        let started = view.update(cx, |view, cx| {
            view.run_piped(
                terminal::Options {
                    shell: Some((program.clone(), args.clone())),
                    working_directory: directory.clone(),
                    env,
                    ..Default::default()
                },
                cx,
            )
        });
        started?;
        let source = format!("task:{}", task.id);
        let base = directory
            .clone()
            .or_else(|| self.project_directory().map(Path::to_path_buf));
        self.clear_build_problems(&source);
        self.tasks
            .runs
            .retain(|old| old.task != task.id || old.state == RunState::Running);
        self.tasks.runs.push(TaskRun {
            run,
            task: task.id.clone(),
            name: task.name().to_string(),
            view,
            state: RunState::Running,
            background: task.background,
            ready,
            output: String::new(),
            parser: OutputParser::new(base.as_deref().unwrap_or(Path::new(".")), source),
            _subscription: subscription,
        });
        self.tasks.selected = Some(run);
        self.status = format!("Running task \"{}\"", task.name());
        Ok(run)
    }

    fn task_event(&mut self, run: u64, event: &TerminalEvent, cx: &mut Context<Self>) {
        let Some(ix) = self.tasks.runs.iter().position(|task| task.run == run) else {
            return;
        };
        match event {
            TerminalEvent::Output(bytes) => {
                let problems = self.tasks.runs[ix].parser.feed(bytes);
                self.add_build_problems(problems);
                let task = &mut self.tasks.runs[ix];
                if task.state == RunState::Running && task.background {
                    task.output.push_str(&String::from_utf8_lossy(bytes));
                    let stripped = ide_core::problems::strip_ansi(&task.output);
                    if task
                        .ready
                        .as_ref()
                        .is_some_and(|ready| ready.is_match(&stripped))
                    {
                        task.state = RunState::Ready;
                        task.output.clear();
                        self.task_settled(run, cx);
                    } else if task.output.len() > 64 * 1024 {
                        let cut = task.output.len() - 16 * 1024;
                        let cut = (cut..task.output.len())
                            .find(|ix| task.output.is_char_boundary(*ix))
                            .unwrap_or(0);
                        task.output.drain(..cut);
                    }
                }
            }
            TerminalEvent::Finished(code) => {
                let problems = self.tasks.runs[ix].parser.finish();
                self.add_build_problems(problems);
                let task = &mut self.tasks.runs[ix];
                if task.state == RunState::Cancelled {
                    return;
                }
                let was_ready = task.state == RunState::Ready;
                task.state = if *code == Some(0) {
                    RunState::Succeeded
                } else {
                    RunState::Failed(*code)
                };
                if !was_ready {
                    self.task_settled(run, cx);
                }
            }
            TerminalEvent::Exited(_) => {}
        }
        cx.notify();
    }

    fn task_settled(&mut self, run: u64, cx: &mut Context<Self>) {
        let Some(task) = self.tasks.runs.iter().find(|task| task.run == run) else {
            return;
        };
        let (name, state) = (task.name.clone(), task.state.clone());
        let Some(pipeline) = &mut self.tasks.pipeline else {
            return;
        };
        if !pipeline.running.contains(&run) {
            return;
        }
        if let RunState::Failed(_) = state {
            let target = pipeline.target.clone();
            let waiting = pipeline.then.is_some();
            self.tasks.pipeline = None;
            self.status = format!(
                "Task \"{name}\" {}; {}",
                state.label(),
                if waiting {
                    format!("\"{target}\" and the pending launch were not started")
                } else if name == target {
                    "see the Run panel".to_string()
                } else {
                    format!("\"{target}\" was not started")
                }
            );
            return;
        }
        let runs = &self.tasks.runs;
        let all_settled = pipeline.running.iter().all(|id| {
            runs.iter()
                .find(|task| task.run == *id)
                .is_none_or(|task| task.state.settled())
        });
        if all_settled {
            pipeline.running.clear();
            self.start_next_stage(cx);
        }
    }

    pub(super) fn render_task_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(26.))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .text_xs()
            .child(
                div()
                    .id("task-tab-command")
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(self.tasks.selected.is_none(), |tab| tab.bg(theme::hover()))
                    .child("Command")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tasks.selected = None;
                        cx.notify();
                    })),
            )
            .children(self.tasks.runs.iter().map(|task| {
                let run = task.run;
                div()
                    .id(("task-tab", run))
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .when(self.tasks.selected == Some(run), |tab| {
                        tab.bg(theme::hover())
                    })
                    .when(matches!(task.state, RunState::Failed(_)), |tab| {
                        tab.text_color(theme::git_change(Change::Deleted))
                    })
                    .child(format!("{} · {}", task.name, task.state.label()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tasks.selected = Some(run);
                        cx.notify();
                    }))
            }))
    }

    pub(super) fn selected_task_view(&self) -> Entity<TerminalView> {
        self.tasks
            .selected
            .and_then(|run| self.tasks.runs.iter().find(|task| task.run == run))
            .map(|task| task.view.clone())
            .unwrap_or_else(|| self.task_terminal.clone())
    }
}
