use auto_runs::coordination::parse_task_execution_report;

fn make_report_json(outcome: &str) -> String {
    make_report_json_with_files(outcome, &[])
}

fn make_report_json_with_files(outcome: &str, changed_files: &[&str]) -> String {
    let files = changed_files
        .iter()
        .map(|f| format!(r#""{}""#, f))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"outcome":"{}","summary":"did the thing","details":"details here","changed_files":[{}]}}"#,
        outcome, files
    )
}

#[test]
fn test_normalize_outcome_aliases() {
    let with_files = [
        ("success", "completed_with_changes"),
        ("changes", "completed_with_changes"),
    ];
    for (alias, expected) in with_files {
        let json = make_report_json_with_files(alias, &["src/foo.rs"]);
        let report = parse_task_execution_report(&json)
            .unwrap_or_else(|e| panic!("alias '{}' failed: {}", alias, e));
        assert_eq!(
            report.outcome.as_str(),
            expected,
            "alias '{}' -> '{}'",
            alias,
            expected
        );
    }

    let no_files = [
        ("complete", "completed_no_change"),
        ("completed", "completed_no_change"),
        ("done", "completed_no_change"),
        ("no_change", "completed_no_change"),
        ("blocking", "blocked"),
        ("partially_complete", "partial"),
        ("partially_done", "partial"),
        ("failure", "failed"),
        ("error", "failed"),
        ("fail", "failed"),
    ];
    for (alias, expected) in no_files {
        let report = parse_task_execution_report(&make_report_json(alias))
            .unwrap_or_else(|e| panic!("alias '{}' failed: {}", alias, e));
        assert_eq!(
            report.outcome.as_str(),
            expected,
            "alias '{}' -> '{}'",
            alias,
            expected
        );
    }
}

#[test]
fn test_canonical_outcomes_pass_through() {
    let report = parse_task_execution_report(&make_report_json_with_files(
        "completed_with_changes",
        &["src/foo.rs"],
    ))
    .expect("completed_with_changes with files should succeed");
    assert_eq!(report.outcome.as_str(), "completed_with_changes");

    let rest = ["completed_no_change", "blocked", "partial", "failed"];
    for outcome in rest {
        let report = parse_task_execution_report(&make_report_json(outcome))
            .unwrap_or_else(|e| panic!("canonical '{}' failed: {}", outcome, e));
        assert_eq!(report.outcome.as_str(), outcome);
    }
}

#[test]
fn test_unknown_outcome_returns_error() {
    let result = parse_task_execution_report(&make_report_json("bogus_value"));
    assert!(result.is_err(), "unknown outcome should return an error");
}

#[test]
fn test_missing_required_fields_returns_error() {
    let result = parse_task_execution_report(r#"{"outcome":"done"}"#);
    assert!(result.is_err(), "missing summary/details should fail");
}
