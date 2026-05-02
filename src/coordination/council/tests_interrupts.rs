#[test]
fn council_user_reply_interrupt_pauses_run_until_resolution() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.status = CouncilRunStatus::WaitingForUser;
    persist_council_state(&council, coordination::CoordinationRunStatus::Active);

    let interrupt = coordination::create_council_user_reply_interrupt(
        &council.run_id,
        "entry-1",
        "Need a user decision before the council can continue.",
    )
    .expect("interrupt should persist");

    let snapshot = coordination::load_run_snapshot(&council.run_id).expect("snapshot should load");
    assert_eq!(
        snapshot.run.status,
        coordination::CoordinationRunStatus::WaitingForPolicy
    );
    assert!(
        !coordination::drive_run_for_instance(&council.run_id, 41, "codex")
            .expect("driver should honor council interrupt")
    );

    coordination::resolve_interrupt(
        &council.run_id,
        &interrupt.interrupt_id,
        coordination::InterruptResolution::EditedPayload {
            value: serde_json::json!({
                "reply": "resume after the user approved the narrower plan"
            }),
        },
    )
    .expect("interrupt should resolve");

    let updated = load_council_run(&council.run_id).expect("council should reload");
    assert_eq!(updated.status, CouncilRunStatus::Active);
    let ledger = load_council_ledger(&council.run_id).expect("ledger should load");
    assert!(ledger.iter().any(|entry| {
        entry.kind == CouncilEntryKind::UserInterjection
            && entry
                .content
                .contains("resume after the user approved the narrower plan")
    }));

    let snapshot =
        coordination::load_run_snapshot(&council.run_id).expect("snapshot should refresh");
    assert_eq!(snapshot.run.status, coordination::CoordinationRunStatus::Active);
    assert!(!coordination::has_open_interrupts(&council.run_id));

    cleanup_council_run(&council.run_id);
}
