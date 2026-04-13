use fs2::FileExt;
pub fn is_provider_on_cooldown<'a>(
    run: &'a CoordinationRun,
    provider: &str,
    now: DateTime<Utc>,
) -> Option<&'a ProviderCooldown> {
    let normalized = normalize_provider_name(Some(provider))?;
    let cooldown = run.provider_cooldowns.get(normalized.as_str())?;
    (cooldown.unavailable_until > now).then_some(cooldown)
}
