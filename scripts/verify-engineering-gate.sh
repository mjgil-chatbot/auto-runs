#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
exec python3 "$repo_root/scripts/verify_engineering_gate.py" "$@"
