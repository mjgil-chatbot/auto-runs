/// Perf Hill Climber: sequentially tests individual performance improvement ideas against a
/// benchmark script, keeping changes that improve performance and reverting those that do not.
///
/// Task DAG:
///   PerfBaseline (Queued)
///     └─ PerfParse (Blocked on baseline)
///          └─ [expand_perf_parse_task] →
///               PerfApply[0] → PerfBench[0] → PerfEvaluate[0]
///                                                  └─ [expand_perf_evaluate_task] →
///                                                       PerfApply[1] → PerfBench[1] → PerfEvaluate[1]
///                                                                                          └─ ...
use super::*;
use schemars::JsonSchema;

/// Minimum improvement percentage (primary benchmark median) needed to keep a change.
pub(crate) const PERF_IMPROVEMENT_THRESHOLD_PCT: f64 = 2.0;

/// Benchmark script used when the run input does not provide an explicit path.
pub(crate) const DEFAULT_BENCH_SCRIPT: &str = "./bench.sh";

/// Path to scan when running bench.sh. Overridable via a `bench_scan_path=<path>` line in run notes.
pub(crate) const DEFAULT_BENCH_SCAN_PATH: &str = "/snap";

/// Number of bench runs per invocation.
pub(crate) const BENCH_RUNS: u32 = 5;

pub(crate) const PERF_REJECT_CLEANUP_COMMAND: &str = "git checkout -- . && git clean -fd";

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct PerfIdea {
    pub index: u32,
    pub title: String,
    pub description: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerfIdeaList {
    pub ideas: Vec<PerfIdea>,
}

#[derive(Deserialize, Debug)]
pub struct PerfEvaluationResult {
    pub keep: bool,
    pub idea_index: u32,
    pub reason: String,
    #[serde(default)]
    pub improvement_pct: f64,
}

// ─── Seeding ──────────────────────────────────────────────────────────────────

pub fn seed_perf_hill_climber_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let baseline_id = Uuid::new_v4().to_string();
    let parse_id = Uuid::new_v4().to_string();

    let mut baseline_task = make_task(
        run,
        &baseline_id,
        None,
        CoordinationTaskKind::PerfBaseline,
        Some(run.provider.clone()),
        "Capture performance baseline".to_string(),
        build_perf_baseline_prompt(run),
        vec!["bash".to_string(), "filesystem".to_string()],
        vec![".".to_string()],
        vec!["/tmp/perf_baseline.json".to_string()],
        vec![],
        CoordinationTaskStatus::Queued,
    );
    baseline_task.iteration = Some(0);

    let parse_task = make_task(
        run,
        &parse_id,
        Some(baseline_id.clone()),
        CoordinationTaskKind::PerfParse,
        Some(run.provider.clone()),
        "Parse performance improvement ideas".to_string(),
        build_perf_parse_prompt(run),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        vec![baseline_id],
        CoordinationTaskStatus::Blocked,
    );

    vec![baseline_task, parse_task]
}

// ─── Parsers ──────────────────────────────────────────────────────────────────

pub fn parse_perf_ideas(body: &str) -> Result<PerfIdeaList> {
    let mut list = if lookup_contract(CoordinationTaskKind::PerfParse).is_some() {
        if let Ok(envelope) = parse_contract_envelope_from_response(body) {
            let payload = perf_ideas_payload_from_envelope(&envelope)?;
            PerfIdeaList {
                ideas: payload.ideas,
            }
        } else if let Some(json) = extract_first_json_object(body) {
            match serde_json::from_str::<PerfIdeaList>(json) {
                Ok(list) => list,
                Err(err) => {
                    warn!(
                        error = %err,
                        "failed to parse PerfParse JSON; attempting markdown idea fallback"
                    );
                    parse_perf_ideas_from_freeform(body)
                        .with_context(|| format!("invalid perf idea list JSON: {err}"))?
                }
            }
        } else {
            parse_perf_ideas_from_freeform(body)
                .context("missing JSON object in PerfParse output")?
        }
    } else if let Some(json) = extract_first_json_object(body) {
        match serde_json::from_str::<PerfIdeaList>(json) {
            Ok(list) => list,
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to parse PerfParse JSON; attempting markdown idea fallback"
                );
                parse_perf_ideas_from_freeform(body)
                    .with_context(|| format!("invalid perf idea list JSON: {err}"))?
            }
        }
    } else {
        parse_perf_ideas_from_freeform(body).context("missing JSON object in PerfParse output")?
    };
    for (i, idea) in list.ideas.iter_mut().enumerate() {
        idea.index = i as u32;
    }
    Ok(list)
}

fn parse_perf_ideas_from_freeform(body: &str) -> Result<PerfIdeaList> {
    let mut ideas = Vec::new();
    let mut seen_titles = BTreeSet::new();
    let mut current_title: Option<String> = None;
    let mut current_details = Vec::new();

    let mut flush_current = |title: &mut Option<String>, details: &mut Vec<String>| {
        let Some(title_value) = title.take() else {
            return;
        };
        let normalized_title = title_value.to_ascii_lowercase();
        if !seen_titles.insert(normalized_title) {
            details.clear();
            return;
        }
        let description = if details.is_empty() {
            format!("Investigate and benchmark: {}", title_value)
        } else {
            details.join(" ")
        };
        let next_index = ideas.len() as u32;
        ideas.push(PerfIdea {
            index: next_index,
            title: truncate_perf_idea_title(&title_value),
            description,
        });
        details.clear();
    };

    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = extract_freeform_perf_idea_title(trimmed) {
            flush_current(&mut current_title, &mut current_details);
            current_title = Some(title);
            continue;
        }
        if current_title.is_some() {
            if let Some(detail) = normalize_freeform_perf_idea_detail(trimmed) {
                current_details.push(detail);
            }
        }
    }
    flush_current(&mut current_title, &mut current_details);

    if ideas.is_empty() {
        anyhow::bail!("missing JSON object in PerfParse output");
    }
    Ok(PerfIdeaList { ideas })
}

fn extract_freeform_perf_idea_title(line: &str) -> Option<String> {
    let mut candidate = line.trim_start_matches('#').trim();
    if candidate.is_empty() {
        return None;
    }
    if let Some(inner) = candidate
        .strip_prefix("**")
        .and_then(|s| s.strip_suffix("**"))
    {
        candidate = inner.trim();
    }

    let digit_count = candidate
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let rest = candidate[digit_count..].trim_start();
    let rest = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix(')'))
        .map(str::trim_start)?;
    let rest = rest
        .strip_prefix("**")
        .and_then(|s| s.strip_suffix("**"))
        .unwrap_or(rest)
        .trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest.trim_end_matches(':').trim_matches('`').to_string())
}

fn normalize_freeform_perf_idea_detail(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed == "---"
        || trimmed.starts_with("Would you like me to")
        || trimmed.starts_with("Want me to")
    {
        return None;
    }

    let stripped = trimmed
        .trim_start_matches(|ch: char| matches!(ch, '-' | '*' | '>' | ' '))
        .trim();
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.trim_matches('*').to_string())
}

fn truncate_perf_idea_title(title: &str) -> String {
    const MAX_TITLE_CHARS: usize = 60;
    let mut chars = title.chars();
    let truncated: String = chars.by_ref().take(MAX_TITLE_CHARS).collect();
    if chars.next().is_some() {
        let mut shortened: String = truncated.chars().take(MAX_TITLE_CHARS - 3).collect();
        shortened.push_str("...");
        shortened
    } else {
        truncated
    }
}

pub fn parse_perf_evaluation(body: &str) -> Result<PerfEvaluationResult> {
    let json = if let Some(json) = extract_first_json_object(body) {
        json.to_string()
    } else {
        let found = (0..128u32).find_map(|i| {
            let path = format!("/tmp/perf_evaluate_{}.json", i);
            fs::read_to_string(&path).ok()
        });
        found
            .as_deref()
            .and_then(extract_first_json_object)
            .map(|s| s.to_string())
            .context("missing JSON object in PerfEvaluate output")?
    };
    let result: PerfEvaluationResult =
        serde_json::from_str(&json).context("invalid PerfEvaluate JSON")?;
    Ok(result)
}

// ─── Expansion ────────────────────────────────────────────────────────────────

pub fn expand_perf_parse_task(
    run_id: &str,
    parse_task: &CoordinationTask,
    body: &str,
) -> Result<()> {
    let ideas = parse_perf_ideas(body)?;
    if ideas.ideas.is_empty() {
        anyhow::bail!("PerfParse produced no ideas — check the design doc path");
    }

    let total = ideas.ideas.len();
    let run = load_run(run_id)?;
    let baseline_task_id = find_perf_baseline_task_id(run_id)?;
    let first_idea = &ideas.ideas[0];

    spawn_perf_idea_triplet(
        run_id,
        &run,
        first_idea,
        0,
        total,
        &parse_task.task_id,
        &baseline_task_id,
    )?;

    append_event(
        run_id,
        "perf_parse_complete",
        Some(&parse_task.task_id),
        &format!(
            "Parsed {} performance improvement ideas. Starting: {}",
            total, first_idea.title,
        ),
    )?;
    Ok(())
}

pub fn expand_perf_evaluate_task(
    run_id: &str,
    eval_task: &CoordinationTask,
    body: &str,
) -> Result<()> {
    let evaluation = parse_perf_evaluation(body)?;
    let current_index = eval_task
        .iteration
        .context("PerfEvaluate task missing iteration field")?;

    let action = if evaluation.keep { "kept" } else { "reverted" };
    append_event(
        run_id,
        "perf_evaluate",
        Some(&eval_task.task_id),
        &format!(
            "Idea {} {}: {:.1}% — {}",
            current_index, action, evaluation.improvement_pct, evaluation.reason,
        ),
    )?;

    let ideas = load_perf_ideas_from_run(run_id)?;
    let next_index = current_index + 1;

    if next_index as usize >= ideas.ideas.len() {
        append_event(
            run_id,
            "perf_hill_climber_done",
            Some(&eval_task.task_id),
            &format!(
                "All {} performance improvement ideas evaluated.",
                ideas.ideas.len()
            ),
        )?;
        return Ok(());
    }

    let existing_children = load_tasks(run_id)?
        .into_iter()
        .filter(|t| t.parent_task_id.as_deref() == Some(eval_task.task_id.as_str()))
        .count();
    if existing_children > 0 {
        return Ok(());
    }

    let run = load_run(run_id)?;
    let baseline_task_id = find_perf_baseline_task_id(run_id)?;
    let next_idea = &ideas.ideas[next_index as usize];

    spawn_perf_idea_triplet(
        run_id,
        &run,
        next_idea,
        next_index,
        ideas.ideas.len(),
        &eval_task.task_id,
        &baseline_task_id,
    )?;

    append_event(
        run_id,
        "perf_next_idea",
        Some(&eval_task.task_id),
        &format!(
            "Moving to idea {} of {}: {}",
            next_index + 1,
            ideas.ideas.len(),
            next_idea.title,
        ),
    )?;
    Ok(())
}

fn find_perf_baseline_task_id(run_id: &str) -> Result<String> {
    load_tasks(run_id)?
        .into_iter()
        .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
        .map(|t| t.task_id)
        .context("no PerfBaseline task found in run")
}

fn load_perf_ideas_from_run(run_id: &str) -> Result<PerfIdeaList> {
    let tasks = load_tasks(run_id)?;
    let parse_task = tasks
        .iter()
        .find(|t| t.kind == CoordinationTaskKind::PerfParse)
        .context("no PerfParse task found in run")?;
    let artifact_path = artifact_result_path(run_id, &parse_task.task_id);
    let raw = fs::read_to_string(&artifact_path).context("failed to read PerfParse artifact")?;
    let artifact: TaskArtifact =
        serde_json::from_str(&raw).context("failed to parse PerfParse artifact")?;
    parse_perf_ideas(&artifact.body)
}

fn spawn_perf_idea_triplet(
    run_id: &str,
    run: &CoordinationRun,
    idea: &PerfIdea,
    idea_index: u32,
    total: usize,
    gating_task_id: &str,
    baseline_task_id: &str,
) -> Result<()> {
    let apply_id = Uuid::new_v4().to_string();
    let bench_id = Uuid::new_v4().to_string();
    let eval_id = Uuid::new_v4().to_string();

    let mut apply_task = make_task(
        run,
        &apply_id,
        Some(gating_task_id.to_string()),
        CoordinationTaskKind::PerfApply,
        Some(run.provider.clone()),
        format!("Apply perf idea {}: {}", idea_index + 1, idea.title),
        build_perf_apply_prompt(idea, idea_index, total, run),
        vec![
            "filesystem".to_string(),
            "edit".to_string(),
            "bash".to_string(),
        ],
        vec!["**".to_string()],
        vec![".".to_string()],
        vec![gating_task_id.to_string()],
        CoordinationTaskStatus::Blocked,
    );
    apply_task.iteration = Some(idea_index);

    let mut bench_task = make_task(
        run,
        &bench_id,
        Some(apply_id.clone()),
        CoordinationTaskKind::PerfBench,
        Some(run.provider.clone()),
        format!("Bench perf idea {}: {}", idea_index + 1, idea.title),
        build_perf_bench_prompt(idea, idea_index, run),
        vec!["bash".to_string(), "filesystem".to_string()],
        vec![".".to_string()],
        vec![format!("/tmp/perf_bench_{}.json", idea_index)],
        vec![apply_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    bench_task.iteration = Some(idea_index);

    let mut eval_task = make_task(
        run,
        &eval_id,
        Some(bench_id.clone()),
        CoordinationTaskKind::PerfEvaluate,
        Some(run.provider.clone()),
        format!("Evaluate perf idea {}: {}", idea_index + 1, idea.title),
        build_perf_evaluate_prompt(idea, idea_index, run),
        vec!["bash".to_string(), "filesystem".to_string()],
        vec![
            "/tmp/perf_baseline.json".to_string(),
            format!("/tmp/perf_bench_{}.json", idea_index),
        ],
        vec![
            format!("/tmp/perf_evaluate_{}.json", idea_index),
            ".".to_string(),
        ],
        vec![bench_id.clone(), baseline_task_id.to_string()],
        CoordinationTaskStatus::Blocked,
    );
    eval_task.iteration = Some(idea_index);

    write_json_atomic(&task_path(run_id, &apply_id), &apply_task)?;
    write_json_atomic(&task_path(run_id, &bench_id), &bench_task)?;
    write_json_atomic(&task_path(run_id, &eval_id), &eval_task)?;

    let mut run = load_run(run_id)?;
    run.task_ids.push(apply_id);
    run.task_ids.push(bench_id);
    run.task_ids.push(eval_id);
    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;

    Ok(())
}

// ─── Artifact report helpers (called from artifacts.rs) ───────────────────────

pub fn perf_baseline_report(response: &str) -> Result<TaskExecutionReport> {
    let benchmark = parse_perf_benchmark_artifact(response)?;
    if benchmark.build_failed {
        let error = benchmark
            .error
            .clone()
            .unwrap_or_else(|| "baseline benchmark failed".to_string());
        return Ok(TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Failed,
            summary: "Baseline benchmark failed".to_string(),
            details: error,
            ..Default::default()
        });
    }
    let summary = match benchmark.primary_case_median_s() {
        Some(m) => format!("Baseline captured: primary median {:.2}s", m),
        None => "Baseline captured".to_string(),
    };
    Ok(TaskExecutionReport {
        outcome: CoordinationTaskOutcome::CompletedWithChanges,
        summary,
        details: response.trim().to_string(),
        changed_files: vec!["/tmp/perf_baseline.json".to_string()],
        ..Default::default()
    })
}

pub fn perf_parse_report(response: &str) -> Result<TaskExecutionReport> {
    let ideas = parse_perf_ideas(response)?;
    Ok(TaskExecutionReport {
        outcome: CoordinationTaskOutcome::CompletedWithChanges,
        summary: format!("Parsed {} performance improvement ideas", ideas.ideas.len()),
        details: response.trim().to_string(),
        ..Default::default()
    })
}

pub fn perf_bench_report(response: &str) -> Result<TaskExecutionReport> {
    let benchmark = parse_perf_benchmark_artifact(response)?;
    let idea_index = benchmark.idea_index.unwrap_or(0);
    let build_failed = benchmark.build_failed;
    let idea_display = idea_index + 1;

    if build_failed {
        let error = benchmark
            .error
            .unwrap_or_else(|| "unknown build error".to_string());
        return Ok(TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: format!("Bench idea {}: release build failed", idea_display),
            details: error,
            changed_files: vec![format!("/tmp/perf_bench_{}.json", idea_index)],
            ..Default::default()
        });
    }

    let median = benchmark.primary_case_median_s();
    let correctness = benchmark.correctness.status.label();
    let summary = match median {
        Some(m) => format!(
            "Bench idea {}: primary median {:.2}s ({})",
            idea_display, m, correctness
        ),
        None => format!("Bench idea {} complete", idea_display),
    };
    Ok(TaskExecutionReport {
        outcome: CoordinationTaskOutcome::CompletedWithChanges,
        summary,
        details: response.trim().to_string(),
        changed_files: vec![format!("/tmp/perf_bench_{}.json", idea_index)],
        ..Default::default()
    })
}

pub fn perf_evaluate_report(response: &str) -> Result<TaskExecutionReport> {
    let evaluation = parse_perf_evaluation(response)?;
    let action = if evaluation.keep { "kept" } else { "reverted" };
    let summary = format!(
        "Idea {} {}: {:.1}% — {}",
        evaluation.idea_index, action, evaluation.improvement_pct, evaluation.reason,
    );
    Ok(TaskExecutionReport {
        outcome: CoordinationTaskOutcome::CompletedWithChanges,
        summary,
        details: response.trim().to_string(),
        changed_files: vec![format!("/tmp/perf_evaluate_{}.json", evaluation.idea_index)],
        ..Default::default()
    })
}
