use auto_runs::workflow::workflow_specs;
use std::collections::HashSet;
use std::path::Path;

#[test]
fn workflow_registry_contains_expected_core_workflows() {
    let workflow_names: HashSet<&str> = workflow_specs()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect();

    for name in [
        "design-consensus",
        "design-implementation",
        "freeform-council",
        "code-review",
    ] {
        assert!(
            workflow_names.contains(name),
            "workflow registry missing expected workflow {:?}",
            name
        );
    }
}

#[test]
fn workflow_registry_names_are_unique() {
    let workflow_names: Vec<&str> = workflow_specs()
        .iter()
        .map(|spec| spec.name.as_str())
        .collect();
    let unique_names: HashSet<&str> = workflow_names.iter().copied().collect();
    assert_eq!(
        workflow_names.len(),
        unique_names.len(),
        "workflow registry contains duplicate names"
    );
}

#[test]
fn all_registry_specs_have_required_fields() {
    for spec in workflow_specs() {
        assert!(!spec.name.is_empty(), "spec has empty name");
        assert!(!spec.title.is_empty(), "spec {} has empty title", spec.name);
        assert!(
            !spec.summary.is_empty(),
            "spec {} has empty summary",
            spec.name
        );
        assert!(
            !spec.steps.is_empty(),
            "spec {} has no steps defined",
            spec.name
        );
    }
}

#[test]
fn workflow_registry_context_files_exist() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for spec in workflow_specs() {
        for path in &spec.context_files {
            assert!(
                root.join(path).exists(),
                "spec {} references missing context file {:?}",
                spec.name,
                path
            );
        }
    }
}

#[test]
fn code_review_profile_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("docs/review-profile.md");
    assert!(
        path.is_file(),
        "code review profile missing at {:?}; src/code_review.rs expects this path",
        path
    );
}
