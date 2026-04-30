//! Auto-runs-owned workflow registry and prompt expansion.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rust_debug::tracing::{info, warn};
use serde::Deserialize;

use crate::workspace;

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub title: String,
    pub section: String,
    #[serde(default)]
    pub order: u32,
    #[serde(default)]
    pub arguments: String,
    pub summary: String,
    #[serde(default)]
    pub when_to_use: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<WorkflowInput>,
    #[serde(default)]
    pub context_files: Vec<String>,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowInput {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct ExpandedWorkflow {
    pub name: String,
    pub prompt: String,
}

static WORKFLOW_SPECS: OnceLock<Vec<WorkflowSpec>> = OnceLock::new();
static PROJECT_ROOT: OnceLock<PathBuf> = OnceLock::new();

fn cached_project_root() -> PathBuf {
    PROJECT_ROOT
        .get_or_init(workspace::current_project_root)
        .clone()
}

pub fn workflow_specs() -> &'static [WorkflowSpec] {
    WORKFLOW_SPECS.get_or_init(load_workflow_specs).as_slice()
}

pub fn workflow_command_names() -> Vec<String> {
    workflow_specs()
        .iter()
        .map(|spec| format!("/{}", spec.name))
        .collect()
}

pub fn expand_invocation(invocation: &str) -> Option<ExpandedWorkflow> {
    let project_root = cached_project_root();
    expand_invocation_for_project_root(invocation, &project_root)
}

pub fn expand_invocation_for_project_root(
    invocation: &str,
    project_root: &Path,
) -> Option<ExpandedWorkflow> {
    let trimmed = invocation.trim();
    let command = trimmed.strip_prefix('/')?;
    let (name, args) = split_command_and_args(command);
    let spec = workflow_specs().iter().find(|spec| spec.name == name)?;

    Some(ExpandedWorkflow {
        name: spec.name.clone(),
        prompt: render_prompt(spec, trimmed, args, project_root),
    })
}

pub fn expand_message_for_history(input: &str) -> Cow<'_, str> {
    let project_root = cached_project_root();
    expand_message_for_history_in_project_root(input, &project_root)
}

pub fn expand_message_for_history_in_project_root<'a>(
    input: &'a str,
    project_root: &Path,
) -> Cow<'a, str> {
    match expand_invocation_for_project_root(input, project_root) {
        Some(expanded) => Cow::Owned(expanded.prompt),
        None => Cow::Borrowed(input),
    }
}

fn split_command_and_args(invocation: &str) -> (&str, &str) {
    if let Some(index) = invocation.find(char::is_whitespace) {
        let (name, rest) = invocation.split_at(index);
        (name.trim(), rest.trim())
    } else {
        (invocation.trim(), "")
    }
}

fn render_prompt(
    spec: &WorkflowSpec,
    raw_invocation: &str,
    args: &str,
    project_root: &Path,
) -> String {
    let expected_arguments = if spec.arguments.trim().is_empty() {
        "<none>"
    } else {
        spec.arguments.trim()
    };
    let user_arguments = if args.is_empty() { "<none>" } else { args };

    let mut sections = vec![
        format!("Workflow command: /{}", spec.name),
        format!("Title: {}", spec.title),
        format!("Section: {}", spec.section),
        format!("Summary: {}", spec.summary),
        format!("Raw invocation: {}", raw_invocation),
        format!("Expected arguments: {}", expected_arguments),
        format!("User-supplied arguments: {}", user_arguments),
    ];

    if !spec.when_to_use.is_empty() {
        sections.push(format!("When to use:\n{}", render_list(&spec.when_to_use)));
    }

    if !spec.inputs.is_empty() {
        let inputs = spec
            .inputs
            .iter()
            .map(|input| {
                let requirement = if input.required {
                    "required"
                } else {
                    "optional"
                };
                format!("- {} ({requirement}): {}", input.name, input.description)
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Expected inputs:\n{}", inputs));
    }

    if !spec.context_files.is_empty() {
        let context_files = spec
            .context_files
            .iter()
            .map(|path| format!("- {}", project_root.join(path).display()))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("Repository context files:\n{}", context_files));
    }

    if !spec.steps.is_empty() {
        sections.push(format!("Workflow steps:\n{}", render_list(&spec.steps)));
    }

    if !spec.outputs.is_empty() {
        sections.push(format!("Output contract:\n{}", render_list(&spec.outputs)));
    }

    if !spec.examples.is_empty() {
        sections.push(format!("Examples:\n{}", render_list(&spec.examples)));
    }

    sections.push(
        "If any required input is missing or ambiguous, ask a clarifying question before making changes."
            .to_string(),
    );

    sections.join("\n\n")
}

fn render_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {}", item))
        .collect::<Vec<_>>()
        .join("\n")
}

fn load_workflow_specs() -> Vec<WorkflowSpec> {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("workflow-registry/commands");
    let mut specs = Vec::new();

    let entries = match std::fs::read_dir(&commands_dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!(
                path = %commands_dir.display(),
                error = %error,
                "workflow registry directory unavailable"
            );
            return specs;
        }
    };

    for entry in entries.filter_map(|entry| match entry {
        Ok(e) => Some(e),
        Err(error) => {
            warn!(
                path = %commands_dir.display(),
                error = %error,
                "failed to read workflow registry directory entry"
            );
            None
        }
    }) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<WorkflowSpec>(&contents) {
                Ok(spec) => specs.push(spec),
                Err(error) => {
                    warn!(path = %path.display(), error = %error, "invalid workflow spec")
                }
            },
            Err(error) => {
                warn!(path = %path.display(), error = %error, "failed to read workflow spec")
            }
        }
    }

    specs.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.name.cmp(&right.name))
    });

    info!(count = specs.len(), path = %commands_dir.display(), "loaded workflow specs");
    specs
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{expand_invocation, expand_invocation_for_project_root, workflow_specs};

    #[test]
    fn loads_known_workflow() {
        let spec = workflow_specs()
            .iter()
            .find(|spec| spec.name == "code-review")
            .expect("expected code-review workflow");

        assert_eq!(spec.title, "Code Review");
        assert!(!spec.steps.is_empty());
    }

    #[test]
    fn expands_known_workflow_invocation() {
        let expanded = expand_invocation("/code-review --scope worktree")
            .expect("expected workflow expansion");

        assert_eq!(expanded.name, "code-review");
        assert!(expanded.prompt.contains("Workflow command: /code-review"));
        assert!(expanded
            .prompt
            .contains("User-supplied arguments: --scope worktree"));
        assert!(expanded.prompt.contains("Output contract:"));
    }

    #[test]
    fn expands_known_workflow_invocation_for_explicit_project_root() {
        let expanded = expand_invocation_for_project_root(
            "/code-review --scope worktree",
            Path::new("/tmp/workspace"),
        )
        .expect("expected workflow expansion");

        assert!(expanded
            .prompt
            .contains("Repository context files:\n- /tmp/workspace/"));
    }

    #[test]
    fn ignores_unknown_workflow_invocation() {
        assert!(expand_invocation("/not-a-real-workflow arg").is_none());
        assert!(expand_invocation("plain text").is_none());
    }
}
