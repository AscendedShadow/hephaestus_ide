//! Task dependency planning and variable substitution (IDE-019).
//!
//! A task's `depends_on` list runs before the task. With `sequence` order (default) the
//! dependencies run one after another in list order; with `parallel` order they start
//! together. A task needed by several others runs once per pipeline. Missing ids and
//! dependency cycles are reported before anything starts.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub depends_on: Vec<String>,
    pub parallel: bool,
}

/// Plan the stages for running `target`: each stage's tasks start together; a stage
/// starts when the previous one succeeded.
pub fn plan(nodes: &[Node], target: &str) -> Result<Vec<Vec<String>>, String> {
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|node| (node.id.as_str(), node)).collect();
    let mut stages = Vec::new();
    let mut done = HashSet::new();
    let mut visiting = Vec::new();
    visit(target, &by_id, &mut visiting, &mut done, &mut stages)?;
    Ok(stages)
}

fn visit<'a>(
    id: &'a str,
    nodes: &HashMap<&'a str, &'a Node>,
    visiting: &mut Vec<&'a str>,
    done: &mut HashSet<&'a str>,
    stages: &mut Vec<Vec<String>>,
) -> Result<(), String> {
    if done.contains(id) {
        return Ok(());
    }
    if let Some(start) = visiting.iter().position(|seen| *seen == id) {
        let cycle: Vec<&str> = visiting[start..].iter().copied().chain([id]).collect();
        return Err(format!("Task dependency cycle: {}", cycle.join(" → ")));
    }
    let node = nodes.get(id).ok_or_else(|| match visiting.last() {
        Some(parent) => format!("Task \"{parent}\" depends on unknown task \"{id}\""),
        None => format!("Unknown task \"{id}\""),
    })?;
    visiting.push(id);
    if node.parallel && node.depends_on.len() > 1 {
        // Plan each dependency separately, then run their stages side by side.
        let mut lanes = Vec::new();
        for dependency in &node.depends_on {
            let mut lane = Vec::new();
            visit(dependency, nodes, visiting, done, &mut lane)?;
            lanes.push(lane);
        }
        let depth = lanes.iter().map(Vec::len).max().unwrap_or(0);
        for level in 0..depth {
            let mut stage = Vec::new();
            for lane in &lanes {
                if let Some(tasks) = lane.get(level) {
                    stage.extend(tasks.iter().cloned());
                }
            }
            stages.push(stage);
        }
    } else {
        for dependency in &node.depends_on {
            visit(dependency, nodes, visiting, done, stages)?;
        }
    }
    visiting.pop();
    done.insert(id);
    stages.push(vec![id.to_string()]);
    Ok(())
}

/// Substitute `${name}` variables (and `${env:NAME}`) in `text`. Unknown variables are
/// reported as an error naming the variable.
pub fn substitute(text: &str, variables: &HashMap<&str, String>) -> Result<String, String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("Unclosed variable in {text:?}"))?;
        let name = &after[..end];
        let value = if let Some(variable) = name.strip_prefix("env:") {
            std::env::var(variable).unwrap_or_default()
        } else {
            variables
                .get(name)
                .cloned()
                .ok_or_else(|| format!("Unknown variable ${{{name}}}"))?
        };
        result.push_str(&value);
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, depends_on: &[&str], parallel: bool) -> Node {
        Node {
            id: id.into(),
            depends_on: depends_on.iter().map(|id| id.to_string()).collect(),
            parallel,
        }
    }

    #[test]
    fn sequential_and_parallel_dependencies_become_stages() {
        let nodes = vec![
            node("gen", &[], false),
            node("lint", &[], false),
            node("build", &["gen"], false),
            node("check", &["build", "lint"], true),
            node("all", &["check", "gen"], false),
        ];
        assert_eq!(
            plan(&nodes, "build").unwrap(),
            vec![vec!["gen"], vec!["build"]]
        );
        assert_eq!(
            plan(&nodes, "check").unwrap(),
            vec![vec!["gen", "lint"], vec!["build"], vec!["check"]]
        );
        // `gen` runs once even though two tasks need it.
        assert_eq!(
            plan(&nodes, "all").unwrap(),
            vec![
                vec!["gen", "lint"],
                vec!["build"],
                vec!["check"],
                vec!["all"]
            ]
        );
    }

    #[test]
    fn cycles_and_missing_tasks_are_reported() {
        let nodes = vec![
            node("a", &["b"], false),
            node("b", &["a"], false),
            node("c", &["x"], false),
        ];
        assert_eq!(
            plan(&nodes, "a").unwrap_err(),
            "Task dependency cycle: a → b → a"
        );
        assert_eq!(
            plan(&nodes, "c").unwrap_err(),
            "Task \"c\" depends on unknown task \"x\""
        );
        assert!(plan(&nodes, "zzz").unwrap_err().contains("Unknown task"));
    }

    #[test]
    fn variables_expand() {
        let variables = HashMap::from([
            ("workspaceFolder", "/w".to_string()),
            ("file", "/w/a.rs".to_string()),
        ]);
        assert_eq!(
            substitute("${workspaceFolder}/out --input ${file}", &variables).unwrap(),
            "/w/out --input /w/a.rs"
        );
        assert!(
            substitute("${nope}", &variables)
                .unwrap_err()
                .contains("${nope}")
        );
        assert!(substitute("${file", &variables).is_err());
    }
}
