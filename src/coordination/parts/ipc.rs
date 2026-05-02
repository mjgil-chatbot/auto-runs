//! Worker queue IPC via Unix named pipes (FIFOs).
//!
//! Design: a pair of FIFOs per coordination root:
//!   - `worker-queue-req.fifo`  — workers write requests here
//!   - `worker-queue-resp.fifo`  — server writes responses here
//!
//! Protocol (one request/response per worker open):
//!   Worker → Server:  "{run_id}\t{instance_pid}\t{provider}\n"
//!   Server → Worker:  "{task_id}\n"   (on success)
//!                    "\n"           (no task available)
//!                    "error:{msg}\n" (on error)

use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rust_debug::tracing::{debug, error, info, warn};

/// Path to the worker queue request FIFO.
pub fn worker_queue_req_fifo_path() -> PathBuf {
    coordination_root().join("worker-queue-req.fifo")
}

/// Path to the worker queue response FIFO.
pub fn worker_queue_resp_fifo_path() -> PathBuf {
    coordination_root().join("worker-queue-resp.fifo")
}

/// Path to the coordination root.
fn coordination_root() -> PathBuf {
    super::super::config::shared_infra_dir().join("coordination")
}

/// Creates both FIFOs if they do not yet exist.  Idempotent — succeeds
/// silently if the paths already exist as FIFOs.
pub fn ensure_worker_queue_fifos() -> Result<()> {
    for path in &[worker_queue_req_fifo_path(), worker_queue_resp_fifo_path()] {
        if path.exists() {
            continue;
        }
        // SAFETY: mkfifo is safe and we own the path (doesn't exist).
        let path_str = path.to_string_lossy();
        let ret = unsafe { libc::mkfifo(path_str.as_ptr() as *const libc::c_char, 0o600) };
        if ret != 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::AlreadyExists {
                anyhow::bail!("mkfifo {} failed: {}", path.display(), err);
            }
        }
        debug!(path = %path.display(), "created worker queue FIFO");
    }
    Ok(())
}

/// Maximum time a worker waits for a FIFO response before backing off.
const FIFO_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Default polling backoff base for workers with no task available.
const BACKOFF_BASE_SECS: u64 = 1;
/// Maximum backoff cap.
const BACKOFF_CAP_SECS: u64 = 30;

/// A request parsed from a worker's FIFO payload.
#[derive(Debug)]
pub struct WorkerRequest {
    pub run_id: String,
    pub instance_pid: u32,
    pub provider: String,
}

/// The result of processing a worker request.
#[derive(Debug)]
pub enum WorkerQueueResult {
    /// A task was successfully claimed.
    TaskClaimed { task_id: String },
    /// No task was available.
    NoTask,
    /// An error occurred while processing the request.
    Error(String),
}

impl WorkerRequest {
    fn parse(raw: &str) -> Option<Self> {
        let mut fields = raw.split('\t');
        let run_id = fields.next()?.trim().to_string();
        let instance_pid: u32 = fields.next()?.trim().parse().ok()?;
        let provider = fields.next()?.trim().to_string();
        Some(WorkerRequest {
            run_id,
            instance_pid,
            provider,
        })
    }
}

/// The worker queue server loop.
///
/// Opens the request FIFO in read mode and processes incoming worker requests.
/// For each valid request it calls `claim_next_runnable_task` and writes the
/// result to the response FIFO.  Exits on fatal errors; transient errors are
/// logged and skipped.
pub fn run_worker_queue_server() -> Result<()> {
    let req_path = worker_queue_req_fifo_path();

    ensure_worker_queue_fifos()?;

    info!(req_fifo = %req_path.display(), "worker queue server started");

    loop {
        let req_file = fs::OpenOptions::new()
            .read(true)
            .open(&req_path)
            .with_context(|| format!("failed to open request FIFO {}", req_path.display()))?;
        let mut reader = BufReader::new(req_file);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    debug!("request FIFO writer closed; reopening");
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    let request = match WorkerRequest::parse(trimmed) {
                        Some(r) => r,
                        None => {
                            warn!(raw = %trimmed, "malformed worker request; skipping");
                            continue;
                        }
                    };

                    debug!(
                        run_id = %request.run_id,
                        instance_pid = request.instance_pid,
                        provider = %request.provider,
                        "processing worker request"
                    );

                    let result = process_worker_request(&request);

                    if let Err(err) = send_response(&result) {
                        warn!(error = %err, "failed to send worker response");
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(err) => {
                    error!(error = %err, "fatal error reading request FIFO");
                    break;
                }
            }
        }
    }
}

fn process_worker_request(req: &WorkerRequest) -> WorkerQueueResult {
    match claim_next_runnable_provider_task(&req.run_id, req.instance_pid, &req.provider) {
        Ok(Some(task)) => WorkerQueueResult::TaskClaimed {
            task_id: task.task_id,
        },
        Ok(None) => WorkerQueueResult::NoTask,
        Err(err) => WorkerQueueResult::Error(err.to_string()),
    }
}

fn send_response(result: &WorkerQueueResult) -> Result<()> {
    let resp_path = worker_queue_resp_fifo_path();
    let mut resp_file = OpenOptions::new()
        .write(true)
        .append(true)
        .open(&resp_path)
        .with_context(|| format!("failed to open response FIFO {}", resp_path.display()))?;

    let line = match result {
        WorkerQueueResult::TaskClaimed { task_id } => format!("{task_id}\n"),
        WorkerQueueResult::NoTask => "\n".to_string(),
        WorkerQueueResult::Error(msg) => format!("error:{msg}\n"),
    };

    resp_file
        .write_all(line.as_bytes())
        .with_context(|| "failed to write to response FIFO")?;
    resp_file.flush()?;
    Ok(())
}

// ── Worker-side client ────────────────────────────────────────────────────────

/// Maximum total time a worker spends polling before giving up.
const WORKER_POLL_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Information returned to the caller of `poll_task_until_found`.
#[derive(Debug)]
pub struct WorkerTaskAssignment {
    pub task_id: String,
}

/// Request a task assignment by contacting the worker queue FIFO.
///
/// Workers call this on startup.  If no task is available the function
/// retries with exponential backoff until either a task is assigned or the
/// overall poll timeout is reached.  Returns `None` if the timeout expires
/// with no task available.
pub fn poll_task_until_found(
    run_id: &str,
    instance_pid: u32,
    provider: &str,
) -> Option<WorkerTaskAssignment> {
    let deadline = Instant::now() + WORKER_POLL_TIMEOUT;
    let req_path = worker_queue_req_fifo_path();
    let resp_path = worker_queue_resp_fifo_path();
    let mut backoff_secs = BACKOFF_BASE_SECS;

    loop {
        let now = Instant::now();
        if now >= deadline {
            warn!(
                run_id = %run_id,
                instance_pid,
                provider = %provider,
                "worker poll timeout; giving up"
            );
            return None;
        }

        if !req_path.exists() || !resp_path.exists() {
            debug!("FIFOs not yet ready; backing off");
            thread::sleep(Duration::from_secs(1));
            backoff_secs = (backoff_secs / 2).max(1);
            continue;
        }

        let mut req_file = match OpenOptions::new().write(true).open(&req_path) {
            Ok(f) => f,
            Err(err) => {
                debug!(error = %err, "failed to open request FIFO; backing off");
                thread::sleep(Duration::from_secs(1));
                backoff_secs = (backoff_secs / 2).max(1);
                continue;
            }
        };

        let request_line = format!("{run_id}\t{instance_pid}\t{provider}\n");
        if let Err(err) = req_file.write_all(request_line.as_bytes()) {
            debug!(error = %err, "failed to write request; backing off");
            thread::sleep(Duration::from_secs(1));
            continue;
        }
        drop(req_file); // close write end so server can read

        let resp_file = match OpenOptions::new().read(true).open(&resp_path) {
            Ok(f) => f,
            Err(err) => {
                debug!(error = %err, "failed to open response FIFO; backing off");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let fd = get_raw_fd(&resp_file);
        let mut reader = BufReader::new(resp_file); // resp_file consumed here
        let reader_deadline = now + FIFO_RESPONSE_TIMEOUT;
        let mut line = String::new();

        let got_response = loop {
            let remaining = reader_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break false;
            }
            if fd < 0 {
                break false;
            }

            // SAFETY: poll_fds is a valid single-element array.
            let mut poll_fds = [libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            }];
            let timeout_ms = remaining.as_millis() as libc::c_int;

            let nevents = unsafe { libc::poll(&mut poll_fds as *mut _, 1, timeout_ms) };

            if nevents < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break false;
            }
            if nevents == 0 {
                continue;
            }

            let revents = poll_fds[0].revents;
            if revents & libc::POLLHUP != 0 {
                return None;
            }
            if revents & libc::POLLIN == 0 {
                continue;
            }

            match reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {
                    if !line.is_empty() {
                        break true;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => break false,
            }
        };

        if got_response {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                return parse_assignment_response(trimmed);
            }
        }

        let jitter = rand_jitter();
        let sleep_ms = ((backoff_secs as f64) * (1.0 + jitter * 0.4)) as u64 * 1000;
        debug!(backoff_ms = sleep_ms, "no task; backing off");
        thread::sleep(Duration::from_millis(sleep_ms));
        backoff_secs = (backoff_secs * 2).min(BACKOFF_CAP_SECS);
    }
}

fn parse_assignment_response(line: &str) -> Option<WorkerTaskAssignment> {
    if line.is_empty() {
        None
    } else if line.starts_with("error:") {
        warn!(error = %line, "worker queue returned error");
        None
    } else {
        Some(WorkerTaskAssignment {
            task_id: line.to_string(),
        })
    }
}

static RAND_STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0xdeadbeef);

fn rand_jitter() -> f64 {
    let mut state = RAND_STATE.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        let new = state ^ (state << 13);
        let new = new ^ (new >> 7);
        let new = new ^ (new << 17);
        match RAND_STATE.compare_exchange_weak(
            state,
            new,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => {
                state = new;
                return (state as f64) / (u64::MAX as f64);
            }
            Err(actual) => state = actual,
        }
    }
}

fn get_raw_fd(file: &File) -> libc::c_int {
    use std::os::unix::io::AsRawFd;
    file.as_raw_fd()
}

#[cfg(test)]
mod tests {
    use super::WorkerRequest;

    #[test]
    fn worker_request_parse_accepts_tab_delimited_frame() {
        let request = WorkerRequest::parse("run-1\t42\tminimax").expect("request should parse");
        assert_eq!(request.run_id, "run-1");
        assert_eq!(request.instance_pid, 42);
        assert_eq!(request.provider, "minimax");
    }

    #[test]
    fn worker_request_parse_rejects_legacy_multiline_frame() {
        assert!(WorkerRequest::parse("run-1\n42\nminimax").is_none());
    }
}
