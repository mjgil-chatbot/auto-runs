use super::*;
#[allow(unused_imports)]
use crate::ReviewFixMode;
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptKind {
    ReviewApproval,
    ToolApproval,
    MissingInput,
    BudgetExceeded,
    CouncilUserReply,
    ManualPolicyDecision,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptStatus {
    Open,
    Resolved,
    Expired,
    Canceled,
}
impl Default for InterruptStatus {
    fn default() -> Self {
        Self::Open
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum InterruptResolution {
    Approved { note: String },
    Rejected { note: String },
    EditedPayload { value: serde_json::Value },
    Deferred { note: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoordinationInterrupt {
    pub interrupt_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council_entry_id: Option<String>,
    pub kind: InterruptKind,
    #[serde(default)]
    pub status: InterruptStatus,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<InterruptResolution>,
}
fn interrupts_dir(run_id: &str) -> PathBuf {
    coordination_root_dir().join(run_id).join("interrupts")
}
fn interrupt_path(run_id: &str, interrupt_id: &str) -> PathBuf {
    interrupts_dir(run_id).join(format!("{interrupt_id}.json"))
}
fn interrupt_subject_id(interrupt: &CoordinationInterrupt) -> Option<&str> {
    interrupt
        .task_id
        .as_deref()
        .or(interrupt.council_entry_id.as_deref())
}
fn interrupt_kind_label(kind: &InterruptKind) -> &'static str {
    match kind {
        InterruptKind::ReviewApproval => "review_approval",
        InterruptKind::ToolApproval => "tool_approval",
        InterruptKind::MissingInput => "missing_input",
        InterruptKind::BudgetExceeded => "budget_exceeded",
        InterruptKind::CouncilUserReply => "council_user_reply",
        InterruptKind::ManualPolicyDecision => "manual_policy_decision",
    }
}
fn interrupt_resolution_label(resolution: &InterruptResolution) -> &'static str {
    match resolution {
        InterruptResolution::Approved { .. } => "approved",
        InterruptResolution::Rejected { .. } => "rejected",
        InterruptResolution::EditedPayload { .. } => "edited_payload",
        InterruptResolution::Deferred { .. } => "deferred",
    }
}
fn append_interrupt_event(
    interrupt: &CoordinationInterrupt,
    event_type: &str,
    message: &str,
) -> Result<()> {
    append_event(
        &interrupt.run_id,
        event_type,
        interrupt_subject_id(interrupt),
        message,
    )
}
pub fn create_interrupt(interrupt: &CoordinationInterrupt) -> Result<()> {
    let dir = interrupts_dir(&interrupt.run_id);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create interrupts dir {}", dir.display()))?;
    write_json_atomic(
        &interrupt_path(&interrupt.run_id, &interrupt.interrupt_id),
        interrupt,
    )?;
    append_interrupt_event(
        interrupt,
        "interrupt_opened",
        &format!(
            "Opened {} interrupt: {}",
            interrupt_kind_label(&interrupt.kind),
            interrupt.title
        ),
    )?;
    Ok(())
}
pub fn load_interrupt(run_id: &str, interrupt_id: &str) -> Result<CoordinationInterrupt> {
    let path = interrupt_path(run_id, interrupt_id);
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read interrupt {}", path.display()))?;
    serde_json::from_str(&raw).context("invalid interrupt JSON")
}
pub fn load_interrupts(run_id: &str) -> Vec<CoordinationInterrupt> {
    let dir = interrupts_dir(run_id);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut interrupts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(interrupt) = serde_json::from_str::<CoordinationInterrupt>(&raw) {
                interrupts.push(interrupt);
            }
        }
    }
    interrupts.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    interrupts
}
pub fn has_open_interrupts(run_id: &str) -> bool {
    load_interrupts(run_id)
        .iter()
        .any(|i| i.status == InterruptStatus::Open)
}
pub fn create_review_approval_interrupt(
    run_id: &str,
    task_id: &str,
    title: &str,
    prompt: &str,
    payload: serde_json::Value,
) -> Result<CoordinationInterrupt> {
    let interrupt = CoordinationInterrupt {
        interrupt_id: Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        task_id: Some(task_id.to_string()),
        council_entry_id: None,
        kind: InterruptKind::ReviewApproval,
        status: InterruptStatus::Open,
        title: title.to_string(),
        prompt: prompt.to_string(),
        payload,
        created_at: Utc::now(),
        resolved_at: None,
        resolution: None,
    };
    create_interrupt(&interrupt)?;
    Ok(interrupt)
}
pub fn maybe_create_review_approval_interrupt(
    run_id: &str,
    task: &CoordinationTask,
    artifact: &TaskArtifact,
) -> Result<Option<CoordinationInterrupt>> {
    if task.kind != CoordinationTaskKind::ReviewArtifacts {
        return Ok(None);
    }
    let run = load_run(run_id)?;
    if uses_lightweight_runtime_mode(&run.workflow_mode) {
        return Ok(None);
    }
    if !run.require_review_before_apply {
        return Ok(None);
    }
    if matches!(run.review_fix_mode, Some(ReviewFixMode::ReportOnly)) {
        return Ok(None);
    }
    if artifact.report.fix_tasks.is_empty() {
        return Ok(None);
    }
    let title = format!("Review approval needed for {}", task.title);
    let prompt = format!(
        "Review task {} produced {} fix task(s). Approve before any apply task is allowed to run.\n\nReview summary: {}",
        task.task_id,
        artifact.report.fix_tasks.len(),
        artifact.summary
    );
    let payload = serde_json::json!({
        "summary": artifact.summary,
        "fix_tasks": artifact.report.fix_tasks,
    });
    let interrupt =
        create_review_approval_interrupt(run_id, &task.task_id, &title, &prompt, payload)?;
    Ok(Some(interrupt))
}
pub fn create_council_user_reply_interrupt(
    run_id: &str,
    council_entry_id: &str,
    prompt: &str,
) -> Result<CoordinationInterrupt> {
    let interrupt = CoordinationInterrupt {
        interrupt_id: Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        task_id: None,
        council_entry_id: Some(council_entry_id.to_string()),
        kind: InterruptKind::CouncilUserReply,
        status: InterruptStatus::Open,
        title: "Council requests user input".to_string(),
        prompt: prompt.to_string(),
        payload: serde_json::json!({ "prompt": prompt }),
        created_at: Utc::now(),
        resolved_at: None,
        resolution: None,
    };
    create_interrupt(&interrupt)?;
    Ok(interrupt)
}
fn apply_review_approval_resolution(
    run_id: &str,
    review_task_id: Option<&str>,
    resolution: &InterruptResolution,
) -> Result<()> {
    match resolution {
        InterruptResolution::Approved { .. } | InterruptResolution::EditedPayload { .. } => Ok(()),
        InterruptResolution::Rejected { note } => fail_apply_tasks_for_review_resolution(
            run_id,
            review_task_id,
            "Review approval rejected",
            note,
        ),
        InterruptResolution::Deferred { note } => fail_apply_tasks_for_review_resolution(
            run_id,
            review_task_id,
            "Review approval deferred",
            note,
        ),
    }
}
fn fail_apply_tasks_for_review_resolution(
    run_id: &str,
    review_task_id: Option<&str>,
    reason_prefix: &str,
    note: &str,
) -> Result<()> {
    let mut changed_any = false;
    let now = Utc::now();
    let reason = if note.trim().is_empty() {
        reason_prefix.to_string()
    } else {
        format!("{reason_prefix}: {}", note.trim())
    };
    let mut tasks = load_tasks(run_id)?;
    for task in tasks.iter_mut() {
        if task.kind != CoordinationTaskKind::ApplyApprovedEdits {
            continue;
        }
        if matches!(
            task.status,
            CoordinationTaskStatus::Completed
                | CoordinationTaskStatus::Failed
                | CoordinationTaskStatus::Canceled
        ) {
            continue;
        }
        if let Some(review_task_id) = review_task_id {
            if !task
                .dependencies
                .iter()
                .any(|dependency| dependency == review_task_id)
            {
                continue;
            }
        }
        task.status = CoordinationTaskStatus::Failed;
        task.claimed_by = None;
        task.artifact_summary = Some(reason.clone());
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        let _ = fs::remove_file(claim_path(run_id, &task.task_id));
        append_event(
            run_id,
            "task_failed",
            Some(&task.task_id),
            &format!(
                "Marked {} failed after policy resolution. {}",
                task.title, reason
            ),
        )?;
        changed_any = true;
    }
    if changed_any {
        let mut run = load_run(run_id)?;
        run.updated_at = now;
        write_json_atomic(&run_path(run_id), &run)?;
        index_run(&run);
    }
    Ok(())
}
fn council_user_reply_text(resolution: &InterruptResolution) -> String {
    match resolution {
        InterruptResolution::Approved { note } => {
            if note.trim().is_empty() {
                "User approved the requested council action.".to_string()
            } else {
                note.trim().to_string()
            }
        }
        InterruptResolution::Rejected { note } => {
            if note.trim().is_empty() {
                "User rejected the requested council action.".to_string()
            } else {
                format!("User rejected the request: {}", note.trim())
            }
        }
        InterruptResolution::Deferred { note } => {
            if note.trim().is_empty() {
                "User deferred the requested council action.".to_string()
            } else {
                format!("User deferred the request: {}", note.trim())
            }
        }
        InterruptResolution::EditedPayload { value } => interrupt_resolution_payload_text(value),
    }
}
fn interrupt_resolution_payload_text(value: &serde_json::Value) -> String {
    if let Some(text) = value.as_str() {
        return text.trim().to_string();
    }
    if let Some(text) = value.get("message").and_then(|inner| inner.as_str()) {
        return text.trim().to_string();
    }
    if let Some(text) = value.get("content").and_then(|inner| inner.as_str()) {
        return text.trim().to_string();
    }
    if let Some(text) = value.get("reply").and_then(|inner| inner.as_str()) {
        return text.trim().to_string();
    }
    serde_json::to_string(value).unwrap_or_default()
}
