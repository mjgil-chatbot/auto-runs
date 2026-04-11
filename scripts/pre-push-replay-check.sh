#!/usr/bin/env bash
set -euo pipefail

checker="../lean-orchestration/tools/replay-checker/target/release/replay-checker"
coord_dir="$HOME/.rust-chatbot/coordination"

if [ ! -x "$checker" ]; then
    echo "[pre-push] replay-checker not built, skipping validation"
    exit 0
fi

if [ ! -d "$coord_dir" ]; then
    exit 0
fi

# Find the most recently modified run with events
latest=$(find "$coord_dir" -name events.jsonl -maxdepth 2 -printf '%T@ %h\n' 2>/dev/null \
    | sort -rn \
    | sed -n '1s/^[^ ]* //p')

if [ -z "$latest" ]; then
    exit 0
fi

run_id=$(basename "$latest")
echo "[pre-push] Validating latest run: $run_id"

if ! "$checker" "$latest" --json 2>/dev/null | python3 -c "
import json, sys
d = json.load(sys.stdin)
if d['valid']:
    print(f'  OK: {d[\"events_checked\"]} events, 0 violations')
else:
    print(f'  {d[\"violations\"]} violations in {d[\"events_checked\"]} events')
    sys.exit(1)
" 2>/dev/null; then
    echo "[pre-push] WARNING: replay violations found (push continues)"
fi

exit 0
