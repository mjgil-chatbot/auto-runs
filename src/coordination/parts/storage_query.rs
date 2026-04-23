pub fn fallback_readable_index() -> Option<StorageIndex> {
    if !crate::storage::index_db_path().exists() {
        return None;
    }
    crate::storage::get_index()
}

pub fn run_matches_query(run: &CoordinationRun, query: &RunQuery) -> bool {
    if let Some(run_id) = query.run_id.as_deref() {
        if run.run_id != run_id {
            return false;
        }
    }
    if let Some(provider) = query.provider.as_deref() {
        if run.provider != provider {
            return false;
        }
    }
    if let Some(status) = query.status.as_deref() {
        if run.status.as_str() != status {
            return false;
        }
    }
    if let Some(workflow_mode) = query.workflow_mode.as_deref() {
        if run.workflow_mode.as_str() != workflow_mode {
            return false;
        }
    }
    if let Some(created_after) = query.created_after {
        if run.created_at < created_after {
            return false;
        }
    }
    if let Some(created_before) = query.created_before {
        if run.created_at > created_before {
            return false;
        }
    }
    true
}

pub fn query_runs_from_authoritative_files(
    query: &RunQuery,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<RunSummaryRow>> {
    if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
        return Ok(Vec::new());
    }

    let mut runs = Vec::new();
    for run_id in authoritative_run_ids(stop_signal.clone())? {
        if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
            break;
        }
        let run = match load_run(&run_id) {
            Ok(run) => run,
            Err(err) => {
                debug!(
                    run_id=%run_id,
                    error=%err,
                    "query_runs: skipping unreadable authoritative run during fallback"
                );
                continue;
            }
        };
        if run_matches_query(&run, query) {
            runs.push(RunSummaryRow::from(&run));
        }
    }

    runs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.run_id.cmp(&left.run_id))
    });
    if let Some(limit) = query.limit {
        runs.truncate(limit);
    }
    Ok(runs)
}

pub fn query_runs_with_fallback(
    query: &RunQuery,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<RunSummaryRow>> {
    if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
        return Ok(Vec::new());
    }
    if let Some(idx) = fallback_readable_index() {
        match list_runs_from_index(&idx, query) {
            Ok(records) => {
                return Ok(records
                    .into_iter()
                    .map(|record: RunRecord| RunSummaryRow::from(record))
                    .collect());
            }
            Err(err) => {
                debug!(
                    error=%err,
                    "query_runs: SQLite query failed, falling back to authoritative files"
                );
            }
        }
    }

    query_runs_from_authoritative_files(query, stop_signal)
}

pub fn event_matches_query(
    event: &CoordinationEvent,
    resolved_provider: Option<&str>,
    query: &CoordinationEventQuery,
) -> bool {
    if let Some(event_type) = query.event_type.as_deref() {
        if event.event_type != event_type {
            return false;
        }
    }
    if let Some(provider) = query.provider.as_deref() {
        if resolved_provider != Some(provider) {
            return false;
        }
    }
    if let Some(at_after) = query.at_after {
        if event.at < at_after {
            return false;
        }
    }
    if let Some(at_before) = query.at_before {
        if event.at > at_before {
            return false;
        }
    }
    true
}

pub fn resolve_event_provider_fallback(
    run: &CoordinationRun,
    event: &CoordinationEvent,
) -> Option<String> {
    event.provider.clone().or_else(|| {
        event.task_id.as_deref().and_then(|task_id| {
            load_task(&run.run_id, task_id)
                .ok()
                .and_then(|task| task.assigned_provider)
                .or_else(|| Some(run.provider.clone()))
        })
    })
}

pub fn query_events_from_authoritative_files(
    query: &CoordinationEventQuery,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<EventRow>> {
    if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
        return Ok(Vec::new());
    }

    let run_ids = if let Some(run_id) = query.run_id.as_deref() {
        vec![run_id.to_string()]
    } else {
        authoritative_run_ids(stop_signal.clone())?
    };

    let mut events = Vec::new();
    let mut next_id = -1_i64;
    for run_id in run_ids {
        if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
            break;
        }
        let run = match load_run(&run_id) {
            Ok(run) => run,
            Err(err) => {
                debug!(
                    run_id=%run_id,
                    error=%err,
                    "query_events: skipping unreadable authoritative run during fallback"
                );
                continue;
            }
        };
        let run_events = match load_events(&run_id, None, stop_signal.clone()) {
            Ok(events) => events,
            Err(err) => {
                debug!(
                    run_id=%run_id,
                    error=%err,
                    "query_events: skipping unreadable authoritative event log during fallback"
                );
                continue;
            }
        };

        for event in run_events {
            let resolved_provider = resolve_event_provider_fallback(&run, &event);
            if !event_matches_query(&event, resolved_provider.as_deref(), query) {
                continue;
            }
            let category_str = event
                .category
                .as_ref()
                .and_then(|c| serde_json::to_value(c).ok())
                .and_then(|v| v.as_str().map(String::from));
            events.push(EventRow {
                id: next_id,
                run_id: run_id.clone(),
                event_type: event.event_type,
                task_id: event.task_id,
                provider: resolved_provider,
                category: category_str,
                fallback_from: event.fallback_from,
                message: Some(event.message),
                at: event.at.to_rfc3339(),
            });
            next_id -= 1;
        }
    }

    events.sort_by(|left, right| right.at.cmp(&left.at).then_with(|| right.id.cmp(&left.id)));
    if let Some(limit) = query.limit {
        events.truncate(limit);
    }
    Ok(events)
}

fn authoritative_run_ids(stop_signal: Option<Arc<AtomicBool>>) -> Result<Vec<String>> {
    let root = coordination_root_dir();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read coordination root {}", root.display()));
        }
    };

    let mut run_ids = Vec::new();
    for entry in entries.flatten() {
        if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
            break;
        }
        if !entry.file_type().map(|file_type| file_type.is_dir()).unwrap_or(false) {
            continue;
        }
        if !entry.path().join("run.json").is_file() {
            continue;
        }
        run_ids.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(run_ids)
}

pub fn query_events_with_fallback(
    query: &CoordinationEventQuery,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<EventRow>> {
    if stop_signal.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
        return Ok(Vec::new());
    }
    if let Some(idx) = fallback_readable_index() {
        match list_events_from_index(&idx, query) {
            Ok(records) => {
                return Ok(records
                    .into_iter()
                    .map(|record: CoordinationEventRecord| EventRow::from(record))
                    .collect());
            }
            Err(err) => {
                debug!(
                    error=%err,
                    "query_events: SQLite query failed, falling back to authoritative files"
                );
            }
        }
    }

    query_events_from_authoritative_files(query, stop_signal)
}

/// List coordination runs, optionally filtered by status and/or provider.
/// Falls back to authoritative run.json files when the SQLite index is
/// missing, disabled, or unreadable.
pub fn query_runs(status: Option<&str>, provider: Option<&str>) -> Result<Vec<RunSummaryRow>> {
    query_runs_filtered(status, provider, None, None, None)
}

pub fn query_runs_filtered(
    status: Option<&str>,
    provider: Option<&str>,
    workflow_mode: Option<&str>,
    limit: Option<usize>,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<RunSummaryRow>> {
    let query = RunQuery {
        status: status.map(str::to_string),
        provider: provider.map(str::to_string),
        workflow_mode: workflow_mode.map(str::to_string),
        limit,
        ..Default::default()
    };
    query_runs_with_fallback(&query, stop_signal)
}

/// Query coordination events, optionally filtered by run_id, event_type,
/// provider, and date range.
/// Falls back to authoritative events.jsonl files when the SQLite index is
/// missing, disabled, or unreadable.
pub fn query_events(
    run_id: Option<&str>,
    event_type: Option<&str>,
    provider: Option<&str>,
    after: Option<&str>,
    before: Option<&str>,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<Vec<EventRow>> {
    let query = CoordinationEventQuery {
        run_id: run_id.map(str::to_string),
        event_type: event_type.map(str::to_string),
        provider: provider.map(str::to_string),
        at_after: after.and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        at_before: before.and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        ..Default::default()
    };
    query_events_with_fallback(&query, stop_signal)
}

/// Rebuild the coordination portion of the SQLite index from authoritative
/// spool files on disk.  Delegates to the shared StorageIndex rebuild which
/// scans all session files and coordination directories.
pub fn rebuild_coordination_index() -> Result<()> {
    let idx = get_storage_index().context("coordination index unavailable")?;
    let _report = idx.rebuild_all()?;
    Ok(())
}

/// Delete coordination run directories whose `updated_at` is older than
/// `age_days`.  Returns the number of runs deleted.
pub fn prune_old_coordination_runs(age_days: u32) -> Result<u32> {
    let cutoff = Utc::now() - chrono::Duration::days(i64::from(age_days));
    let root = coordination_root_dir();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read coordination root {}", root.display()));
        }
    };

    let mut deleted = 0u32;
    for entry in entries.flatten() {
        if !entry
            .file_type()
            .map(|ft| ft.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let run_id = entry.file_name().to_string_lossy().into_owned();
        let run = match load_run(&run_id) {
            Ok(run) => run,
            Err(_) => continue,
        };
        if run.updated_at < cutoff {
            let run_path = run_dir(&run_id);
            if let Err(err) = fs::remove_dir_all(&run_path) {
                warn!(
                    run_id = %run_id,
                    path = %run_path.display(),
                    error = %err,
                    "prune_old_coordination_runs: failed to delete run directory"
                );
            } else {
                deleted += 1;
            }
        }
    }
    Ok(deleted)
}
