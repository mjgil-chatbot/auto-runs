#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/driver-service.sh"

require_installed_binaries
ensure_driver_service_running

echo "Launching auto-runs..."
exec "${AUTO_RUNS_BIN}"
