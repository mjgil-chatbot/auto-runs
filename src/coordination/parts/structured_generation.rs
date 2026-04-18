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
#[derive(schemars::JsonSchema)]
struct ContractEnvelopeSchema<P: schemars::JsonSchema> {
    outcome: CoordinationTaskOutcome,
    summary: String,
    details: String,
    changed_files: Option<Vec<String>>,
    tests_run: Vec<String>,
    warnings: Vec<String>,
    blockers: Vec<String>,
    remaining_gaps: Vec<String>,
    design_requirements_satisfied: Vec<String>,
    validation_errors: Vec<String>,
    payload_kind: TaskPayloadKind,
    contract_version: u32,
    payload: P,
}
fn enforce_no_additional_properties(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let is_object_type = obj
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s == "object")
        .unwrap_or(false);
    let has_properties = obj.contains_key("properties");
    if is_object_type || has_properties {
        obj.insert(
            "additionalProperties".to_string(),
            serde_json::Value::Bool(false),
        );
    }
    if let Some(props) = obj.get_mut("properties") {
        if let Some(props_obj) = props.as_object_mut() {
            for (_, prop_val) in props_obj.iter_mut() {
                enforce_no_additional_properties(prop_val);
            }
        }
    }
    if let Some(items) = obj.get_mut("items") {
        enforce_no_additional_properties(items);
    }
    if let Some(defs) = obj.get_mut("$defs") {
        if let Some(defs_obj) = defs.as_object_mut() {
            for (_, def_val) in defs_obj.iter_mut() {
                enforce_no_additional_properties(def_val);
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(arr) = obj.get_mut(key).and_then(|v| v.as_array_mut()) {
            for item in arr.iter_mut() {
                enforce_no_additional_properties(item);
            }
        }
    }
}
fn enforce_all_properties_required(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(props) = obj.get("properties").and_then(|v| v.as_object()) {
        let all_keys: Vec<String> = props.keys().cloned().collect();
        obj.insert(
            "required".to_string(),
            serde_json::Value::Array(
                all_keys
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(props) = obj.get_mut("properties") {
        if let Some(props_obj) = props.as_object_mut() {
            for (_, prop_val) in props_obj.iter_mut() {
                enforce_all_properties_required(prop_val);
            }
        }
    }
    if let Some(items) = obj.get_mut("items") {
        enforce_all_properties_required(items);
    }
    if let Some(defs) = obj.get_mut("$defs") {
        if let Some(defs_obj) = defs.as_object_mut() {
            for (_, def_val) in defs_obj.iter_mut() {
                enforce_all_properties_required(def_val);
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(arr) = obj.get_mut(key).and_then(|v| v.as_array_mut()) {
            for item in arr.iter_mut() {
                enforce_all_properties_required(item);
            }
        }
    }
}
