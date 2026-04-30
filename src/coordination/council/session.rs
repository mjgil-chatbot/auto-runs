use super::*;
use crate::coordination;
use crate::data::ChatSession;

pub(crate) fn make_council_provider_session(
    speaker: CouncilSpeaker,
    run: &coordination::CoordinationRun,
) -> ChatSession {
    ChatSession {
        id: format!("council-{}-{}", run.run_id, speaker.mention_name()),
        name: format!("auto-council-{}", speaker.label()),
        system_prompt: None,
        json_summary: None,
        claude_session_id: None,
        minimax_session_id: None,
        qwen_session_id: None,
        kimi_session_id: None,
        codex_session_id: None,
        nvidia_session_id: None,
        gemini_session_id: None,
        openrouter_session_id: None,
        opencode_session_id: None,
        hermes_session_id: None,
        forge_session_id: None,
        claw_session_id: None,
        omniforge_session_id: None,
        omniforge_model_id: None,
        launched_from: Some(run.workspace_path.clone()),
        runsc_dirs: Vec::new(),
        runsc_image: None,
        created_at: run.created_at,
        updated_at: run.updated_at,
        messages: Vec::new(),
    }
}
