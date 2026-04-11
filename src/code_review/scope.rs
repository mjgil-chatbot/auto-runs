use std::io::Write;

use super::*;

pub(super) fn resolve_scope_files(
    root: &Path,
    scope: &ReviewScope,
) -> Result<(Vec<String>, Vec<String>)> {
    match scope {
        ReviewScope::Worktree => Ok((
            filter_gitignored_paths(root, collect_reviewable_files(root, root)?)?,
            Vec::new(),
        )),
        ReviewScope::Paths(paths) => {
            let mut collected = BTreeSet::new();
            let mut warnings = Vec::new();
            for path in paths {
                let candidate = root.join(path);
                if !candidate.exists() {
                    warnings.push(format!("Scoped path `{}` does not exist.", path));
                    continue;
                }
                if candidate.is_dir() {
                    for item in
                        filter_gitignored_paths(root, collect_reviewable_files(root, &candidate)?)?
                    {
                        collected.insert(item);
                    }
                } else if is_reviewable_path(&candidate) {
                    let relative = candidate
                        .strip_prefix(root)
                        .unwrap_or(candidate.as_path())
                        .to_string_lossy()
                        .into_owned();
                    collected.insert(relative);
                }
            }
            Ok((collected.into_iter().collect(), warnings))
        }
        ReviewScope::Globs(globs) => {
            let all_files = filter_gitignored_paths(root, collect_reviewable_files(root, root)?)?;
            let mut collected = BTreeSet::new();
            let mut warnings = Vec::new();

            for pattern in globs {
                let normalized = normalize_glob_pattern(pattern);
                if normalized.is_empty() {
                    continue;
                }

                let mut matched = false;
                for file in &all_files {
                    if glob_matches_path(&normalized, file) {
                        collected.insert(file.clone());
                        matched = true;
                    }
                }
                if !matched {
                    warnings.push(format!(
                        "Scoped glob `{}` matched no reviewable files.",
                        pattern
                    ));
                }
            }

            Ok((collected.into_iter().collect(), warnings))
        }
        ReviewScope::Diff { base } => {
            let output = Command::new("git")
                .args(["diff", "--name-only", base, "--"])
                .current_dir(root)
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    let files = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(str::to_string)
                        .filter(|file| is_reviewable_path(&root.join(file)))
                        .collect();
                    Ok((files, Vec::new()))
                }
                Ok(output) => Ok((
                    Vec::new(),
                    vec![format!(
                        "Failed to resolve diff scope from `{}`: {}",
                        base,
                        String::from_utf8_lossy(&output.stderr).trim()
                    )],
                )),
                Err(err) => Ok((
                    Vec::new(),
                    vec![format!(
                        "Failed to run `git diff --name-only {}`: {}",
                        base, err
                    )],
                )),
            }
        }
    }
}

fn normalize_glob_pattern(pattern: &str) -> String {
    pattern.trim().replace('\\', "/")
}

fn glob_matches_path(pattern: &str, path: &str) -> bool {
    let pattern_segments = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let path_segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    glob_segments_match(&pattern_segments, &path_segments)
}

fn glob_segments_match(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }

    if pattern[0] == "**" {
        return (0..=path.len()).any(|skip| glob_segments_match(&pattern[1..], &path[skip..]));
    }

    if path.is_empty() || !glob_component_matches(pattern[0], path[0]) {
        return false;
    }

    glob_segments_match(&pattern[1..], &path[1..])
}

fn glob_component_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut pattern_idx, mut text_idx) = (0usize, 0usize);
    let (mut star_idx, mut star_text_idx) = (None, 0usize);

    while text_idx < text.len() {
        if pattern_idx < pattern.len()
            && (pattern[pattern_idx] == b'?' || pattern[pattern_idx] == text[text_idx])
        {
            pattern_idx += 1;
            text_idx += 1;
        } else if pattern_idx < pattern.len() && pattern[pattern_idx] == b'*' {
            star_idx = Some(pattern_idx);
            pattern_idx += 1;
            star_text_idx = text_idx;
        } else if let Some(star) = star_idx {
            pattern_idx = star + 1;
            star_text_idx += 1;
            text_idx = star_text_idx;
        } else {
            return false;
        }
    }

    while pattern_idx < pattern.len() && pattern[pattern_idx] == b'*' {
        pattern_idx += 1;
    }

    pattern_idx == pattern.len()
}

fn collect_reviewable_files(root: &Path, current: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in
        fs::read_dir(current).with_context(|| format!("failed to read {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            files.extend(collect_reviewable_files(root, &path)?);
        } else if file_type.is_file() && is_reviewable_path(&path) {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .into_owned();
            files.push(relative);
        }
    }
    Ok(files)
}

fn filter_gitignored_paths(root: &Path, files: Vec<String>) -> Result<Vec<String>> {
    if files.is_empty() {
        return Ok(files);
    }

    let mut child = match Command::new("git")
        .args(["check-ignore", "--stdin"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Ok(files),
    };

    {
        let Some(mut stdin) = child.stdin.take() else {
            return Ok(files);
        };
        for file in &files {
            writeln!(stdin, "{file}")?;
        }
    }

    let output = child.wait_with_output()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Ok(files);
    }

    let ignored_output = String::from_utf8_lossy(&output.stdout);
    let ignored = ignored_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();

    if ignored.is_empty() {
        return Ok(files);
    }

    Ok(files
        .into_iter()
        .filter(|file| !ignored.contains(file.as_str()))
        .collect())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "node_modules" | "dist" | ".claude-desktop" | ".rag_model_cache")
    )
}

#[cfg(test)]
pub(super) fn glob_matches_path_test(pattern: &str, path: &str) -> bool {
    glob_matches_path(pattern, path)
}

#[cfg(test)]
pub(super) fn should_skip_dir_test(path: &Path) -> bool {
    should_skip_dir(path)
}

#[cfg(test)]
pub(super) fn is_reviewable_path_test(path: &Path) -> bool {
    is_reviewable_path(path)
}

fn is_reviewable_path(path: &Path) -> bool {
    // Filter out deleted files - only include paths that exist on disk
    path.exists()
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some(
                "rs" | "toml" | "md" | "json" | "yaml" | "yml" | "sh" | "ts" | "tsx" | "js" | "jsx"
            )
        )
}
