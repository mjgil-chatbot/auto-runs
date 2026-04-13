use super::*;
use fs2::FileExt;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;

const LIGHTWEIGHT_PUBLISH_EXCLUDED_DIRS: [&str; 4] = [".git", "target", ".zig-cache", "zig-out"];
const MAX_AUTO_PUBLISH_UNTRACKED_FILE_BYTES: u64 = 2 * 1024 * 1024;
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Published { commit: String },
    NoChange,
    AlreadyApplied,
    DeferredForResolution { conflicts: Vec<PatchConflict> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchConflict {
    pub path: PathBuf,
    pub hunk_index: usize,
    pub base_oid: String,
    pub their_oid: String,
}

#[derive(Clone, Debug, Default)]
pub struct PatchSeries {
    pub patches: Vec<PatchFile>,
}

#[derive(Clone, Debug)]
pub struct PatchFile {
    pub path: PathBuf,
    pub hunks: Vec<PatchHunk>,
    pub preimage_blob_oid: String,
    pub postimage_lines: Vec<String>,
    pub apply_patch_text: String,
}

#[derive(Clone, Debug)]
pub struct PatchHunk {
    pub preimage_start: usize,
    pub postimage_start: usize,
    pub preimage_lines: Vec<String>,
    pub postimage_lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyPatchResult {
    Clean,
    Partial { applied: usize, total: usize },
    Conflict { conflicts: Vec<PatchConflict> },
}

pub fn publish_lightweight_generated_write_task(
    task: &CoordinationTask,
    materialized_files: &[MaterializedArtifactFile],
) -> Result<Option<String>> {
    if task.kind != CoordinationTaskKind::GeneratedWriteTask {
        return Ok(None);
    }

    let workspace_root = Path::new(&task.cwd);
    git_stdout(workspace_root, &["rev-parse", "--show-toplevel"])?;
    let target_ref = lightweight_publish_target_ref(task, workspace_root)?;
    let _publish_lock = acquire_lightweight_publish_lock(workspace_root, &target_ref)?;
    let origin_remote_url = git_stdout(workspace_root, &["remote", "get-url", "origin"])?;
    let github_remote_url = git_optional_stdout(workspace_root, &["remote", "get-url", "github"])?;
    let reconciled_files = reconcile_materialized_files_for_publish(
        workspace_root,
        &task.write_scope,
        materialized_files,
    )?;
    if reconciled_files.is_empty() {
        return push_github_branch_fix_recovery_head(task, workspace_root, &target_ref);
    }

    if !git_head_exists(workspace_root)? {
        let published = publish_initial_lightweight_state(
            task,
            workspace_root,
            &reconciled_files,
            &target_ref,
            origin_remote_url.trim(),
            github_remote_url.as_deref(),
        )?;
        if let Some(commit) = published.as_deref() {
            maybe_sync_workspace_to_published_head(
                task,
                workspace_root,
                &target_ref,
                &reconciled_files,
                commit,
            )?;
        }
        return Ok(published);
    }

    ensure_origin_branch_seeded(workspace_root, &target_ref)?;

    let patch_series =
        build_patch_series_from_materialized_files(workspace_root, &reconciled_files)?;
    if patch_series.patches.is_empty() {
        return push_github_branch_fix_recovery_head(task, workspace_root, &target_ref);
    }

    let commit_message = lightweight_commit_message_for_task(task)?;
    match apply_patch_series_with_retry(
        workspace_root,
        &patch_series,
        &target_ref,
        origin_remote_url.trim(),
        github_remote_url.as_deref(),
        &commit_message,
    ) {
        Ok(PublishOutcome::Published { commit }) => {
            maybe_sync_workspace_to_published_head(
                task,
                workspace_root,
                &target_ref,
                &reconciled_files,
                &commit,
            )?;
            Ok(Some(commit))
        }
        Ok(PublishOutcome::NoChange) => Ok(None),
        Ok(PublishOutcome::AlreadyApplied) => {
            let head = git_remote_branch_oid(workspace_root, "origin", &target_ref)?
                .unwrap_or_else(|| {
                    git_stdout(workspace_root, &["rev-parse", "HEAD"]).unwrap_or_default()
                });
            maybe_sync_workspace_to_published_head(
                task,
                workspace_root,
                &target_ref,
                &reconciled_files,
                head.trim(),
            )?;
            Ok(Some(head.trim().to_string()))
        }
        Ok(PublishOutcome::DeferredForResolution { conflicts }) => {
            anyhow::bail!(
                "patch conflicts requiring resolution: {}",
                conflicts
                    .iter()
                    .map(|conflict| conflict.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Err(err) => Err(err),
    }
}

fn reconcile_materialized_files_for_publish(
    workspace_root: &Path,
    write_scope: &[String],
    materialized_files: &[MaterializedArtifactFile],
) -> Result<Vec<MaterializedArtifactFile>> {
    let mut files = BTreeMap::new();
    for file in materialized_files {
        let path = normalize_scope(&file.path);
        if path.is_empty() || should_skip_lightweight_publish_rel_path(&path) {
            continue;
        }
        files.insert(path, file.content.clone());
    }
    if files.is_empty() {
        files.extend(collect_explicit_write_scope_file_contents(
            workspace_root,
            write_scope,
        )?);
    }
    Ok(files
        .into_iter()
        .map(|(path, content)| MaterializedArtifactFile { path, content })
        .collect())
}

fn collect_explicit_write_scope_file_contents(
    root: &Path,
    write_scope: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for scope in write_scope {
        let normalized_scope = normalize_scope(scope);
        if normalized_scope.is_empty()
            || normalized_scope.contains('*')
            || should_skip_lightweight_publish_rel_path(&normalized_scope)
        {
            continue;
        }

        let candidate = root.join(&normalized_scope);
        if candidate.is_file() {
            insert_publishable_workspace_file(root, &candidate, &mut files)?;
        }
    }
    Ok(files)
}

fn collect_publishable_write_scope_file_contents(
    root: &Path,
    write_scope: &[String],
) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    if write_scope.is_empty() {
        return Ok(files);
    }

    for scope in write_scope {
        collect_publishable_scope_file_contents(root, scope, &mut files)?;
    }

    Ok(files)
}

fn collect_publishable_scope_file_contents(
    root: &Path,
    scope: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    if let Some(expanded) = expand_braces(scope) {
        for entry in expanded {
            collect_publishable_scope_file_contents(root, &entry, files)?;
        }
        return Ok(());
    }

    let normalized_scope = normalize_scope(scope);
    if normalized_scope.is_empty() {
        return Ok(());
    }

    if normalized_scope == "." || normalized_scope == "**" {
        collect_publishable_tree_file_contents(root, root, files)?;
        return Ok(());
    }

    if let Some(prefix) = wildcard_prefix(&normalized_scope) {
        let candidate = root.join(prefix.trim_matches('/'));
        collect_publishable_tree_file_contents(root, &candidate, files)?;
        return Ok(());
    }

    let candidate = root.join(&normalized_scope);
    if candidate.is_dir() {
        collect_publishable_tree_file_contents(root, &candidate, files)?;
    } else if candidate.is_file() {
        insert_publishable_workspace_file(root, &candidate, files)?;
    }

    Ok(())
}

fn collect_publishable_tree_file_contents(
    root: &Path,
    candidate: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let Ok(metadata) = fs::metadata(candidate) else {
        return Ok(());
    };
    if metadata.is_file() {
        insert_publishable_workspace_file(root, candidate, files)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }

    let Ok(entries) = fs::read_dir(candidate) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if should_exclude_publish_rel_path(&relative.to_string_lossy()) {
                continue;
            }
            collect_publishable_tree_file_contents(root, &path, files)?;
        } else {
            insert_publishable_workspace_file(root, &path, files)?;
        }
    }

    Ok(())
}

fn insert_publishable_workspace_file(
    root: &Path,
    candidate: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let Ok(relative) = candidate.strip_prefix(root) else {
        return Ok(());
    };
    let relative = normalize_scope(&relative.to_string_lossy());
    if relative.is_empty() || should_exclude_publish_rel_path(&relative) {
        return Ok(());
    }

    if !git_path_is_tracked(root, &relative)? {
        if git_path_is_ignored(root, &relative)? {
            return Ok(());
        }
        let metadata = fs::metadata(candidate)
            .with_context(|| format!("failed to read {}", candidate.display()))?;
        if metadata.len() > MAX_AUTO_PUBLISH_UNTRACKED_FILE_BYTES {
            return Ok(());
        }
    }

    let Ok(content) = fs::read_to_string(candidate) else {
        return Ok(());
    };
    files.insert(relative, content);
    Ok(())
}

fn should_exclude_publish_rel_path(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| LIGHTWEIGHT_PUBLISH_EXCLUDED_DIRS.contains(&name))
    })
}

fn should_skip_lightweight_publish_rel_path(path: &str) -> bool {
    should_exclude_publish_rel_path(path) || is_volatile_lightweight_publish_path(path)
}

fn is_volatile_lightweight_publish_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.starts_with("docs/design-pass-") && normalized.ends_with("-status.md")
}

fn acquire_lightweight_publish_lock(
    workspace_root: &Path,
    target_ref: &str,
) -> Result<std::fs::File> {
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical_root.hash(&mut hasher);
    target_ref.hash(&mut hasher);
    let lock_dir = std::env::temp_dir().join("auto-runs-publish-locks");
    fs::create_dir_all(&lock_dir)?;
    let lock_path = lock_dir.join(format!("{:016x}.lock", hasher.finish()));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open publish lock {}", lock_path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock publish slot {}", lock_path.display()))?;
    Ok(file)
}

fn lightweight_publish_target_ref(
    task: &CoordinationTask,
    workspace_root: &Path,
) -> Result<String> {
    let run = load_run(&task.run_id)?;
    if let Some(branch) = github_branch_fix_branch(&run).filter(|value| !value.trim().is_empty()) {
        return Ok(branch);
    }

    let branch = git_optional_stdout(workspace_root, &["symbolic-ref", "--short", "HEAD"])?
        .or_else(|| {
            git_optional_stdout(workspace_root, &["rev-parse", "--abbrev-ref", "HEAD"])
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| "main".to_string());
    let branch = branch.trim();
    Ok(if branch.is_empty() || branch == "HEAD" {
        "main".to_string()
    } else {
        branch.to_string()
    })
}

fn build_patch_series_from_materialized_files(
    workspace_root: &Path,
    materialized_files: &[MaterializedArtifactFile],
) -> Result<PatchSeries> {
    let mut patches = Vec::new();
    for file in materialized_files {
        let rel_path = normalize_scope(&file.path);
        if rel_path.is_empty() {
            continue;
        }

        let (preimage_blob_oid, preimage_content) =
            git_head_blob_for_path(workspace_root, &rel_path)?;
        let preimage_lines = split_diff_lines(preimage_content.as_deref());
        let postimage_lines = split_diff_lines(Some(&file.content));
        let hunks = compute_diff_hunks(&preimage_lines, &postimage_lines);
        if hunks.is_empty() {
            continue;
        }

        patches.push(PatchFile {
            path: PathBuf::from(&rel_path),
            hunks,
            preimage_blob_oid: preimage_blob_oid.clone().unwrap_or_default(),
            postimage_lines,
            apply_patch_text: build_apply_patch_text(
                &rel_path,
                preimage_content.as_deref(),
                &file.content,
                preimage_blob_oid.as_deref(),
            )?,
        });
    }
    Ok(PatchSeries { patches })
}

fn build_apply_patch_text(
    rel_path: &str,
    preimage_content: Option<&str>,
    postimage_content: &str,
    preimage_blob_oid: Option<&str>,
) -> Result<String> {
    if preimage_blob_oid.is_none() {
        let preimage_lines = split_diff_lines(preimage_content);
        let postimage_lines = split_diff_lines(Some(postimage_content));
        let hunks = compute_diff_hunks(&preimage_lines, &postimage_lines);
        return Ok(hunks_to_unified_diff(
            Path::new(rel_path),
            &hunks,
            preimage_blob_oid.unwrap_or_default(),
        ));
    }

    let temp_root = std::env::temp_dir().join(format!(
        "auto-runs-patch-apply-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    let before_path = temp_root.join("before").join(rel_path);
    let after_path = temp_root.join("after").join(rel_path);
    if let Some(parent) = before_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = after_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&before_path, preimage_content.unwrap_or_default())?;
    fs::write(&after_path, postimage_content)?;

    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--unified=3",
            "--no-color",
            before_path
                .to_str()
                .context("before patch path should be utf-8")?,
            after_path
                .to_str()
                .context("after patch path should be utf-8")?,
        ])
        .output()
        .with_context(|| format!("failed to build contextual diff for {rel_path}"))?;
    let _ = fs::remove_dir_all(&temp_root);

    match output.status.code() {
        Some(0) => Ok(String::new()),
        Some(1) => {
            let raw = String::from_utf8_lossy(&output.stdout);
            let body = raw
                .lines()
                .skip_while(|line| !line.starts_with("@@ "))
                .collect::<Vec<_>>();
            if body.is_empty() {
                anyhow::bail!("contextual diff for {rel_path} did not contain any hunks");
            }
            let safe_oid = preimage_blob_oid
                .filter(|oid| oid.len() == 40)
                .unwrap_or("0000000000000000000000000000000000000000");
            let mut lines = vec![
                format!("diff --git a/{rel_path} b/{rel_path}"),
                format!("index {safe_oid}..0000000000000000000000000000000000000000 100644"),
                format!("--- a/{rel_path}"),
                format!("+++ b/{rel_path}"),
            ];
            lines.extend(body.into_iter().map(str::to_string));
            lines.push(String::new());
            Ok(lines.join("\n"))
        }
        _ => anyhow::bail!(
            "git diff --no-index failed for {}: {}",
            rel_path,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

fn git_head_blob_for_path(root: &Path, rel_path: &str) -> Result<(Option<String>, Option<String>)> {
    if !git_head_exists(root)? {
        return Ok((None, None));
    }

    let spec = format!("HEAD:{rel_path}");
    let Some(blob_oid) = git_optional_stdout(root, &["rev-parse", "--verify", spec.as_str()])?
    else {
        return Ok((None, None));
    };
    let content = git_stdout_raw(root, &["show", spec.as_str()])?;
    Ok((Some(blob_oid.trim().to_string()), Some(content)))
}

fn split_diff_lines(content: Option<&str>) -> Vec<String> {
    content
        .map(|value| value.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn compute_diff_hunks(preimage: &[String], postimage: &[String]) -> Vec<PatchHunk> {
    if preimage.iter().eq(postimage.iter()) {
        return Vec::new();
    }

    let temp_root = std::env::temp_dir().join(format!(
        "auto-runs-patch-diff-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    fs::create_dir_all(&temp_root).expect("patch temp dir should be creatable");
    let before_path = temp_root.join("before.txt");
    let after_path = temp_root.join("after.txt");
    fs::write(&before_path, join_diff_lines(preimage)).expect("before diff snapshot should write");
    fs::write(&after_path, join_diff_lines(postimage)).expect("after diff snapshot should write");

    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--no-index",
            "--unified=0",
            "--no-color",
            "--no-prefix",
            before_path.to_str().expect("before path should be utf-8"),
            after_path.to_str().expect("after path should be utf-8"),
        ])
        .output()
        .expect("git diff should run for patch generation");
    let _ = fs::remove_dir_all(&temp_root);

    match output.status.code() {
        Some(0) => Vec::new(),
        Some(1) => parse_zero_context_diff_hunks(&String::from_utf8_lossy(&output.stdout)),
        _ => panic!(
            "git diff --no-index failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

fn join_diff_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn parse_zero_context_diff_hunks(diff_text: &str) -> Vec<PatchHunk> {
    let mut hunks = Vec::new();
    let mut current: Option<PatchHunk> = None;

    for line in diff_text.lines() {
        if line.starts_with("@@ ") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let (preimage_start, postimage_start) = parse_hunk_header(line)
                .unwrap_or_else(|| panic!("invalid unified diff hunk header: {line}"));
            current = Some(PatchHunk {
                preimage_start,
                postimage_start,
                preimage_lines: Vec::new(),
                postimage_lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        if let Some(content) = line.strip_prefix('-') {
            hunk.preimage_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix('+') {
            hunk.postimage_lines.push(content.to_string());
        }
    }

    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }

    hunks
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    let rest = header.strip_prefix("@@ ")?;
    let body = rest.split(" @@").next()?;
    let mut parts = body.split_whitespace();
    let pre = parts.next()?;
    let post = parts.next()?;
    Some((parse_hunk_range(pre)?, parse_hunk_range(post)?))
}

fn parse_hunk_range(token: &str) -> Option<usize> {
    let value = token.strip_prefix(['-', '+'])?;
    let (start, _count) = value.split_once(',').unwrap_or((value, "1"));
    start.parse().ok()
}

fn hunks_to_unified_diff(path: &Path, hunks: &[PatchHunk], preimage_blob_oid: &str) -> String {
    let mut lines = Vec::new();
    let display = path.display().to_string();
    let is_new_file = preimage_blob_oid.is_empty()
        && hunks
            .iter()
            .all(|hunk| hunk.preimage_lines.is_empty() && !hunk.postimage_lines.is_empty());
    lines.push(format!("diff --git a/{display} b/{display}"));
    if is_new_file {
        lines.push("new file mode 100644".to_string());
        lines.push("index 0000000000000000000000000000000000000000..0000000000000000000000000000000000000000 100644".to_string());
        lines.push("--- /dev/null".to_string());
    } else {
        let safe_oid = if preimage_blob_oid.len() == 40 {
            preimage_blob_oid
        } else {
            "0000000000000000000000000000000000000000"
        };
        lines.push(format!(
            "index {safe_oid}..0000000000000000000000000000000000000000 100644"
        ));
        lines.push(format!("--- a/{display}"));
    }
    lines.push(format!("+++ b/{display}"));
    for hunk in hunks {
        let pre_count = hunk.preimage_lines.len();
        let post_count = hunk.postimage_lines.len();
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            hunk.preimage_start, pre_count, hunk.postimage_start, post_count
        ));
        for line in &hunk.preimage_lines {
            lines.push(format!("-{line}"));
        }
        for line in &hunk.postimage_lines {
            lines.push(format!("+{line}"));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn apply_patch_series_with_retry(
    workspace_root: &Path,
    patch_series: &PatchSeries,
    target_ref: &str,
    origin_remote_url: &str,
    github_remote_url: Option<&str>,
    commit_message: &str,
) -> Result<PublishOutcome> {
    if patch_series.patches.is_empty() {
        return Ok(PublishOutcome::NoChange);
    }

    let mut attempt = 0;
    let max_attempts = 5;
    loop {
        attempt += 1;
        if attempt > max_attempts {
            anyhow::bail!(
                "publish retry limit reached after {} attempts",
                max_attempts
            );
        }

        let latest_head = git_remote_branch_oid(workspace_root, "origin", target_ref)?
            .ok_or_else(|| anyhow::anyhow!("origin is missing refs/heads/{target_ref}"))?;
        let latest_head = latest_head.trim().to_string();
        let worktree = checkout_temp_worktree(&latest_head, origin_remote_url)?;
        mirror_workspace_git_hooks(workspace_root, &worktree)?;
        ensure_publish_git_identity(&worktree)?;

        let mut all_clean = true;
        let mut conflicts = Vec::new();
        for patch_file in &patch_series.patches {
            if worktree_file_matches_patch_postimage(&worktree, patch_file)? {
                continue;
            }

            if try_apply_lightweight_task_doc_merge(&worktree, patch_file)? {
                continue;
            }

            let diff = &patch_file.apply_patch_text;
            let applied = git_apply_three_way(&worktree, &diff)?;
            let previous_local_publish = git_optional_stdout(
                workspace_root,
                &[
                    "rev-parse",
                    "--verify",
                    &local_publish_file_ref(
                        workspace_root,
                        target_ref,
                        &patch_file.path.to_string_lossy(),
                    )?,
                ],
            )?;
            if !matches!(applied, ApplyPatchResult::Clean)
                && previous_local_publish
                    .as_deref()
                    .is_some_and(|head| head.trim() == latest_head)
                && merge_local_publish_continuation(workspace_root, &worktree, patch_file)?
            {
                continue;
            }
            match Ok::<_, anyhow::Error>(applied) {
                Ok(ApplyPatchResult::Clean) => {}
                Ok(ApplyPatchResult::Partial { applied, total }) => {
                    all_clean = false;
                    if applied < total {
                        conflicts.push(PatchConflict {
                            path: patch_file.path.clone(),
                            hunk_index: applied,
                            base_oid: latest_head.clone(),
                            their_oid: String::new(),
                        });
                    }
                }
                Ok(ApplyPatchResult::Conflict { conflicts: current }) => {
                    all_clean = false;
                    conflicts.extend(current);
                }
                Err(err) => anyhow::bail!("git apply failed: {err}"),
            }
        }

        if !conflicts.is_empty() {
            let _ = fs::remove_dir_all(&worktree);
            return Ok(PublishOutcome::DeferredForResolution { conflicts });
        }

        if !all_clean {
            let _ = fs::remove_dir_all(&worktree);
            return Ok(PublishOutcome::DeferredForResolution { conflicts });
        }

        if !git_has_staged_changes(&worktree)? {
            let _ = fs::remove_dir_all(&worktree);
            return Ok(PublishOutcome::AlreadyApplied);
        }

        let commit_hash = commit_all_changes(&worktree, commit_message)?;
        let push_result = run_git_output(
            Some(&worktree),
            &["push", "origin", &format!("HEAD:refs/heads/{target_ref}")],
        );
        let _ = fs::remove_dir_all(&worktree);
        match push_result {
            Ok(output) if output.status.success() => {
                if let Some(github_remote_url) = github_remote_url {
                    push_commit_to_remote(
                        origin_remote_url,
                        github_remote_url,
                        &commit_hash,
                        target_ref,
                    )?;
                }
                return Ok(PublishOutcome::Published {
                    commit: commit_hash,
                });
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if is_retryable_publish_race(stderr.as_ref()) {
                    continue;
                }
                anyhow::bail!("git push failed: {}", stderr.trim());
            }
            Err(err) => anyhow::bail!("git push failed: {err}"),
        }
    }
}

fn is_retryable_publish_race(message: &str) -> bool {
    message.contains("non-fast-forward")
        || message.contains("fetch first")
        || message.contains("reference already exists")
        || message.contains("cannot lock ref 'refs/heads/")
        || message.contains("cannot lock ref 'refs/remotes/")
        || message.contains("unable to update local ref")
}

fn local_publish_file_ref(workspace: &Path, target: &str, path: &str) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new("git")
        .args(["hash-object", "--stdin"])
        .current_dir(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let mut input = child.stdin.take().context("missing Git hash input")?;
        input.write_all(target.as_bytes())?;
        input.write_all(&[0])?;
        input.write_all(path.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        anyhow::bail!("failed to identify local publish base");
    }
    let digest = String::from_utf8(output.stdout)?;
    Ok(format!("refs/auto-runs/published-files/{}", digest.trim()))
}

// A subsequent task in the same workspace may intentionally revise a hunk
// published by its predecessor while the workspace HEAD remains unchanged.
// This fallback is allowed only against that exact successful local publish.
fn merge_local_publish_continuation(
    workspace: &Path,
    worktree: &Path,
    patch: &PatchFile,
) -> Result<bool> {
    let relative = patch.path.to_string_lossy();
    let restored = run_git_output(
        Some(worktree),
        &[
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
            &relative,
        ],
    )?;
    if !restored.status.success() {
        return Ok(false);
    }
    let temporary = std::env::temp_dir().join(format!("auto-runs-continuation-{}", Uuid::new_v4()));
    fs::create_dir_all(&temporary)?;
    let result = (|| -> Result<bool> {
        let current = temporary.join("current");
        let base = temporary.join("base");
        let updated = temporary.join("updated");
        fs::copy(worktree.join(&patch.path), &current)?;
        let before = if patch.preimage_blob_oid.is_empty() {
            Vec::new()
        } else {
            let output = run_git_output(
                Some(workspace),
                &["cat-file", "blob", &patch.preimage_blob_oid],
            )?;
            if !output.status.success() {
                return Ok(false);
            }
            output.stdout
        };
        fs::write(&base, before)?;
        fs::write(&updated, join_diff_lines(&patch.postimage_lines))?;
        let output = run_git_output(
            Some(worktree),
            &[
                "merge-file",
                "--theirs",
                "-p",
                current.to_str().unwrap(),
                base.to_str().unwrap(),
                updated.to_str().unwrap(),
            ],
        )?;
        if !output.status.success() {
            return Ok(false);
        }
        fs::write(worktree.join(&patch.path), output.stdout)?;
        git_with_paths(worktree, &["add", "--"], &[relative.into_owned()])?;
        Ok(true)
    })();
    let _ = fs::remove_dir_all(temporary);
    result
}

fn git_apply_three_way(workdir: &Path, patch_text: &str) -> Result<ApplyPatchResult> {
    let tmp_patch = workdir.join(".patch.temp");
    fs::write(&tmp_patch, patch_text)?;
    let output = run_git_output(
        Some(workdir),
        &[
            "apply",
            "--3way",
            "--index",
            "--whitespace=nowarn",
            tmp_patch.to_str().unwrap(),
        ],
    );
    let _ = fs::remove_file(&tmp_patch);

    match output {
        Ok(output) if output.status.success() => Ok(ApplyPatchResult::Clean),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let conflicts = parse_apply_conflicts(&stderr);
            if conflicts.is_empty() {
                Ok(ApplyPatchResult::Partial {
                    applied: 0,
                    total: 1,
                })
            } else {
                Ok(ApplyPatchResult::Conflict { conflicts })
            }
        }
        Err(err) => anyhow::bail!("git apply failed: {err}"),
    }
}

fn parse_apply_conflicts(stderr: &str) -> Vec<PatchConflict> {
    let mut conflicts = Vec::new();
    let mut current_path = PathBuf::from("unknown");
    let mut hunk_index = 0;

    for line in stderr.lines() {
        if line.starts_with("Checking patch ") || line.starts_with("error: ") {
            if let Some(path) = line.split(' ').last() {
                current_path = PathBuf::from(path.trim_end_matches(':'));
            }
        }
        if line.contains("remaining hunks") || line.contains("hunk") {
            hunk_index += 1;
        }
        if line.contains("could not apply") {
            conflicts.push(PatchConflict {
                path: current_path.clone(),
                hunk_index,
                base_oid: String::new(),
                their_oid: String::new(),
            });
        }
    }
    conflicts
}

fn checkout_temp_worktree(base_ref: &str, remote_url: &str) -> Result<PathBuf> {
    let tmp = clone_temp_repo(remote_url)?;
    let checkout_output = run_git_output(Some(&tmp), &["checkout", base_ref])?;
    if !checkout_output.status.success() {
        anyhow::bail!(
            "git checkout {} failed: {}",
            base_ref,
            String::from_utf8_lossy(&checkout_output.stderr)
        );
    }

    Ok(tmp)
}

fn clone_temp_repo(remote_url: &str) -> Result<PathBuf> {
    let tmp = std::env::temp_dir().join(format!(
        "auto-runs-publish-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).ok();
    }

    let output = run_git_output(None, &["clone", remote_url, tmp.to_str().unwrap()])?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(tmp)
}

fn publish_initial_lightweight_state(
    task: &CoordinationTask,
    workspace_root: &Path,
    materialized_files: &[MaterializedArtifactFile],
    target_ref: &str,
    origin_remote_url: &str,
    github_remote_url: Option<&str>,
) -> Result<Option<String>> {
    let files = materialized_files
        .iter()
        .filter_map(|file| {
            let path = normalize_scope(&file.path);
            (!path.is_empty()).then(|| (path, file.content.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    if files.is_empty() {
        return Ok(None);
    }

    let worktree =
        checkout_temp_initial_publish_worktree(workspace_root, origin_remote_url, target_ref)?;
    mirror_workspace_git_hooks(workspace_root, &worktree)?;
    ensure_publish_git_identity(&worktree)?;
    clear_git_worktree_contents(&worktree)?;

    for (relative_path, content) in &files {
        let destination = worktree.join(relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, content)?;
    }
    git_stdout(&worktree, &["add", "--all"])?;
    if !git_has_staged_changes(&worktree)? {
        let _ = fs::remove_dir_all(&worktree);
        return Ok(None);
    }

    let commit_message = lightweight_commit_message_for_task(task)?;
    let commit_hash = commit_all_changes(&worktree, &commit_message)?;
    git_push_lightweight_remotes(&worktree, target_ref)?;
    if let Some(github_remote_url) = github_remote_url {
        push_commit_to_remote(
            origin_remote_url,
            github_remote_url,
            &commit_hash,
            target_ref,
        )?;
    }
    let _ = fs::remove_dir_all(&worktree);
    Ok(Some(commit_hash))
}

fn checkout_temp_initial_publish_worktree(
    workspace_root: &Path,
    remote_url: &str,
    target_ref: &str,
) -> Result<PathBuf> {
    let tmp = clone_temp_repo(remote_url)?;
    if let Some(latest_head) = git_remote_branch_oid(workspace_root, "origin", target_ref)? {
        let checkout_output = run_git_output(Some(&tmp), &["checkout", latest_head.trim()])?;
        if !checkout_output.status.success() {
            anyhow::bail!(
                "git checkout {} failed: {}",
                latest_head.trim(),
                String::from_utf8_lossy(&checkout_output.stderr).trim()
            );
        }
    } else {
        let checkout_output = run_git_output(Some(&tmp), &["checkout", "--orphan", target_ref])?;
        if !checkout_output.status.success() {
            anyhow::bail!(
                "git checkout --orphan {} failed: {}",
                target_ref,
                String::from_utf8_lossy(&checkout_output.stderr).trim()
            );
        }
    }
    Ok(tmp)
}

fn clear_git_worktree_contents(worktree: &Path) -> Result<()> {
    for entry in fs::read_dir(worktree)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn ensure_origin_branch_seeded(workspace_root: &Path, target_ref: &str) -> Result<()> {
    if git_remote_ref_exists(workspace_root, "origin", target_ref)?
        || !git_head_exists(workspace_root)?
    {
        return Ok(());
    }
    git_stdout(
        workspace_root,
        &[
            "push",
            "-u",
            "origin",
            &format!("HEAD:refs/heads/{target_ref}"),
        ],
    )?;
    Ok(())
}

fn git_remote_ref_exists(root: &Path, remote: &str, branch: &str) -> Result<bool> {
    let output = run_git_output(
        Some(root),
        &[
            "ls-remote",
            "--exit-code",
            "--heads",
            remote,
            &format!("refs/heads/{branch}"),
        ],
    )
    .with_context(|| {
        format!(
            "failed to inspect git remote {remote} in {}",
            root.display()
        )
    })?;
    Ok(output.status.success())
}

fn worktree_file_matches_patch_postimage(worktree: &Path, patch_file: &PatchFile) -> Result<bool> {
    if patch_file.hunks.is_empty() {
        return Ok(false);
    }
    let target = worktree.join(&patch_file.path);
    let current = if target.exists() {
        fs::read_to_string(&target)?
    } else {
        String::new()
    };
    Ok(split_diff_lines(Some(&current)) == patch_file.postimage_lines)
}

fn try_apply_lightweight_task_doc_merge(worktree: &Path, patch_file: &PatchFile) -> Result<bool> {
    if !is_lightweight_task_doc_path(&patch_file.path) {
        return Ok(false);
    }

    let target = worktree.join(&patch_file.path);
    let current = if target.exists() {
        fs::read_to_string(&target)?
    } else {
        String::new()
    };
    let current_lines = split_diff_lines(Some(&current));
    let Some(merged_lines) =
        merge_patch_hunks_into_current_lines(&current_lines, &patch_file.hunks)
    else {
        return Ok(false);
    };

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, join_diff_lines(&merged_lines))?;
    git_with_paths(
        worktree,
        &["add", "--"],
        &[patch_file.path.to_string_lossy().into_owned()],
    )?;
    Ok(true)
}

fn is_lightweight_task_doc_path(path: &Path) -> bool {
    let normalized = normalize_scope(&path.to_string_lossy());
    normalized.starts_with("docs/") && normalized.ends_with(".md") && normalized.contains("task")
}

fn merge_patch_hunks_into_current_lines(
    current_lines: &[String],
    hunks: &[PatchHunk],
) -> Option<Vec<String>> {
    let mut merged = current_lines.to_vec();
    let mut line_offset: isize = 0;

    for hunk in hunks {
        if hunk.preimage_lines.is_empty() && hunk.postimage_lines.is_empty() {
            continue;
        }

        if hunk.preimage_lines.is_empty() {
            if find_subsequence(&merged, &hunk.postimage_lines).is_some() {
                continue;
            }
            let insert_at =
                translated_insertion_index(hunk.preimage_start, line_offset, merged.len());
            merged.splice(insert_at..insert_at, hunk.postimage_lines.clone());
            line_offset += hunk.postimage_lines.len() as isize;
            continue;
        }

        let expected = translated_line_index(hunk.preimage_start, line_offset, merged.len());
        let replacement_range = find_hunk_match_range(&merged, &hunk.preimage_lines, expected)
            .or_else(|| {
                find_subsequence(&merged, &hunk.postimage_lines)
                    .map(|start| start..start + hunk.postimage_lines.len())
            })?;

        if merged[replacement_range.clone()] == hunk.postimage_lines {
            continue;
        }

        let replaced_len = replacement_range
            .end
            .saturating_sub(replacement_range.start);
        merged.splice(replacement_range, hunk.postimage_lines.clone());
        line_offset += hunk.postimage_lines.len() as isize - replaced_len as isize;
    }

    Some(merged)
}

fn translated_line_index(start_1_based: usize, line_offset: isize, len: usize) -> usize {
    let zero_based = start_1_based.saturating_sub(1) as isize + line_offset;
    zero_based.clamp(0, len as isize) as usize
}

fn translated_insertion_index(start_1_based: usize, line_offset: isize, len: usize) -> usize {
    let zero_based = if start_1_based == 0 {
        line_offset
    } else {
        start_1_based as isize + line_offset
    };
    zero_based.clamp(0, len as isize) as usize
}

fn find_hunk_match_range(
    lines: &[String],
    needle: &[String],
    expected_start: usize,
) -> Option<std::ops::Range<usize>> {
    if needle.is_empty() {
        return Some(expected_start..expected_start);
    }
    if lines.len() < needle.len() {
        return None;
    }
    if expected_start + needle.len() <= lines.len()
        && lines[expected_start..expected_start + needle.len()] == *needle
    {
        return Some(expected_start..expected_start + needle.len());
    }
    let start = find_subsequence_near(lines, needle, expected_start)?;
    Some(start..start + needle.len())
}

fn find_subsequence_near(
    lines: &[String],
    needle: &[String],
    expected_start: usize,
) -> Option<usize> {
    let matches = find_all_subsequence_starts(lines, needle);
    matches
        .into_iter()
        .min_by_key(|start| start.abs_diff(expected_start))
}

fn find_subsequence(lines: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    find_all_subsequence_starts(lines, needle)
        .into_iter()
        .next()
}

fn find_all_subsequence_starts(lines: &[String], needle: &[String]) -> Vec<usize> {
    if needle.is_empty() || lines.len() < needle.len() {
        return Vec::new();
    }
    let mut starts = Vec::new();
    for start in 0..=lines.len() - needle.len() {
        if lines[start..start + needle.len()] == *needle {
            starts.push(start);
        }
    }
    starts
}

fn git_remote_branch_oid(root: &Path, remote: &str, branch: &str) -> Result<Option<String>> {
    let refspec = format!("refs/heads/{branch}");
    let output = run_git_output(Some(root), &["ls-remote", remote, refspec.as_str()])
        .with_context(|| {
            format!(
                "failed to inspect git remote {remote} in {}",
                root.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-remote {} {} failed in {}: {}",
            remote,
            refspec,
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .map(str::to_string))
}

pub(super) fn git_head_exists(root: &Path) -> Result<bool> {
    let output = run_git_output(Some(root), &["rev-parse", "--verify", "HEAD"])
        .with_context(|| format!("failed to inspect git HEAD in {}", root.display()))?;
    Ok(output.status.success())
}

fn git_optional_stdout(root: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = run_git_output(Some(root), args)
        .with_context(|| format!("failed to run git {} in {}", args.join(" "), root.display()))?;
    if output.status.success() {
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    } else {
        Ok(None)
    }
}

fn git_path_is_tracked(root: &Path, rel_path: &str) -> Result<bool> {
    let output = run_git_output(Some(root), &["ls-files", "--error-unmatch", "--", rel_path])
        .with_context(|| {
            format!(
                "failed to inspect tracked git path {rel_path} in {}",
                root.display()
            )
        })?;
    Ok(output.status.success())
}

fn git_path_is_ignored(root: &Path, rel_path: &str) -> Result<bool> {
    let output = run_git_output(
        Some(root),
        &["check-ignore", "-q", "--no-index", "--", rel_path],
    )
    .with_context(|| {
        format!(
            "failed to inspect ignored git path {rel_path} in {}",
            root.display()
        )
    })?;
    Ok(output.status.success())
}

fn lightweight_commit_message_for_task(task: &CoordinationTask) -> Result<String> {
    let message = task
        .title
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if message.is_empty() {
        anyhow::bail!(
            "lightweight generated write task is missing a title for automatic git commit"
        );
    }
    Ok(message)
}

fn push_github_branch_fix_recovery_head(
    task: &CoordinationTask,
    workspace_root: &Path,
    target_ref: &str,
) -> Result<Option<String>> {
    if !github_branch_fix_task_has_publish_recovery(task) {
        return Ok(None);
    }
    let commit_hash = git_stdout(workspace_root, &["rev-parse", "HEAD"])?;
    git_push_lightweight_remotes(workspace_root, target_ref)?;
    Ok(Some(commit_hash))
}

fn commit_all_changes(worktree: &Path, commit_message: &str) -> Result<String> {
    git_stdout(worktree, &["commit", "-m", commit_message])?;
    git_stdout(worktree, &["rev-parse", "HEAD"])
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String> {
    Ok(git_stdout_raw(root, args)?.trim().to_string())
}

fn git_stdout_raw(root: &Path, args: &[&str]) -> Result<String> {
    let output = run_git_output(Some(root), args)
        .with_context(|| format!("failed to run git {} in {}", args.join(" "), root.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_push_lightweight_remotes(root: &Path, target_ref: &str) -> Result<()> {
    let origin_ref = format!("HEAD:refs/heads/{target_ref}");
    git_stdout(root, &["push", "-u", "origin", origin_ref.as_str()])?;
    if git_remote_exists(root, "github")? {
        let github_ref = format!("HEAD:refs/heads/{target_ref}");
        git_stdout(root, &["push", "-u", "github", github_ref.as_str()])?;
    }
    Ok(())
}

fn git_remote_exists(root: &Path, remote: &str) -> Result<bool> {
    let output = run_git_output(Some(root), &["remote", "get-url", remote]).with_context(|| {
        format!(
            "failed to inspect git remote {} in {}",
            remote,
            root.display()
        )
    })?;
    if output.status.success() {
        Ok(true)
    } else if output.status.code().is_some() {
        Ok(false)
    } else {
        anyhow::bail!(
            "git remote get-url {} failed in {}: {}",
            remote,
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn git_with_paths(root: &Path, fixed_args: &[&str], paths: &[String]) -> Result<()> {
    let _ = git_stdout_with_paths(root, fixed_args, paths)?;
    Ok(())
}

fn git_stdout_with_paths(root: &Path, fixed_args: &[&str], paths: &[String]) -> Result<String> {
    let output = run_git_output_with_paths(root, fixed_args, paths).with_context(|| {
        format!(
            "failed to run git {} in {}",
            fixed_args.join(" "),
            root.display()
        )
    })?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed in {}: {}",
            fixed_args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_paths_have_staged_changes(root: &Path, paths: &[String]) -> Result<bool> {
    let output = run_git_output_with_paths(root, &["diff", "--cached", "--quiet", "--"], paths)
        .with_context(|| format!("failed to check staged git diff in {}", root.display()))?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => anyhow::bail!("git diff --cached --quiet failed in {}", root.display()),
    }
}

fn git_has_staged_changes(root: &Path) -> Result<bool> {
    let output = run_git_output(Some(root), &["diff", "--cached", "--quiet"])
        .with_context(|| format!("failed to check staged git diff in {}", root.display()))?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => anyhow::bail!("git diff --cached --quiet failed in {}", root.display()),
    }
}

fn run_git_output(current_dir: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=10");
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    run_command_with_timeout(&mut command, GIT_COMMAND_TIMEOUT).with_context(|| match current_dir {
        Some(dir) => format!("failed to run git {} in {}", args.join(" "), dir.display()),
        None => format!("failed to run git {}", args.join(" ")),
    })
}

fn run_git_output_with_paths(root: &Path, fixed_args: &[&str], paths: &[String]) -> Result<Output> {
    let mut command = Command::new("git");
    command
        .args(fixed_args)
        .args(paths)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=10");
    run_command_with_timeout(&mut command, GIT_COMMAND_TIMEOUT).with_context(|| {
        format!(
            "failed to run git {} in {}",
            fixed_args.join(" "),
            root.display()
        )
    })
}

fn run_command_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output> {
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    match child.wait_timeout(timeout)? {
        Some(_) => child
            .wait_with_output()
            .context("failed to collect command output"),
        None => {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            anyhow::bail!("command timed out after {}s", timeout.as_secs());
        }
    }
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(250));
            if child.try_wait().ok().flatten().is_none() {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }

    let _ = child.kill();
}

fn ensure_publish_git_identity(worktree: &Path) -> Result<()> {
    let has_name = git_optional_stdout(worktree, &["config", "--get", "user.name"])?
        .is_some_and(|value| !value.trim().is_empty());
    let has_email = git_optional_stdout(worktree, &["config", "--get", "user.email"])?
        .is_some_and(|value| !value.trim().is_empty());
    if has_name && has_email {
        return Ok(());
    }

    let user_name = ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME", "USER", "USERNAME"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "local-user".to_string());
    let user_email = ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL", "EMAIL"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let host = std::env::var("HOSTNAME")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "local".to_string());
            format!("{user_name}@{host}.local")
        });

    git_stdout(worktree, &["config", "user.name", user_name.as_str()])?;
    git_stdout(worktree, &["config", "user.email", user_email.as_str()])?;
    Ok(())
}

fn maybe_sync_workspace_to_published_head(
    task: &CoordinationTask,
    workspace_root: &Path,
    target_ref: &str,
    materialized_files: &[MaterializedArtifactFile],
    published_commit: &str,
) -> Result<()> {
    let fetch_result = git_stdout(
        workspace_root,
        &["fetch", "origin", &format!("refs/heads/{target_ref}")],
    );
    if let Err(err) = fetch_result {
        let message = err.to_string();
        if is_retryable_publish_race(&message) || message.contains("index.lock") {
            return Ok(());
        }
        return Err(err);
    }
    // Bind bookkeeping and branch advancement to this operation's successful
    // commit, even if an external writer advanced origin before the fetch.
    let published = published_commit.to_string();
    // Remember only successful local publishes. A newer external remote head
    // must still go through ordinary conflict detection.
    for file in materialized_files {
        let output = run_git_output(
            Some(workspace_root),
            &["show", &format!("{}:{}", published.trim(), file.path)],
        )?;
        // A merged remote file may contain changes the local worker never saw.
        // Only a byte-exact known local postimage authorizes a later override.
        if output.status.success() && output.stdout == file.content.as_bytes() {
            let reference = local_publish_file_ref(workspace_root, target_ref, &file.path)?;
            git_stdout(
                workspace_root,
                &["update-ref", &reference, published.trim()],
            )?;
        }
    }
    if load_run(&task.run_id)?.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        let branch = git_optional_stdout(workspace_root, &["symbolic-ref", "-q", "HEAD"])?;
        if branch.as_deref().map(str::trim) == Some(format!("refs/heads/{target_ref}").as_str()) {
            if let Some(current) =
                git_optional_stdout(workspace_root, &["rev-parse", "--verify", "HEAD"])?
            {
                let ancestor = run_git_output(
                    Some(workspace_root),
                    &[
                        "merge-base",
                        "--is-ancestor",
                        current.trim(),
                        published.trim(),
                    ],
                )?;
                if ancestor.status.success() {
                    // Compare-and-swap the owned branch, preserving both index
                    // contents and every working file, including unrelated edits.
                    git_stdout(
                        workspace_root,
                        &[
                            "update-ref",
                            branch.as_deref().unwrap().trim(),
                            published.trim(),
                            current.trim(),
                        ],
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn mirror_workspace_git_hooks(workspace_root: &Path, worktree: &Path) -> Result<()> {
    let source = workspace_root.join(".git/hooks");
    if !source.is_dir() {
        return Ok(());
    }
    let destination = worktree.join(".git/hooks");
    fs::create_dir_all(&destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        if !entry_type.is_file() {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        fs::copy(&source_path, &destination_path)?;
        let permissions = fs::metadata(&source_path)?.permissions();
        fs::set_permissions(destination_path, permissions)?;
    }
    Ok(())
}

fn push_commit_to_remote(
    origin_remote_url: &str,
    remote_url: &str,
    commit_hash: &str,
    target_ref: &str,
) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!(
        "auto-runs-publish-mirror-{}-{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    let output = run_git_output(None, &["clone", origin_remote_url, tmp.to_str().unwrap()])?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone for remote mirror failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let result = (|| -> Result<()> {
        git_stdout(&tmp, &["remote", "add", "github", remote_url])?;
        git_stdout(
            &tmp,
            &[
                "push",
                "github",
                &format!("{commit_hash}:refs/heads/{target_ref}"),
            ],
        )?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_run_git(workspace: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(workspace)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed in {}: {}",
            args.join(" "),
            workspace.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_git_repo_with_remote_for_test(workspace: &Path) -> PathBuf {
        std::fs::create_dir_all(workspace).unwrap();
        test_run_git(workspace, &["init", "-b", "main"]);
        test_run_git(workspace, &["config", "user.name", "Test User"]);
        test_run_git(workspace, &["config", "user.email", "test@example.com"]);

        let remote_root = std::env::temp_dir().join(format!(
            "auto-runs-lightweight-publish-remote-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&remote_root).unwrap();
        let remote = remote_root.join("origin.git");
        test_run_git(&remote_root, &["init", "--bare", remote.to_str().unwrap()]);
        test_run_git(
            workspace,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        remote
    }

    #[test]
    fn compute_diff_hunks_splits_non_overlapping_edits() {
        let preimage = vec![
            "- [x] scaffold".to_string(),
            "- [ ] logic".to_string(),
            "- [ ] styling".to_string(),
            "- [ ] verify".to_string(),
            "storage_key = todo-app.items".to_string(),
        ];
        let postimage = vec![
            "- [x] scaffold".to_string(),
            "- [x] logic".to_string(),
            "- [ ] styling".to_string(),
            "- [x] verify".to_string(),
            "storage_key = todo-app-todos".to_string(),
        ];

        let hunks = compute_diff_hunks(&preimage, &postimage);
        assert_eq!(hunks.len(), 2);

        assert_eq!(hunks[0].preimage_start, 2);
        assert_eq!(hunks[0].postimage_start, 2);
        assert_eq!(hunks[0].preimage_lines, vec!["- [ ] logic".to_string()]);
        assert_eq!(hunks[0].postimage_lines, vec!["- [x] logic".to_string()]);

        assert_eq!(hunks[1].preimage_start, 4);
        assert_eq!(hunks[1].postimage_start, 4);
        assert_eq!(
            hunks[1].preimage_lines,
            vec![
                "- [ ] verify".to_string(),
                "storage_key = todo-app.items".to_string()
            ]
        );
        assert_eq!(
            hunks[1].postimage_lines,
            vec![
                "- [x] verify".to_string(),
                "storage_key = todo-app-todos".to_string()
            ]
        );
    }

    #[test]
    fn parse_hunk_header_accepts_trailing_section_text() {
        assert_eq!(
            parse_hunk_header(
                "@@ -8,2 +8,2 @@ Status legend: `pending`, `in_progress`, `complete`"
            ),
            Some((8, 8))
        );
    }

    #[test]
    fn merge_patch_hunks_into_current_lines_keeps_parallel_task_doc_updates() {
        let base = vec![
            "# Tasks".to_string(),
            String::new(),
            "| Task | Status | Notes |".to_string(),
            "|------|--------|-------|".to_string(),
            "| Style responsive todo app interface | ⬜ PENDING | — |".to_string(),
            "| Implement todo interactions with localStorage | ⬜ PENDING | — |".to_string(),
            String::new(),
            "## Notes".to_string(),
            String::new(),
            "- Created `index.html`".to_string(),
        ];
        let remote = vec![
            "# Tasks".to_string(),
            String::new(),
            "| Task | Status | Notes |".to_string(),
            "|------|--------|-------|".to_string(),
            "| Style responsive todo app interface | ✅ COMPLETE | styles.css created |"
                .to_string(),
            "| Implement todo interactions with localStorage | ⬜ PENDING | — |".to_string(),
            String::new(),
            "## Notes".to_string(),
            String::new(),
            "- Created `index.html`".to_string(),
            "- styles.css adds responsive layout".to_string(),
        ];
        let worker_postimage = vec![
            "# Tasks".to_string(),
            String::new(),
            "| Task | Status | Notes |".to_string(),
            "|------|--------|-------|".to_string(),
            "| Style responsive todo app interface | ⬜ PENDING | — |".to_string(),
            "| Implement todo interactions with localStorage | ✅ COMPLETE | app.js implements add/toggle/delete |".to_string(),
            String::new(),
            "## Notes".to_string(),
            String::new(),
            "- Created `index.html`".to_string(),
            "- app.js adds localStorage-backed behavior".to_string(),
        ];

        let hunks = compute_diff_hunks(&base, &worker_postimage);
        let merged = merge_patch_hunks_into_current_lines(&remote, &hunks).unwrap();

        assert_eq!(
            merged,
            vec![
                "# Tasks".to_string(),
                String::new(),
                "| Task | Status | Notes |".to_string(),
                "|------|--------|-------|".to_string(),
                "| Style responsive todo app interface | ✅ COMPLETE | styles.css created |".to_string(),
                "| Implement todo interactions with localStorage | ✅ COMPLETE | app.js implements add/toggle/delete |".to_string(),
                String::new(),
                "## Notes".to_string(),
                String::new(),
                "- Created `index.html`".to_string(),
                "- app.js adds localStorage-backed behavior".to_string(),
                "- styles.css adds responsive layout".to_string(),
            ]
        );
    }

    #[test]
    fn custom_docs_task_markdown_paths_use_lightweight_task_doc_merge() {
        assert!(is_lightweight_task_doc_path(Path::new("docs/task-lisk.md")));
        assert!(is_lightweight_task_doc_path(Path::new("docs/tasks.md")));
        assert!(!is_lightweight_task_doc_path(Path::new("docs/design.md")));
        assert!(!is_lightweight_task_doc_path(Path::new("src/tasks.rs")));
    }

    #[test]
    fn merge_patch_hunks_into_current_lines_keeps_parallel_custom_task_doc_updates() {
        let base = vec![
            "# Rustelixir Tasks".to_string(),
            "- [ ] Create manifest".to_string(),
            "- [ ] Add hooks".to_string(),
        ];
        let remote = vec![
            "# Rustelixir Tasks".to_string(),
            "- [x] Create manifest".to_string(),
            "- [ ] Add hooks".to_string(),
            "- scaffold copied".to_string(),
        ];
        let worker_postimage = vec![
            "# Rustelixir Tasks".to_string(),
            "- [ ] Create manifest".to_string(),
            "- [x] Add hooks".to_string(),
            "- hooks copied".to_string(),
        ];

        let hunks = compute_diff_hunks(&base, &worker_postimage);
        let merged = merge_patch_hunks_into_current_lines(&remote, &hunks).unwrap();

        assert_eq!(
            merged,
            vec![
                "# Rustelixir Tasks".to_string(),
                "- [x] Create manifest".to_string(),
                "- [x] Add hooks".to_string(),
                "- hooks copied".to_string(),
                "- scaffold copied".to_string(),
            ]
        );
    }

    #[test]
    fn initial_publish_uses_temp_clone_instead_of_live_workspace_hooks() {
        let workspace = std::env::temp_dir().join(format!(
            "auto-runs-lightweight-initial-publish-workspace-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let remote_path = init_git_repo_with_remote_for_test(&workspace);
        test_run_git(&workspace, &["checkout", "--orphan", "auto-runs-1"]);
        std::fs::create_dir_all(workspace.join(".githooks")).unwrap();
        std::fs::write(
            workspace.join(".githooks/pre-commit"),
            "#!/usr/bin/env bash\nset -euo pipefail\nrepo_root=\"$(git rev-parse --show-toplevel)\"\nif [ -e \"$repo_root/FAIL_HOOK\" ]; then\n  echo \"workspace hook should not run in live repo\" >&2\n  exit 1\nfi\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(workspace.join(".githooks/pre-commit"))
                .unwrap()
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(workspace.join(".githooks/pre-commit"), permissions).unwrap();
        }
        test_run_git(&workspace, &["config", "core.hooksPath", ".githooks"]);
        std::fs::write(
            workspace.join("FAIL_HOOK"),
            "present only in live workspace\n",
        )
        .unwrap();

        let task = CoordinationTask {
            kind: CoordinationTaskKind::GeneratedWriteTask,
            title: "Convert scaffolded Rust workspace to current repo".to_string(),
            cwd: workspace.display().to_string(),
            ..CoordinationTask::default()
        };
        let materialized_files = vec![MaterializedArtifactFile {
            path: "README.md".to_string(),
            content: "# Rustelixir\n".to_string(),
        }];

        let commit = publish_initial_lightweight_state(
            &task,
            &workspace,
            &materialized_files,
            "auto-runs-1",
            remote_path.to_str().unwrap(),
            None,
        )
        .unwrap()
        .unwrap();

        assert!(!commit.trim().is_empty());
        let remote_head = test_run_git(
            &workspace,
            &["ls-remote", "origin", "refs/heads/auto-runs-1"],
        );
        assert!(remote_head.contains(commit.trim()));
    }

    #[test]
    fn contextual_patch_text_applies_for_sequential_same_file_edit() {
        let workspace = std::env::temp_dir().join(format!(
            "auto-runs-lightweight-publish-workspace-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let remote_path = init_git_repo_with_remote_for_test(&workspace);
        std::fs::write(
            workspace.join("app.js"),
            r#"(function () {
  'use strict';

  let todos = [];
  let filter = 'all';

  function addTodo(text) {
    todos = [{ text, completed: false }, ...todos];
    saveTodos(todos);
    render();
  }

  function toggleTodo(index) {
    todos = todos.map((todo, i) =>
      i === index ? { ...todo, completed: !todo.completed } : todo
    );
    saveTodos(todos);
    render();
  }

  function deleteTodo(index) {
    todos = todos.filter((_, i) => i !== index);
    saveTodos(todos);
    render();
  }

  function setFilter(value) {
    filter = value;
    saveFilter(filter);
    render();
  }

  function saveTodos() {}
  function saveFilter() {}
  function loadTodos() { return []; }
  function loadFilter() { return 'all'; }
  function render() {}

  function init() {
    todos = loadTodos();
    filter = loadFilter();
    render();
  }

  window.TodoApp = { addTodo, toggleTodo, deleteTodo, setFilter };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
"#,
        )
        .unwrap();
        test_run_git(&workspace, &["add", "app.js"]);
        test_run_git(
            &workspace,
            &["commit", "-m", "Implement todo state and rendering logic"],
        );
        test_run_git(
            &workspace,
            &["push", "-u", "origin", "HEAD:refs/heads/main"],
        );

        let materialized_files = vec![MaterializedArtifactFile {
            path: "app.js".to_string(),
            content: r#"(function () {
  'use strict';

  let todos = [];
  let filter = 'all';

  function addTodo(text) {
    todos = [{ text, completed: false }, ...todos];
    saveTodos(todos);
    render();
  }

  function toggleTodo(index) {
    todos = todos.map((todo, i) =>
      i === index ? { ...todo, completed: !todo.completed } : todo
    );
    saveTodos(todos);
    render();
  }

  function deleteTodo(index) {
    todos = todos.filter((_, i) => i !== index);
    saveTodos(todos);
    render();
  }

  function setFilter(value) {
    filter = value;
    saveFilter(filter);
    render();
  }

  function saveTodos() {}
  function saveFilter() {}
  function loadTodos() { return []; }
  function loadFilter() { return 'all'; }
  function render() {}

  function bindEvents() {
    document.getElementById('todo-form')?.addEventListener('submit', function (event) {
      event.preventDefault();
      const input = document.getElementById('todo-input');
      const value = input && 'value' in input ? input.value : '';
      addTodo(value);
      if (input && 'value' in input) {
        input.value = '';
      }
    });

    document.getElementById('todo-list')?.addEventListener('change', function (event) {
      const target = event.target;
      if (!(target instanceof HTMLInputElement) || !target.classList.contains('todo-checkbox')) {
        return;
      }
      const item = target.closest('[data-index]');
      if (!item) {
        return;
      }
      toggleTodo(Number(item.getAttribute('data-index')));
    });
  }

  function init() {
    todos = loadTodos();
    filter = loadFilter();
    render();
    bindEvents();
  }

  window.TodoApp = { addTodo, toggleTodo, deleteTodo, setFilter, init };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
"#
            .to_string(),
        }];

        let patch_series =
            build_patch_series_from_materialized_files(&workspace, &materialized_files).unwrap();
        let patch_file = patch_series
            .patches
            .iter()
            .find(|patch| patch.path == PathBuf::from("app.js"))
            .unwrap();

        let worktree = checkout_temp_worktree(
            &test_run_git(&remote_path, &["rev-parse", "refs/heads/main"]),
            remote_path.to_str().unwrap(),
        )
        .unwrap();
        let apply = git_apply_three_way(&worktree, &patch_file.apply_patch_text).unwrap();

        assert_eq!(apply, ApplyPatchResult::Clean);
        let applied = std::fs::read_to_string(worktree.join("app.js")).unwrap();
        assert!(applied.contains("function bindEvents()"));
        assert!(applied.contains("bindEvents();"));

        let _ = std::fs::remove_dir_all(worktree);
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(remote_path.parent().unwrap());
    }

    #[test]
    fn retryable_publish_race_recognizes_ref_lock_failures() {
        assert!(is_retryable_publish_race(
            "cannot lock ref 'refs/heads/main': is at abc but expected def"
        ));
        assert!(is_retryable_publish_race(
            "cannot lock ref 'refs/remotes/origin/main': is at abc but expected def"
        ));
        assert!(is_retryable_publish_race(
            "remote rejected] HEAD -> main (reference already exists)"
        ));
    }

    #[test]
    fn volatile_lightweight_publish_paths_skip_status_docs() {
        assert!(is_volatile_lightweight_publish_path(
            "docs/design-pass-5-status.md"
        ));
        assert!(!is_volatile_lightweight_publish_path(
            "docs/design-pass-5.md"
        ));
        assert!(!is_volatile_lightweight_publish_path("src/lib.rs"));
    }
}
