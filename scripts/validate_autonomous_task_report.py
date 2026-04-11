#!/usr/bin/env python3
import argparse
import json
import sys
from pathlib import Path


STANDARD_REQUIRED_FIELDS = [
    "outcome",
    "summary",
    "details",
    "changed_files",
    "tests_run",
    "warnings",
    "blockers",
    "remaining_gaps",
    "design_requirements_satisfied",
]

REVIEW_REQUIRED_FIELDS = STANDARD_REQUIRED_FIELDS + ["fix_tasks"]
TYPED_ENVELOPE_REQUIRED_FIELDS = STANDARD_REQUIRED_FIELDS + [
    "payload_kind",
    "contract_version",
    "payload",
]
VALID_OUTCOMES = {
    "completed_with_changes",
    "completed_no_change",
    "blocked",
    "partial",
    "failed",
}
MIN_ESTIMATED_MINUTES = 1
MAX_ESTIMATED_MINUTES = 180
FIX_TASK_REQUIRED_FIELDS = [
    "title",
    "prompt",
    "kind",
    "estimated_minutes",
    "read_scope",
    "write_scope",
    "allowed_tools",
    "acceptance_checks",
]
PLAN_REQUIRED_FIELDS = [
    "title",
    "prompt",
    "kind",
    "estimated_minutes",
    "read_scope",
    "write_scope",
    "allowed_tools",
    "acceptance_checks",
]
VALID_PROVIDERS = {"claude", "codex", "minimax", "gemini", "glm", "qwen", "nemotron"}
VALID_PLAN_KINDS = {"read", "write", "edit"}
VALID_REVIEW_SEVERITIES = {"high", "medium", "low"}
FINDING_REQUIRED_FIELDS = [
    "severity",
    "confidence",
    "file",
    "line",
    "dimension",
    "description",
    "fix_suggestion",
]
PLACEHOLDER_MARKERS = [
    "not yet implemented",
    "todo!",
    "unimplemented!",
    "fixme",
    "placeholder",
]


def load_text(path: str | None) -> str:
    if path is None or path == "-":
        return sys.stdin.read()
    return Path(path).read_text()


def require_string_list(value, field_name: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise ValueError(f"`{field_name}` must be an array of strings")
    return value


def validate_provider(value: str, field_name: str) -> None:
    if value.strip().lower() not in VALID_PROVIDERS:
        accepted = ", ".join(sorted(VALID_PROVIDERS))
        raise ValueError(f"`{field_name}` must be one of: {accepted}")


def validate_fix_tasks(value) -> None:
    if not isinstance(value, list):
        raise ValueError("`fix_tasks` must be an array")
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            raise ValueError(f"`fix_tasks[{index}]` must be an object")
        for field in FIX_TASK_REQUIRED_FIELDS:
            if field not in item:
                raise ValueError(f"`fix_tasks[{index}]` is missing `{field}`")
        for field in ["title", "prompt", "kind"]:
            if not isinstance(item[field], str) or not item[field].strip():
                raise ValueError(f"`fix_tasks[{index}].{field}` must be a non-empty string")
        estimated_minutes = item["estimated_minutes"]
        if (
            not isinstance(estimated_minutes, int)
            or isinstance(estimated_minutes, bool)
            or estimated_minutes < MIN_ESTIMATED_MINUTES
            or estimated_minutes > MAX_ESTIMATED_MINUTES
        ):
            raise ValueError(
                f"`fix_tasks[{index}].estimated_minutes` must be an integer between "
                f"{MIN_ESTIMATED_MINUTES} and {MAX_ESTIMATED_MINUTES}"
            )
        provider = item.get("provider")
        if provider is not None:
            if not isinstance(provider, str) or not provider.strip():
                raise ValueError(f"`fix_tasks[{index}].provider` must be a non-empty string")
            validate_provider(provider, f"fix_tasks[{index}].provider")
        for field in ["read_scope", "write_scope", "allowed_tools", "acceptance_checks"]:
            require_string_list(item[field], f"fix_tasks[{index}].{field}")


def validate_findings(value) -> None:
    if not isinstance(value, list):
        raise ValueError("`findings` must be an array")
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            raise ValueError(f"`findings[{index}]` must be an object")
        for field in FINDING_REQUIRED_FIELDS:
            if field not in item:
                raise ValueError(f"`findings[{index}]` is missing `{field}`")
        if item["severity"] not in VALID_REVIEW_SEVERITIES:
            accepted = ", ".join(sorted(VALID_REVIEW_SEVERITIES))
            raise ValueError(f"`findings[{index}].severity` must be one of: {accepted}")
        for field in ["confidence", "file", "dimension", "description", "fix_suggestion"]:
            if not isinstance(item[field], str) or not item[field].strip():
                raise ValueError(f"`findings[{index}].{field}` must be a non-empty string")
        if not isinstance(item["line"], int) or item["line"] <= 0:
            raise ValueError(f"`findings[{index}].line` must be a positive integer")


def validate_workspace_outputs(report: dict, workspace_root: str | None) -> None:
    if not workspace_root:
        return

    root = Path(workspace_root)
    for changed_file in report.get("changed_files", []):
        if not changed_file.strip():
            continue
        candidate = Path(changed_file)
        if not candidate.is_absolute():
            candidate = root / changed_file
        if not candidate.exists():
            raise ValueError(f"changed file does not exist: {changed_file}")
        if not candidate.is_file():
            continue
        lowered = candidate.read_text(errors="ignore").lower()
        for marker in PLACEHOLDER_MARKERS:
            if marker in lowered:
                raise ValueError(
                    f"changed file '{changed_file}' still contains placeholder markers"
                )


def validate_report(report: dict, kind: str, allow_missing_changed_files: bool = False) -> None:
    required_fields = REVIEW_REQUIRED_FIELDS if kind == "review" else STANDARD_REQUIRED_FIELDS
    if allow_missing_changed_files:
        required_fields = [field for field in required_fields if field != "changed_files"]
    for field in required_fields:
        if field not in report:
            raise ValueError(f"missing required field `{field}`")

    outcome = report["outcome"]
    if not isinstance(outcome, str) or outcome not in VALID_OUTCOMES:
        raise ValueError(
            "`outcome` must be one of "
            + ", ".join(sorted(VALID_OUTCOMES))
        )

    for field in ["summary", "details"]:
        value = report[field]
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"`{field}` must be a non-empty string")

    for field in ["tests_run", "warnings", "blockers", "remaining_gaps", "design_requirements_satisfied"]:
        require_string_list(report[field], field)

    if allow_missing_changed_files and "changed_files" not in report:
        pass
    else:
        require_string_list(report["changed_files"], "changed_files")

    if kind == "review":
        validate_fix_tasks(report["fix_tasks"])


def validate_plan(plan) -> None:
    if not isinstance(plan, list) or not plan:
        raise ValueError("top-level JSON value must be a non-empty array")
    for index, item in enumerate(plan):
        if not isinstance(item, dict):
            raise ValueError(f"plan item {index} must be an object")
        for field in PLAN_REQUIRED_FIELDS:
            if field not in item:
                raise ValueError(f"plan item {index} is missing `{field}`")
        for field in ["title", "prompt", "kind"]:
            if not isinstance(item[field], str) or not item[field].strip():
                raise ValueError(f"plan item {index} field `{field}` must be a non-empty string")
        estimated_minutes = item["estimated_minutes"]
        if (
            not isinstance(estimated_minutes, int)
            or isinstance(estimated_minutes, bool)
            or estimated_minutes < MIN_ESTIMATED_MINUTES
            or estimated_minutes > MAX_ESTIMATED_MINUTES
        ):
            raise ValueError(
                f"plan item {index} field `estimated_minutes` must be an integer between "
                f"{MIN_ESTIMATED_MINUTES} and {MAX_ESTIMATED_MINUTES}"
            )
        provider = item.get("provider")
        if provider is not None:
            if not isinstance(provider, str) or not provider.strip():
                raise ValueError(f"plan item {index} field `provider` must be a non-empty string")
            validate_provider(provider, f"plan item {index} field `provider`")
        if item["kind"].strip().lower() not in VALID_PLAN_KINDS:
            raise ValueError(f"plan item {index} uses an invalid kind")
        for field in ["read_scope", "write_scope", "allowed_tools", "acceptance_checks"]:
            require_string_list(item[field], f"plan item {index} field `{field}`")
        if item["kind"].strip().lower() in {"write", "edit"}:
            write_scope = item["write_scope"]
            if not write_scope:
                raise ValueError(f"plan item {index} write task is missing `write_scope`")
            if len(write_scope) > 3:
                raise ValueError(f"plan item {index} write task has too many `write_scope` entries")
            if any(scope.strip() in {".", "**"} for scope in write_scope):
                raise ValueError(f"plan item {index} write task uses a broad `write_scope`")
            if len(item["acceptance_checks"]) < 2:
                raise ValueError(
                    f"plan item {index} write task must include at least 2 acceptance checks"
                )


def validate_typed_plan_payload(payload) -> None:
    if not isinstance(payload, dict):
        raise ValueError("`payload` must be an object")
    if "generated_tasks" not in payload:
        raise ValueError("typed plan payload is missing `generated_tasks`")
    validate_plan(payload["generated_tasks"])


def validate_typed_review_payload(payload) -> None:
    if not isinstance(payload, dict):
        raise ValueError("`payload` must be an object")
    if "findings" not in payload:
        raise ValueError("typed review payload is missing `findings`")
    if "fix_tasks" not in payload:
        raise ValueError("typed review payload is missing `fix_tasks`")
    validate_findings(payload["findings"])
    validate_fix_tasks(payload["fix_tasks"])


def validate_typed_design_doc_payload(payload) -> None:
    if not isinstance(payload, dict):
        raise ValueError("`payload` must be an object")
    markdown = payload.get("markdown")
    if not isinstance(markdown, str) or not markdown.strip():
        raise ValueError("typed design-doc payload requires non-empty `markdown`")


def validate_typed_perf_parse_payload(payload) -> None:
    if not isinstance(payload, dict):
        raise ValueError("`payload` must be an object")
    ideas = payload.get("ideas")
    if not isinstance(ideas, list) or not ideas:
        raise ValueError("typed perf-parse payload requires non-empty `ideas`")
    for index, idea in enumerate(ideas):
        if not isinstance(idea, dict):
            raise ValueError(f"`ideas[{index}]` must be an object")
        for field in ["index", "title", "description"]:
            if field not in idea:
                raise ValueError(f"`ideas[{index}]` is missing `{field}`")
        if not isinstance(idea["index"], int) or idea["index"] < 0:
            raise ValueError(f"`ideas[{index}].index` must be a non-negative integer")
        for field in ["title", "description"]:
            if not isinstance(idea[field], str) or not idea[field].strip():
                raise ValueError(f"`ideas[{index}].{field}` must be a non-empty string")


def validate_typed_envelope(
    report: dict, kind: str, allow_missing_changed_files: bool = False
) -> None:
    required_fields = TYPED_ENVELOPE_REQUIRED_FIELDS
    if allow_missing_changed_files:
        required_fields = [field for field in required_fields if field != "changed_files"]
    for field in required_fields:
        if field not in report:
            raise ValueError(f"typed envelope is missing `{field}`")
    validate_report(report, "standard", allow_missing_changed_files)
    if not isinstance(report["contract_version"], int) or report["contract_version"] <= 0:
        raise ValueError("`contract_version` must be a positive integer")
    payload_kind = report["payload_kind"]
    if not isinstance(payload_kind, str) or not payload_kind.strip():
        raise ValueError("`payload_kind` must be a non-empty string")
    payload = report["payload"]
    if kind == "plan":
        if payload_kind != "plan_tasks":
            raise ValueError("typed plan payload_kind must be `plan_tasks`")
        validate_typed_plan_payload(payload)
    elif kind == "review":
        if payload_kind != "review_findings":
            raise ValueError("typed review payload_kind must be `review_findings`")
        validate_typed_review_payload(payload)
    elif kind == "perf_parse":
        if payload_kind != "perf_ideas":
            raise ValueError("typed perf-parse payload_kind must be `perf_ideas`")
        validate_typed_perf_parse_payload(payload)
    elif payload_kind == "design_doc":
        validate_typed_design_doc_payload(payload)


def validate_ralph_verify(result) -> None:
    if not isinstance(result, dict):
        raise ValueError("top-level JSON value must be an object")
    if "complete" not in result or "reason" not in result:
        raise ValueError("Ralph verify output must contain `complete` and `reason`")
    if not isinstance(result["complete"], bool):
        raise ValueError("`complete` must be a boolean")
    if not isinstance(result["reason"], str) or not result["reason"].strip():
        raise ValueError("`reason` must be a non-empty string")


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate autonomous task execution report JSON."
    )
    parser.add_argument(
        "--kind",
        choices=["standard", "review", "plan", "perf_parse", "ralph_verify"],
        default="standard",
        help="Expected task report schema.",
    )
    parser.add_argument(
        "--workspace-root",
        help="Optional workspace root used to validate changed_files and placeholder markers.",
    )
    parser.add_argument(
        "--allow-missing-changed-files",
        action="store_true",
        help="Allow reports that omit changed_files entirely.",
    )
    parser.add_argument(
        "path",
        nargs="?",
        help="Path to the JSON file to validate, or omit / use - for stdin.",
    )
    args = parser.parse_args()

    try:
        raw = load_text(args.path)
        parsed = json.loads(raw)
        if args.kind == "plan":
            if isinstance(parsed, dict) and "payload_kind" in parsed:
                validate_typed_envelope(
                    parsed, args.kind, args.allow_missing_changed_files
                )
                validate_workspace_outputs(parsed, args.workspace_root)
            else:
                validate_plan(parsed)
        elif args.kind == "perf_parse":
            if not isinstance(parsed, dict):
                raise ValueError("top-level JSON value must be an object")
            if "payload_kind" in parsed:
                validate_typed_envelope(parsed, args.kind)
            else:
                validate_typed_perf_parse_payload(parsed)
        elif args.kind == "ralph_verify":
            validate_ralph_verify(parsed)
        else:
            if not isinstance(parsed, dict):
                raise ValueError("top-level JSON value must be an object")
            if "payload_kind" in parsed:
                validate_typed_envelope(
                    parsed, args.kind, args.allow_missing_changed_files
                )
            else:
                validate_report(parsed, args.kind, args.allow_missing_changed_files)
            validate_workspace_outputs(parsed, args.workspace_root)
    except Exception as exc:  # noqa: BLE001
        print(f"INVALID: {exc}", file=sys.stderr)
        if args.kind == "ralph_verify":
            print(
                '\nExpected format:\n{"complete": true, "reason": "All criteria satisfied."}',
                file=sys.stderr,
            )
        return 1

    print(f"VALID: {args.kind} task execution report")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
