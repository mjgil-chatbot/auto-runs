use super::*;

fn push_runsc_mount(
    mounts: &mut Vec<ai_providers::sandbox::RunscMount>,
    mount: ai_providers::sandbox::RunscMount,
) {
    if !mount.host.exists() {
        return;
    }
    if mounts
        .iter()
        .any(|existing| existing.host == mount.host && existing.target == mount.target)
    {
        return;
    }
    mounts.push(mount);
}

fn push_same_path_mount_with_canonical(
    mounts: &mut Vec<ai_providers::sandbox::RunscMount>,
    path: PathBuf,
) {
    push_runsc_mount(
        mounts,
        ai_providers::sandbox::RunscMount::same_path(path.clone()),
    );
    if let Ok(canonical) = path.canonicalize() {
        if canonical != path {
            push_runsc_mount(
                mounts,
                ai_providers::sandbox::RunscMount::same_path(canonical),
            );
        }
    }
}

fn push_container_home_mount(
    mounts: &mut Vec<ai_providers::sandbox::RunscMount>,
    host: PathBuf,
    relative: impl AsRef<Path>,
) {
    push_runsc_mount(
        mounts,
        ai_providers::sandbox::RunscMount::container_home(host, relative),
    );
}

fn push_repo_mcp_mount(mounts: &mut Vec<ai_providers::sandbox::RunscMount>) {
    push_same_path_mount_with_canonical(mounts, PathBuf::from(config::mcp_json_path()));
}

pub(super) fn codex_home_sidecar_dir() -> PathBuf {
    coordination_root_dir()
        .join("runsc-sidecars")
        .join("codex-home")
}

fn seed_sidecar_file_if_missing(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() || !source.exists() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    let permissions = fs::metadata(source)
        .with_context(|| format!("failed to stat {}", source.display()))?
        .permissions();
    fs::set_permissions(destination, permissions)
        .with_context(|| format!("failed to set permissions on {}", destination.display()))?;
    Ok(())
}

fn prepare_codex_home_sidecar_dir_in(sidecar_dir: &Path, codex_toml: &Path) -> Result<PathBuf> {
    fs::create_dir_all(sidecar_dir)
        .with_context(|| format!("failed to create {}", sidecar_dir.display()))?;

    seed_sidecar_file_if_missing(codex_toml, &sidecar_dir.join("config.toml"))?;
    if let Some(codex_dir) = codex_toml.parent() {
        seed_sidecar_file_if_missing(&codex_dir.join("auth.json"), &sidecar_dir.join("auth.json"))?;
    }

    Ok(sidecar_dir.to_path_buf())
}

fn prepare_codex_home_sidecar_dir(codex_toml: &Path) -> Result<PathBuf> {
    prepare_codex_home_sidecar_dir_in(&codex_home_sidecar_dir(), codex_toml)
}

fn push_claude_family_home_sidecars(
    mounts: &mut Vec<ai_providers::sandbox::RunscMount>,
    config_dir: &Path,
    claude_json_path: Option<PathBuf>,
) {
    let claude_json = claude_json_path.unwrap_or_else(|| config_dir.join(".claude.json"));
    push_container_home_mount(mounts, claude_json, ".claude.json");
    push_container_home_mount(
        mounts,
        config_dir.join(".credentials.json"),
        ".claude/.credentials.json",
    );
    push_container_home_mount(
        mounts,
        config_dir.join("settings.json"),
        ".claude/settings.json",
    );
}

fn push_codex_home_sidecars(mounts: &mut Vec<ai_providers::sandbox::RunscMount>) {
    let codex_toml = PathBuf::from(config::codex_toml_path());
    push_same_path_mount_with_canonical(mounts, codex_toml.clone());
    match prepare_codex_home_sidecar_dir(&codex_toml) {
        Ok(sidecar_dir) => push_container_home_mount(mounts, sidecar_dir, ".codex"),
        Err(err) => {
            warn!(
                error = %err,
                codex_toml = %codex_toml.display(),
                "failed to prepare Codex runsc sidecar directory"
            );
        }
    }
}

fn resolved_gemini_config_dir() -> PathBuf {
    config::get_config_dir(ModelProvider::Gemini)
        .map(PathBuf::from)
        .unwrap_or_else(|| config::canonical_shared_root_dir().join(".gemini"))
}

fn push_gemini_home_sidecars(mounts: &mut Vec<ai_providers::sandbox::RunscMount>) {
    let gemini_dir = resolved_gemini_config_dir();
    let settings_path = ["settings.local.json", "settings.json"]
        .into_iter()
        .map(|name| gemini_dir.join(name))
        .find(|path| path.exists());
    if let Some(settings_path) = settings_path {
        push_container_home_mount(
            mounts,
            settings_path,
            Path::new(".gemini").join("settings.json"),
        );
    }
    for sidecar in [
        "google_accounts.json",
        "installation_id",
        "mcp-oauth-tokens-v2.json",
        "oauth_creds.json",
        "state.json",
        "trustedFolders.json",
    ] {
        push_container_home_mount(
            mounts,
            gemini_dir.join(sidecar),
            Path::new(".gemini").join(sidecar),
        );
    }
}

fn push_qwen_home_sidecars(mounts: &mut Vec<ai_providers::sandbox::RunscMount>, config_dir: &Path) {
    for sidecar in [
        "settings.json",
        "oauth_creds.json",
        "installation_id",
        "source.json",
        "api.key",
        ".qwen.json",
    ] {
        push_container_home_mount(
            mounts,
            config_dir.join(sidecar),
            Path::new(".qwen").join(sidecar),
        );
    }
}

pub fn resolve_host_cli_binary(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = PathBuf::from(trimmed);
    if candidate.components().count() > 1 || candidate.is_absolute() {
        return candidate.exists().then_some(candidate);
    }

    let path = std::env::var_os("PATH")?;
    for entry in std::env::split_paths(&path) {
        let resolved = entry.join(trimmed);
        if resolved.exists() {
            return Some(resolved);
        }
    }
    None
}

pub fn autonomous_worker_runsc_mounts() -> Vec<ai_providers::sandbox::RunscMount> {
    let mut mounts = vec![ai_providers::sandbox::RunscMount::container_home(
        coordination_root_dir(),
        Path::new(".rust-chatbot/coordination"),
    )];
    push_repo_mcp_mount(&mut mounts);
    for provider in [
        ModelProvider::Claude,
        ModelProvider::MiniMax,
        ModelProvider::Qwen,
        ModelProvider::Codex,
        ModelProvider::Nvidia,
        ModelProvider::Gemini,
    ] {
        if let Some(config_dir) = config::get_config_dir(provider).map(PathBuf::from) {
            push_same_path_mount_with_canonical(&mut mounts, config_dir);
        }
    }
    if let Some(config_dir) = config::get_config_dir(ModelProvider::Claude).map(PathBuf::from) {
        if let Some(claude_json_path) = config::claude_json_path().map(PathBuf::from) {
            push_same_path_mount_with_canonical(&mut mounts, claude_json_path);
        }
        push_claude_family_home_sidecars(
            &mut mounts,
            &config_dir,
            config::claude_json_path().map(PathBuf::from),
        );
    }
    for provider in [ModelProvider::MiniMax] {
        if let Some(config_dir) = config::get_config_dir(provider).map(PathBuf::from) {
            push_claude_family_home_sidecars(&mut mounts, &config_dir, None);
        }
    }
    push_codex_home_sidecars(&mut mounts);
    push_gemini_home_sidecars(&mut mounts);
    if let Some(config_dir) = config::get_config_dir(ModelProvider::Qwen).map(PathBuf::from) {
        push_qwen_home_sidecars(&mut mounts, &config_dir);
    }
    let cli_names: Vec<String> = vec![
        "claude".to_string(),
        "gemini".to_string(),
        config::qwen_code_cli_path(),
        config::codex_cli_path(),
    ];
    for cli_name in &cli_names {
        for dir in ai_providers::sandbox::program_directory_mounts(cli_name) {
            push_runsc_mount(
                &mut mounts,
                ai_providers::sandbox::RunscMount::same_path(dir),
            );
        }
    }
    mounts
}

#[cfg(test)]
mod tests {
    use super::{prepare_codex_home_sidecar_dir_in, seed_sidecar_file_if_missing};
    use std::fs;

    #[test]
    fn prepare_codex_home_sidecar_dir_seeds_missing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source/.codex");
        let sidecar_dir = temp.path().join("sidecar/.codex");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("config.toml"), "model = \"gpt-5.4\"\n").unwrap();
        fs::write(source_dir.join("auth.json"), "{\"token\":\"abc\"}\n").unwrap();

        let prepared =
            prepare_codex_home_sidecar_dir_in(&sidecar_dir, &source_dir.join("config.toml"))
                .unwrap();

        assert_eq!(prepared, sidecar_dir);
        assert_eq!(
            fs::read_to_string(sidecar_dir.join("config.toml")).unwrap(),
            "model = \"gpt-5.4\"\n"
        );
        assert_eq!(
            fs::read_to_string(sidecar_dir.join("auth.json")).unwrap(),
            "{\"token\":\"abc\"}\n"
        );
    }

    #[test]
    fn prepare_codex_home_sidecar_dir_preserves_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let source_dir = temp.path().join("source/.codex");
        let sidecar_dir = temp.path().join("sidecar/.codex");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&sidecar_dir).unwrap();
        fs::write(source_dir.join("config.toml"), "source = true\n").unwrap();
        fs::write(sidecar_dir.join("config.toml"), "sidecar = true\n").unwrap();

        prepare_codex_home_sidecar_dir_in(&sidecar_dir, &source_dir.join("config.toml")).unwrap();

        assert_eq!(
            fs::read_to_string(sidecar_dir.join("config.toml")).unwrap(),
            "sidecar = true\n"
        );
    }

    #[test]
    fn seed_sidecar_file_if_missing_skips_absent_source() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("sidecar/config.toml");

        seed_sidecar_file_if_missing(&temp.path().join("missing.toml"), &destination).unwrap();

        assert!(!destination.exists());
    }
}
