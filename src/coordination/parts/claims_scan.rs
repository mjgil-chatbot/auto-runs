use super::*;
pub(super) fn load_claim_from_entry(entry: &fs::DirEntry) -> Option<TaskClaim> {
    let Ok(raw) = fs::read_to_string(entry.path()) else {
        return None;
    };
    serde_json::from_str(&raw).ok()
}
fn claim_worker_pids_for_stop(claim: &TaskClaim) -> Vec<u32> {
    let mut pids = Vec::new();
    if let Some(worker_pid) = claim.worker_pid {
        pids.push(worker_pid);
    }
    if claim
        .launcher_pid
        .map(|launcher_pid| launcher_pid != claim.instance_pid)
        .unwrap_or(true)
        && !pids.contains(&claim.instance_pid)
    {
        pids.push(claim.instance_pid);
    }
    pids
}
