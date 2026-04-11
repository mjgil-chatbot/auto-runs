#!/usr/bin/env python3
import argparse
import fnmatch
import subprocess
import sys
import tomllib
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the engineering evidence gate for staged or explicit files."
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=None,
        help="Repository root. Defaults to git rev-parse --show-toplevel.",
    )
    parser.add_argument(
        "--registry",
        type=Path,
        default=None,
        help="Registry path. Defaults to contracts/engineering-gate.toml under the repo root.",
    )
    parser.add_argument(
        "--files",
        nargs="*",
        default=None,
        help="Explicit changed files relative to the repo root. Bypasses git staged-file discovery.",
    )
    parser.add_argument(
        "--diff-base",
        default=None,
        help="Git revision to diff from instead of using staged-file discovery.",
    )
    parser.add_argument(
        "--diff-head",
        default="HEAD",
        help="Git revision to diff to when --diff-base is used. Defaults to HEAD.",
    )
    parser.add_argument(
        "--run-group",
        action="append",
        default=None,
        help="Run a named command group from the registry without staged-file discovery.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print the active contracts and commands without executing them.",
    )
    return parser.parse_args()


def resolve_repo_root(explicit: Path | None) -> Path:
    if explicit is not None:
        return explicit.resolve()
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(result.stdout.strip()).resolve()


def load_registry(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def validate_repo_identity(repo_root: Path, registry: dict) -> str:
    repo_id = str(registry.get("repo_id", repo_root.name))
    expected_roots = [str(name) for name in registry.get("repo_root_names", []) if str(name).strip()]
    if expected_roots and repo_root.name not in expected_roots:
        expected = ", ".join(expected_roots)
        raise SystemExit(
            f"engineering-gate: FAIL: registry for {repo_id} cannot run from repo root "
            f"{repo_root.name!r}; expected one of [{expected}]"
        )
    return repo_id


def staged_files(repo_root: Path) -> list[str]:
    result = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR"],
        check=True,
        cwd=repo_root,
        capture_output=True,
        text=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def diff_files(repo_root: Path, base: str, head: str) -> list[str]:
    result = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=ACMR", base, head],
        check=True,
        cwd=repo_root,
        capture_output=True,
        text=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def matches_any(path: str, patterns: list[str]) -> bool:
    return any(fnmatch.fnmatch(path, pattern) for pattern in patterns)


def matches_any_of_changed_files(changed_files: list[str], patterns: list[str]) -> bool:
    return any(matches_any(changed_file, patterns) for changed_file in changed_files)


def normalize_files(files: list[str] | None) -> list[str]:
    if not files:
        return []
    return [file.replace("\\", "/") for file in files if file]


def command_group_map(registry: dict) -> dict[str, dict]:
    groups: dict[str, dict] = {}
    for group in registry.get("command_group", []):
        group_id = str(group.get("id", "")).strip()
        if not group_id:
            raise SystemExit("engineering-gate: FAIL: command_group entry is missing a non-empty id")
        groups[group_id] = group
    return groups


def append_unique_commands(target: list[str], additions: list[str]) -> None:
    for command in additions:
        if command not in target:
            target.append(command)


def resolve_contract_commands(
    contract: dict,
    groups: dict[str, dict],
    failures: list[str],
) -> list[str]:
    resolved: list[str] = []
    append_unique_commands(resolved, [str(command) for command in contract.get("commands", [])])
    for group_id in contract.get("command_groups", []):
        group = groups.get(str(group_id))
        if group is None:
            failures.append(
                f"{contract.get('id', '<unknown>')}: unknown command group {group_id!r}"
            )
            continue
        append_unique_commands(resolved, [str(command) for command in group.get("commands", [])])
    return resolved


def resolve_named_groups(
    group_ids: list[str],
    groups: dict[str, dict],
) -> tuple[list[str], list[str]]:
    commands: list[str] = []
    failures: list[str] = []
    for group_id in group_ids:
        group = groups.get(group_id)
        if group is None:
            failures.append(f"unknown command group {group_id!r}")
            continue
        append_unique_commands(commands, [str(command) for command in group.get("commands", [])])
    return commands, failures


def execute_commands(repo_root: Path, repo_id: str, commands: list[str], dry_run: bool) -> int:
    if not commands:
        print(f"engineering-gate[{repo_id}]: no commands registered for active selection")
        return 0

    print(f"engineering-gate[{repo_id}]: commands:")
    for command in commands:
        print(f"  - {command}")

    if dry_run:
        return 0

    for command in commands:
        result = subprocess.run(command, cwd=repo_root, shell=True)
        if result.returncode != 0:
            print(
                f"engineering-gate: FAIL: command exited with status {result.returncode}: {command}",
                file=sys.stderr,
            )
            return result.returncode

    print(f"engineering-gate[{repo_id}]: PASS")
    return 0


def main() -> int:
    args = parse_args()
    if args.files is not None and args.diff_base is not None:
        raise SystemExit("engineering-gate: FAIL: --files and --diff-base are mutually exclusive")
    if args.run_group and (args.files is not None or args.diff_base is not None):
        raise SystemExit(
            "engineering-gate: FAIL: --run-group cannot be combined with --files or --diff-base"
        )

    repo_root = resolve_repo_root(args.repo_root)
    registry_path = (
        args.registry.resolve()
        if args.registry is not None
        else repo_root / "contracts" / "engineering-gate.toml"
    )
    registry = load_registry(registry_path)
    repo_id = validate_repo_identity(repo_root, registry)
    groups = command_group_map(registry)

    if args.run_group:
        commands, failures = resolve_named_groups([str(group) for group in args.run_group], groups)
        if failures:
            for failure in failures:
                print(f"engineering-gate: FAIL: {failure}", file=sys.stderr)
            return 1
        print(f"engineering-gate[{repo_id}]: active command groups:")
        for group_id in args.run_group:
            print(f"  - {group_id}")
        return execute_commands(repo_root, repo_id, commands, args.dry_run)

    changed_files = normalize_files(args.files)
    if args.diff_base is not None:
        changed_files = normalize_files(diff_files(repo_root, args.diff_base, args.diff_head))
    elif args.files is None:
        changed_files = staged_files(repo_root)

    if not changed_files:
        print(f"engineering-gate[{repo_id}]: no changed files; nothing to do")
        return 0

    active_contracts: list[dict] = []
    commands: list[str] = []
    failures: list[str] = []
    for contract in registry.get("contract", []):
        trigger_globs = contract.get("trigger_globs", [])
        if not matches_any_of_changed_files(changed_files, trigger_globs):
            continue

        active_contracts.append(contract)
        required_any = contract.get("required_any_staged", [])
        if required_any and not any(required in changed_files for required in required_any):
            failures.append(
                f"{contract['id']}: staged change touched protected paths but did not stage any required documentation or verification file from {required_any}"
            )

        append_unique_commands(commands, resolve_contract_commands(contract, groups, failures))

    if not active_contracts:
        print(f"engineering-gate[{repo_id}]: no active contracts for changed files")
        return 0

    print(f"engineering-gate[{repo_id}]: active contracts:")
    for contract in active_contracts:
        print(f"  - {contract['id']}: {contract.get('description', '').strip()}")

    if failures:
        for failure in failures:
            print(f"engineering-gate: FAIL: {failure}", file=sys.stderr)
        return 1

    return execute_commands(repo_root, repo_id, commands, args.dry_run)


if __name__ == "__main__":
    raise SystemExit(main())
