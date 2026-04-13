use super::perf_hill_climber::{
    BENCH_RUNS, DEFAULT_BENCH_SCAN_PATH, DEFAULT_BENCH_SCRIPT, PERF_IMPROVEMENT_THRESHOLD_PCT,
    PERF_REJECT_CLEANUP_COMMAND,
};
use super::*;

pub fn build_perf_baseline_prompt(run: &CoordinationRun) -> String {
    let bench_script = bench_script_command_for_run(run);
    let scan_path = bench_scan_path_for_run(run);
    let workspace = perf_workspace_path_for_run(run);
    let contract = perf_benchmark_contract_text(false);
    format!(
        r#"Capture performance baseline for the workspace at {workspace}.

Instructions:
1. Build the release binary:
   cd {workspace} && cargo build --release 2>&1 | tail -20
2. Run the benchmark ({runs} runs):
   cd {workspace} && {bench_script} {scan_path} {runs}
3. `bench.sh` already emits the canonical perf benchmark JSON shown below.
   Treat that stdout as authoritative benchmark data.
   Do not rename fields, recalculate metrics, or rewrite values.
4. Write that exact JSON stdout to /tmp/perf_baseline.json.
5. Return that exact same JSON object as your final stdout.
6. If `bench.sh` does not emit valid benchmark JSON matching this schema, fail the task instead of inventing replacement data.

Schema reference:
{contract}"#,
        workspace = workspace,
        bench_script = bench_script,
        scan_path = scan_path,
        runs = BENCH_RUNS,
        contract = contract,
    )
}

pub fn build_perf_parse_prompt(run: &CoordinationRun) -> String {
    format!(
        r#"Read the performance design documents and extract individual, independently testable performance improvements.

Design documents: {doc_path}

Instructions:
- Read every document under the path above thoroughly.
- If dependency benchmark artifacts are provided, use their benchmark data exactly as written.
- Extract each distinct performance improvement idea.
- Each idea must be self-contained, concrete, and testable via benchmark.
- Your final stdout must be a single JSON object. If you brainstorm in prose first, discard it and return only the JSON object.
- Output ONLY JSON:
  {{
    "ideas": [
      {{
        "index": 0,
        "title": "Short title (max 60 chars)",
        "description": "Full detail: which files, what to change, and why it improves performance."
      }}
    ]
  }}
- Order ideas from largest expected speedup to smallest.
- Split multi-step ideas into separate entries."#,
        doc_path = run.design_doc_path,
    )
}

pub fn build_perf_apply_prompt(
    idea: &PerfIdea,
    idea_index: u32,
    total: usize,
    run: &CoordinationRun,
) -> String {
    let workspace = perf_workspace_path_for_run(run);
    format!(
        r#"Apply performance improvement {n} of {total}: {title}

{description}

Workspace: {workspace}

Instructions:
- Apply ONLY this specific change. No other changes, cleanups, or formatting fixes.
- Do not commit.
- Verify compilation after applying:
  cd {workspace} && cargo check 2>&1 | tail -10
  Do NOT do a full release build here — that is the bench step's job.
- Fill in the standard task report JSON (outcome, summary, details, changed_files).
  - outcome: completed_with_changes | completed_no_change | blocked | failed
  - changed_files: every file you modified (required if outcome=completed_with_changes)
  - If cargo check fails, set outcome=failed and explain in details."#,
        n = idea_index + 1,
        total = total,
        title = idea.title,
        description = idea.description,
        workspace = workspace,
    )
}

pub fn build_perf_bench_prompt(idea: &PerfIdea, idea_index: u32, run: &CoordinationRun) -> String {
    let bench_script = bench_script_command_for_run(run);
    let scan_path = bench_scan_path_for_run(run);
    let workspace = perf_workspace_path_for_run(run);
    let contract = perf_benchmark_contract_text(true);
    format!(
        r#"Build and benchmark the workspace after applying performance improvement {n}: {title}

Workspace: {workspace}

Instructions:
1. Build the release binary:
   cd {workspace} && cargo build --release 2>&1 | tail -20
   If the build fails, stop and output:
   {{
     "schema_version": {schema_version},
     "benchmark": "perf_hill_climber",
     "idea_index": {idx},
     "build_failed": true,
     "error": "<last error lines>",
     "correctness": {{
       "status": "unknown",
       "summary": "build failed before benchmark correctness checks could run",
       "checks": []
     }},
     "metrics": {{ "primary_case": "{primary_case}", "cases": [] }},
     "parallel_median_s": 0.0,
     "parallel_min_s": 0.0,
     "parallel_sub_trees_median_s": 0.0,
     "parallel_sub_trees_min_s": 0.0,
     "raw": ""
   }}
2. Run the benchmark ({runs} runs) with idea metadata injected:
   cd {workspace} && PERF_IDEA_INDEX={idx} {bench_script} {scan_path} {runs}
3. `bench.sh` already emits the canonical perf benchmark JSON shown below.
   Treat that stdout as authoritative benchmark data.
   Do not rename fields, recalculate metrics, or rewrite values.
4. Write that exact JSON stdout to /tmp/perf_bench_{idx}.json.
5. Return that exact same JSON object as your final stdout.
6. If `bench.sh` does not emit valid benchmark JSON matching this schema, fail the task instead of inventing replacement data.

Schema reference:
{contract}"#,
        n = idea_index + 1,
        title = idea.title,
        idx = idea_index,
        workspace = workspace,
        bench_script = bench_script,
        scan_path = scan_path,
        runs = BENCH_RUNS,
        schema_version = PERF_BENCHMARK_SCHEMA_VERSION,
        primary_case = PERF_PRIMARY_BENCH_CASE,
        contract = contract,
    )
}

pub fn build_perf_evaluate_prompt(
    idea: &PerfIdea,
    idea_index: u32,
    run: &CoordinationRun,
) -> String {
    let workspace = perf_workspace_path_for_run(run);
    format!(
        r#"Evaluate benchmark result for performance improvement {n}: {title}

Workspace: {workspace}

Instructions:
1. Read /tmp/perf_baseline.json and /tmp/perf_bench_{idx}.json.
2. Both files are benchmark artifact JSON produced by bench.sh.
3. If the benchmark artifact is missing or malformed, return `keep=false` and explain the benchmark artifact problem.
4. Determine correctness:
   - Prefer `correctness.status` from the benchmark JSON.
   - If the bench file says `fail`, revert and set keep=false even if it is faster.
   - If correctness is `unknown` or absent, treat it as legacy timing-only output and mention that in the reason.
5. Determine the primary benchmark median:
   - Prefer `metrics.primary_case` and the matching entry in `metrics.cases[*].median_s`.
   - Fall back to the legacy top-level `parallel_median_s` field if needed.
6. Calculate improvement:
   improvement_pct = (baseline_median - current_median) / baseline_median * 100
   Positive = faster, negative = slower.
7. Decision:
   - improvement_pct >= {threshold:.1}: KEEP — do not touch any source files.
   - Otherwise: REVERT — run:
     cd {workspace} && {reject_cleanup}
8. Write your decision to /tmp/perf_evaluate_{idx}.json:
   {{
     "keep": true | false,
     "idea_index": {idx},
     "reason": "One sentence with measured numbers.",
     "improvement_pct": <float>
   }}
9. Output ONLY that JSON — no markdown, no explanation."#,
        n = idea_index + 1,
        title = idea.title,
        idx = idea_index,
        workspace = workspace,
        threshold = PERF_IMPROVEMENT_THRESHOLD_PCT,
        reject_cleanup = PERF_REJECT_CLEANUP_COMMAND,
    )
}

fn bench_scan_path_for_run(run: &CoordinationRun) -> &str {
    for line in run.notes.lines() {
        if let Some(rest) = line.trim().strip_prefix("bench_scan_path=") {
            let path = rest.trim();
            if !path.is_empty() {
                return path;
            }
        }
    }
    DEFAULT_BENCH_SCAN_PATH
}

fn bench_script_command_for_run(run: &CoordinationRun) -> String {
    explicit_bench_script_path_for_run(run)
        .map(|path| shell_quote(&path))
        .unwrap_or_else(|| DEFAULT_BENCH_SCRIPT.to_string())
}

fn explicit_bench_script_path_for_run(run: &CoordinationRun) -> Option<String> {
    note_setting_value(&run.notes, "bench_script_path")
        .map(str::to_string)
        .or_else(|| explicit_bench_script_path_from_goal(&run.goal))
}

fn explicit_bench_script_path_from_goal(goal: &str) -> Option<String> {
    goal.split_whitespace().find_map(|token| {
        let candidate = token.trim_matches(|c: char| {
            matches!(
                c,
                ',' | '.' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '`'
            )
        });
        if candidate.starts_with('/') && candidate.ends_with(".sh") {
            Some(candidate.to_string())
        } else {
            None
        }
    })
}

fn note_setting_value<'a>(notes: &'a str, key: &str) -> Option<&'a str> {
    notes.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = format!("{key}=");
        trimmed.strip_prefix(&prefix).and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then_some(value)
        })
    })
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}
