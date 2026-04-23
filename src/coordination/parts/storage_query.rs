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
