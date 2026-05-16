#[test]
fn rebuild_index_excludes_non_terminal_tasks_from_provider_usage() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let tasks = vec![fixture.make_task(
        &run,
        "task-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write module",
        &["src"],
        &["src/mod.rs"],
        &[],
        CoordinationTaskStatus::Running,
    )];
    fixture.persist_run_with_tasks(&run, &tasks);

    let index = crate::storage::StorageIndex::open().expect("index should open");
    let report = index.rebuild_all().expect("rebuild should succeed");

    assert_eq!(report.sessions_indexed, 0);
    assert_eq!(report.source_counts.session_index, 0);
    assert_eq!(report.indexed_counts.session_index, 0);
}

#[test]
fn index_task_binds_every_declared_tasks_column() {
    let fixture = CoordinationTestFixture::named("index-task");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let task = fixture.make_task(
        &run,
        "task-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write module",
        &["src"],
        &["src/mod.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    {
        let index = crate::storage::StorageIndex::open().expect("index should open");
        let conn = index.connection();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tasks (
                task_id TEXT NOT NULL,
                run_id TEXT NOT NULL,
                title TEXT,
                status TEXT,
                category TEXT,
                provider TEXT,
                fallback_from TEXT,
                write_scope TEXT,
                claimed_by TEXT,
                claimed_at TEXT,
                completed_at TEXT,
                attempt_count INTEGER,
                PRIMARY KEY (run_id, task_id)
            );
            "#,
        )
        .expect("tasks table should exist");
    }

    index_task(&task);

    let index = crate::storage::StorageIndex::open().expect("index should reopen");
    let conn = index.connection();
    let row = conn
        .query_row(
            "SELECT title, attempt_count FROM tasks WHERE run_id = ?1 AND task_id = ?2",
            rusqlite::params![task.run_id, task.task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )
        .expect("task row should be indexed");

    assert_eq!(row.0, "Write module");
    assert_eq!(row.1, 0);
}
