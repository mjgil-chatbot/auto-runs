use super::*;

pub const PERF_BENCHMARK_SCHEMA_VERSION: u32 = 1;
pub const PERF_PRIMARY_BENCH_CASE: &str = "parallel";
pub const PERF_SECONDARY_BENCH_CASE: &str = "parallel-sub-trees";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PerfCorrectnessStatus {
    Pass,
    Fail,
    #[default]
    Unknown,
}

impl PerfCorrectnessStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PerfBenchmarkCheck {
    pub name: String,
    #[serde(default)]
    pub status: PerfCorrectnessStatus,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PerfBenchmarkCorrectness {
    #[serde(default)]
    pub status: PerfCorrectnessStatus,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub checks: Vec<PerfBenchmarkCheck>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PerfBenchmarkCaseMetrics {
    pub name: String,
    pub median_s: f64,
    pub min_s: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PerfBenchmarkMetrics {
    #[serde(default)]
    pub primary_case: String,
    #[serde(default)]
    pub cases: Vec<PerfBenchmarkCaseMetrics>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PerfBenchmarkArtifact {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub benchmark: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idea_index: Option<u32>,
    #[serde(default)]
    pub build_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub correctness: PerfBenchmarkCorrectness,
    #[serde(default)]
    pub metrics: PerfBenchmarkMetrics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_median_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_min_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_sub_trees_median_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_sub_trees_min_s: Option<f64>,
    #[serde(default)]
    pub raw: String,
}

impl PerfBenchmarkArtifact {
    pub fn normalize(mut self) -> Self {
        if self.schema_version == 0 {
            self.schema_version = PERF_BENCHMARK_SCHEMA_VERSION;
        }
        if self.benchmark.trim().is_empty() {
            self.benchmark = "perf_hill_climber".to_string();
        }
        if self.metrics.cases.is_empty() {
            if let (Some(median_s), Some(min_s)) = (self.parallel_median_s, self.parallel_min_s) {
                self.metrics.cases.push(PerfBenchmarkCaseMetrics {
                    name: PERF_PRIMARY_BENCH_CASE.to_string(),
                    median_s,
                    min_s,
                });
            }
            if let (Some(median_s), Some(min_s)) = (
                self.parallel_sub_trees_median_s,
                self.parallel_sub_trees_min_s,
            ) {
                self.metrics.cases.push(PerfBenchmarkCaseMetrics {
                    name: PERF_SECONDARY_BENCH_CASE.to_string(),
                    median_s,
                    min_s,
                });
            }
        }
        if self.metrics.primary_case.trim().is_empty() {
            self.metrics.primary_case = if self.case_metrics(PERF_PRIMARY_BENCH_CASE).is_some() {
                PERF_PRIMARY_BENCH_CASE.to_string()
            } else {
                self.metrics
                    .cases
                    .first()
                    .map(|case| case.name.clone())
                    .unwrap_or_else(|| PERF_PRIMARY_BENCH_CASE.to_string())
            };
        }

        if self.parallel_median_s.is_none() || self.parallel_min_s.is_none() {
            if let Some((median_s, min_s)) = self
                .case_metrics(PERF_PRIMARY_BENCH_CASE)
                .map(|case| (case.median_s, case.min_s))
            {
                self.parallel_median_s.get_or_insert(median_s);
                self.parallel_min_s.get_or_insert(min_s);
            }
        }
        if self.parallel_sub_trees_median_s.is_none() || self.parallel_sub_trees_min_s.is_none() {
            if let Some((median_s, min_s)) = self
                .case_metrics(PERF_SECONDARY_BENCH_CASE)
                .map(|case| (case.median_s, case.min_s))
            {
                self.parallel_sub_trees_median_s.get_or_insert(median_s);
                self.parallel_sub_trees_min_s.get_or_insert(min_s);
            }
        }

        if self.correctness.summary.trim().is_empty() {
            self.correctness.summary = match self.correctness.status {
                PerfCorrectnessStatus::Pass => {
                    "benchmark output passed correctness checks".to_string()
                }
                PerfCorrectnessStatus::Fail => {
                    "benchmark output failed correctness checks".to_string()
                }
                PerfCorrectnessStatus::Unknown => {
                    "benchmark output did not report correctness checks".to_string()
                }
            };
        }

        self
    }

    pub fn primary_case_name(&self) -> &str {
        if self.metrics.primary_case.trim().is_empty() {
            PERF_PRIMARY_BENCH_CASE
        } else {
            self.metrics.primary_case.as_str()
        }
    }

    pub fn primary_case_median_s(&self) -> Option<f64> {
        self.case_metrics(self.primary_case_name())
            .map(|case| case.median_s)
            .or(self.parallel_median_s)
    }

    pub fn case_metrics(&self, name: &str) -> Option<&PerfBenchmarkCaseMetrics> {
        self.metrics.cases.iter().find(|case| case.name == name)
    }
}

pub fn parse_perf_benchmark_artifact(body: &str) -> Result<PerfBenchmarkArtifact> {
    let json =
        extract_first_json_object(body).context("missing JSON object in perf benchmark output")?;
    let value: serde_json::Value =
        serde_json::from_str(json).context("invalid perf benchmark JSON")?;
    if value.get("metrics").is_some()
        || value.get("correctness").is_some()
        || value.get("schema_version").is_some()
    {
        let artifact: PerfBenchmarkArtifact =
            serde_json::from_value(value).context("invalid perf benchmark artifact")?;
        return Ok(artifact.normalize());
    }

    Ok(parse_legacy_perf_benchmark_artifact(value))
}

pub fn make_failed_perf_benchmark_artifact(
    idea_index: Option<u32>,
    error: impl Into<String>,
) -> PerfBenchmarkArtifact {
    PerfBenchmarkArtifact {
        schema_version: PERF_BENCHMARK_SCHEMA_VERSION,
        benchmark: "perf_hill_climber".to_string(),
        idea_index,
        build_failed: true,
        error: Some(error.into()),
        correctness: PerfBenchmarkCorrectness {
            status: PerfCorrectnessStatus::Unknown,
            summary: "build failed before benchmark correctness checks could run".to_string(),
            checks: Vec::new(),
        },
        metrics: PerfBenchmarkMetrics {
            primary_case: PERF_PRIMARY_BENCH_CASE.to_string(),
            cases: Vec::new(),
        },
        parallel_median_s: Some(0.0),
        parallel_min_s: Some(0.0),
        parallel_sub_trees_median_s: Some(0.0),
        parallel_sub_trees_min_s: Some(0.0),
        raw: String::new(),
    }
}

pub fn perf_benchmark_contract_text(include_idea_index: bool) -> String {
    let idea_index_line = if include_idea_index {
        "\"idea_index\": <integer, 0-based>,\n       "
    } else {
        ""
    };
    format!(
        r#"{{
       "schema_version": {schema_version},
       "benchmark": "perf_hill_climber",
       {idea_index_line}"build_failed": false,
       "correctness": {{
         "status": "pass" | "fail" | "unknown",
         "summary": "<one-line correctness summary>",
         "checks": [
           {{
             "name": "<check name>",
             "status": "pass" | "fail" | "unknown",
             "summary": "<one-line check result>",
             "expected": "<optional expected output summary>",
             "actual": "<optional actual output summary>"
           }}
         ]
       }},
       "metrics": {{
         "primary_case": "{primary_case}",
         "cases": [
           {{ "name": "{primary_case}", "median_s": <float>, "min_s": <float> }},
           {{ "name": "{secondary_case}", "median_s": <float>, "min_s": <float> }}
         ]
       }},
       "parallel_median_s": <float>,
       "parallel_min_s": <float>,
       "parallel_sub_trees_median_s": <float>,
       "parallel_sub_trees_min_s": <float>,
       "raw": "<full benchmark stdout>"
     }}"#,
        schema_version = PERF_BENCHMARK_SCHEMA_VERSION,
        idea_index_line = idea_index_line,
        primary_case = PERF_PRIMARY_BENCH_CASE,
        secondary_case = PERF_SECONDARY_BENCH_CASE,
    )
}

fn parse_legacy_perf_benchmark_artifact(value: serde_json::Value) -> PerfBenchmarkArtifact {
    let idea_index = value["idea_index"].as_u64().map(|idx| idx as u32);
    let build_failed = value["build_failed"].as_bool().unwrap_or(false);

    PerfBenchmarkArtifact {
        schema_version: PERF_BENCHMARK_SCHEMA_VERSION,
        benchmark: "perf_hill_climber".to_string(),
        idea_index,
        build_failed,
        error: value["error"].as_str().map(str::to_string),
        correctness: PerfBenchmarkCorrectness {
            status: PerfCorrectnessStatus::Unknown,
            summary: if build_failed {
                "build failed before benchmark correctness checks could run".to_string()
            } else {
                "legacy benchmark output did not report correctness checks".to_string()
            },
            checks: Vec::new(),
        },
        metrics: PerfBenchmarkMetrics::default(),
        parallel_median_s: value["parallel_median_s"].as_f64(),
        parallel_min_s: value["parallel_min_s"].as_f64(),
        parallel_sub_trees_median_s: value["parallel_sub_trees_median_s"].as_f64(),
        parallel_sub_trees_min_s: value["parallel_sub_trees_min_s"].as_f64(),
        raw: value["raw"].as_str().unwrap_or_default().to_string(),
    }
    .normalize()
}
