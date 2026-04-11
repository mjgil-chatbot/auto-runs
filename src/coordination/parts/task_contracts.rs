use super::*;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use schemars::JsonSchema;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPayloadKind {
    Generic,
    PlanTasks,
    ReviewFindings,
    DesignDoc,
    PerfIdeas,
    PerfBenchmark,
    CouncilControl,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskArtifactEnvelope {
    pub outcome: CoordinationTaskOutcome,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub changed_files: Option<Vec<String>>,
    #[serde(default)]
    pub tests_run: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub remaining_gaps: Vec<String>,
    #[serde(default)]
    pub design_requirements_satisfied: Vec<String>,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    pub payload_kind: TaskPayloadKind,
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
    #[serde(default)]
    pub payload: serde_json::Value,
}
fn default_contract_version() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractResponseMode {
    JsonOnly,
    JsonWithEmbeddedMarkdown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReaskStrategy {
    Full,
    Skeleton,
    Field,
}
#[derive(Clone, Debug)]
pub struct TaskContract {
    pub task_kind: CoordinationTaskKind,
    pub payload_kind: TaskPayloadKind,
    pub contract_version: u32,
    pub max_validation_reasks: u8,
    pub response_mode: ContractResponseMode,
    pub reask_strategy: ReaskStrategy,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct ReviewFindingsPayload {
    #[serde(default)]
    pub findings: Vec<super::code_review::ReviewFinding>,
    #[serde(default)]
    pub fix_tasks: Vec<GeneratedTaskSpec>,
}
static CONTRACT_REGISTRY: std::sync::LazyLock<Vec<TaskContract>> =
    std::sync::LazyLock::new(build_contract_registry);
fn build_contract_registry() -> Vec<TaskContract> {
    vec![
        TaskContract {
            task_kind: CoordinationTaskKind::PlanFlatTasks,
            payload_kind: TaskPayloadKind::PlanTasks,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::PlanCodeReview,
            payload_kind: TaskPayloadKind::PlanTasks,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::ReviewArtifacts,
            payload_kind: TaskPayloadKind::ReviewFindings,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::DraftDesignDoc,
            payload_kind: TaskPayloadKind::DesignDoc,
            contract_version: 1,
            max_validation_reasks: 1,
            response_mode: ContractResponseMode::JsonWithEmbeddedMarkdown,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::ConvergeDesignDoc,
            payload_kind: TaskPayloadKind::DesignDoc,
            contract_version: 1,
            max_validation_reasks: 1,
            response_mode: ContractResponseMode::JsonWithEmbeddedMarkdown,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::PerfParse,
            payload_kind: TaskPayloadKind::PerfIdeas,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Skeleton,
        },
    ]
}
pub fn lookup_contract(kind: CoordinationTaskKind) -> Option<&'static TaskContract> {
    CONTRACT_REGISTRY.iter().find(|c| c.task_kind == kind)
}
pub(crate) fn normalize_provider_response(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(raw) = obj
        .get("outcome")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        let canonical = match raw.trim().to_ascii_lowercase().as_str() {
            "success" | "changes" => Some("completed_with_changes"),
            "complete" | "completed" | "done" | "no_change" | "review_completed" => {
                Some("completed_no_change")
            }
            "blocking" => Some("blocked"),
            "partial" | "partially_complete" | "partially_done" => Some("partial"),
            "failure" | "error" | "fail" => Some("failed"),
            _ => None,
        };
        if let Some(canonical) = canonical {
            warn!(
                "Normalizing non-canonical provider outcome '{}' -> '{}'",
                raw, canonical
            );
            obj.insert(
                "outcome".to_string(),
                serde_json::Value::String(canonical.to_string()),
            );
        }
    }
    if let Some(payload) = obj.get_mut("payload") {
        if let Some(payload_obj) = payload.as_object_mut() {
            if let Some(findings) = payload_obj.get_mut("findings") {
                if let Some(arr) = findings.as_array_mut() {
                    for finding in arr.iter_mut() {
                        normalize_finding_fields(finding);
                    }
                }
            }
        }
    }
}
fn normalize_finding_fields(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(line_val) = obj.get("line") {
        if let Some(s) = line_val.as_str() {
            if let Ok(n) = s.parse::<u64>() {
                obj.insert("line".to_string(), serde_json::Value::Number(n.into()));
            }
        }
    }
}
pub fn parse_typed_envelope(json: &str) -> Result<TaskArtifactEnvelope> {
    let mut value: serde_json::Value =
        serde_json::from_str(json).context("invalid envelope JSON")?;
    let obj = value
        .as_object()
        .context("envelope must be a JSON object")?;
    if !obj.contains_key("payload_kind") {
        anyhow::bail!("missing payload_kind field — not a typed envelope");
    }
    normalize_provider_response(&mut value);
    let envelope: TaskArtifactEnvelope =
        serde_json::from_value(value).context("invalid typed envelope structure")?;
    Ok(envelope)
}
pub fn parse_contract_envelope_from_response(response: &str) -> Result<TaskArtifactEnvelope> {
    let json = extract_first_json_object(response).context("missing JSON object in task output")?;
    parse_typed_envelope(json)
}
pub fn typed_envelope_to_legacy_report(envelope: &TaskArtifactEnvelope) -> TaskExecutionReport {
    let fix_tasks = if envelope.payload_kind == TaskPayloadKind::ReviewFindings {
        review_findings_payload_from_envelope(envelope)
            .map(|payload| payload.fix_tasks)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    TaskExecutionReport {
        outcome: envelope.outcome.clone(),
        summary: envelope.summary.clone(),
        details: envelope.details.clone(),
        changed_files: envelope.changed_files.clone().unwrap_or_default(),
        tests_run: envelope.tests_run.clone(),
        warnings: envelope.warnings.clone(),
        blockers: envelope.blockers.clone(),
        remaining_gaps: envelope.remaining_gaps.clone(),
        design_requirements_satisfied: envelope.design_requirements_satisfied.clone(),
        validation_errors: envelope.validation_errors.clone(),
        fix_tasks,
        failure_kind: None,
        recovery_action: None,
    }
}
pub fn review_findings_payload_from_envelope(
    envelope: &TaskArtifactEnvelope,
) -> Result<ReviewFindingsPayload> {
    serde_json::from_value(envelope.payload.clone()).context("invalid ReviewFindings payload")
}
