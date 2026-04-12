use super::*;

use serde_json::Value;

pub(crate) fn build_lightweight_execution_artifact(
    task: &CoordinationTask,
    response: &str,
) -> Option<TaskArtifact> {
    let run = load_run(&task.run_id).ok()?;
    if !uses_lightweight_runtime_mode(&run.workflow_mode) {
        return None;
    }
    if !matches!(
        task.kind,
        CoordinationTaskKind::GeneratedWriteTask | CoordinationTaskKind::GeneratedReadTask
    ) {
        return None;
    }

    let trimmed = response.trim();
    let mut parsed_json = extract_first_json_object(trimmed)
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .unwrap_or(Value::Null);
    normalize_provider_response(&mut parsed_json);
    let object = parsed_json.as_object();

    let outcome = object
        .and_then(|obj| obj.get("outcome"))
        .and_then(parse_outcome_value)
        .unwrap_or_else(|| infer_lightweight_outcome_from_text(task, trimmed));
    let summary = object
        .and_then(|obj| obj.get("summary"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| infer_lightweight_summary(task.title.as_str(), trimmed));
    let details = object
        .and_then(|obj| obj.get("details"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| trimmed.to_string());
    let blockers = object
        .and_then(|obj| obj.get("blockers"))
        .map(loose_string_list)
        .unwrap_or_else(|| infer_lightweight_blockers(trimmed, outcome));
    let remaining_gaps = object
        .and_then(|obj| obj.get("remaining_gaps"))
        .map(loose_string_list)
        .unwrap_or_else(|| infer_lightweight_remaining_gaps(trimmed, outcome));
    let design_requirements_satisfied = object
        .and_then(|obj| obj.get("design_requirements_satisfied"))
        .map(loose_string_list)
        .unwrap_or_default();

    Some(TaskArtifact {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: task.assigned_provider.clone(),
        task_category: task.task_category,
        fallback_from: task.provider_hint.clone(),
        summary: summary.clone(),
        body: if details.is_empty() {
            trimmed.to_string()
        } else {
            details.clone()
        },
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome,
            summary,
            details,
            warnings: Vec::new(),
            blockers,
            remaining_gaps,
            design_requirements_satisfied,
            ..Default::default()
        },
    })
}

fn parse_outcome_value(value: &Value) -> Option<CoordinationTaskOutcome> {
    let as_str = value.as_str()?;
    serde_json::from_value::<CoordinationTaskOutcome>(Value::String(as_str.to_string())).ok()
}

fn loose_string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Value::String(item) => {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                Vec::new()
            } else {
                vec![trimmed.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

fn infer_lightweight_outcome_from_text(
    task: &CoordinationTask,
    response: &str,
) -> CoordinationTaskOutcome {
    let lower = response.trim_start().to_ascii_lowercase();
    if lower.starts_with("failed:") {
        CoordinationTaskOutcome::Failed
    } else if lower.starts_with("blocked:") {
        CoordinationTaskOutcome::Blocked
    } else if lower.starts_with("partial:") {
        CoordinationTaskOutcome::Partial
    } else if lower.starts_with("no_change:") {
        CoordinationTaskOutcome::CompletedNoChange
    } else if lower.starts_with("completed:") {
        if task.kind == CoordinationTaskKind::GeneratedWriteTask {
            CoordinationTaskOutcome::CompletedWithChanges
        } else {
            CoordinationTaskOutcome::CompletedNoChange
        }
    } else if task.kind == CoordinationTaskKind::GeneratedWriteTask {
        CoordinationTaskOutcome::CompletedWithChanges
    } else {
        CoordinationTaskOutcome::CompletedNoChange
    }
}

fn infer_lightweight_summary(task_title: &str, response: &str) -> String {
    let first_line = response
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(task_title);
    first_line
        .split_once(':')
        .map(|(_, rest)| rest.trim())
        .filter(|line| !line.is_empty())
        .unwrap_or(first_line)
        .to_string()
}

fn infer_lightweight_blockers(response: &str, outcome: CoordinationTaskOutcome) -> Vec<String> {
    if outcome != CoordinationTaskOutcome::Blocked {
        return Vec::new();
    }
    let summary = infer_lightweight_summary("", response);
    if summary.is_empty() {
        vec!["Lightweight task reported a blocker.".to_string()]
    } else {
        vec![summary]
    }
}

fn infer_lightweight_remaining_gaps(
    response: &str,
    outcome: CoordinationTaskOutcome,
) -> Vec<String> {
    if outcome != CoordinationTaskOutcome::Partial {
        return Vec::new();
    }
    let summary = infer_lightweight_summary("", response);
    if summary.is_empty() {
        vec!["Lightweight task reported remaining work.".to_string()]
    } else {
        vec![summary]
    }
}
