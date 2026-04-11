import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
HOOK = ROOT / ".githooks" / "pre-commit"
SCRIPT = ROOT / "scripts" / "verify-engineering-gate.sh"
PY_SCRIPT = ROOT / "scripts" / "verify_engineering_gate.py"
COORDINATION_SCRIPT = ROOT / "scripts" / "verify-coordination-gate.sh"
REGISTRY = ROOT / "contracts" / "engineering-gate.toml"
WORKFLOW = ROOT / ".github" / "workflows" / "engineering-gate.yml"


class EngineeringGateTests(unittest.TestCase):
    def test_shell_scripts_parse(self) -> None:
        result = subprocess.run(
            ["bash", "-n", str(HOOK), str(SCRIPT), str(COORDINATION_SCRIPT)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_registry_declares_repo_local_contracts(self) -> None:
        content = REGISTRY.read_text()
        self.assertIn('repo_id = "auto-runs"', content)
        self.assertIn('id = "workflow_registry_checks"', content)
        self.assertIn('id = "coordination_runtime_checks"', content)
        self.assertIn('id = "engineering_docs"', content)
        self.assertIn('id = "workflow_registry"', content)
        self.assertIn('id = "coordination_runtime"', content)

    def test_hook_runs_engineering_gate_script(self) -> None:
        content = HOOK.read_text()
        self.assertIn('echo "pre-commit: running engineering gate"', content)
        self.assertIn('"$repo_root/scripts/verify-engineering-gate.sh"', content)

    def test_coordination_wrapper_uses_isolated_target_dir(self) -> None:
        content = COORDINATION_SCRIPT.read_text()
        self.assertIn('TARGET_SUFFIX="${ENGINEERING_GATE_TARGET_SUFFIX:-${USER:-uid$(id -u)}}"', content)
        self.assertIn(
            'TARGET_DIR="${CARGO_TARGET_DIR:-${APP_ROOT}/target/engineering-gate-local-${TARGET_SUFFIX}}"',
            content,
        )
        self.assertIn('--run-group workflow_registry_checks', content)
        self.assertIn('--run-group coordination_runtime_checks', content)

    def test_ci_workflow_runs_engineering_gate_with_diff_base(self) -> None:
        content = WORKFLOW.read_text()
        self.assertIn("name: Engineering Gate", content)
        self.assertIn("fetch-depth: 0", content)
        self.assertIn("scripts/verify-engineering-gate.sh", content)
        self.assertIn("--diff-base", content)
        self.assertIn("ENGINEERING_GATE_TARGET_SUFFIX: ci", content)

    def test_gate_fails_when_coordination_changes_without_docs(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(PY_SCRIPT),
                "--repo-root",
                str(ROOT),
                "--files",
                "src/coordination/mod.rs",
                "--dry-run",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("coordination_runtime", result.stderr)
        self.assertIn("required documentation", result.stderr)

    def test_gate_passes_when_workflow_and_docs_change_together(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(PY_SCRIPT),
                "--repo-root",
                str(ROOT),
                "--files",
                "src/workflow.rs",
                "docs/verification.md",
                "--dry-run",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("workflow_registry", result.stdout)
        self.assertIn("cargo test --release --test workflow_contracts", result.stdout)

    def test_gate_can_run_named_command_group(self) -> None:
        result = subprocess.run(
            [
                "python3",
                str(PY_SCRIPT),
                "--repo-root",
                str(ROOT),
                "--run-group",
                "workflow_registry_checks",
                "--dry-run",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("active command groups", result.stdout)
        self.assertIn("workflow_registry_checks", result.stdout)
        self.assertIn("cargo test --release --test workflow_contracts", result.stdout)

    def test_gate_uses_real_staged_file_discovery(self) -> None:
        with tempfile.TemporaryDirectory(prefix="auto-runs-stage-") as tmpdir:
            repo_root = Path(tmpdir) / "auto-runs"
            (repo_root / "contracts").mkdir(parents=True)
            (repo_root / "src").mkdir()
            (repo_root / "docs").mkdir()
            (repo_root / "src" / "workflow.rs").write_text("pub fn workflow_specs() {}\n")
            (repo_root / "docs" / "verification.md").write_text("# verification\n")
            (repo_root / "contracts" / "engineering-gate.toml").write_text(
                textwrap.dedent(
                    """
                    version = 1
                    repo_id = "auto-runs"
                    repo_root_names = ["auto-runs"]

                    [[contract]]
                    id = "staged_contract"
                    description = "staged file discovery coverage"
                    trigger_globs = ["src/*.rs", "src/**/*.rs"]
                    required_any_staged = ["docs/verification.md"]
                    commands = ['python3 -c "print(\\"ok\\")"']
                    """
                ).strip()
                + "\n",
                encoding="utf-8",
            )

            subprocess.run(["git", "init"], cwd=repo_root, check=True, capture_output=True, text=True)
            subprocess.run(
                ["git", "add", "src/workflow.rs", "docs/verification.md", "contracts/engineering-gate.toml"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )

            result = subprocess.run(
                [
                    "python3",
                    str(PY_SCRIPT),
                    "--repo-root",
                    str(repo_root),
                    "--registry",
                    str(repo_root / "contracts" / "engineering-gate.toml"),
                    "--dry-run",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("engineering-gate[auto-runs]: active contracts", result.stdout)
            self.assertIn("staged_contract", result.stdout)

    def test_gate_uses_diff_range_file_discovery(self) -> None:
        with tempfile.TemporaryDirectory(prefix="auto-runs-diff-") as tmpdir:
            repo_root = Path(tmpdir) / "auto-runs"
            (repo_root / "contracts").mkdir(parents=True)
            (repo_root / "src").mkdir()
            (repo_root / "docs").mkdir()
            (repo_root / "src" / "workflow.rs").write_text("pub fn workflow_specs() {}\n")
            (repo_root / "docs" / "verification.md").write_text("# verification\n")
            (repo_root / "contracts" / "engineering-gate.toml").write_text(
                textwrap.dedent(
                    """
                    version = 1
                    repo_id = "auto-runs"
                    repo_root_names = ["auto-runs"]

                    [[contract]]
                    id = "diff_contract"
                    description = "diff discovery coverage"
                    trigger_globs = ["src/*.rs", "src/**/*.rs"]
                    required_any_staged = ["docs/verification.md"]
                    commands = ['python3 -c "print(\\"ok\\")"']
                    """
                ).strip()
                + "\n",
                encoding="utf-8",
            )

            subprocess.run(["git", "init"], cwd=repo_root, check=True, capture_output=True, text=True)
            subprocess.run(
                ["git", "config", "user.name", "Auto Runs Test"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                ["git", "config", "user.email", "auto-runs-test@example.com"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                ["git", "add", "."],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                ["git", "commit", "-m", "base"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )

            (repo_root / "src" / "workflow.rs").write_text("pub fn workflow_specs() { println!(\"changed\"); }\n")
            (repo_root / "docs" / "verification.md").write_text("# verification\nchanged\n")
            subprocess.run(
                ["git", "add", "src/workflow.rs", "docs/verification.md"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                ["git", "commit", "-m", "head"],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
            )

            result = subprocess.run(
                [
                    "python3",
                    str(PY_SCRIPT),
                    "--repo-root",
                    str(repo_root),
                    "--registry",
                    str(repo_root / "contracts" / "engineering-gate.toml"),
                    "--diff-base",
                    "HEAD~1",
                    "--diff-head",
                    "HEAD",
                    "--dry-run",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("engineering-gate[auto-runs]: active contracts", result.stdout)
            self.assertIn("diff_contract", result.stdout)

    def test_gate_rejects_registry_for_different_repo_root(self) -> None:
        with tempfile.TemporaryDirectory(prefix="auto-runs-other-repo-") as tmpdir:
            repo_root = Path(tmpdir) / "other-repo"
            repo_root.mkdir()
            result = subprocess.run(
                [
                    "python3",
                    str(PY_SCRIPT),
                    "--repo-root",
                    str(repo_root),
                    "--registry",
                    str(REGISTRY),
                    "--dry-run",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(
                "registry for auto-runs cannot run from repo root 'other-repo'",
                result.stderr,
            )


if __name__ == "__main__":
    unittest.main()
