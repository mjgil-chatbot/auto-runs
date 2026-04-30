//! Shared library for the standalone auto-runs desktop app.

use std::sync::Mutex;

// Test environment lock — used by integration tests to serialize access to
// shared environment variables (e.g. $HOME) that affect provider behavior.
pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

// Types shared with rust-chatbot via contracts crate
pub use auto_runs_contracts::{
    default_design_doc_path_for_run, default_design_doc_path_for_workflow,
    is_generated_design_doc_path, AutonomousExecutionEnvironment, AutonomousWorkflowMode,
    ReviewFixMode, ReviewScope, TurnPolicy,
};
// Re-export chatbot-worker modules that auto-runs still depends on
pub use chatbot_lib::config;
// Keep data re-export for types not yet moved to contracts (e.g., ChatSession)
pub use chatbot_lib::data;
pub use chatbot_lib::debug;
pub use chatbot_lib::ensure_path_symlink;
pub use chatbot_lib::memory;
pub use chatbot_lib::model;
pub use chatbot_lib::provider_bridge;
pub use chatbot_lib::storage;
pub use chatbot_lib::stream;
pub use chatbot_lib::worker;
pub use chatbot_lib::workspace;

pub mod coordination;
pub mod driver;
pub mod process_limits;
pub mod sync;
pub mod workflow;

#[cfg(test)]
mod cap_tests {
    use cap::Cap;
    use std::alloc;

    const TEST_ALLOCATION_LIMIT_BYTES: usize = 64 * 1024 * 1024;

    #[global_allocator]
    static TEST_ALLOCATOR: Cap<alloc::System> =
        Cap::new(alloc::System, TEST_ALLOCATION_LIMIT_BYTES);

    #[test]
    fn small_allocations_fit_under_test_cap() {
        let bytes: Vec<u8> = std::iter::repeat(0_u8).take(16 * 1024).collect();
        assert_eq!(bytes.len(), 16 * 1024);
    }
}
