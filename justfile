set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

install:
    ./scripts/driver-service.sh install-binaries

install-driver-service:
    ./scripts/driver-service.sh install

uninstall-driver-service:
    ./scripts/driver-service.sh uninstall

start:
    ./scripts/start-auto-runs.sh

restart:
    ./scripts/restart-auto-runs.sh

start-driver-service:
    ./scripts/driver-service.sh start

stop-driver-service:
    ./scripts/driver-service.sh stop

restart-driver-service:
    ./scripts/driver-service.sh restart

status-driver-service:
    ./scripts/driver-service.sh status

logs-driver-service:
    ./scripts/driver-service.sh logs

drive-forever:
    "${CARGO_HOME:-$HOME/.cargo}/bin/auto-runs" drive-forever

test:
    cargo test --release

proof-check:
    (cd proof && lake build)

proof-aeneas-check:
    (cd proof/aeneas-model && ../../../others/aeneas-tool/charon/bin/charon cargo --preset=aeneas --dest-file=../aeneas-check/auto_runs_aeneas_model.llbc)
    rm -f proof/aeneas-check/AutoRunsExtract/Generated/Funs.lean proof/aeneas-check/AutoRunsExtract/Generated/Types.lean proof/aeneas-check/AutoRunsExtract/Generated/FunsExternal_Template.lean
    ../others/aeneas-tool/bin/aeneas -backend lean proof/aeneas-check/auto_runs_aeneas_model.llbc -dest proof/aeneas-check -subdir /AutoRunsExtract/Generated -split-files -namespace AutoRunsExtract
    (cd proof/aeneas-check && lake build)

# Configuration for scaffold-based workspace bootstrapping (optional)
# Set these environment variables to point to your scaffold repositories:
#   AUTO_RUNS_RUST_SCAFFOLD_ROOT  - Path to a Rust project scaffold (for Rust repo configuration)
#   AUTO_RUNS_ZIG_SCAFFOLD_ROOT   - Path to a Zig project scaffold (for Zig repo configuration)
