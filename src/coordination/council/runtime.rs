use std::sync::atomic::{AtomicU32, Ordering};
pub(crate) use super::make_council_provider_session;
use super::parsing::*;
use super::runtime_support::*;
use super::*;
use crate::coordination;
use fs2::FileExt;
const RAND_MULT: u32 = 1103515245;
const RAND_INC: u32 = 12345;
const RAND_MOD: u32 = 1 << 31;
static RAND_SEED: AtomicU32 = AtomicU32::new(0xdeadbeef);
fn rand_u32() -> u32 {
    let mut old = RAND_SEED.load(Ordering::Relaxed);
    loop {
        let next = old.wrapping_mul(RAND_MULT).wrapping_add(RAND_INC) % RAND_MOD;
        match RAND_SEED.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => old = actual,
        }
    }
}
struct CouncilMutationLock(fs::File);
#[derive(Serialize, Deserialize, Clone, Debug)]
struct CouncilPendingCommit {
    council: CouncilRun,
    run: coordination::CoordinationRun,
    entries: Vec<CouncilEntry>,
}
impl Drop for CouncilMutationLock {
    fn drop(&mut self) {
        if let Err(e) = self.0.unlock() {
            debug!("council mutation lock unlock failed: {}", e);
        }
    }
}
fn open_council_lock_file(run_id: &str) -> Result<fs::File> {
    let path = coordination::council_lock_path(run_id);
    let parent = path.parent().context("council lock path missing parent")?;
    fs::create_dir_all(parent)?;
    fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))
}
fn try_acquire_council_lock(run_id: &str) -> Result<Option<CouncilMutationLock>> {
    let file = open_council_lock_file(run_id)?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(CouncilMutationLock(file))),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("failed to acquire council lock for run {}", run_id))
        }
    }
}
fn acquire_council_lock(run_id: &str) -> Result<CouncilMutationLock> {
    const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    let file = open_council_lock_file(run_id)?;
    let deadline = std::time::Instant::now() + LOCK_TIMEOUT;
    let mut wait_duration = std::time::Duration::from_millis(50);

    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(CouncilMutationLock(file)),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    anyhow::bail!(
                        "timed out after {}s acquiring council lock for run {}",
                        LOCK_TIMEOUT.as_secs(),
                        run_id
                    );
                }
                // Yield to the OS scheduler before retrying — gives the lock holder
                // a chance to complete and release. Add a small random jitter to avoid
                // thundering herd when multiple processes compete for the same lock.
                let jitter = (rand_u32() % 20) as u64;
                std::thread::sleep(wait_duration + std::time::Duration::from_millis(jitter));
                // Cap at 2 seconds max between retries.
                wait_duration =
                    std::time::Duration::min(wait_duration * 2, std::time::Duration::from_secs(2));
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("failed to acquire council lock for run {}", run_id));
            }
        }
    }
}
fn council_commit_path(run_id: &str) -> std::path::PathBuf {
    coordination::council_json_path(run_id).with_file_name("council-commit.json")
}
fn file_missing_or_empty(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.len() == 0,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}
fn can_reseed_pristine_council_state(run_id: &str, run: &coordination::CoordinationRun) -> bool {
    let workspace_exists = Path::new(&run.workspace_path).is_dir();
    let pending_commit_missing = !council_commit_path(run_id).exists();
    let ledger_pristine = file_missing_or_empty(&coordination::ledger_path(run_id));
    let events_path = coordination::council_json_path(run_id).with_file_name("events.jsonl");
    let events_pristine = file_missing_or_empty(&events_path);
    let survey_missing = !coordination::artifact_result_path(run_id, SURVEY_ARTIFACT_ID).exists();

    workspace_exists
        && pending_commit_missing
        && ledger_pristine
        && events_pristine
        && survey_missing
}
fn synthesize_failed_council_state(run: &coordination::CoordinationRun) -> CouncilRun {
    let max_turns = run.council_max_turns.unwrap_or(40);
    CouncilRun {
        run_id: run.run_id.clone(),
        root_chat_id: run.root_chat_id.clone(),
        workspace_path: run.workspace_path.clone(),
        goal: run.goal.clone(),
        completion_criteria: run.completion_criteria.clone(),
        status: CouncilRunStatus::Failed,
        turn_policy: run.council_turn_policy.clone().unwrap_or_default(),
        max_turns,
        current_turn: 0,
        active_claims: Vec::new(),
        revoked_claims: Vec::new(),
        votes_complete: Vec::new(),
        turns_since_progress: 0,
        total_work_reports: 0,
        created_at: run.created_at,
        updated_at: Utc::now(),
        allow_write_tasks: run.allow_write_tasks,
        notes: run.notes.clone(),
        termination_policy: CouncilTerminationPolicy::for_max_turns(max_turns),
    }
}
fn quarantine_missing_council_state(
    run_id: &str,
    run: &mut coordination::CoordinationRun,
) -> Result<()> {
    let workspace_exists = Path::new(&run.workspace_path).is_dir();
    let pending_commit_exists = council_commit_path(run_id).exists();
    let ledger_path = coordination::ledger_path(run_id);
    let ledger_present = ledger_path.exists();
    let events_path = coordination::council_json_path(run_id).with_file_name("events.jsonl");
    let events_present = events_path.exists();

    let council = synthesize_failed_council_state(run);
    coordination::write_json_atomic(&coordination::council_json_path(run_id), &council)?;
    if !ledger_path.exists() {
        fs::File::create(&ledger_path)
            .with_context(|| format!("failed to create {}", ledger_path.display()))?;
    }

    run.status = coordination::CoordinationRunStatus::Failed;
    run.updated_at = Utc::now();
    coordination::write_json_atomic(&coordination::run_path(run_id), run)?;
    coordination::append_event(
        run_id,
        "council_state_corrupt",
        None,
        &format!(
            "Marked Freeform Council run failed because council.json was missing and state could not be safely reconstructed. workspace_exists={}, pending_commit_exists={}, ledger_present={}, events_present={}.",
            workspace_exists, pending_commit_exists, ledger_present, events_present
        ),
    )?;
    Ok(())
}
fn termination_entries_for_state(
    run_id: &str,
    council: &mut CouncilRun,
    run_status: coordination::CoordinationRunStatus,
) -> Result<Vec<CouncilEntry>> {
    let Some(outcome) = evaluate_termination(council, run_status) else {
        return Ok(Vec::new());
    };

    let entries = match outcome {
        CouncilTerminationOutcome::Complete => {
            council.status = CouncilRunStatus::VerifyingCompletion;
            let completion_issues = verify_completion(run_id, council)?;
            if completion_issues.is_empty() {
                council.status = CouncilRunStatus::Completed;
                vec![CouncilEntry::new(
                    CouncilSpeaker::System,
                    CouncilEntryKind::VotesAccepted,
                    format!(
                        "All {} council models voted complete and broker verification passed against: {}",
                        CouncilSpeaker::MODEL_ORDER.len(),
                        council.completion_criteria.trim()
                    ),
                )]
            } else {
                council.status = CouncilRunStatus::Active;
                council.votes_complete.clear();
                vec![CouncilEntry::new(
                    CouncilSpeaker::System,
                    CouncilEntryKind::VotesRejected,
                    format!(
                        "Completion verification failed: {}",
                        completion_issues.join("; ")
                    ),
                )]
            }
        }
        CouncilTerminationOutcome::Fail(reason) => {
            council.status = CouncilRunStatus::Failed;
            vec![CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::SystemNudge,
                termination_failure_message(run_id, council, reason),
            )]
        }
        CouncilTerminationOutcome::Cancel => {
            council.status = CouncilRunStatus::Canceled;
            vec![CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::SystemNudge,
                termination_failure_message(run_id, council, "user-stop"),
            )]
        }
    };

    council.updated_at = Utc::now();
    Ok(entries)
}
fn ensure_seed_entries(council: &CouncilRun, ledger: &[CouncilEntry]) -> Result<Vec<CouncilEntry>> {
    if !ledger.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    entries.push(CouncilEntry::new(
        CouncilSpeaker::System,
        CouncilEntryKind::SystemContext,
        format!(
            "Goal: {}\nCompletion criteria: {}",
            council.goal.trim(),
            council.completion_criteria.trim()
        ),
    ));

    let survey = write_workspace_survey_artifact(council)?;
    entries.push(CouncilEntry::new(
        CouncilSpeaker::System,
        CouncilEntryKind::SystemContext,
        format!(
            "Workspace survey attached: {}\n\n{}",
            survey.summary, survey.body
        ),
    ));

    Ok(entries)
}
fn write_workspace_survey_artifact(council: &CouncilRun) -> Result<coordination::TaskArtifact> {
    let now = Utc::now();
    let task = coordination::CoordinationTask {
        provider_hint: None,
        required_role: None,
        task_category: None,
        task_id: SURVEY_ARTIFACT_ID.to_string(),
        run_id: council.run_id.clone(),
        parent_task_id: None,
        kind: coordination::CoordinationTaskKind::SurveyWorkspace,
        assigned_provider: None,
        iteration: None,
        title: "Council workspace survey".to_string(),
        prompt: String::new(),
        cwd: council.workspace_path.clone(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: coordination::default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 1,
        publish_attempt_count: 0,
        estimated_minutes: Some(coordination::default_task_estimated_minutes_for_kind(
            &coordination::CoordinationTaskKind::SurveyWorkspace,
        )),
        status: coordination::CoordinationTaskStatus::Completed,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    let artifact = coordination::execute_survey_workspace(&task)?;
    coordination::write_json_atomic(
        &coordination::artifact_result_path(&council.run_id, SURVEY_ARTIFACT_ID),
        &artifact,
    )?;
    Ok(artifact)
}
fn append_ledger_entries(run_id: &str, entries: &[CouncilEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }

    let path = coordination::ledger_path(run_id);
    let parent = path.parent().context("ledger path missing parent")?;
    fs::create_dir_all(parent)?;

    let mut existing = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        existing.push(b'\n');
    }
    for entry in entries {
        serde_json::to_writer(&mut existing, entry)?;
        existing.push(b'\n');
    }

    let tmp_path = parent.join(format!(".ledger-{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(&existing)?;
        file.flush()?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, &path)?;
    let parent_file = fs::File::open(parent)?;
    parent_file.sync_all()?;

    Ok(())
}
fn append_events_for_entries(run_id: &str, entries: &[CouncilEntry]) -> Result<()> {
    let events_path = coordination::council_json_path(run_id).with_file_name("events.jsonl");
    let existing_event_ids = match coordination::load_events(run_id, None, None) {
        Ok(events) => events
            .into_iter()
            .filter_map(|event| event.task_id)
            .collect::<HashSet<_>>(),
        Err(_) if !events_path.exists() => HashSet::new(),
        Err(err) => return Err(err),
    };
    for entry in entries {
        if existing_event_ids.contains(&entry.entry_id) {
            continue;
        }
        let event_type = match entry.kind {
            CouncilEntryKind::Discussion => "council_discussion",
            CouncilEntryKind::ClaimRequest => "council_claim_requested",
            CouncilEntryKind::ClaimGranted => "council_claim_granted",
            CouncilEntryKind::ClaimRejected => "council_claim_rejected",
            CouncilEntryKind::ClaimRevoked => "council_claim_revoked",
            CouncilEntryKind::ReleaseScope => "council_scope_released",
            CouncilEntryKind::WorkReport => "council_work_reported",
            CouncilEntryKind::WorkValidated => "council_work_validated",
            CouncilEntryKind::WorkValidationFailed => "council_work_validation_failed",
            CouncilEntryKind::Delegation => "council_delegation",
            CouncilEntryKind::VoteComplete => "council_vote_complete",
            CouncilEntryKind::VotesAccepted => "council_votes_accepted",
            CouncilEntryKind::VotesRejected => "council_votes_rejected",
            CouncilEntryKind::SystemContext => "council_system_context",
            CouncilEntryKind::SystemNudge => "council_system_nudge",
            CouncilEntryKind::UserInterjection => "council_user_interjection",
            CouncilEntryKind::HandoffRequested => "council_handoff_requested",
            CouncilEntryKind::HandoffAccepted => "council_handoff_accepted",
            CouncilEntryKind::UserInputRequested => "council_user_input_requested",
        };
        coordination::append_event(run_id, event_type, Some(&entry.entry_id), &entry.content)?;
    }
    Ok(())
}
fn persist_council_and_run(
    council: &CouncilRun,
    run: &mut coordination::CoordinationRun,
) -> Result<()> {
    coordination::write_json_atomic(&coordination::council_json_path(&council.run_id), council)?;
    let desired_status = match council.status {
        CouncilRunStatus::Completed => coordination::CoordinationRunStatus::Completed,
        CouncilRunStatus::Failed => coordination::CoordinationRunStatus::Failed,
        CouncilRunStatus::Canceled => coordination::CoordinationRunStatus::Canceled,
        CouncilRunStatus::Active
        | CouncilRunStatus::WaitingForUser
        | CouncilRunStatus::VerifyingCompletion => coordination::CoordinationRunStatus::Active,
    };
    if run.status != desired_status {
        run.status = desired_status;
    }
    run.updated_at = Utc::now();
    coordination::write_json_atomic(&coordination::run_path(&council.run_id), run)?;
    Ok(())
}
fn call_council_provider(
    speaker: CouncilSpeaker,
    prompt: &str,
    run: &coordination::CoordinationRun,
) -> Result<String> {
    let provider = speaker
        .model_provider()
        .context("non-model speaker cannot take a council provider turn")?;
    let _runsc_env_guard = coordination::AutonomousRunscEnvGuard::activate(run)?;
    // Convert from auto_runs_contracts::AutonomousExecutionEnvironment to
    // chatbot_lib::AutonomousExecutionEnvironment (structurally identical, different crates).
    let chatbot_env = match run.execution_environment {
        AutonomousExecutionEnvironment::Host => chatbot_lib::AutonomousExecutionEnvironment::Host,
        AutonomousExecutionEnvironment::Runsc => chatbot_lib::AutonomousExecutionEnvironment::Runsc,
    };
    let overrides = autonomous_provider_overrides_with_execution(
        provider,
        Some(chatbot_env),
        Some(run.runsc_workspace_includes.clone()),
    );
    let session = Some(make_council_provider_session(speaker, run));
    let (tx, rx) = mpsc::channel();
    let stop_signal = Arc::new(AtomicBool::new(false));

    let result = call_provider_cli_stream_with_overrides(
        provider,
        prompt,
        None,
        None,
        session,
        Some(&overrides),
        tx,
        Arc::clone(&stop_signal),
    );
    let response = coordination::collect_stream_response(rx);
    if let Some(provider_failure) =
        coordination::extract_provider_command_failure(&response).map(str::to_owned)
    {
        return Err(anyhow::Error::msg(provider_failure));
    }
    result?;

    if response.trim().is_empty() {
        bail!("provider returned an empty response");
    }

    Ok(response)
}
