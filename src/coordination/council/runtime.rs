use std::sync::atomic::{AtomicU32, Ordering};
use super::parsing::*;
use super::runtime_support::*;
use super::*;
use crate::coordination;
use fs2::FileExt;
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
fn council_commit_path(run_id: &str) -> std::path::PathBuf {
    coordination::council_json_path(run_id).with_file_name("council-commit.json")
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
