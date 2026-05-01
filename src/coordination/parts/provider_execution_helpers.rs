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

pub(crate) fn resolve_autonomous_provider(provider_name: &str) -> Result<ModelProvider> {
    match normalize_provider_name(Some(provider_name)).as_deref() {
        Some("claude") => Ok(ModelProvider::Claude),
        Some("minimax") => Ok(ModelProvider::MiniMax),
        Some("qwen") => Ok(ModelProvider::Qwen),
        Some("codex") => Ok(ModelProvider::Codex),
        Some("nemotron") => Ok(ModelProvider::Nvidia),
        Some("gemini") => Ok(ModelProvider::Gemini),
        _ => anyhow::bail!("unsupported provider {}", provider_name.trim()),
    }
}

pub const QWEN_PROVIDER_TASK_TIMEOUT: Duration = Duration::from_secs(20 * 60);

fn provider_timeout_cap(provider_name: &str) -> Option<Duration> {
    match provider_name {
        "qwen" => Some(QWEN_PROVIDER_TASK_TIMEOUT),
        _ => None,
    }
}

fn estimated_task_timeout(task: &CoordinationTask) -> Duration {
    Duration::from_secs(
        (normalize_task_estimated_minutes(task.estimated_minutes, &task.kind) as u64) * 60,
    )
}

pub fn provider_task_timeout(provider_name: &str, task: &CoordinationTask) -> Option<Duration> {
    let estimate_timeout = estimated_task_timeout(task);
    match provider_timeout_cap(provider_name) {
        Some(cap) => Some(estimate_timeout.min(cap)),
        None => Some(estimate_timeout),
    }
}

pub fn provider_timeout_failure(provider_name: &str, timeout: Duration) -> String {
    format!(
        "{} task timed out after {}.",
        provider_display_name(provider_name),
        format_duration(timeout)
    )
}

pub fn provider_timeout_prefix(provider_name: &str) -> String {
    format!(
        "{} task timed out after",
        provider_display_name(provider_name)
    )
}

fn provider_display_name(provider_name: &str) -> &str {
    match provider_name {
        "claude" => "Claude",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "glm" => "GLM",
        "minimax" => "MiniMax",
        "nemotron" => "Nemotron",
        "nvidia" => "Nvidia",
        "qwen" => "Qwen",
        other => other,
    }
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs % 60 == 0 {
        let minutes = secs / 60;
        format!("{} minute{}", minutes, if minutes == 1 { "" } else { "s" })
    } else {
        format!("{} second{}", secs, if secs == 1 { "" } else { "s" })
    }
}

pub fn collect_stream_response(rx: mpsc::Receiver<crate::stream::StreamMessage>) -> String {
    let mut last_response = String::new();
    let mut last_non_empty_response = String::new();
    let mut last_final_non_empty_response = String::new();
    for msg in rx.iter() {
        if !msg.response.trim().is_empty() {
            last_non_empty_response = msg.response.clone();
            if msg.is_final {
                last_final_non_empty_response = msg.response.clone();
            }
        }
        last_response = msg.response;
    }
    if !last_final_non_empty_response.is_empty() {
        last_final_non_empty_response
    } else if !last_non_empty_response.is_empty() {
        last_non_empty_response
    } else {
        last_response
    }
}

pub fn extract_provider_command_failure(response: &str) -> Option<&str> {
    let trimmed = response.trim();
    [
        "Claude command failed",
        "Codex command failed",
        "Gemini command failed",
        "Nemotron command failed",
        "Nvidia command failed",
        "qwen-code command failed",
        "Failed to spawn ",
    ]
    .iter()
    .find_map(|prefix| trimmed.starts_with(prefix).then_some(trimmed))
}

pub fn extract_provider_cooldown(
    provider_name: &str,
    response: &str,
    now: DateTime<Utc>,
) -> Option<ProviderCooldown> {
    let provider = normalize_provider_name(Some(provider_name))?;
    match provider.as_str() {
        "gemini" => extract_gemini_cooldown(response, now),
        "glm" => extract_glm_cooldown(response, now),
        _ => None,
    }
    .or_else(|| extract_provider_auth_failure(&provider, response, now))
}

pub fn extract_gemini_cooldown(response: &str, now: DateTime<Utc>) -> Option<ProviderCooldown> {
    let lower = response.to_ascii_lowercase();
    let has_quota_signal = response.contains("TerminalQuotaError")
        || response.contains("QUOTA_EXHAUSTED")
        || lower.contains("quota will reset after")
        || lower.contains("resource_exhausted")
        || lower.contains("model_capacity_exhausted")
        || lower.contains("ratelimitexceeded")
        || lower.contains("too many requests")
        || lower.contains("\"code\": 429")
        || lower.contains("\"code\":429")
        || lower.contains("status 429");
    if !has_quota_signal {
        return None;
    }

    let retry_after = extract_retry_delay_ms(response)
        .map(chrono::Duration::milliseconds)
        .or_else(|| extract_retry_delay_duration(response))
        .or_else(|| extract_quota_reset_delay(response))
        .unwrap_or_else(|| chrono::Duration::minutes(5));
    let unavailable_until = now + retry_after;
    Some(ProviderCooldown {
        unavailable_until,
        reason: "Gemini API quota exhausted.".to_string(),
    })
}

pub fn extract_glm_cooldown(response: &str, now: DateTime<Utc>) -> Option<ProviderCooldown> {
    let lower = response.to_ascii_lowercase();
    let has_quota_signal = lower.contains("429")
        || lower.contains("\"code\":\"1302\"")
        || lower.contains("\"code\": \"1302\"")
        || lower.contains("\"code\":\"1308\"")
        || lower.contains("\"code\": \"1308\"")
        || response.contains("已达到")
        || response.contains("速率限制")
        || response.contains("使用上限")
        || response.contains("限额将在");
    if !has_quota_signal {
        return None;
    }

    let unavailable_until = match extract_glm_reset_timestamp(response) {
        Some(ts) if ts > now => ts,
        _ => now + chrono::Duration::minutes(5),
    };

    Some(ProviderCooldown {
        unavailable_until,
        reason: "GLM API quota exhausted.".to_string(),
    })
}

pub fn extract_provider_auth_failure(
    provider_name: &str,
    response: &str,
    now: DateTime<Utc>,
) -> Option<ProviderCooldown> {
    let lower = response.to_ascii_lowercase();
    let has_auth_signal = lower.contains("failed to authenticate")
        || lower.contains("authentication_error")
        || lower.contains("invalid authentication credentials")
        || lower.contains("401 unauthorized")
        || lower.contains("missing bearer or basic authentication")
        || lower.contains("missing api key")
        || lower.contains("please set an auth method");
    if !has_auth_signal {
        return None;
    }

    let reason = match provider_name {
        "claude" => "Claude authentication failed.".to_string(),
        "codex" => "Codex authentication failed.".to_string(),
        "gemini" => "Gemini authentication failed.".to_string(),
        "glm" => "GLM authentication failed.".to_string(),
        "minimax" => "MiniMax authentication failed.".to_string(),
        "nemotron" => "Nemotron authentication failed.".to_string(),
        "nvidia" => "Nvidia authentication failed.".to_string(),
        "qwen" => "Qwen authentication failed.".to_string(),
        other => format!("{other} authentication failed."),
    };
    Some(ProviderCooldown {
        unavailable_until: now + chrono::Duration::minutes(15),
        reason,
    })
}

pub fn extract_retry_delay_ms(response: &str) -> Option<i64> {
    let marker = "retryDelayMs:";
    let start = response.find(marker)? + marker.len();
    let rest = response.get(start..)?.trim_start();
    let numeric: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    if numeric.is_empty() {
        return None;
    }
    let millis = numeric.parse::<f64>().ok()?.ceil();
    if !millis.is_finite() || millis < 1.0 {
        return None;
    }
    Some(millis as i64)
}

pub fn extract_quota_reset_delay(response: &str) -> Option<chrono::Duration> {
    let lower = response.to_ascii_lowercase();
    let marker = "reset after ";
    let start = lower.find(marker)? + marker.len();
    let tail = lower.get(start..)?;
    let raw_duration: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, 'h' | 'm' | 's'))
        .collect();
    parse_compact_duration(&raw_duration)
}

pub fn extract_glm_reset_timestamp(response: &str) -> Option<DateTime<Utc>> {
    for marker in ["限额将在 ", "限额将在", "will reset at ", "reset at "] {
        let Some(pos) = response.find(marker) else {
            continue;
        };
        let start = pos + marker.len();
        let Some(tail) = response.get(start..) else {
            continue;
        };
        let tail = tail.trim_start();
        let raw_timestamp: String = tail
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '-' | ':' | ' '))
            .collect();
        let raw_timestamp = raw_timestamp.trim();
        if raw_timestamp.is_empty() {
            continue;
        }

        let naive =
            chrono::NaiveDateTime::parse_from_str(raw_timestamp, "%Y-%m-%d %H:%M:%S").ok()?;
        let offset = chrono::FixedOffset::east_opt(8 * 3600)?;
        let reset_at = naive.and_local_timezone(offset).single()?;
        return Some(reset_at.with_timezone(&Utc));
    }
    None
}

pub fn parse_compact_duration(raw: &str) -> Option<chrono::Duration> {
    if raw.is_empty() {
        return None;
    }

    let mut total_seconds = 0i64;
    let mut digits = String::new();
    let mut saw_unit = false;
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let value = digits.parse::<i64>().ok()?;
        digits.clear();
        match ch {
            'h' => total_seconds = total_seconds.saturating_add(value.saturating_mul(3600)),
            'm' => total_seconds = total_seconds.saturating_add(value.saturating_mul(60)),
            's' => total_seconds = total_seconds.saturating_add(value),
            _ => return None,
        }
        saw_unit = true;
    }

    if !digits.is_empty() || !saw_unit || total_seconds <= 0 {
        return None;
    }
    Some(chrono::Duration::seconds(total_seconds))
}

pub fn extract_retry_delay_duration(response: &str) -> Option<chrono::Duration> {
    let lower = response.to_ascii_lowercase();
    for marker in [
        "\"retrydelay\":\"",
        "\"retrydelay\": \"",
        "retrydelay: \"",
        "retry in ",
    ] {
        let Some(start) = lower.find(marker).map(|idx| idx + marker.len()) else {
            continue;
        };
        let Some(tail) = lower.get(start..) else {
            continue;
        };
        let raw_duration: String = tail
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | 'h' | 'm' | 's'))
            .collect();
        let raw_duration = raw_duration.trim_end_matches('.');
        if raw_duration.is_empty() {
            continue;
        }
        if let Some(duration) = parse_retry_delay_duration(raw_duration) {
            return Some(duration);
        }
    }
    None
}

pub fn parse_retry_delay_duration(raw: &str) -> Option<chrono::Duration> {
    if raw.is_empty() {
        return None;
    }
    if let Some(seconds) = raw.strip_suffix('s') {
        if seconds.contains('.') {
            let seconds = seconds.parse::<f64>().ok()?.ceil();
            if !seconds.is_finite() || seconds <= 0.0 {
                return None;
            }
            return Some(chrono::Duration::seconds(seconds as i64));
        }
    }
    parse_compact_duration(raw)
}
