use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_debug::tracing::{debug, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::config;
use crate::data::ChatSession;
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
#[path = "../code_review.rs"]
mod code_review;
pub mod council;
#[path = "parts/types.rs"]
pub mod types;
pub use types::*;
#[path = "parts/scopes.rs"]
pub mod scopes;
pub use scopes::*;
#[path = "parts/artifacts.rs"]
pub mod artifacts;
pub use artifacts::*;
#[path = "parts/artifacts_lightweight.rs"]
pub mod artifacts_lightweight;
pub use artifacts_lightweight::*;
#[path = "parts/artifact_helpers.rs"]
pub mod artifact_helpers;
pub use artifact_helpers::*;
#[path = "parts/seeding.rs"]
pub mod seeding;
pub use seeding::*;
#[path = "parts/storage.rs"]
pub mod storage_parts;
pub use storage_parts::*;
#[path = "parts/runsc_shared_workspace.rs"]
pub mod runsc_shared_workspace;
pub use runsc_shared_workspace::*;
#[path = "parts/generated_tasks.rs"]
pub mod generated_tasks;
pub use generated_tasks::*;
#[path = "parts/task_contracts.rs"]
pub mod task_contracts;
pub use task_contracts::*;
#[path = "parts/lightweight_impl.rs"]
pub mod lightweight_impl;
pub use lightweight_impl::*;
