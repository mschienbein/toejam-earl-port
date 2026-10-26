"""ROM-free campaign tests with an authored stub registry and frozen schema-1 packets."""
from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

import replay_campaign as campaign

A = "func_000322"
B = "func_00032a"


def body(instance: int = 1) -> dict:
    return {"instance": instance, "form": "values", "origin": "candidate_override"}


def contract() -> dict:
    return {"execution_class": "Atomic", "proof_envelope": "ReplayedEnvironment",
            "route": "authored-stub", "profile": "authored-stub"}


def witness(invocation: int = 1, parent: int | None = None) -> dict:
    return {"trajectory_id": 1, "invocation_id": invocation, "parent_invocation_id": parent,
            "entry_frame": 0, "return_frame": 1, "entry_pc": 0x200, "entry_sp": 0xff7000,
            "entry_sr": 0x2000, "exception_depth": 0, "caller_pc": 0x180, "return_pc": 0x204}


def obligation(status: str = "discharged") -> dict:
    outputs = {"registers": [{"field": "D0", "bits": 65535}],
               "nonframe_ram": True, "ordered_device_effects": True}
    return {"id": 1, "caller_invocation_id": 1, "callee_inventory_id": B,
            "callee_witness": witness(2, 1), "body": body(2), "effective_contract": contract(),
            "required_outputs": outputs, "status": status,
            "discharged_by": ({"witness": witness(2, 1), "body": body(2),
                                "effective_contract": contract(), "checked_outputs": outputs}
                               if status == "discharged" else None), "reason": None}


class StubRegistry:
    """Test-only registry; it is never a production worker or fallback path."""
    def __init__(self, unsupported: set[str] | None = None) -> None:
        self.unsupported = unsupported or set()

    def report(self, requested: list[str], *, called: set[str] | None = None,
               failed: set[str] | None = None, conditional: bool = False) -> dict:
        supported = [ident for ident in requested if ident not in self.unsupported]
        called = set(supported) if called is None else called
        failed = failed or set()
        failures = []
        rows = {}
        for ident in supported:
            count = int(ident in called)
            index = None
            if ident in failed:
                index = len(failures)
                failures.append({"inventory_id": ident, "verdict": "mismatch",
                                 "witness": witness(), "body": body(), "effective_contract": contract(),
                                 "unsupported": None,
                                 "check": {"detail": "authored mismatch", "detail_truncated": True},
                                 "comparison": {"point": "own_access", "own_access_index": 7,
                                                "expected": {"address": 0xff9000, "width": 1,
                                                             "purpose": "semantic", "value": 1},
                                                "actual": {"address": 0xff9000, "width": 1,
                                                           "purpose": "semantic", "value": 2}}})
            matched = count and ident not in failed
            rows[ident] = {"inventory_id": ident, "name": ident, "role": "replayed_requested",
                           "body": body(), "effective_contract": contract(),
                           "entered": count, "reference_completed": count, "checked": count,
                           "own_matches": int(matched), "passed": int(matched and not conditional),
                           "failed": int(ident in failed), "unsupported": 0, "open": 0,
                           "provisional": int(matched and conditional), "first_failure_index": index}
        complete = all(row["passed"] > 0 for row in rows.values()) and not failed and not conditional
        evidence = {"schema_version": 1,
                    "label": "replayed environment, callees substituted (proven separately)",
                    "trajectory_id": 1, "attempted": len(called), "reference_completions": len(called),
                    "own_matches": sum(row["own_matches"] for row in rows.values()),
                    "completed_proofs": sum(row["passed"] for row in rows.values()),
                    "substitutions": int(conditional), "open_segments": 0, "unsupported": 0,
                    "mismatches": len(failed), "unresolved_obligations": int(conditional),
                    "finalized": True, "complete": complete, "by_function": rows,
                    "failures": failures, "obligations": [obligation("pending")] if conditional else []}
        exclusions = [{"inventory_id": ident, "name": ident, "kind": "constructor_unsupported",
                       "detail": "authored unsupported control root"}
                      for ident in requested if ident in self.unsupported]
        return {"schema_version": 1, "protocol": "tje-replayed-worker/1", "revision": "rev02",
                "requested_functions": [{"inventory_id": ident, "name": ident} for ident in requested],
                "constructed_functions": supported, "constructor_exclusions": exclusions,
                "configuration_error": None, "frames_requested": 3, "frames_completed": 3,
                "run_error": None, "finalization_error": None if complete else "authored incomplete scope",
                "complete": complete and not exclusions,
                "checks": {"replayed_environment": evidence} if supported else None}


def receipt(payload: dict, requested: list[str], revision: str = "rev02", job: str = "authored") -> dict:
    return {"job": {"id": job, "revision": revision, "report": f"{job}.json"},
            "report_sha256": "authored-report-hash",
            "analysis": campaign.analyze_worker(payload, requested)}


class ReplayCampaignTests(unittest.TestCase):
    def test_windows_report_sync_uses_write_access_without_directory_fd(self) -> None:
        path = mock.MagicMock(spec=Path)
        path.open.return_value.__enter__.return_value.fileno.return_value = 42
        windows = mock.Mock(name="windows_os")
        windows.name = "nt"
        with mock.patch.object(campaign, "os", windows):
            campaign.sync_file(path)
        path.open.assert_called_once_with("r+b")
        windows.fsync.assert_called_once_with(42)
        windows.open.assert_not_called()

    def test_posix_directory_sync_keeps_fsync_and_closes_on_error(self) -> None:
        posix = mock.Mock(name="posix_os")
        posix.name, posix.O_RDONLY, posix.O_DIRECTORY = "posix", 0, 65536
        posix.open.return_value = 43
        posix.fsync.side_effect = OSError("authored directory sync failure")
        path = Path("authored-directory")
        with mock.patch.object(campaign, "os", posix), self.assertRaises(OSError):
            campaign.sync_directory(path)
        posix.open.assert_called_once_with(path, 65536)
        posix.fsync.assert_called_once_with(43)
        posix.close.assert_called_once_with(43)

    def test_windows_worker_launch_is_native_not_a_posix_shell(self) -> None:
        windows = mock.Mock(name="windows_os")
        windows.name = "nt"
        worker, env_script = Path("worker with spaces.exe"), Path("unused env.sh")
        with mock.patch.object(campaign, "os", windows):
            command = campaign.worker_command(worker, env_script, "replay-catalogue")
        self.assertEqual(command, [str(worker), "replay-catalogue"])

    def test_posix_worker_launch_preserves_sourced_environment(self) -> None:
        posix = mock.Mock(name="posix_os")
        posix.name = "posix"
        worker, env_script = Path("worker with spaces"), Path("authored env.sh")
        with mock.patch.object(campaign, "os", posix):
            command = campaign.worker_command(worker, env_script, "replay-catalogue")
        self.assertEqual(command, ["bash", "-c", '. "$1" && shift && exec "$@"',
                                  "replayed-campaign", str(env_script), str(worker),
                                  "replay-catalogue"])

    def test_execute_job_owns_write_log_through_subprocess_and_fsync(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            job = {"revision": "rev02", "input": str(root / "input.json"),
                   "report": str(root / "report.json"), "log": str(root / "worker.log")}
            streams = []

            def execute(command: list[str], **kwargs: object) -> SimpleNamespace:
                self.assertTrue(kwargs["close_fds"])
                self.assertEqual(kwargs["stderr"], subprocess.STDOUT)
                output = kwargs["stdout"]
                self.assertFalse(output.closed)
                output.write(b"authored redirected worker output\n")
                streams.append(output)
                return SimpleNamespace(returncode=0)

            with mock.patch.object(campaign.subprocess, "run", side_effect=execute):
                code, _ = campaign.execute_job(job, root / "worker", root / "env.sh",
                                               {"rev02": root / "rom.bin"}, [A])
            self.assertEqual(code, 0)
            self.assertTrue(streams[0].closed)
            self.assertEqual(Path(job["log"]).read_bytes(), b"authored redirected worker output\n")

    def test_constructor_exclusion_keeps_original_scope_incomplete(self) -> None:
        payload = StubRegistry({B}).report([A, B])
        self.assertTrue(payload["checks"]["replayed_environment"]["complete"])
        self.assertFalse(payload["complete"])
        rows = [receipt(payload, [A, B], revision) for revision in ("rev00", "rev02")]
        result = campaign.summarize(rows, [A, B])
        self.assertFalse(result["complete"])
        self.assertTrue(result["constructor_exclusions_force_scope_incomplete"])
        self.assertEqual(result["zero_checked"]["rev00"], [B])
        self.assertEqual(result["functions"][A]["rev00"]["passed"], 1)

    def test_all_unsupported_has_zero_checks_and_no_fabricated_packet(self) -> None:
        payload = StubRegistry({A}).report([A])
        analysis = campaign.analyze_worker(payload, [A])
        self.assertEqual(analysis["rows"][A]["checked"], 0)
        self.assertIsNone(analysis["rows"][A]["first_failure"])
        self.assertFalse(analysis["host_finalized"])

    def test_absent_replay_report_is_not_zero_mismatch_success(self) -> None:
        payload = StubRegistry().report([A])
        payload["checks"] = {"mismatches": 0}
        with self.assertRaises(campaign.CampaignError):
            campaign.analyze_worker(payload, [A])

    def test_own_match_does_not_discharge_a_callee(self) -> None:
        payload = StubRegistry().report([A], conditional=True)
        rows = [receipt(payload, [A], revision) for revision in ("rev00", "rev02")]
        result = campaign.summarize(rows, [A])
        self.assertFalse(result["complete"])
        self.assertEqual(result["functions"][A]["rev02"]["own_matches"], 1)
        self.assertEqual(result["functions"][A]["rev02"]["passed"], 0)
        linked = result["obligations_by_callee"][B][0]
        self.assertEqual(linked["callee_witness"], witness(2, 1))
        self.assertEqual(linked["body"], body(2))
        self.assertIsNone(linked["discharged_by"])

    def test_actual_discharge_packets_remain_scoped_to_their_job(self) -> None:
        payload = StubRegistry().report([A])
        payload["checks"]["replayed_environment"]["obligations"] = [obligation()]
        rows = [receipt(payload, [A], revision, f"job-{revision}") for revision in ("rev00", "rev02")]
        result = campaign.summarize(rows, [A])
        linked = result["obligations_by_callee"][B]
        self.assertEqual(len(linked), 2)
        self.assertNotEqual(linked[0]["job_id"], linked[1]["job_id"])
        self.assertEqual(linked[0]["discharged_by"], obligation()["discharged_by"])
        self.assertTrue(result["complete"])

    def test_count_or_visit_cannot_stand_in_for_discharge_packet(self) -> None:
        payload = StubRegistry().report([A])
        broken = obligation()
        broken["discharged_by"] = None
        payload["checks"]["replayed_environment"]["obligations"] = [broken]
        with self.assertRaises(campaign.CampaignError):
            campaign.analyze_worker(payload, [A])

    def test_exact_first_failure_index_packet_and_truncation_survive(self) -> None:
        payload = StubRegistry().report([A], failed={A})
        analysis = campaign.analyze_worker(payload, [A])
        first = analysis["rows"][A]["first_failure"]
        self.assertEqual(first["index"], 0)
        self.assertEqual(first["packet"], payload["checks"]["replayed_environment"]["failures"][0])
        self.assertTrue(first["packet"]["check"]["detail_truncated"])
        self.assertEqual(first["packet"]["comparison"]["own_access_index"], 7)
        payload["checks"]["replayed_environment"]["by_function"][A]["first_failure_index"] = 7
        with self.assertRaises(campaign.CampaignError):
            campaign.analyze_worker(payload, [A])

    def test_unfinalized_or_open_evidence_cannot_pass(self) -> None:
        payload = StubRegistry().report([A])
        evidence = payload["checks"]["replayed_environment"]
        evidence["finalized"] = False
        evidence["complete"] = False
        evidence["by_function"][A]["open"] = 1
        rows = [receipt(payload, [A], revision) for revision in ("rev00", "rev02")]
        self.assertFalse(campaign.summarize(rows, [A])["complete"])

    def test_zero_checked_requested_rows_are_never_dropped(self) -> None:
        payload = StubRegistry().report([A, B], called={A})
        rows = [receipt(payload, [A, B], revision) for revision in ("rev00", "rev02")]
        result = campaign.summarize(rows, [A, B])
        self.assertEqual(result["zero_checked"], {"rev00": [B], "rev02": [B]})
        self.assertFalse(result["complete"])

    def test_actual_packet_trajectory_must_match_its_own_report(self) -> None:
        payload = StubRegistry().report([A])
        wrong = obligation()
        wrong["callee_witness"]["trajectory_id"] = 2
        payload["checks"]["replayed_environment"]["obligations"] = [wrong]
        with self.assertRaises(campaign.CampaignError):
            campaign.analyze_worker(payload, [A])

    def test_committed_sessions_plan_both_roms_without_other_input_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory) / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Authored Test"], cwd=repo, check=True)
            recording = {"format": 1, "revision": "rev00", "start": {"kind": "power_on"},
                         "frames": 3, "tool": "authored", "description": "controller inputs only",
                         "runs": [{"frames": 3, "pads": [256, 0]}]}
            source = repo / "sessions" / "authored.tje.json"
            campaign.atomic_json(source, recording)
            campaign.atomic_json(repo / "sessions" / "untracked.tje.json", recording)
            subprocess.run(["git", "add", "sessions/authored.tje.json"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-qm", "authored input"], cwd=repo, check=True)
            jobs = campaign.plan_jobs(repo, Path(directory) / "out", {"authored": True})
            self.assertEqual(len(jobs), 2)
            for job in jobs:
                expected = deepcopy(recording)
                expected["revision"] = job["revision"]
                self.assertEqual(campaign.load_json(Path(job["input"])), expected)
                self.assertEqual(job["source_sha256"], campaign.digest(source))

    def make_job(self, directory: str, payload: dict) -> dict:
        root = Path(directory)
        job = {"id": "authored", "identity": "bound-authored-identity", "revision": "rev02",
               "frames": 3, "report": str(root / "report.json"), "log": str(root / "worker.log"),
               "receipt": str(root / "receipt.json")}
        campaign.atomic_json(Path(job["report"]), payload)
        Path(job["log"]).write_text("authored worker finished\n")
        return job

    def test_completion_fsyncs_report_before_atomic_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            job = self.make_job(directory, StubRegistry().report([A]))
            calls = []
            real_sync, real_atomic = campaign.sync_file, campaign.atomic_json
            with mock.patch.object(campaign, "sync_file", side_effect=lambda path: (calls.append("sync-report"), real_sync(path))[-1]), mock.patch.object(campaign, "atomic_json", side_effect=lambda path, value: (calls.append("receipt"), real_atomic(path, value))[-1]):
                campaign.complete_job(job, 0, ["authored-worker"], [A])
            self.assertEqual(calls, ["sync-report", "receipt"])
            self.assertIsNotNone(campaign.reusable(job, [A]))

    def test_resume_rejects_truncation_and_cached_analysis_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            job = self.make_job(directory, StubRegistry().report([A]))
            campaign.complete_job(job, 0, ["authored-worker"], [A])
            receipt_path = Path(job["receipt"])
            saved = campaign.load_json(receipt_path)
            saved["analysis"]["rows"][A]["passed"] = 999
            campaign.atomic_json(receipt_path, saved)
            self.assertIsNone(campaign.reusable(job, [A]))
            receipt_path.write_text('{"unfinished":')
            self.assertIsNone(campaign.reusable(job, [A]))

    def test_valid_recorded_failure_is_done_not_retried_on_resume(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            job = self.make_job(directory, StubRegistry().report([A], failed={A}))
            campaign.complete_job(job, 1, ["authored-worker"], [A])
            self.assertIsNotNone(campaign.reusable(job, [A]))

    def test_resolution_keeps_durable_inventory_keys_and_rejects_duplicates(self) -> None:
        catalogue = {"functions": [{"inventory_id": A, "name": "Authored", "production": False},
                                    {"inventory_id": B, "name": "Other", "production": True}]}
        self.assertEqual(campaign.resolve_roots(catalogue, ["Authored"], False), [A])
        self.assertEqual(campaign.resolve_roots(catalogue, [], True), [A])
        with self.assertRaises(campaign.CampaignError):
            campaign.resolve_roots(catalogue, [A, "Authored"], False)

    def test_session_zero_coverage_is_aggregated_without_waiving_open_cases(self) -> None:
        rows = []
        for revision in ("rev00", "rev02"):
            for called in ({A}, {B}):
                payload = StubRegistry().report([A, B], called=called)
                self.assertFalse(payload["complete"])
                rows.append(receipt(payload, [A, B], revision, f"{revision}-{sorted(called)}"))
        result = campaign.summarize(rows, [A, B])
        self.assertTrue(result["complete"])
        self.assertEqual(result["zero_checked"], {"rev00": [], "rev02": []})
        self.assertEqual(len(result["finalization_errors"]), 4)

    def test_coverage_reason_cannot_hide_an_unresolved_obligation_packet(self) -> None:
        payload = StubRegistry().report([A, B], called={A})
        evidence = payload["checks"]["replayed_environment"]
        evidence["obligations"] = [obligation("pending")]
        # Deliberately inconsistent counts demonstrate that the packet, not the
        # coverage-only error string or zero counter, keeps the campaign closed.
        evidence["unresolved_obligations"] = 0
        rows = [receipt(payload, [A, B], revision) for revision in ("rev00", "rev02")]
        self.assertFalse(campaign.summarize(rows, [A, B])["complete"])

    def test_runtime_error_is_not_approved_by_clean_counters(self) -> None:
        payload = StubRegistry().report([A])
        payload["run_error"] = "authored late execution failure"
        payload["complete"] = False
        rows = [receipt(payload, [A], revision) for revision in ("rev00", "rev02")]
        self.assertFalse(campaign.summarize(rows, [A])["complete"])

    def test_metadata_failure_retains_all_zero_checked_requested_rows(self) -> None:
        payload = StubRegistry().report([A, B])
        payload.update(checks=None, constructed_functions=[], configuration_error="authored metadata error",
                       frames_completed=0, complete=False)
        analysis = campaign.analyze_worker(payload, [A, B])
        self.assertEqual(set(analysis["rows"]), {A, B})
        self.assertTrue(all(row["checked"] == 0 for row in analysis["rows"].values()))
        result = campaign.summarize([receipt(payload, [A, B])], [A, B])
        self.assertFalse(result["complete"])

    def test_subprocess_campaign_fsync_resume_and_only_damaged_job_rerun(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            repo = base / "repo"
            repo.mkdir()
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.name", "Authored Test"], cwd=repo, check=True)
            recording = {"format": 1, "revision": "rev00", "start": {"kind": "power_on"},
                         "frames": 3, "tool": "authored", "description": "stub inputs",
                         "runs": [{"frames": 3, "pads": [256, 0]}]}
            campaign.atomic_json(repo / "sessions" / "authored.tje.json", recording)
            subprocess.run(["git", "add", "sessions"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-qm", "authored session"], cwd=repo, check=True)
            calls = base / "worker-calls.log"
            worker = base / "stub-worker.py"
            packet = json.dumps(StubRegistry().report([A]))
            worker.write_text("\n".join([
                "import argparse, json",
                "from pathlib import Path",
                "parser = argparse.ArgumentParser()",
                "parser.add_argument('command')",
                "parser.add_argument('--rom')",
                "parser.add_argument('--session')",
                "parser.add_argument('--report')",
                "parser.add_argument('--function', nargs='+')",
                "args = parser.parse_args()",
                f"payload = json.loads({packet!r})",
                "payload['revision'] = json.loads(Path(args.session).read_text())['revision']",
                "Path(args.report).write_text(json.dumps(payload) + '\\n')",
                f"with Path({str(calls)!r}).open('a') as log: log.write(payload['revision'] + '\\n')",
                "print('authored stub registry completed')",
            ]) + "\n")
            env_script = base / "env.sh"
            env_script.write_text("# authored empty environment\n")
            catalogue = {"protocol": "tje-replayed-worker/1", "replayed_environment_available": True,
                         "functions": [{"inventory_id": A, "name": "Authored", "production": False}]}
            source = {"authored_source": "one immutable build"}
            build_path = base / "build.json"
            campaign.atomic_json(build_path, {"worker": str(worker), "binary_sha256": campaign.digest(worker),
                                              "source": source, "catalogue_sha256": campaign.canonical_hash(catalogue)})
            rom00, rom02 = base / "authored-rev00.bin", base / "authored-rev02.bin"
            rom00.write_bytes(b"authored non-ROM test input zero")
            rom02.write_bytes(b"authored non-ROM test input two")
            args = SimpleNamespace(repo=repo, out=base / "out", build_receipt=build_path,
                                   env_script=env_script, function=[A], all_pending=False,
                                   rom_rev00=rom00, rom_rev02=rom02, workers=2)
            # Execute the authored Python worker explicitly on every platform;
            # Windows does not implement POSIX executable bits or shebang launch.
            with mock.patch.object(campaign, "source_identity", return_value=source), \
                    mock.patch.object(campaign, "read_catalogue", return_value=catalogue), \
                    mock.patch.object(campaign, "worker_command",
                                      side_effect=lambda executable, _env, *arguments:
                                      [sys.executable, str(executable), *arguments]):
                self.assertTrue(campaign.run_campaign(args))
                self.assertTrue(campaign.run_campaign(args))
                self.assertEqual(len(calls.read_text().splitlines()), 2)
                damaged = next((args.out / "receipts").glob("*.json"))
                damaged.write_text('{"interrupted":')
                self.assertTrue(campaign.run_campaign(args))
                self.assertEqual(len(calls.read_text().splitlines()), 3)
            result = campaign.load_json(args.out / "result.json")
            self.assertTrue(result["complete"])
            self.assertEqual(result["zero_checked"], {"rev00": [], "rev02": []})


if __name__ == "__main__":
    unittest.main()
