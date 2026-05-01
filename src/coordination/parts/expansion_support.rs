use super::*;
#[allow(unused_imports)]
use crate::config;
#[allow(unused_imports)]
use crate::data::ChatSession;
#[allow(unused_imports)]
use crate::memory;
#[allow(unused_imports)]
use crate::model::ModelProvider;
#[allow(unused_imports)]
use crate::provider_bridge::{
    autonomous_provider_overrides, call_provider_cli_stream_with_overrides,
};
#[allow(unused_imports)]
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
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

pub fn default_task_category_for_kind(kind: &CoordinationTaskKind) -> Option<TaskCategory> {
    match kind {
        CoordinationTaskKind::GeneratedWriteTask | CoordinationTaskKind::ApplyApprovedEdits => {
            Some(TaskCategory::DeepWrite)
        }
        CoordinationTaskKind::GeneratedReadTask => Some(TaskCategory::Research),
        CoordinationTaskKind::ReviewArtifacts => Some(TaskCategory::Review),
        _ => None,
    }
}

pub fn seeded_acceptance_checks(
    kind: &CoordinationTaskKind,
    write_scope: &[String],
) -> Vec<String> {
    match kind {
        CoordinationTaskKind::MaterializeDesignDoc => vec![
            "The canonical design doc was written to the declared path.".to_string(),
            "No files outside the declared write scope were changed.".to_string(),
        ],
        CoordinationTaskKind::PlanCodeReview => vec![
            "The review planner emitted read-only review tasks.".to_string(),
            "Review profile freshness was checked before review execution.".to_string(),
        ],
        CoordinationTaskKind::RalphPlan => vec![
            "The plan explicitly enumerates every completion criterion from the run configuration."
                .to_string(),
            "Each criterion is tracked with a status row and current evidence.".to_string(),
        ],
        CoordinationTaskKind::GeneratedWriteTask
        | CoordinationTaskKind::ApplyApprovedEdits
        | CoordinationTaskKind::RalphExecute
        | CoordinationTaskKind::PerfApply => vec![
            "The owned capability is implemented in the declared write scope.".to_string(),
            "Changed files contain no blocked stub markers.".to_string(),
        ],
        CoordinationTaskKind::PerfEvaluate => vec![
            "The workspace was either kept (if improved) or reverted via git checkout.".to_string(),
            "The decision JSON was written to /tmp/perf_evaluate_N.json.".to_string(),
        ],
        CoordinationTaskKind::ReviewArtifacts => vec![
            "The review inspected current workspace state, not just prior claims.".to_string(),
            "Any remaining blockers were turned into concrete fix tasks.".to_string(),
        ],
        _ if !write_scope.is_empty() => vec![
            "No files outside the declared write scope were changed.".to_string(),
            "The task reported concrete effect, not only execution.".to_string(),
        ],
        _ => Vec::new(),
    }
}

pub fn generated_task_label(kind: &CoordinationTaskKind) -> &'static str {
    match kind {
        CoordinationTaskKind::DraftDesignDoc => "design",
        CoordinationTaskKind::ConvergeDesignDoc => "synthesis",
        CoordinationTaskKind::MaterializeDesignDoc => "doc_write",
        CoordinationTaskKind::PlanCodeReview => "review_plan",
        CoordinationTaskKind::PlanFlatTasks => "planning",
        CoordinationTaskKind::RalphPlan => "ralph_plan",
        CoordinationTaskKind::GeneratedWriteTask => "write",
        CoordinationTaskKind::GeneratedReadTask => "read",
        CoordinationTaskKind::ReviewArtifacts => "review",
        CoordinationTaskKind::ApplyApprovedEdits => "apply",
        CoordinationTaskKind::RalphExecute => "ralph_execute",
        CoordinationTaskKind::RalphVerify => "ralph_verify",
        CoordinationTaskKind::SurveyWorkspace => "survey",
        CoordinationTaskKind::PerfBaseline => "perf_baseline",
        CoordinationTaskKind::PerfParse => "perf_parse",
        CoordinationTaskKind::PerfApply => "perf_apply",
        CoordinationTaskKind::PerfBench => "perf_bench",
        CoordinationTaskKind::PerfEvaluate => "perf_evaluate",
    }
}

pub fn normalize_allowed_tools(
    kind: &CoordinationTaskKind,
    mut allowed_tools: Vec<String>,
) -> Vec<String> {
    if allowed_tools.is_empty() {
        allowed_tools = match kind {
            CoordinationTaskKind::GeneratedWriteTask => {
                vec!["filesystem".to_string(), "edit".to_string()]
            }
            CoordinationTaskKind::GeneratedReadTask => {
                vec!["filesystem".to_string(), "search".to_string()]
            }
            _ => Vec::new(),
        };
    }
    allowed_tools
}

pub fn normalize_read_scope(kind: &CoordinationTaskKind, read_scope: Vec<String>) -> Vec<String> {
    if read_scope.is_empty() {
        match kind {
            CoordinationTaskKind::GeneratedReadTask | CoordinationTaskKind::GeneratedWriteTask => {
                vec!["**".to_string()]
            }
            _ => Vec::new(),
        }
    } else {
        read_scope
    }
}

pub fn normalize_write_scope(kind: &CoordinationTaskKind, write_scope: Vec<String>) -> Vec<String> {
    if kind == &CoordinationTaskKind::GeneratedWriteTask {
        write_scope
            .into_iter()
            .map(|scope| normalize_scope(&scope))
            .collect()
    } else {
        Vec::new()
    }
}

pub fn normalize_acceptance_checks(
    kind: &CoordinationTaskKind,
    acceptance_checks: Vec<String>,
) -> Vec<String> {
    let checks = normalize_string_list(acceptance_checks);
    if !checks.is_empty() {
        return checks;
    }

    match kind {
        CoordinationTaskKind::GeneratedWriteTask => vec![
            "The declared capability is implemented in the owned files.".to_string(),
            "The owned files contain no blocked stub markers.".to_string(),
        ],
        _ => Vec::new(),
    }
}
