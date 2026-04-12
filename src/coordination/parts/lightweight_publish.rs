use super::*;
use fs2::FileExt;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use wait_timeout::ChildExt;
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
pub(super) fn git_head_exists(root: &Path) -> Result<bool> {
    let output = run_git_output(Some(root), &["rev-parse", "--verify", "HEAD"])
        .with_context(|| format!("failed to inspect git HEAD in {}", root.display()))?;
    Ok(output.status.success())
}
fn run_git_output(current_dir: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=10");
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    run_command_with_timeout(&mut command, GIT_COMMAND_TIMEOUT).with_context(|| match current_dir {
        Some(dir) => format!("failed to run git {} in {}", args.join(" "), dir.display()),
        None => format!("failed to run git {}", args.join(" ")),
    })
}
fn run_command_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output> {
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn()?;
    match child.wait_timeout(timeout)? {
        Some(_) => child
            .wait_with_output()
            .context("failed to collect command output"),
        None => {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            anyhow::bail!("command timed out after {}s", timeout.as_secs());
        }
    }
}
fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(250));
            if child.try_wait().ok().flatten().is_none() {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }

    let _ = child.kill();
}
