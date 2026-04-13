use super::*;

pub fn file_contains_stub_markers(root: &Path, changed_file: &str) -> Result<bool> {
    let candidate = if Path::new(changed_file).is_absolute() {
        PathBuf::from(changed_file)
    } else {
        root.join(changed_file)
    };
    if !candidate.is_file() {
        return Ok(false);
    }
    let Ok(contents) = fs::read_to_string(&candidate) else {
        return Ok(false);
    };

    let content_to_scan = if candidate.extension().and_then(|ext| ext.to_str()) == Some("rs") {
        strip_rust_string_literals(&contents)
    } else {
        contents
    };
    Ok(content_contains_stub_markers(&content_to_scan))
}

fn content_contains_stub_markers(contents: &str) -> bool {
    let lowered = contents.to_ascii_lowercase();
    let markers = [
        ["not yet", "implemented"].join(" "),
        ["to", "do!"].concat(),
        ["unimplemented", "!"].concat(),
        ["fix", "me"].concat(),
        ["place", "holder"].concat(),
    ];
    markers.iter().any(|marker| lowered.contains(marker))
}

fn strip_rust_string_literals(contents: &str) -> String {
    let bytes = contents.as_bytes();
    let mut out = String::with_capacity(contents.len());
    let mut i = 0usize;

    while i < bytes.len() {
        if let Some(skip_len) = rust_string_literal_len(bytes, i) {
            out.extend(std::iter::repeat_n(' ', skip_len));
            i += skip_len;
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

fn rust_string_literal_len(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes[start] == b'"' {
        return Some(quoted_string_len(bytes, start));
    }

    if bytes[start] == b'b' && start + 1 < bytes.len() && bytes[start + 1] == b'"' {
        return Some(quoted_string_len(bytes, start + 1) + 1);
    }

    if bytes[start] == b'r' {
        return raw_string_len(bytes, start, 0);
    }

    if bytes[start] == b'b' && start + 1 < bytes.len() && bytes[start + 1] == b'r' {
        return raw_string_prefix_len(bytes, start + 1).map(|len| len + 1);
    }

    None
}

fn quoted_string_len(bytes: &[u8], quote_idx: usize) -> usize {
    let mut i = quote_idx + 1;
    let mut escaped = false;
    while i < bytes.len() {
        let current = bytes[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if current == b'\\' {
            escaped = true;
            i += 1;
            continue;
        }
        if current == b'"' {
            return i + 1 - quote_idx;
        }
        i += 1;
    }
    bytes.len() - quote_idx
}

fn raw_string_prefix_len(bytes: &[u8], start: usize) -> Option<usize> {
    for hashes in 0..=255usize {
        let quote_idx = start + 1 + hashes;
        if quote_idx >= bytes.len() {
            return None;
        }
        if bytes[start + 1..quote_idx].iter().all(|b| *b == b'#') && bytes[quote_idx] == b'"' {
            return raw_string_len(bytes, start, hashes);
        }
        if bytes[quote_idx] != b'#' {
            return None;
        }
    }
    None
}

fn raw_string_len(bytes: &[u8], start: usize, hashes: usize) -> Option<usize> {
    let quote_idx = start + 1 + hashes;
    if quote_idx >= bytes.len() || bytes[quote_idx] != b'"' {
        return None;
    }

    let mut i = quote_idx + 1;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let end_hashes_start = i + 1;
        let end_hashes_end = end_hashes_start + hashes;
        if end_hashes_end <= bytes.len()
            && bytes[end_hashes_start..end_hashes_end]
                .iter()
                .all(|b| *b == b'#')
        {
            return Some(end_hashes_end - start);
        }
        i += 1;
    }
    Some(bytes.len() - start)
}
