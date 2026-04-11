use std::collections::HashMap;
use super::*;
/// Builds review chunks and returns them alongside a content cache keyed by relative file path.
/// The cache can be passed to [`dimensions_for_chunk_cached`] to avoid re-reading files from disk.
pub(super) fn build_chunks_with_cache(
    root: &Path,
    files: &[String],
) -> Result<(Vec<Vec<ReviewChunkTarget>>, HashMap<String, String>)> {
    let mut cache: HashMap<String, String> = HashMap::new();
    let mut segments = Vec::new();
    for file in files {
        let contents = fs::read_to_string(root.join(file))
            .with_context(|| format!("failed to read {}", file))?;
        let line_count = contents.lines().count().max(1);
        if line_count <= MAX_REVIEW_CHUNK_LINES {
            segments.push(ReviewChunkTarget {
                path: file.clone(),
                start_line: 1,
                end_line: line_count,
            });
        } else {
            segments.extend(split_large_file(file, &contents));
        }
        cache.insert(file.clone(), contents);
    }

    let mut chunks = Vec::new();
    let mut current_chunk = Vec::new();
    let mut current_lines = 0usize;
    for segment in segments {
        let segment_lines = segment.end_line - segment.start_line + 1;
        if !current_chunk.is_empty()
            && (current_lines + segment_lines > MAX_REVIEW_CHUNK_LINES
                || current_chunk.len() >= MAX_REVIEW_CHUNK_FILES)
        {
            chunks.push(current_chunk);
            current_chunk = Vec::new();
            current_lines = 0;
        }
        current_lines += segment_lines;
        current_chunk.push(segment);
    }
    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }
    Ok((chunks, cache))
}
fn split_large_file(path: &str, contents: &str) -> Vec<ReviewChunkTarget> {
    let mut impl_boundaries = Vec::new();
    let lines: Vec<&str> = contents.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("impl ") || trimmed.starts_with("impl<") {
            impl_boundaries.push(index + 1);
        }
    }
    if impl_boundaries.is_empty() {
        return split_fixed_width(path, lines.len().max(1));
    }

    let mut ranges = Vec::new();
    let mut start = 1usize;
    for boundary in impl_boundaries.into_iter().skip(1) {
        ranges.push((start, boundary.saturating_sub(1)));
        start = boundary;
    }
    ranges.push((start, lines.len().max(1)));

    let mut chunks = Vec::new();
    let mut current_start = None;
    let mut current_end = 0usize;
    let mut current_len = 0usize;
    for (range_start, range_end) in ranges {
        let len = range_end.saturating_sub(range_start) + 1;
        if len > MAX_REVIEW_CHUNK_LINES {
            if let Some(start_line) = current_start.take() {
                chunks.push(ReviewChunkTarget {
                    path: path.to_string(),
                    start_line,
                    end_line: current_end,
                });
                current_len = 0;
            }
            chunks.extend(split_fixed_width(path, len).into_iter().map(|target| {
                ReviewChunkTarget {
                    start_line: target.start_line + range_start - 1,
                    end_line: target.end_line + range_start - 1,
                    ..target
                }
            }));
            continue;
        }

        if current_len + len > MAX_REVIEW_CHUNK_LINES {
            if let Some(start_line) = current_start.take() {
                chunks.push(ReviewChunkTarget {
                    path: path.to_string(),
                    start_line,
                    end_line: current_end,
                });
            }
            current_start = Some(range_start);
            current_end = range_end;
            current_len = len;
        } else {
            current_start.get_or_insert(range_start);
            current_end = range_end;
            current_len += len;
        }
    }
    if let Some(start_line) = current_start {
        chunks.push(ReviewChunkTarget {
            path: path.to_string(),
            start_line,
            end_line: current_end,
        });
    }
    chunks
}
fn split_fixed_width(path: &str, total_lines: usize) -> Vec<ReviewChunkTarget> {
    let mut chunks = Vec::new();
    let mut start = 1usize;
    while start <= total_lines {
        let end = (start + MAX_REVIEW_CHUNK_LINES - 1).min(total_lines);
        chunks.push(ReviewChunkTarget {
            path: path.to_string(),
            start_line: start,
            end_line: end,
        });
        start = end + 1;
    }
    chunks
}
/// Detects review dimensions using a pre-built content cache (no disk I/O).
/// Use this with the cache returned by [`build_chunks_with_cache`].
pub(super) fn dimensions_for_chunk_cached(
    cache: &HashMap<String, String>,
    chunk: &[ReviewChunkTarget],
    selected_dimensions: Option<&[String]>,
) -> Result<Vec<String>> {
    let mut dims = BTreeSet::new();
    if let Some(selected) = selected_dimensions {
        for dim in selected {
            let trimmed = dim.trim();
            if !trimmed.is_empty() {
                dims.insert(trimmed.to_string());
            }
        }
    }

    let mut combined = String::new();
    for target in chunk {
        let contents = cache
            .get(&target.path)
            .ok_or_else(|| anyhow::anyhow!("content not cached for {}", target.path))?;
        combined.push_str(contents);
        combined.push('\n');
        if target.path.contains("test") || target.path.contains("/tests/") {
            dims.insert("test_coverage".to_string());
        }
        if target.path.ends_with(".md") {
            dims.insert("comment_accuracy".to_string());
        }
    }

    if dims.is_empty() {
        dims.insert("code_quality".to_string());
    }
    if combined.contains("///") || combined.contains("//!") {
        dims.insert("comment_accuracy".to_string());
    }
    if combined.contains("Result<")
        || combined.contains("anyhow")
        || combined.contains("bail!")
        || combined.contains("unwrap(")
    {
        dims.insert("error_handling".to_string());
    }
    if combined.contains("struct ") || combined.contains("enum ") || combined.contains("trait ") {
        dims.insert("type_design".to_string());
    }
    dims.insert("architecture".to_string());

    Ok(dims.into_iter().collect())
}
pub(super) fn build_chunk_title(index: usize, chunk: &[ReviewChunkTarget]) -> String {
    if chunk.len() == 1 {
        let target = &chunk[0];
        format!(
            "Review chunk {}: {}:{}-{}",
            index, target.path, target.start_line, target.end_line
        )
    } else {
        format!("Review chunk {}: {} files", index, chunk.len())
    }
}
pub(super) fn build_chunk_prompt(metadata: &ReviewTaskMetadata) -> String {
    let chunk_lines = metadata
        .chunk
        .iter()
        .map(|target| {
            format!(
                "- {}:{}-{}",
                target.path, target.start_line, target.end_line
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dimensions = metadata
        .dimensions
        .iter()
        .map(|dimension| format!("- `{}`: {}", dimension, dimension_guidance(dimension)))
        .collect::<Vec<_>>()
        .join("\n");
    let checks = if metadata.automated_checks.is_empty() {
        "- No automated checks were recorded.".to_string()
    } else {
        metadata
            .automated_checks
            .iter()
            .map(|check| format!("- {}: {} ({})", check.command, check.status, check.summary))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let profile_status = if metadata.profile_warnings.is_empty() {
        format!("Use `{}` as architecture context.", metadata.profile_path)
    } else {
        format!(
            "Use `{}` as architecture context. Profile warnings:\n{}",
            metadata.profile_path,
            metadata
                .profile_warnings
                .iter()
                .map(|warning| format!("- {}", warning))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let fix_mode = if metadata.fix_mode.clone().unwrap_or_default() == ReviewFixMode::GenerateFixes
    {
        "If a finding requires follow-up code changes, emit small `fix_tasks` scoped to the affected files."
    } else {
        "This is a report-only review chunk. Keep `fix_tasks` empty even when you find issues."
    };
    format!(
        "Review the current workspace files in this code review chunk.\n\nScope: {}\nFiles in scope:\n{}\n\nReview dimensions:\n{}\n\n{}\n\nAutomated checks:\n{}\n\nFocus on concrete bugs, regressions, missing tests, and architecture contract violations in the current workspace state. Every finding must cite the strongest available `file:line` reference. {}",
        metadata.scope_description, chunk_lines, dimensions, profile_status, checks, fix_mode
    )
}
pub(super) fn dimension_guidance(dimension: &str) -> &'static str {
    match dimension.trim().to_ascii_lowercase().as_str() {
        "architecture" => "Check the repo-specific invariants and cross-module contracts from the review profile.",
        "code_quality" | "correctness" => {
            "Look for behavioral regressions, incorrect logic, and mismatches with surrounding code."
        }
        "comment_accuracy" => "Check whether comments and docs match the implemented behavior.",
        "test_coverage" => "Check whether tests cover the risky paths and whether missing tests would hide regressions.",
        "error_handling" => "Check for silent failures, swallowed errors, and missing propagation or recovery paths.",
        "type_design" => "Check whether APIs and types encode the right invariants and ownership boundaries.",
        "simplification" | "maintainability" => "Check for unnecessary complexity and obvious simplifications.",
        "security" => "Check for unsafe assumptions, injection risks, and trust-boundary mistakes.",
        "performance" => "Check for avoidable hot-path work, unnecessary allocations, and wasteful I/O.",
        "concurrency" => "Check for races, ordering hazards, stale-state reads, and coordination bugs.",
        _ => "Apply this dimension directly to the current files and call out concrete evidence.",
    }
}
