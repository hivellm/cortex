"""Preview production Jev requests; send only with explicit reviewed-hash opt-in."""

from __future__ import annotations

import argparse
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal
import json
import math
import os
from pathlib import Path
import tempfile

if __package__:
    from .evaluate_local_memory import (
        CortexCli,
        canonical,
        digest,
        require,
    )
else:
    from evaluate_local_memory import (
        CortexCli,
        canonical,
        digest,
        require,
    )


FIXTURES = (
    Path(__file__).resolve().parents[1]
    / "crates/cortex-local/tests/fixtures/classification-eval-v1"
)
MODEL = "jev-1.13.0"
CATEGORIES = ("decision", "lesson", "preference", "fact", "transient", "unknown")
PRICE_DATE = date(2026, 10, 18)
PRICE_SOURCE = "https://docs.typesafe.ai/models"
INPUT_USD_PER_MILLION = Decimal("0.042")
CONTEXT_TOKENS = 65536
RESERVE = Decimal(CONTEXT_TOKENS) * INPUT_USD_PER_MILLION / Decimal(1_000_000)


class LiveCli(CortexCli):
    """Only this explicitly selected runner supplies the provider credential."""

    def __init__(self, binary, home, key):
        super().__init__(binary, home, timeout=15)
        self._key = key

    def environment(self):
        env = super().environment()
        env["TYPESAFE_API_KEY"] = self._key
        return env


def policy(today):
    return {
        "valid_on_run_date": PRICE_DATE <= today <= PRICE_DATE + timedelta(days=30),
        "model": MODEL,
        "price_source": PRICE_SOURCE,
        "price_as_of": PRICE_DATE.isoformat(),
        "price_expires": (PRICE_DATE + timedelta(days=30)).isoformat(),
        "input_usd_per_million": str(INPUT_USD_PER_MILLION),
        "output_usd_per_million": "0",
        "context_tokens_assumed_maximum": CONTEXT_TOKENS,
        "reserved_usd_per_attempt": str(RESERVE),
        "assumptions": "Default 64k context, no long-context header, output free. Client admission guard under declared prices; no native provider billing cap. Unknown usage retains the full reservation.",
        "max_cases": 6,
        "max_attempts_per_case": 1,
        "provider_timeout_seconds": 10,
        "drain_deadline_seconds": 12,
        "subprocess_timeout_seconds": 15,
    }


def load_inputs(path):
    raw = (path / "cases.json").read_bytes()
    require(len(raw) <= 256 * 1024, "Fixture exceeds bound")
    fixture = json.loads(raw)
    require(
        fixture["schema_version"] == 1 and fixture["license"] == "CC0-1.0",
        "Unsupported fixture",
    )
    cases = fixture["cases"]
    require(1 <= len(cases) <= 6, "Expected one to six reviewed cases")
    require(len({c["id"] for c in cases}) == len(cases), "Duplicate case ID")
    for case in cases:
        require(case["expected_category"] in CATEGORIES, "Invalid judgment")
    questions_bytes = (path / "questions.json").read_bytes()
    require(len(questions_bytes) <= 24 * 1024, "Questions exceed bound")
    return cases, json.loads(questions_bytes), digest(raw), digest(questions_bytes)


def prepare_case(binary, root, case, questions):
    root.mkdir()
    cli = CortexCli(binary, root / "home", timeout=15)
    request = {
        "id": case["id"],
        "repository": case["repository"],
        "kind": "note",
        "authority": case["authority"],
        "title": case["title"],
        "body": case["body"],
        "source": {
            "kind": "public_fixture",
            "locator": "fiction://classification/" + case["id"],
            "source_id": case["id"],
            "captured_at": "2026-02-03T09:00:00Z",
            "source_hash": digest(
                canonical({k: case[k] for k in ("title", "body", "authority")})
            ),
            "payload": {"license": "CC0-1.0"},
        },
        "links": [],
        "metadata": {},
    }
    input_path = root / "capture.json"
    input_path.write_bytes(canonical(request))
    captured = cli.run("capture", "--input", input_path)
    require(captured.get("created") is True, "Fresh capture failed")
    record = captured["record"]
    job = cli.run(
        "--repository",
        case["repository"],
        "classify",
        "enqueue",
        case["id"],
        "--model",
        MODEL,
    )
    prepared = job["prepared"]
    require(job["status"] == "pending" and job["attempts"] == 0, "Fresh queue required")
    require(
        prepared["schema_version"] == 1 and prepared["requested_model"] == MODEL,
        "Production classification schema/model mismatch",
    )
    require(
        (
            prepared["repository"],
            prepared["record_id"],
            prepared["revision"],
            prepared["content_sha256"],
        )
        == (
            record["repository"],
            record["id"],
            record["revision"],
            record["content_sha256"],
        ),
        "Prepared source identity differs",
    )
    body = {"state": prepared["state"], "model": MODEL, "questions": questions}
    require(len(canonical(body)) <= 24 * 1024, "Request exceeds production bound")
    require(
        digest(canonical(body)) == prepared["request_sha256"],
        "Questions differ from production request",
    )
    return cli, record, job, body


def validate_advice(advice):
    require(
        advice["model"] == MODEL and advice["category"] in CATEGORIES,
        "Invalid advice model/category",
    )
    probabilities = advice["probabilities"]
    require(set(probabilities) == set(CATEGORIES), "Invalid probability categories")
    values = list(probabilities.values()) + [
        advice["confidence"],
        advice["durable_probability"],
    ]
    require(
        all(
            type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1 for v in values
        ),
        "Invalid probabilities",
    )
    require(abs(sum(probabilities.values()) - 1) <= 1e-5, "Invalid probability sum")
    require(
        probabilities[advice["category"]] + 1e-10 >= max(probabilities.values()),
        "Invalid choice",
    )
    usage = advice["usage"]
    require(
        all(
            type(usage[k]) is int and usage[k] >= 0
            for k in ("input_tokens", "output_tokens")
        ),
        "Invalid usage",
    )


def scrub(value, secret):
    if isinstance(value, str):
        return value.replace(secret, "[REDACTED]") if secret else value
    if isinstance(value, dict):
        return {scrub(k, secret): scrub(v, secret) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v, secret) for v in value]
    return value


def evaluate(
    binary,
    report_path,
    fixtures=FIXTURES,
    *,
    live=False,
    reviewed_payload_sha256=None,
    budget_usd=None,
    today=None,
):
    secret = os.environ.get("TYPESAFE_API_KEY", "")
    report = {
        "schema_version": 1,
        "mode": "live" if live else "preview",
        "success": False,
        "requests": [],
        "cases": [],
        "attempted": 0,
        "success_scope": "Execution and contract only; no live quality floor established. Unknown is a valid category, not proof of known usage.",
        "metric_scope": "Small reviewed fixture only; synthetic responses and labels are never sent.",
    }
    try:
        report["policy"] = policy(today or datetime.now(timezone.utc).date())
        require(
            not live or report["policy"]["valid_on_run_date"],
            "Pricing expired; review official prices and update policy before live use",
        )
        cases, questions, fixture_hash, questions_hash = load_inputs(Path(fixtures))
        binary = Path(binary).resolve()
        binary_sha256 = digest(binary.read_bytes())
        report["binary_sha256"] = binary_sha256
        report.update(
            fixture_sha256=fixture_hash,
            questions_sha256=questions_hash,
            required_reserved_usd=str(RESERVE * len(cases)),
        )
        with tempfile.TemporaryDirectory(prefix="cortex-jev-eval-") as scratch:
            prepared_cases = []
            for ordinal, case in enumerate(cases):
                prepared_cases.append(
                    prepare_case(binary, Path(scratch) / str(ordinal), case, questions)
                )
            requests = [item[3] for item in prepared_cases]
            require(
                digest(binary.read_bytes()) == binary_sha256,
                "Binary changed while preparing requests",
            )
            require(
                not secret or secret not in canonical(requests).decode(),
                "Credential present in fixture payload",
            )
            report["requests"] = requests
            report["payload_sha256"] = digest(canonical(requests))
            if not live:
                report["success"] = True
            else:
                require(
                    reviewed_payload_sha256 == report["payload_sha256"],
                    "Reviewed payload hash required",
                )
                budget = Decimal(str(budget_usd))
                require(
                    budget.is_finite() and budget >= RESERVE * len(cases),
                    "Budget below declared reservation",
                )
                require(bool(secret.strip()), "Provider credential unavailable")
                report["budget_usd"] = str(budget)
                charged = Decimal(0)
                scored = []
                halt = False
                for case, (offline, original, job, _body) in zip(cases, prepared_cases):
                    outcome = {
                        "id": case["id"],
                        "status": "error",
                        "source_preserved": False,
                        "usage_known": False,
                        "attempted": False,
                        "reserved_or_estimated_usd": "0",
                    }
                    report["cases"].append(outcome)
                    if halt:
                        outcome["status"] = "not_attempted"
                        continue
                    halt = True
                    try:
                        require(
                            digest(binary.read_bytes()) == binary_sha256,
                            "Binary changed after request preparation",
                        )
                        cli = LiveCli(binary, offline.home, secret)
                        outcome["attempted"] = True
                        outcome["reserved_or_estimated_usd"] = str(RESERVE)
                        report["attempted"] += 1
                        drain = cli.run(
                            "--repository",
                            case["repository"],
                            "classify",
                            "run",
                            "--max-jobs",
                            "1",
                            "--seconds",
                            "12",
                        )
                        current = offline.run(
                            "--repository", case["repository"], "get", case["id"]
                        )
                        outcome["source_preserved"] = current == original
                        require(
                            outcome["source_preserved"],
                            "Classification changed original source",
                        )
                        final_job = offline.run(
                            "--repository",
                            case["repository"],
                            "classify",
                            "get",
                            job["key"],
                        )
                        require(
                            final_job["attempts"] <= 1 and drain["attempted"] <= 1,
                            "Attempt bound exceeded",
                        )
                        outcome["status"] = final_job["status"]
                        outcome["unknown_usage_attempts"] = final_job[
                            "unknown_usage_attempts"
                        ]
                        outcome["known_input_tokens"] = final_job["known_input_tokens"]
                        outcome["known_output_tokens"] = final_job[
                            "known_output_tokens"
                        ]
                        if final_job["status"] == "completed":
                            result = offline.run(
                                "--repository",
                                case["repository"],
                                "get",
                                "classification:" + job["key"],
                            )
                            require(
                                result["kind"] == "event"
                                and result["authority"] == "observation"
                                and result["links"]
                                == [
                                    {
                                        "target_id": original["id"],
                                        "relation": "derived_from",
                                    }
                                ],
                                "Advice promoted authority or changed subject link",
                            )
                            require(
                                result["metadata"]["prepared"] == job["prepared"],
                                "Advice subject mismatch",
                            )
                            advice = result["metadata"]["advice"]
                            validate_advice(advice)
                            require(
                                final_job["unknown_usage_attempts"] == 0,
                                "Completed usage unknown",
                            )
                            require(
                                final_job["known_input_tokens"]
                                == advice["usage"]["input_tokens"]
                                and final_job["known_output_tokens"]
                                == advice["usage"]["output_tokens"],
                                "Usage accounting differs",
                            )
                            outcome["advice"] = advice
                            outcome["usage_known"] = True
                            outcome["reserved_or_estimated_usd"] = str(
                                Decimal(advice["usage"]["input_tokens"])
                                * INPUT_USD_PER_MILLION
                                / Decimal(1_000_000)
                            )
                            if advice["usage"]["input_tokens"] > CONTEXT_TOKENS:
                                outcome["policy_violation"] = (
                                    "pricing_assumption_exceeded"
                                )
                                require(False, "Usage exceeds pricing assumption")
                            outcome["correct"] = (
                                advice["category"] == case["expected_category"]
                            )
                            outcome["brier_multiclass_sum"] = sum(
                                (
                                    advice["probabilities"][category]
                                    - int(category == case["expected_category"])
                                )
                                ** 2
                                for category in CATEGORIES
                            )
                            scored.append(outcome)
                        halt = False
                    except Exception:
                        outcome["status"] = "error"
                        outcome["error"] = (
                            "Case failed validation or bounded execution; provider diagnostics withheld."
                        )
                        # A failed provider call still must not mutate its subject.
                        try:
                            outcome["source_preserved"] = (
                                offline.run(
                                    "--repository",
                                    case["repository"],
                                    "get",
                                    case["id"],
                                )
                                == original
                            )
                        except Exception:
                            outcome["source_preserved"] = False
                    charged += Decimal(outcome["reserved_or_estimated_usd"])
                report["cost_reserved_or_estimated_usd"] = str(charged)
                report["metrics"] = {
                    "total_cases": len(cases),
                    "completed_scored": len(scored),
                    "pending": sum(c["status"] == "pending" for c in report["cases"]),
                    "failed_or_error": sum(
                        c["status"] not in ("completed", "pending", "not_attempted")
                        for c in report["cases"]
                    ),
                    "not_attempted": sum(
                        c["status"] == "not_attempted" for c in report["cases"]
                    ),
                    "usage_unknown_cases": sum(
                        c["attempted"] and not c["usage_known"] for c in report["cases"]
                    ),
                    "unknown_predictions": sum(
                        c["advice"]["category"] == "unknown" for c in scored
                    ),
                    "accuracy_completed": sum(c["correct"] for c in scored)
                    / len(scored)
                    if scored
                    else None,
                    "brier_completed": sum(c["brier_multiclass_sum"] for c in scored)
                    / len(scored)
                    if scored
                    else None,
                }
                report["success"] = len(scored) == len(cases) and all(
                    c["source_preserved"] for c in report["cases"]
                )
    except Exception:
        report["error"] = (
            "Evaluation blocked by input, pricing, authorization, credential or execution validation. No provider diagnostics retained."
        )
    finally:
        report = scrub(report, secret)
        Path(report_path).write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--fixtures", type=Path, default=FIXTURES)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--reviewed-payload-sha256")
    parser.add_argument("--budget-usd")
    args = parser.parse_args()
    result = evaluate(
        args.binary,
        args.report,
        args.fixtures,
        live=args.live,
        reviewed_payload_sha256=args.reviewed_payload_sha256,
        budget_usd=args.budget_usd,
    )
    print(
        "Evaluation report written; "
        + ("validation passed." if result["success"] else "validation failed.")
    )
    raise SystemExit(0 if result["success"] else 1)


if __name__ == "__main__":
    main()
