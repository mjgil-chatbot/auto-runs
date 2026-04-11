#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/driver-service.sh"

install_auto_runs_binaries
ensure_driver_service_running
stop_running_auto_runs_ui_instances
launch_auto_runs_background
