use super::*;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use serde_json::Value;
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
pub fn extract_task_execution_report_json(body: &str) -> Option<&str> {
    let mut offset = 0usize;

    while offset < body.len() {
        let next_start = body[offset..].find('{')?;
        let start = offset + next_start;
        let json = extract_balanced_json(&body[start..], '{', '}')?;
        if json_has_required_report_fields(json) {
            return Some(json);
        }
        offset = start + json.len();
    }

    None
}
pub fn json_has_required_report_fields(json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    ["outcome", "summary", "details"]
        .iter()
        .all(|field| object.contains_key(*field))
}
