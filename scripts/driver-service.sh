#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
AUTO_RUNS_BIN="${CARGO_BIN_DIR}/auto-runs"
AUTO_RUNS_WORKER_BIN="${CARGO_BIN_DIR}/auto-runs-worker"
DRIVER_SERVICE_NAME="auto-runs-driver"
DRIVER_LAUNCHD_LABEL="com.mjgil.auto-runs-driver"
SYSTEMD_UNIT_PATH="${HOME}/.config/systemd/user/${DRIVER_SERVICE_NAME}.service"
LAUNCHD_PLIST_PATH="${HOME}/Library/LaunchAgents/${DRIVER_LAUNCHD_LABEL}.plist"
SHARED_INFRA_DIR="${HOME}/.rust-chatbot"
DRIVER_HEARTBEAT_PATH="${SHARED_INFRA_DIR}/coordination/background-driver.json"
DRIVER_LOG_DIR="${SHARED_INFRA_DIR}/logs"
DRIVER_STDOUT_LOG="${DRIVER_LOG_DIR}/auto-runs-driver.stdout.log"
DRIVER_STDERR_LOG="${DRIVER_LOG_DIR}/auto-runs-driver.stderr.log"
AUTO_RUNS_UI_LOG="${DRIVER_LOG_DIR}/auto-runs-ui.log"
AUTO_RUNS_BINS_CHANGED=0

os_name() {
    uname -s
}

require_supported_os() {
    case "$(os_name)" in
        Linux|Darwin) ;;
        *)
            echo "Unsupported OS: $(os_name)" >&2
            exit 1
            ;;
    esac
}

command_exists() {
    command -v "$1" >/dev/null 2>&1
}

binary_mtime_epoch() {
    local path="$1"
    case "$(os_name)" in
        Linux) stat -c %Y "${path}" ;;
        Darwin) stat -f %m "${path}" ;;
    esac
}

binary_fingerprint() {
    local path="$1"
    local digest
    local size
    local mtime

    if [ ! -f "${path}" ]; then
        echo "missing"
        return 0
    fi

    if command_exists sha256sum; then
        sha256sum "${path}" | awk '{print $1}'
        return 0
    fi

    if command_exists shasum; then
        shasum -a 256 "${path}" | awk '{print $1}'
        return 0
    fi

    if command_exists python3; then
        python3 - "${path}" <<'PY'
import hashlib
import sys

hasher = hashlib.sha256()
with open(sys.argv[1], "rb") as fh:
    for chunk in iter(lambda: fh.read(1024 * 1024), b""):
        hasher.update(chunk)
print(hasher.hexdigest())
PY
        return 0
    fi

    size="$(wc -c < "${path}" | tr -d '[:space:]')"
    mtime="$(binary_mtime_epoch "${path}")"
    digest="size:${size}:mtime:${mtime}"
    echo "${digest}"
}

install_auto_runs_binaries() {
    local auto_runs_before
    local auto_runs_after
    local worker_before
    local worker_after

    auto_runs_before="$(binary_fingerprint "${AUTO_RUNS_BIN}")"
    worker_before="$(binary_fingerprint "${AUTO_RUNS_WORKER_BIN}")"

    echo ""
    echo "Installing auto-runs binaries..."
    CARGO_TARGET_DIR="${REPO_ROOT}/target/install" \
        cargo install --path "${REPO_ROOT}" --locked --force --bin auto-runs --bin auto-runs-worker

    auto_runs_after="$(binary_fingerprint "${AUTO_RUNS_BIN}")"
    worker_after="$(binary_fingerprint "${AUTO_RUNS_WORKER_BIN}")"
    if [ "${auto_runs_before}" != "${auto_runs_after}" ] || [ "${worker_before}" != "${worker_after}" ]; then
        AUTO_RUNS_BINS_CHANGED=1
        echo "Installed auto-runs service binaries changed."
        return
    fi

    AUTO_RUNS_BINS_CHANGED=0
    echo "Installed auto-runs service binaries unchanged."
}

require_installed_binaries() {
    if [ ! -x "${AUTO_RUNS_BIN}" ]; then
        echo "auto-runs binary not found at ${AUTO_RUNS_BIN}" >&2
        exit 1
    fi
    if [ ! -x "${AUTO_RUNS_WORKER_BIN}" ]; then
        echo "auto-runs-worker binary not found at ${AUTO_RUNS_WORKER_BIN}" >&2
        exit 1
    fi
}

ensure_log_dir() {
    mkdir -p "${DRIVER_LOG_DIR}"
}

service_is_installed() {
    case "$(os_name)" in
        Linux) [ -f "${SYSTEMD_UNIT_PATH}" ] ;;
        Darwin) [ -f "${LAUNCHD_PLIST_PATH}" ] ;;
    esac
}

service_is_running() {
    case "$(os_name)" in
        Linux)
            systemctl --user is-active --quiet "${DRIVER_SERVICE_NAME}"
            ;;
        Darwin)
            launchctl print "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}" >/dev/null 2>&1
            ;;
    esac
}

write_systemd_unit() {
    mkdir -p "$(dirname "${SYSTEMD_UNIT_PATH}")"
    systemd_unit_contents > "${SYSTEMD_UNIT_PATH}"
}

systemd_unit_contents() {
    cat <<EOF
[Unit]
Description=auto-runs background driver

[Service]
Type=simple
ExecStart=${AUTO_RUNS_BIN} drive-forever
# The driver must come back even if something external sends SIGTERM.
# `systemctl stop` still suppresses restart, but unexpected clean-looking
# exits should not leave autonomous runs undriven.
Restart=always
RestartSec=1
KillMode=process
SendSIGKILL=no

[Install]
WantedBy=default.target
EOF
}

write_launchd_plist() {
    mkdir -p "$(dirname "${LAUNCHD_PLIST_PATH}")"
    ensure_log_dir
    launchd_plist_contents > "${LAUNCHD_PLIST_PATH}"
}

launchd_plist_contents() {
    cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${DRIVER_LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${AUTO_RUNS_BIN}</string>
    <string>drive-forever</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${DRIVER_STDOUT_LOG}</string>
  <key>StandardErrorPath</key>
  <string>${DRIVER_STDERR_LOG}</string>
</dict>
</plist>
EOF
}

service_definition_is_current() {
    case "$(os_name)" in
        Linux)
            [ -f "${SYSTEMD_UNIT_PATH}" ] || return 1
            diff -q "${SYSTEMD_UNIT_PATH}" <(systemd_unit_contents) >/dev/null 2>&1
            ;;
        Darwin)
            [ -f "${LAUNCHD_PLIST_PATH}" ] || return 1
            diff -q "${LAUNCHD_PLIST_PATH}" <(launchd_plist_contents) >/dev/null 2>&1
            ;;
    esac
}

install_driver_service() {
    require_supported_os
    require_installed_binaries

    case "$(os_name)" in
        Linux)
            write_systemd_unit
            systemctl --user daemon-reload
            systemctl --user enable "${DRIVER_SERVICE_NAME}"
            if service_is_running; then
                systemctl --user restart "${DRIVER_SERVICE_NAME}"
                echo "Installed and restarted ${DRIVER_SERVICE_NAME}"
            else
                systemctl --user start "${DRIVER_SERVICE_NAME}"
                echo "Installed and started ${DRIVER_SERVICE_NAME}"
            fi
            ;;
        Darwin)
            write_launchd_plist
            launchctl bootout "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}" >/dev/null 2>&1 || true
            launchctl bootstrap "gui/$(id -u)" "${LAUNCHD_PLIST_PATH}"
            echo "Installed and started ${DRIVER_LAUNCHD_LABEL}"
            ;;
    esac
}

uninstall_driver_service() {
    require_supported_os

    case "$(os_name)" in
        Linux)
            systemctl --user stop "${DRIVER_SERVICE_NAME}" >/dev/null 2>&1 || true
            systemctl --user disable "${DRIVER_SERVICE_NAME}" >/dev/null 2>&1 || true
            rm -f "${SYSTEMD_UNIT_PATH}"
            systemctl --user daemon-reload
            ;;
        Darwin)
            launchctl bootout "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}" >/dev/null 2>&1 || true
            rm -f "${LAUNCHD_PLIST_PATH}"
            ;;
    esac

    echo "Driver service uninstalled."
}

start_driver_service() {
    require_supported_os
    install_auto_runs_binaries

    if ! service_is_installed; then
        install_driver_service
        return
    fi

    case "$(os_name)" in
        Linux)
            systemctl --user start "${DRIVER_SERVICE_NAME}"
            ;;
        Darwin)
            if service_is_running; then
                launchctl kickstart -k "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}"
            else
                launchctl bootstrap "gui/$(id -u)" "${LAUNCHD_PLIST_PATH}"
            fi
            ;;
    esac

    echo "Driver service started."
}

stop_driver_service() {
    require_supported_os

    case "$(os_name)" in
        Linux)
            systemctl --user stop "${DRIVER_SERVICE_NAME}"
            ;;
        Darwin)
            launchctl bootout "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}"
            ;;
    esac

    echo "Driver service stopped."
}

restart_driver_service() {
    require_supported_os
    install_auto_runs_binaries

    if ! service_is_installed; then
        install_driver_service
        return
    fi

    case "$(os_name)" in
        Linux)
            systemctl --user restart "${DRIVER_SERVICE_NAME}"
            ;;
        Darwin)
            launchctl bootout "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}" >/dev/null 2>&1 || true
            launchctl bootstrap "gui/$(id -u)" "${LAUNCHD_PLIST_PATH}"
            ;;
    esac

    echo "Driver service restarted."
}

status_driver_service() {
    require_supported_os

    case "$(os_name)" in
        Linux)
            systemctl --user status "${DRIVER_SERVICE_NAME}" --no-pager
            ;;
        Darwin)
            launchctl print "gui/$(id -u)/${DRIVER_LAUNCHD_LABEL}"
            ;;
    esac

    if [ -f "${DRIVER_HEARTBEAT_PATH}" ]; then
        echo ""
        echo "Heartbeat:"
        cat "${DRIVER_HEARTBEAT_PATH}"
    else
        echo ""
        echo "Heartbeat: missing"
    fi
}

logs_driver_service() {
    require_supported_os

    case "$(os_name)" in
        Linux)
            exec journalctl --user -u "${DRIVER_SERVICE_NAME}" -f --no-pager
            ;;
        Darwin)
            ensure_log_dir
            touch "${DRIVER_STDOUT_LOG}" "${DRIVER_STDERR_LOG}"
            exec tail -n 200 -f "${DRIVER_STDOUT_LOG}" "${DRIVER_STDERR_LOG}"
            ;;
    esac
}

ensure_driver_service_running() {
    require_supported_os

    if ! service_is_installed; then
        install_driver_service
        return
    fi

    if ! service_definition_is_current; then
        echo "Detected changes to the installed auto-runs driver service definition."
        install_driver_service
        return
    fi

    if service_is_running; then
        if [ "${AUTO_RUNS_BINS_CHANGED}" -eq 1 ]; then
            echo "Detected changes to the installed auto-runs service binaries."
            restart_driver_service
        fi
        return
    fi

    start_driver_service
}

launch_auto_runs_background() {
    require_installed_binaries
    ensure_log_dir

    echo ""
    echo "Starting auto-runs in background..."
    nohup "${AUTO_RUNS_BIN}" >>"${AUTO_RUNS_UI_LOG}" 2>&1 </dev/null &
    local pid="$!"
    sleep 1
    if ! kill -0 "${pid}" >/dev/null 2>&1; then
        echo "auto-runs exited during startup. Check ${AUTO_RUNS_UI_LOG}" >&2
        exit 1
    fi
    echo "Launched with PID ${pid}"
}

running_auto_runs_ui_pids() {
    pgrep -af "${AUTO_RUNS_BIN}" 2>/dev/null | awk '!/drive-forever/ {print $1}'
}

stop_running_auto_runs_ui_instances() {
    local running_pids
    running_pids="$(running_auto_runs_ui_pids || true)"
    if [ -z "${running_pids}" ]; then
        return
    fi

    echo ""
    echo "Stopping running auto-runs UI instances..."
    while IFS= read -r pid; do
        [ -z "${pid}" ] && continue
        echo "  Stopping PID ${pid}..."
        kill "${pid}" >/dev/null 2>&1 || true
    done <<< "${running_pids}"
    sleep 1

    while IFS= read -r pid; do
        [ -z "${pid}" ] && continue
        if kill -0 "${pid}" >/dev/null 2>&1; then
            echo "  Force-killing PID ${pid}..."
            kill -9 "${pid}" >/dev/null 2>&1 || true
        fi
    done <<< "${running_pids}"
    sleep 1
}

driver_service_usage() {
    echo "Usage: $0 {install-binaries|install|uninstall|start|stop|restart|status|logs}"
}

main() {
    case "${1:-}" in
        install-binaries)
            install_auto_runs_binaries
            ;;
        install)
            install_auto_runs_binaries
            install_driver_service
            ;;
        uninstall)
            uninstall_driver_service
            ;;
        start)
            start_driver_service
            ;;
        stop)
            stop_driver_service
            ;;
        restart)
            restart_driver_service
            ;;
        status)
            status_driver_service
            ;;
        logs)
            logs_driver_service
            ;;
        *)
            driver_service_usage
            exit 1
            ;;
    esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
