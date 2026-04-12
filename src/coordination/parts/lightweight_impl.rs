use super::*;
pub fn uses_lightweight_runtime_mode(mode: &AutonomousWorkflowMode) -> bool {
    matches!(
        mode,
        AutonomousWorkflowMode::LightweightImplementation | AutonomousWorkflowMode::GithubBranchFix
    )
}
