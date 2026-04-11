#!/usr/bin/env bash
set -euo pipefail

APP_ROOT="${APP_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
TARGET_SUFFIX="${ENGINEERING_GATE_TARGET_SUFFIX:-${USER:-uid$(id -u)}}"
TARGET_DIR="${CARGO_TARGET_DIR:-${APP_ROOT}/target/engineering-gate-local-${TARGET_SUFFIX}}"
MODE="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  verify-coordination-gate.sh --workflow
  verify-coordination-gate.sh --coordination
EOF
}

if [[ -z "${MODE}" ]]; then
  usage >&2
  exit 1
fi

cd "$APP_ROOT"
export CARGO_TARGET_DIR="${TARGET_DIR}"

case "${MODE}" in
  --workflow)
    echo "[verify-coordination-gate] target-dir: ${CARGO_TARGET_DIR}"
    python3 "$APP_ROOT/scripts/verify_engineering_gate.py" --run-group workflow_registry_checks
    ;;
  --coordination)
    echo "[verify-coordination-gate] target-dir: ${CARGO_TARGET_DIR}"
    python3 "$APP_ROOT/scripts/verify_engineering_gate.py" --run-group coordination_runtime_checks
    ;;
  -h|--help)
    usage
    ;;
  *)
    printf 'Unknown argument: %s\n' "${MODE}" >&2
    usage >&2
    exit 1
    ;;
esac
