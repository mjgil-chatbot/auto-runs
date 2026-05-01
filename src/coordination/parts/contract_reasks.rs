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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReaskKind {
    Full,
    Skeleton,
    Field,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ValidationReask {
    pub kind: ReaskKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_path: Option<Vec<String>>,
}

pub fn build_reask_from_errors(contract: &TaskContract, errors: &[String]) -> ValidationReask {
    let kind = match contract.reask_strategy {
        ReaskStrategy::Full => ReaskKind::Full,
        ReaskStrategy::Skeleton => ReaskKind::Skeleton,
        ReaskStrategy::Field => ReaskKind::Field,
    };

    let message = format!(
        "Your previous response had validation errors. Please fix and resubmit:\n{}",
        errors
            .iter()
            .enumerate()
            .map(|(i, e)| format!("{}. {}", i + 1, e))
            .collect::<Vec<_>>()
            .join("\n")
    );

    ValidationReask {
        kind,
        message,
        field_path: None,
    }
}

pub fn reask_event(
    task_id: &str,
    provider: Option<&str>,
    reask: &ValidationReask,
    attempt: u32,
) -> CoordinationEvent {
    CoordinationEvent {
        at: Utc::now(),
        event_type: "task_contract_reask".to_string(),
        task_id: Some(task_id.to_string()),
        provider: provider.map(str::to_string),
        category: None,
        fallback_from: None,
        message: format!(
            "reask attempt {} ({:?}): {}",
            attempt,
            reask.kind,
            reask.message.lines().next().unwrap_or("")
        ),
    }
}

pub fn terminal_reask_failure_report(
    raw_response: &str,
    reasks_exhausted: u32,
) -> TaskExecutionReport {
    TaskExecutionReport {
        outcome: CoordinationTaskOutcome::Failed,
        summary: format!(
            "Contract validation failed after {} reask attempt(s).",
            reasks_exhausted
        ),
        details: format!(
            "Reask loop exhausted. Raw provider output preserved below.\n\n{}",
            raw_response.trim()
        ),
        warnings: vec![format!(
            "Exhausted {} validation reask(s) without valid output.",
            reasks_exhausted
        )],
        ..Default::default()
    }
}
