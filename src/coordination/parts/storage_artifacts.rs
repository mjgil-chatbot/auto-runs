pub fn artifacts_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("artifacts")
}

pub fn artifact_dir(run_id: &str, task_id: &str) -> PathBuf {
    artifacts_dir(run_id).join(task_id)
}

pub fn artifact_history_dir(run_id: &str, task_id: &str, generation: u32) -> PathBuf {
    artifact_dir(run_id, task_id)
        .join("history")
        .join(format!("generation-{generation}"))
}

pub fn artifact_result_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("result.json")
}

pub fn artifact_notes_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("notes.md")
}

pub fn artifact_materialized_files_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("materialized-files.json")
}

pub fn review_metadata_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("review-metadata.json")
}

pub fn archived_artifact_result_path(run_id: &str, task_id: &str, generation: u32) -> PathBuf {
    artifact_history_dir(run_id, task_id, generation).join("result.json")
}

pub fn archived_artifact_notes_path(run_id: &str, task_id: &str, generation: u32) -> PathBuf {
    artifact_history_dir(run_id, task_id, generation).join("notes.md")
}

pub fn archived_artifact_materialized_files_path(
    run_id: &str,
    task_id: &str,
    generation: u32,
) -> PathBuf {
    artifact_history_dir(run_id, task_id, generation).join("materialized-files.json")
}

pub fn load_dependency_artifacts(run_id: &str, dependencies: &[String]) -> Result<Vec<String>> {
    let mut artifacts = Vec::new();
    for dependency in dependencies {
        let notes_path = artifact_notes_path(run_id, dependency);
        if !notes_path.exists() {
            continue;
        }
        artifacts.push(fs::read_to_string(notes_path)?);
    }
    Ok(artifacts)
}

pub fn load_dependency_artifacts_for_task(task: &CoordinationTask) -> Result<Vec<String>> {
    let mut artifacts = Vec::new();
    for dependency_id in &task.dependencies {
        let dependency = load_task(&task.run_id, dependency_id)?;
        if dependency.status != CoordinationTaskStatus::Completed
            || effective_dependency_generation(&dependency) != effective_dependency_generation(task)
        {
            continue;
        }
        let notes_path = artifact_notes_path(&task.run_id, dependency_id);
        if !notes_path.exists() {
            continue;
        }
        artifacts.push(fs::read_to_string(notes_path)?);
    }
    Ok(artifacts)
}
