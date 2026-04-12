use fs2::FileExt;
pub fn coordination_root_dir() -> PathBuf {
    config::shared_infra_dir().join("coordination")
}
pub fn run_dir(run_id: &str) -> PathBuf {
    coordination_root_dir().join(run_id)
}
pub fn run_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("run.json")
}
pub fn events_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("events.jsonl")
}
/// Path to the council state file for a run.
pub(crate) fn council_json_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("council.json")
}
/// Path to the append-only council ledger for a run.
pub(crate) fn council_ledger_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("ledger.jsonl")
}
/// Path to the council mutation lock file for a run.
pub(crate) fn council_lock_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("council.lock")
}
/// Alias used by the council submodule.
pub(crate) fn ledger_path(run_id: &str) -> PathBuf {
    council_ledger_path(run_id)
}
pub fn load_events(
    run_id: &str,
    limit: Option<usize>,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<CoordinationEvent>> {
    if limit == Some(0) {
        return Ok(Vec::new());
    }
    let path = events_path(run_id);
    let file =
        fs::File::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut tail = limit.map(|_| std::collections::VecDeque::new());

    for line in reader.lines() {
        if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
            anyhow::bail!("load_events cancelled for run_id={}", run_id);
        }
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: CoordinationEvent = serde_json::from_str(&line)
            .with_context(|| format!("failed to parse event line in {}", path.display()))?;
        if let Some(ref mut tail) = tail {
            tail.push_back(event);
            if tail.len() > limit.unwrap_or_default() {
                tail.pop_front();
            }
        } else {
            events.push(event);
        }
    }

    if let Some(tail) = tail {
        events = tail.into_iter().collect();
    }

    Ok(events)
}
pub fn append_event(
    run_id: &str,
    event_type: &str,
    task_id: Option<&str>,
    message: &str,
) -> Result<()> {
    append_event_with_provider(run_id, event_type, task_id, None, None, None, message)
}
pub fn append_event_with_provider(
    run_id: &str,
    event_type: &str,
    task_id: Option<&str>,
    provider: Option<&str>,
    category: Option<TaskCategory>,
    fallback_from: Option<&str>,
    message: &str,
) -> Result<()> {
    let event = CoordinationEvent {
        at: Utc::now(),
        event_type: event_type.to_string(),
        task_id: task_id.map(str::to_string),
        provider: provider.map(str::to_string),
        category,
        fallback_from: fallback_from.map(str::to_string),
        message: message.to_string(),
    };
    let path = events_path(run_id);
    let parent = path.parent().context("event path missing parent")?;
    fs::create_dir_all(parent)?;

    // Serialize the entire event to a string first, then write as one unit
    // This prevents interleaving if multiple processes write simultaneously
    let mut json = serde_json::to_string(&event)?;
    json.push('\n');

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    file.write_all(json.as_bytes())?;
    file.unlock()
        .with_context(|| format!("failed to unlock {}", path.display()))?;

    // Dual-write: opportunistically index the event in SQLite (non-fatal)
    index_coordination_event(run_id, &event);
    Ok(())
}
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("path missing parent")?;
    fs::create_dir_all(parent)?;
    let json = serde_json::to_string_pretty(value)?;
    crate::storage::atomic::atomic_write(path, json.as_bytes())?;
    Ok(())
}
pub fn collect_workspace_entries(
    root: &Path,
    current: &Path,
    dirs: &mut Vec<String>,
    files: &mut Vec<String>,
    limit: usize,
) -> Result<()> {
    if dirs.len() + files.len() >= limit {
        return Ok(());
    }

    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        if dirs.len() + files.len() >= limit {
            break;
        }
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_dir(relative) {
                continue;
            }
            dirs.push(relative.to_string_lossy().into_owned());
            collect_workspace_entries(root, &path, dirs, files, limit)?;
        } else if file_type.is_file() {
            files.push(relative.to_string_lossy().into_owned());
        }
    }

    Ok(())
}
pub fn should_skip_dir(relative: &Path) -> bool {
    matches!(
        relative.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "node_modules" | "dist")
    )
}
pub fn format_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        "- <none>".to_string()
    } else {
        lines
            .iter()
            .map(|line| format!("- {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
use crate::storage::index::StorageIndex;
/// Obtain a shared StorageIndex, returning None when the database is
/// unavailable (e.g. first launch before any directory exists).  Because
/// `StorageIndex::open()` calls `open_index()` → `ensure_schema()`, the
/// schema is guaranteed to exist before any write or query.
pub fn get_storage_index() -> Option<StorageIndex> {
    StorageIndex::open().ok()
}
/// Index a coordination event into SQLite.
pub fn index_coordination_event(run_id: &str, event: &CoordinationEvent) {
    let Some(idx) = get_storage_index() else {
        return;
    };
    let conn = idx.connection();
    let category_str = event
        .category
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(String::from));
    if let Err(err) = conn.execute(
        r#"
        INSERT INTO coordination_event (run_id, event_type, task_id, provider, category, fallback_from, message, at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        rusqlite::params![
            run_id,
            event.event_type,
            event.task_id,
            event.provider,
            category_str,
            event.fallback_from,
            event.message,
            event.at.to_rfc3339(),
        ],
    ) {
        debug!("index_coordination_event failed for {}: {}", run_id, err);
    }
}
