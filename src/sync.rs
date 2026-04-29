use crate::coordination::AgentRole;

pub use chatbot_lib::sync::{ROLE_CODE_NONE, ROLE_CODE_ORCHESTRATOR, ROLE_CODE_WORKER};

fn map_from_chatbot(role: Option<chatbot_lib::AgentRole>) -> Option<AgentRole> {
    match role {
        None => None,
        Some(chatbot_lib::AgentRole::Orchestrator) => Some(AgentRole::Orchestrator),
        Some(chatbot_lib::AgentRole::DeepWorker) => Some(AgentRole::DeepWorker),
        Some(chatbot_lib::AgentRole::Reviewer) => Some(AgentRole::Reviewer),
        Some(chatbot_lib::AgentRole::Explorer) => Some(AgentRole::Explorer),
        Some(chatbot_lib::AgentRole::Worker) => Some(AgentRole::DeepWorker),
    }
}

fn map_to_chatbot(role: Option<AgentRole>) -> Option<chatbot_lib::AgentRole> {
    match role {
        None => None,
        Some(AgentRole::Orchestrator) => Some(chatbot_lib::AgentRole::Orchestrator),
        Some(AgentRole::DeepWorker) => Some(chatbot_lib::AgentRole::DeepWorker),
        Some(AgentRole::Reviewer) => Some(chatbot_lib::AgentRole::Reviewer),
        Some(AgentRole::Explorer) => Some(chatbot_lib::AgentRole::Explorer),
    }
}

pub fn get_instance_role_by_pid(pid: u32) -> Option<AgentRole> {
    map_from_chatbot(chatbot_lib::sync::get_instance_role_by_pid(pid))
}

pub fn role_to_u8(role: Option<AgentRole>) -> u8 {
    chatbot_lib::sync::role_to_u8(map_to_chatbot(role))
}

pub fn role_from_u8(code: u8) -> Option<AgentRole> {
    map_from_chatbot(chatbot_lib::sync::role_from_u8(code))
}
