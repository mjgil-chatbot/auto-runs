use fs2::FileExt;

pub fn coordination_root_dir() -> PathBuf {
    config::shared_infra_dir().join("coordination")
}

pub fn provider_cooldowns_path() -> PathBuf {
    coordination_root_dir().join("provider_cooldowns.json")
}

pub fn run_dir(run_id: &str) -> PathBuf {
    coordination_root_dir().join(run_id)
}

pub fn run_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("run.json")
}

pub fn tasks_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("tasks")
}

pub fn task_path(run_id: &str, task_id: &str) -> PathBuf {
    tasks_dir(run_id).join(format!("{task_id}.json"))
}

pub fn claims_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("claims")
}

pub fn claim_path(run_id: &str, task_id: &str) -> PathBuf {
    claims_dir(run_id).join(format!("{task_id}.json"))
}

pub fn claim_task_doc_snapshot_path(run_id: &str, task_id: &str) -> PathBuf {
    claims_dir(run_id).join(format!("{task_id}.task-doc-snapshot.md"))
}

pub fn mutation_batches_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("mutations")
}

pub fn mutation_batch_path(run_id: &str, batch_id: &str) -> PathBuf {
    mutation_batches_dir(run_id).join(format!("{batch_id}.json"))
}

pub fn lightweight_broker_snapshot_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("lightweight_broker.json")
}

pub fn lightweight_source_tasks_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("lightweight_source_tasks.json")
}

pub fn lightweight_execution_tasks_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("lightweight_execution_tasks.json")
}

pub fn lightweight_passes_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("lightweight_passes.json")
}

pub fn events_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("events.jsonl")
}

// --- Council-specific paths ---

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

/// Create the council-specific on-disk layout and write the initial CouncilRun state.
pub fn seed_council_run(run: &CoordinationRun) -> Result<()> {
    let max_turns = run.council_max_turns.unwrap_or(40);
    let council_state = council::CouncilRun {
        run_id: run.run_id.clone(),
        root_chat_id: run.root_chat_id.clone(),
        workspace_path: run.workspace_path.clone(),
        goal: run.goal.clone(),
        completion_criteria: run.completion_criteria.clone(),
        status: council::CouncilRunStatus::Active,
        turn_policy: run.council_turn_policy.clone().unwrap_or_default(),
        max_turns,
        current_turn: 0,
        active_claims: Vec::new(),
        revoked_claims: Vec::new(),
        votes_complete: Vec::new(),
        turns_since_progress: 0,
        total_work_reports: 0,
        created_at: run.created_at,
        updated_at: run.updated_at,
        allow_write_tasks: run.allow_write_tasks,
        notes: run.notes.clone(),
        termination_policy: council::CouncilTerminationPolicy::for_max_turns(max_turns),
    };

    write_json_atomic(&council_json_path(&run.run_id), &council_state)?;

    // Create empty ledger file
    let ledger_path = council_ledger_path(&run.run_id);
    if !ledger_path.exists() {
        fs::File::create(&ledger_path)
            .with_context(|| format!("failed to create {}", ledger_path.display()))?;
    }

    // Ensure claims and artifacts dirs exist (shared with task-based runs)
    fs::create_dir_all(claims_dir(&run.run_id))?;
    fs::create_dir_all(artifacts_dir(&run.run_id))?;

    append_event(
        &run.run_id,
        "council_seeded",
        None,
        &format!(
            "Seeded Freeform Council: goal={}, turn_policy={}, max_turns={}",
            run.goal,
            run.council_turn_policy.as_ref().map(|p| p.as_str()).unwrap_or("default"),
            run.council_max_turns.unwrap_or(40)
        ),
    )?;

    Ok(())
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

pub fn presence_dir() -> PathBuf {
    coordination_root_dir().join("presence")
}

pub fn presence_path(instance_pid: u32) -> PathBuf {
    presence_dir().join(format!("{instance_pid}.json"))
}

pub fn create_claim_file(claim: &TaskClaim) -> Result<bool> {
    fs::create_dir_all(claims_dir(&claim.run_id))?;
    let path = claim_path(&claim.run_id, &claim.task_id);
    let json = serde_json::to_string_pretty(claim)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(json.as_bytes())?;
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            // Emit a contention event for observability — does not fail the claim attempt.
            let msg = format!(
                "claim contention on task {} by instance {} for provider {} — another instance won",
                claim.task_id, claim.instance_pid, claim.provider
            );
            let _ = append_event(&claim.run_id, "claim_contention", Some(&claim.task_id), &msg);
            Ok(false)
        }
        Err(err) => Err(err.into()),
    }
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

pub fn begin_mutation_batch(
    run_id: &str,
    task_id: Option<&str>,
    kind: CoordinationMutationKind,
    notes: impl Into<String>,
) -> Result<CoordinationMutationBatch> {
    let batch = CoordinationMutationBatch {
        batch_id: Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        task_id: task_id.map(str::to_string),
        kind,
        created_at: Utc::now(),
        completed_at: None,
        notes: notes.into(),
    };
    write_json_atomic(&mutation_batch_path(run_id, &batch.batch_id), &batch)?;
    Ok(batch)
}

pub fn complete_mutation_batch(batch: &mut CoordinationMutationBatch) -> Result<()> {
    batch.completed_at = Some(Utc::now());
    write_json_atomic(&mutation_batch_path(&batch.run_id, &batch.batch_id), batch)
}

pub fn write_review_task_metadata(
    run_id: &str,
    task_id: &str,
    metadata: &code_review::ReviewTaskMetadata,
) -> Result<()> {
    write_json_atomic(&review_metadata_path(run_id, task_id), metadata)
}

pub fn load_review_task_metadata(
    run_id: &str,
    task_id: &str,
) -> Option<code_review::ReviewTaskMetadata> {
    let path = review_metadata_path(run_id, task_id);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
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

// ── SQLite dual-write and query helpers ────────────────────────────────────
//
// All dual-write helpers are non-fatal: file writes are authoritative, SQLite
// updates are opportunistic.  Connection management and schema initialisation
// are delegated to the shared StorageIndex so that a freshly-created or rebuilt
// index.db always has valid tables before any INSERT.

use crate::storage::index::StorageIndex;

// Query/record types for coordination-specific SQLite queries.
// These are not part of chatbot_lib's StorageIndex — they are local to auto-runs.

#[derive(Default)]
pub struct RunQuery {
    pub run_id: Option<String>,
    pub provider: Option<String>,
    pub status: Option<String>,
    pub workflow_mode: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

pub struct RunRecord {
    pub run_id: String,
    pub provider: Option<String>,
    pub goal: Option<String>,
    pub status: String,
    pub workflow_mode: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Default)]
pub struct CoordinationEventQuery {
    pub run_id: Option<String>,
    pub event_type: Option<String>,
    pub provider: Option<String>,
    pub at_after: Option<DateTime<Utc>>,
    pub at_before: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

pub struct CoordinationEventRecord {
    pub id: i64,
    pub run_id: String,
    pub event_type: String,
    pub task_id: Option<String>,
    pub provider: Option<String>,
    pub category: Option<String>,
    pub fallback_from: Option<String>,
    pub message: Option<String>,
    pub at: String,
}

pub fn list_runs_from_index(idx: &StorageIndex, query: &RunQuery) -> Result<Vec<RunRecord>> {
    let conn = idx.connection();
    let mut sql = String::from(
        "SELECT run_id, provider, goal, status, workflow_mode, created_at, updated_at FROM runs WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(ref run_id) = query.run_id {
        sql.push_str(" AND run_id = ?");
        params.push(Box::new(run_id.clone()));
    }
    if let Some(ref provider) = query.provider {
        sql.push_str(" AND provider = ?");
        params.push(Box::new(provider.clone()));
    }
    if let Some(ref status) = query.status {
        sql.push_str(" AND status = ?");
        params.push(Box::new(status.clone()));
    }
    if let Some(ref workflow_mode) = query.workflow_mode {
        sql.push_str(" AND workflow_mode = ?");
        params.push(Box::new(workflow_mode.clone()));
    }
    if let Some(created_after) = query.created_after {
        sql.push_str(" AND created_at >= ?");
        params.push(Box::new(created_after.to_rfc3339()));
    }
    if let Some(created_before) = query.created_before {
        sql.push_str(" AND created_at <= ?");
        params.push(Box::new(created_before.to_rfc3339()));
    }
    sql.push_str(" ORDER BY created_at DESC");
    if let Some(limit) = query.limit {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(RunRecord {
            run_id: row.get(0)?,
            provider: row.get(1)?,
            goal: row.get(2)?,
            status: row.get(3)?,
            workflow_mode: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(anyhow::Error::from)
}

pub fn list_events_from_index(
    idx: &StorageIndex,
    query: &CoordinationEventQuery,
) -> Result<Vec<CoordinationEventRecord>> {
    let conn = idx.connection();
    let mut sql = String::from(
        "SELECT rowid, run_id, event_type, task_id, provider, category, fallback_from, message, at FROM coordination_event WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(ref run_id) = query.run_id {
        sql.push_str(" AND run_id = ?");
        params.push(Box::new(run_id.clone()));
    }
    if let Some(ref event_type) = query.event_type {
        sql.push_str(" AND event_type = ?");
        params.push(Box::new(event_type.clone()));
    }
    if let Some(ref provider) = query.provider {
        sql.push_str(" AND provider = ?");
        params.push(Box::new(provider.clone()));
    }
    if let Some(at_after) = query.at_after {
        sql.push_str(" AND at >= ?");
        params.push(Box::new(at_after.to_rfc3339()));
    }
    if let Some(at_before) = query.at_before {
        sql.push_str(" AND at <= ?");
        params.push(Box::new(at_before.to_rfc3339()));
    }
    sql.push_str(" ORDER BY at DESC");
    if let Some(limit) = query.limit {
        sql.push_str(&format!(" LIMIT {}", limit));
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(CoordinationEventRecord {
            id: row.get(0)?,
            run_id: row.get(1)?,
            event_type: row.get(2)?,
            task_id: row.get(3)?,
            provider: row.get(4)?,
            category: row.get(5)?,
            fallback_from: row.get(6)?,
            message: row.get(7)?,
            at: row.get(8)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(anyhow::Error::from)
}

/// Obtain a shared StorageIndex, returning None when the database is
/// unavailable (e.g. first launch before any directory exists).  Because
/// `StorageIndex::open()` calls `open_index()` → `ensure_schema()`, the
/// schema is guaranteed to exist before any write or query.
pub fn get_storage_index() -> Option<StorageIndex> {
    StorageIndex::open().ok()
}

/// Upsert a coordination run into the SQLite index.
/// Non-fatal: logs warning on failure but does not propagate errors.
pub fn index_run(run: &CoordinationRun) {
    let Some(idx) = get_storage_index() else {
        return;
    };
    let conn = idx.connection();
    let completed_at: Option<String> = if matches!(
        run.status,
        CoordinationRunStatus::Completed
            | CoordinationRunStatus::Failed
            | CoordinationRunStatus::Canceled
    ) {
        Some(run.updated_at.to_rfc3339())
    } else {
        None
    };
    let workflow_mode = run.workflow_mode.as_str().to_string();

    if let Err(err) = conn.execute(
        r#"
        INSERT INTO runs (run_id, root_chat_id, workspace_path, provider, goal, status, workflow_mode, created_at, updated_at, completed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(run_id) DO UPDATE SET
            status       = excluded.status,
            updated_at   = excluded.updated_at,
            completed_at = excluded.completed_at
        "#,
        rusqlite::params![
            run.run_id,
            run.root_chat_id,
            run.workspace_path,
            run.provider,
            run.goal,
            run.status.as_str(),
            workflow_mode,
            run.created_at.to_rfc3339(),
            run.updated_at.to_rfc3339(),
            completed_at,
        ],
    ) {
        debug!("index_run failed for {}: {}", run.run_id, err);
    }
}

/// Upsert a coordination task into the SQLite index.
/// Preserves provider, category, and fallback_from attribution.
pub fn index_task(task: &CoordinationTask) {
    let Some(idx) = get_storage_index() else {
        return;
    };
    let conn = idx.connection();
    let category = task
        .task_category
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(str::to_string));
    let write_scope_json = serde_json::to_string(&task.write_scope).unwrap_or_default();
    let claimed_at: Option<String> = task.claimed_by.map(|_| task.updated_at.to_rfc3339());
    let completed_at: Option<String> = if matches!(
        task.status,
        CoordinationTaskStatus::Completed
            | CoordinationTaskStatus::Failed
            | CoordinationTaskStatus::Canceled
    ) {
        Some(task.updated_at.to_rfc3339())
    } else {
        None
    };
    if let Err(err) = conn.execute(
        r#"
        INSERT INTO tasks (task_id, run_id, title, status, category, provider, fallback_from, write_scope, claimed_by, claimed_at, completed_at, attempt_count)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ON CONFLICT(run_id, task_id) DO UPDATE SET
            status           = excluded.status,
            category         = excluded.category,
            provider         = excluded.provider,
            fallback_from    = excluded.fallback_from,
            claimed_by       = excluded.claimed_by,
            claimed_at       = excluded.claimed_at,
            completed_at     = excluded.completed_at,
            attempt_count    = excluded.attempt_count
        "#,
        rusqlite::params![
            task.task_id,
            task.run_id,
            task.title,
            format!("{:?}", task.status).to_lowercase(),
            category,
            task.assigned_provider,
            task.provider_hint,
            write_scope_json,
            task.claimed_by.map(|pid| pid.to_string()),
            claimed_at,
            completed_at,
            task.attempt_count,
        ],
    ) {
        debug!("index_task failed for {}/{}: {}", task.run_id, task.task_id, err);
    }
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

/// Look up the cost tier string for a given provider_id from the provider registry.
pub fn provider_cost_tier_for(provider_id: &str) -> Option<String> {
    let profiles = config::load_provider_profiles();
    profiles
        .get(provider_id)
        .map(|p| config_cost_tier_name(p.cost_tier.clone()))
}

pub fn config_cost_tier_name(cost_tier: config::CostTier) -> String {
    match cost_tier {
        config::CostTier::Low => "low",
        config::CostTier::Medium => "medium",
        config::CostTier::High => "high",
        config::CostTier::Premium => "premium",
    }
    .to_string()
}

/// Record provider usage for a completed or failed task.
/// Captures provider, category, cost_tier, and outcome for full attribution.
pub fn index_provider_usage(
    run_id: &str,
    task_id: &str,
    provider_id: &str,
    category: Option<&TaskCategory>,
    outcome: &CoordinationTaskOutcome,
    started_at: Option<&DateTime<Utc>>,
    finished_at: &DateTime<Utc>,
    cost_tier: Option<&str>,
) {
    let Some(idx) = get_storage_index() else {
        return;
    };
    let conn = idx.connection();
    let category_str = category
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(str::to_string));
    if let Err(err) = conn.execute(
        r#"
        INSERT INTO provider_usage (run_id, task_id, provider_id, cost_tier, category, outcome, started_at, finished_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        rusqlite::params![
            run_id,
            task_id,
            provider_id,
            cost_tier,
            category_str,
            outcome.as_str(),
            started_at.map(|dt| dt.to_rfc3339()),
            finished_at.to_rfc3339(),
        ],
    ) {
        debug!("index_provider_usage failed for {}/{}: {}", run_id, task_id, err);
    }
}

// ── Coordination query APIs ────────────────────────────────────────────────
//
// These delegate to the shared StorageIndex query helpers, converting results
// to the legacy types still used within the coordination module.

/// A run summary row returned by coordination queries.
#[derive(Debug, Clone)]
pub struct RunSummaryRow {
    pub run_id: String,
    pub provider: String,
    pub goal: String,
    pub status: String,
    pub workflow_mode: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<RunRecord> for RunSummaryRow {
    fn from(r: RunRecord) -> Self {
        Self {
            run_id: r.run_id,
            provider: r.provider.unwrap_or_default(),
            goal: r.goal.unwrap_or_default(),
            status: r.status,
            workflow_mode: r.workflow_mode,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

impl From<&CoordinationRun> for RunSummaryRow {
    fn from(run: &CoordinationRun) -> Self {
        Self {
            run_id: run.run_id.clone(),
            provider: run.provider.clone(),
            goal: run.goal.clone(),
            status: run.status.as_str().to_string(),
            workflow_mode: Some(run.workflow_mode.as_str().to_string()),
            created_at: run.created_at.to_rfc3339(),
            updated_at: run.updated_at.to_rfc3339(),
        }
    }
}

/// An event row returned by coordination queries.
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: i64,
    pub run_id: String,
    pub event_type: String,
    pub task_id: Option<String>,
    pub provider: Option<String>,
    pub category: Option<String>,
    pub fallback_from: Option<String>,
    pub message: Option<String>,
    pub at: String,
}

impl From<CoordinationEventRecord> for EventRow {
    fn from(r: CoordinationEventRecord) -> Self {
        Self {
            id: r.id,
            run_id: r.run_id,
            event_type: r.event_type,
            task_id: r.task_id,
            provider: r.provider,
            category: r.category,
            fallback_from: r.fallback_from,
            message: r.message,
            at: r.at,
        }
    }
}
