//! Shared library for the standalone auto-runs desktop app.

use std::sync::Mutex;
pub use auto_runs_contracts::{
    default_design_doc_path_for_run, default_design_doc_path_for_workflow,
    is_generated_design_doc_path, AutonomousExecutionEnvironment, AutonomousWorkflowMode,
    ReviewFixMode, ReviewScope, TurnPolicy,
};
pub use chatbot_lib::config;
pub use chatbot_lib::data;
pub use chatbot_lib::model;
pub use chatbot_lib::storage;
pub mod coordination;
