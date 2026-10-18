"""Offline CI wiring contracts using a deliberately narrow stdlib YAML reader."""

import ast
import json
from pathlib import Path
import re
import shlex
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/eval.yml"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def validate_workflow(text):
    # This checks our committed layout, not arbitrary YAML. Reject alternate forms
    # instead of silently overlooking executable or conditional configuration.
    lines = [
        line.rstrip()
        for line in text.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    normalized = "\n".join(lines)
    for forbidden in (
        "schedule:",
        "services:",
        "continue-on-error:",
        "secrets.",
        "TYPESAFE_API_KEY",
        "pull_request_target:",
    ):
        require(
            forbidden not in normalized, "forbidden workflow capability: " + forbidden
        )
    require(
        not re.search(r"(^|\s)[&*][A-Za-z_]", normalized),
        "YAML anchors/aliases unsupported",
    )
    require(
        not re.search(r"^    if:", normalized, re.MULTILINE),
        "job must not be conditional",
    )
    trigger = re.search(r"^on:\n((?: .*(?:\n|$))+)", normalized, re.MULTILINE)
    require(trigger is not None, "missing trigger mapping")
    names = re.findall(r"^  ([a-z_]+):", trigger.group(1), re.MULTILINE)
    require(
        set(names) == {"push", "pull_request", "workflow_dispatch"},
        "offline trigger coverage changed",
    )
    require(
        "          - os: ubuntu-latest\n            executable: cortex" in normalized,
        "Ubuntu release matrix missing",
    )
    require(
        "          - os: windows-latest\n            executable: cortex.exe"
        in normalized,
        "Windows release matrix missing",
    )
    require("    runs-on: ${{ matrix.os }}" in lines, "matrix runner not used")
    require(
        "        shell: bash" in lines, "cross-platform bash command contract missing"
    )
    require(
        "      CORTEX_EVAL_BINARY: ${{ github.workspace }}/target/release/${{ matrix.executable }}"
        in lines,
        "real release binary not configured",
    )
    require("  contents: read" in lines, "read-only permission missing")
    require(
        not re.search(r"\b(write|write-all)\s*$", normalized, re.MULTILINE),
        "write permissions not needed",
    )
    step_starts = [
        index for index, line in enumerate(lines) if line.startswith("      - ")
    ]
    require(step_starts, "steps missing")
    steps = [
        "\n".join(lines[start:end])
        for start, end in zip(step_starts, step_starts[1:] + [len(lines)])
    ]
    runs = []
    artifacts = []
    for step in steps:
        if "uses: actions/upload-artifact@" in step:
            artifacts.append(step)
            continue
        require(
            not re.search(r"^        if:", step, re.MULTILINE),
            "execution step must not be conditional",
        )
        run = re.search(r"^        run: (.*)$", step, re.MULTILINE)
        if run:
            command = run.group(1)
            if command == "|":
                command = "\n".join(
                    line[10:]
                    for line in step.splitlines()
                    if line.startswith("          ")
                )
            command = command.replace("\\\n", " ")
            require(
                not any(
                    token in command
                    for token in (
                        "||",
                        "&&",
                        ";",
                        "exit 0",
                        "set +e",
                        "continue-on-error",
                        "curl ",
                        "docker ",
                        "nohup ",
                        "&\n",
                    )
                ),
                "command can bypass failure or start services",
            )
            require(
                not re.search(r"python\s+scripts/evaluate_jev_live\.py", command),
                "live provider runner must never execute in CI",
            )
            runs.append(shlex.split(command))
    require(
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "-p",
            "cortex-local",
            "--bin",
            "cortex",
        ]
        in runs,
        "standalone release build missing",
    )
    require(
        [
            "cargo",
            "test",
            "--locked",
            "-p",
            "cortex-local",
            "--test",
            "classification_eval",
        ]
        in runs,
        "offline typed classification tests missing",
    )
    require(
        [
            "python",
            "-W",
            "error::ResourceWarning",
            "-m",
            "unittest",
            "discover",
            "-s",
            "scripts/tests",
            "-p",
            "test_*eval*.py",
        ]
        in runs,
        "real-binary Python contract suite missing",
    )
    gate = [
        command
        for command in runs
        if command[:2] == ["python", "scripts/gate_local_memory.py"]
    ]
    require(len(gate) == 1, "exactly one fresh gate command required")
    require(
        gate[0]
        == [
            "python",
            "scripts/gate_local_memory.py",
            "--binary",
            "$CORTEX_EVAL_BINARY",
            "--report",
            "scripts/eval-ci-report.json",
        ],
        "gate must use fresh reports and default frozen policy",
    )
    require(len(artifacts) == 1, "one evidence artifact upload required")
    artifact = artifacts[0]
    require(
        "        if: always()" in artifact, "failed evidence upload must always run"
    )
    require(
        "          path: scripts/eval-ci-report.json" in artifact,
        "artifact path differs from gate output",
    )
    require(
        "${{ matrix.os }}" in artifact and "${{ github.run_attempt }}" in artifact,
        "artifact names can collide across runners/retries",
    )
    return runs


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_committed_workflow_uses_real_release_and_fresh_offline_gate(self):
        validate_workflow(self.workflow)

    def test_schedules_services_credentials_and_allow_failure_rejected(self):
        for injection in (
            "  schedule:\n",
            "    services:\n",
            "        continue-on-error: true\n",
            "      TYPESAFE_API_KEY: ${{ secrets.KEY }}\n",
        ):
            with self.subTest(injection=injection):
                with self.assertRaises(AssertionError):
                    validate_workflow(self.workflow + injection)

    def test_gate_cannot_reuse_an_old_report_or_override_policy(self):
        for option in (
            "--quality-report old.json",
            "--latency-report old.json",
            "--baseline other.json",
            "|| true",
        ):
            changed = self.workflow.replace(
                "--report scripts/eval-ci-report.json",
                "--report scripts/eval-ci-report.json " + option,
            )
            with self.subTest(option=option), self.assertRaises(AssertionError):
                validate_workflow(changed)

    def test_failed_evidence_upload_cannot_be_success_only(self):
        with self.assertRaisesRegex(AssertionError, "always"):
            validate_workflow(self.workflow.replace("if: always()", "if: success()"))

    def test_pull_requests_and_windows_cannot_be_silently_removed(self):
        for changed in (
            self.workflow.replace("  pull_request:", "  ignored:"),
            self.workflow.replace("windows-latest", "ubuntu-24.04"),
        ):
            with self.assertRaises(AssertionError):
                validate_workflow(changed)

    def test_gate_cannot_be_skipped_by_branch_condition(self):
        changed = self.workflow.replace(
            "      - name: Run fresh offline",
            "        if: github.ref == 'refs/heads/main'\n      - name: Run fresh offline",
        )
        with self.assertRaisesRegex(AssertionError, "conditional"):
            validate_workflow(changed)

    def test_live_runner_cannot_be_added_as_execution_step(self):
        with self.assertRaisesRegex(AssertionError, "live provider"):
            validate_workflow(
                self.workflow
                + "\n      - name: Forbidden\n        run: python scripts/evaluate_jev_live.py --apply\n"
            )

    def test_default_frozen_policy_requests_twenty_fresh_latency_samples(self):
        policy = json.loads(
            (ROOT / "crates/cortex-local/baselines/local-eval-v1.json").read_text()
        )
        self.assertEqual(policy["latency"]["min_samples"], 20)
        tree = ast.parse((ROOT / "scripts/gate_local_memory.py").read_text())
        default = next(
            node.value
            for node in ast.walk(tree)
            if isinstance(node, ast.Assign)
            and any(
                isinstance(target, ast.Name) and target.id == "DEFAULT_BASELINE"
                for target in node.targets
            )
        )
        self.assertIn(
            "crates/cortex-local/baselines/local-eval-v1.json", ast.unparse(default)
        )
        baseline_argument = next(
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "add_argument"
            and node.args
            and isinstance(node.args[0], ast.Constant)
            and node.args[0].value == "--baseline"
        )
        self.assertEqual(
            ast.unparse(
                next(
                    keyword.value
                    for keyword in baseline_argument.keywords
                    if keyword.arg == "default"
                )
            ),
            "DEFAULT_BASELINE",
        )
        calls = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            and ast.unparse(node.func) == "latency.evaluate"
        ]
        self.assertEqual(len(calls), 1)
        samples = next(
            keyword.value for keyword in calls[0].keywords if keyword.arg == "samples"
        )
        self.assertEqual(ast.unparse(samples), "policy['latency']['min_samples']")


if __name__ == "__main__":
    unittest.main()
