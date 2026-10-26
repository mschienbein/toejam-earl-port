#!/usr/bin/env python3
"""Build and run immutable, resumable check-only replay-environment campaigns.

Host schema-1 evidence is output only. This tool never imports sealed handles,
constructs callee discharges, changes a game registry, or promotes a function.
"""
from __future__ import annotations

import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ENV = Path.home() / ".local/share/tje-dev/env.sh"
COUNTERS = ("entered", "reference_completed", "checked", "own_matches", "passed",
            "failed", "unsupported", "open", "provisional")
ROOT_REQUIRED = ("inventory_id", "name", "role", "body", "effective_contract",
                 *COUNTERS, "first_failure_index")
REPORT_REQUIRED = ("schema_version", "label", "trajectory_id", "attempted",
                   "reference_completions", "own_matches", "completed_proofs",
                   "substitutions", "open_segments", "unsupported", "mismatches",
                   "unresolved_obligations", "finalized", "complete", "by_function",
                   "failures", "obligations")
WITNESS_REQUIRED = ("trajectory_id", "invocation_id", "parent_invocation_id",
                    "entry_frame", "entry_pc", "entry_sp", "entry_sr",
                    "exception_depth", "caller_pc", "return_pc")


class CampaignError(ValueError):
    """An incomplete scope or malformed evidence cannot be approved."""


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical_hash(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as stream:
        return json.load(stream)


def sync_file(path: Path) -> None:
    # Windows _commit/FlushFileBuffers requires write access, without truncation.
    with path.open("r+b" if os.name == "nt" else "rb") as stream:
        os.fsync(stream.fileno())
    sync_directory(path.parent)


def sync_directory(path: Path) -> None:
    # Windows has no portable directory-fsync descriptor. File contents are still
    # flushed before atomic replacement; POSIX also persists the directory entry.
    if os.name == "posix":
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)


def atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("w", encoding="utf-8", newline="\n") as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    sync_directory(path.parent)


def git(repo: Path, *arguments: str) -> str:
    return subprocess.run(["git", *arguments], cwd=repo, check=True,
                          capture_output=True, text=True).stdout.strip()


def source_identity(repo: Path) -> dict[str, Any]:
    if git(repo, "status", "--porcelain", "--untracked-files=all"):
        raise CampaignError("build/run requires a committed clean source tree")
    return {"commit": git(repo, "rev-parse", "HEAD"),
            "tree": git(repo, "rev-parse", "HEAD^{tree}"),
            "game_tree": git(repo, "rev-parse", "HEAD:crates/tje-game"),
            "host_tree": git(repo, "rev-parse", "HEAD:crates/tje-host"),
            "session_tree": git(repo, "rev-parse", "HEAD:crates/tje-session"),
            "cargo_lock_sha256": digest(repo / "Cargo.lock")}


def worker_command(worker: Path, env_script: Path, *arguments: str) -> list[str]:
    if os.name == "nt":
        return [str(worker), *arguments]
    return ["bash", "-c", '. "$1" && shift && exec "$@"', "replayed-campaign",
            str(env_script), str(worker), *arguments]


def read_catalogue(worker: Path, env_script: Path) -> dict[str, Any]:
    result = subprocess.run(worker_command(worker, env_script, "replay-catalogue"),
                            check=True, capture_output=True, text=True)
    catalogue = json.loads(result.stdout)
    if catalogue.get("protocol") != "tje-replayed-worker/1":
        raise CampaignError("worker does not implement the frozen replay protocol")
    if catalogue.get("replayed_environment_available") is not True:
        raise CampaignError("worker lacks the real replayed-environment API; no fallback is allowed")
    return catalogue


def build_worker(repo: Path, target: Path, destination: Path, env_script: Path) -> None:
    before = source_identity(repo)
    environment = os.environ.copy()
    build_environment = {"CARGO_TARGET_DIR": str(target), "CARGO_PROFILE_DEV_DEBUG": "0",
                         "CARGO_PROFILE_TEST_DEBUG": "0", "CARGO_INCREMENTAL": "0",
                         "CARGO_PROFILE_RELEASE_PANIC": "unwind"}
    environment.update(build_environment)
    arguments = ["build", "--locked", "--release", "-p", "tje-session",
                 "--features", "runner,replayed-environment"]
    command = (["cargo", *arguments] if os.name == "nt" else
               ["bash", "-c", '. "$1" && shift && exec cargo "$@"', "replay-build",
                str(env_script), *arguments])
    subprocess.run(command, cwd=repo, env=environment, check=True)
    after = source_identity(repo)
    if before != after:
        raise CampaignError("source changed during worker compilation")
    worker = target / "release" / ("tje-session.exe" if os.name == "nt" else "tje-session")
    catalogue = read_catalogue(worker, env_script)
    atomic_json(destination, {"schema_version": 1, "source": after,
                              "worker": str(worker.resolve()), "binary_sha256": digest(worker),
                              "catalogue": catalogue, "catalogue_sha256": canonical_hash(catalogue),
                              "build_command": command, "build_environment": build_environment})


def resolve_roots(catalogue: dict[str, Any], requested: list[str], all_pending: bool) -> list[str]:
    functions = catalogue["functions"]
    selected = ([row["inventory_id"] for row in functions
                 if not row["production"] and not row.get("replayed", False)]
                if all_pending else requested)
    result: list[str] = []
    for key in selected:
        matches = [row for row in functions if key in (row["inventory_id"], row["name"])]
        if len(matches) != 1:
            raise CampaignError(f"unknown/ambiguous requested function: {key}")
        ident = matches[0]["inventory_id"]
        if ident in result:
            raise CampaignError(f"duplicate requested function: {ident}")
        result.append(ident)
    if not result:
        raise CampaignError("an explicit nonempty requested function scope is required")
    return sorted(result)


def required(value: Any, fields: tuple[str, ...], location: str) -> None:
    if not isinstance(value, dict) or any(field not in value for field in fields):
        raise CampaignError(f"missing frozen evidence fields at {location}")


def integer(value: Any, location: str, minimum: int = 0) -> None:
    if type(value) is not int or value < minimum:
        raise CampaignError(f"invalid integer at {location}")


def body(value: Any, location: str) -> None:
    required(value, ("instance", "form", "origin"), location)
    integer(value["instance"], location + ".instance", 1)
    if value["form"] not in ("values", "framed") or value["origin"] not in (
            "canonical_production", "candidate_override", "host_instrumentation"):
        raise CampaignError(f"invalid sealed-body description at {location}")


def witness(value: Any, location: str) -> None:
    required(value, WITNESS_REQUIRED, location)
    for key in ("trajectory_id", "invocation_id"):
        integer(value[key], location + "." + key, 1)
    for key in ("entry_frame", "entry_pc", "entry_sp", "entry_sr", "exception_depth"):
        integer(value[key], location + "." + key)


def validate_evidence(evidence: Any) -> None:
    required(evidence, REPORT_REQUIRED, "checks.replayed_environment")
    if type(evidence["schema_version"]) is not int or evidence["schema_version"] != 1:
        raise CampaignError("unsupported replay evidence schema")
    if evidence["label"] != "replayed environment, callees substituted (proven separately)":
        raise CampaignError("wrong proof-tier label")
    integer(evidence["trajectory_id"], "trajectory_id", 1)
    if type(evidence["finalized"]) is not bool or type(evidence["complete"]) is not bool:
        raise CampaignError("invalid finalization flags")
    for key in ("attempted", "reference_completions", "own_matches", "completed_proofs",
                "substitutions", "open_segments", "unsupported", "mismatches", "unresolved_obligations"):
        integer(evidence[key], key)
    if not isinstance(evidence["by_function"], dict) or not isinstance(evidence["failures"], list):
        raise CampaignError("malformed replay root/failure tables")
    for ident, row in evidence["by_function"].items():
        required(row, ROOT_REQUIRED, ident)
        if row["inventory_id"] != ident or row["role"] not in ("replayed_requested", "strict_auto"):
            raise CampaignError(f"mismatched inventory key/role at {ident}")
        body(row["body"], ident + ".body")
        required(row["effective_contract"], ("execution_class", "proof_envelope", "route", "profile"), ident)
        for key in COUNTERS:
            integer(row[key], ident + "." + key)
        index = row["first_failure_index"]
        if index is not None:
            integer(index, ident + ".first_failure_index")
            if index >= len(evidence["failures"]) or evidence["failures"][index]["inventory_id"] != ident:
                raise CampaignError(f"invalid exact first-failure index at {ident}")
        elif row["failed"] or row["unsupported"]:
            raise CampaignError(f"failed/unsupported root lacks exact first packet: {ident}")
    for index, packet in enumerate(evidence["failures"]):
        required(packet, ("inventory_id", "verdict", "witness", "body", "effective_contract",
                          "unsupported", "check", "comparison"), f"failures/{index}")
        if packet["verdict"] not in ("mismatch", "unsupported"):
            raise CampaignError("unknown replay failure verdict")
        witness(packet["witness"], f"failures/{index}/witness")
        body(packet["body"], f"failures/{index}/body")
    if not isinstance(evidence["obligations"], list):
        raise CampaignError("missing individual callee obligations")
    for index, obligation in enumerate(evidence["obligations"]):
        required(obligation, ("id", "caller_invocation_id", "callee_inventory_id", "callee_witness",
                              "body", "effective_contract", "required_outputs", "status",
                              "discharged_by", "reason"), f"obligations/{index}")
        integer(obligation["id"], "obligation.id", 1)
        integer(obligation["caller_invocation_id"], "obligation.caller", 1)
        witness(obligation["callee_witness"], f"obligations/{index}/callee_witness")
        if obligation["callee_witness"]["trajectory_id"] != evidence["trajectory_id"]:
            raise CampaignError("callee witness belongs to another process-local trajectory")
        if obligation["body"] is not None:
            body(obligation["body"], f"obligations/{index}/body")
        required(obligation["required_outputs"], ("registers", "nonframe_ram", "ordered_device_effects"), "required_outputs")
        if obligation["status"] not in ("pending", "discharged", "unsupported", "failed"):
            raise CampaignError("unknown callee obligation status")
        if obligation["status"] == "discharged" and not obligation["discharged_by"]:
            raise CampaignError("callee discharge lacks its actual child proof packet")
        if obligation["status"] != "discharged" and obligation["discharged_by"] is not None:
            raise CampaignError("unresolved obligation cannot carry an approval")
    if evidence["complete"] and (not evidence["finalized"] or evidence["open_segments"]
                                 or evidence["unsupported"] or evidence["mismatches"]
                                 or evidence["unresolved_obligations"] or evidence["failures"]
                                 or any(item["status"] != "discharged"
                                        for item in evidence["obligations"])):
        raise CampaignError("inconsistent successful Host completion")


def analyze_worker(payload: dict[str, Any], requested: list[str]) -> dict[str, Any]:
    required(payload, ("schema_version", "protocol", "revision", "requested_functions",
                       "constructed_functions", "constructor_exclusions", "configuration_error",
                       "frames_requested", "frames_completed", "run_error", "finalization_error",
                       "complete", "checks"), "worker")
    if (payload["protocol"] != "tje-replayed-worker/1"
            or type(payload["schema_version"]) is not int or payload["schema_version"] != 1):
        raise CampaignError("wrong worker protocol")
    if payload["revision"] not in ("rev00", "rev02") or type(payload["complete"]) is not bool:
        raise CampaignError("invalid worker revision/completion flag")
    integer(payload["frames_requested"], "frames_requested")
    integer(payload["frames_completed"], "frames_completed")
    if (not isinstance(payload["requested_functions"], list)
            or not isinstance(payload["constructed_functions"], list)
            or not isinstance(payload["constructor_exclusions"], list)):
        raise CampaignError("invalid worker requested/constructed/excluded tables")
    for row in payload["requested_functions"]:
        required(row, ("inventory_id", "name"), "requested_functions")
    for row in payload["constructor_exclusions"]:
        required(row, ("inventory_id", "kind", "detail"), "constructor_exclusions")
    if sorted(row["inventory_id"] for row in payload["requested_functions"]) != requested:
        raise CampaignError("worker silently changed the original requested scope")
    excluded = {row["inventory_id"]: row for row in payload["constructor_exclusions"]}
    if not set(excluded).issubset(requested):
        raise CampaignError("constructor exclusion is outside the requested scope")
    checks = payload["checks"]
    evidence = checks.get("replayed_environment") if isinstance(checks, dict) else None
    if evidence is not None:
        validate_evidence(evidence)
    elif not excluded and not payload["configuration_error"]:
        raise CampaignError("ordinary strict output/None is not replay evidence")
    rows: dict[str, Any] = {}
    for ident in requested:
        if ident in excluded or (evidence is None and payload["configuration_error"]):
            rows[ident] = {key: 0 for key in COUNTERS}
            rows[ident].update(constructor_exclusion=excluded.get(ident), first_failure=None,
                               body=None, effective_contract=None)
            continue
        if evidence is None or ident not in evidence["by_function"]:
            raise CampaignError(f"missing constructed requested root: {ident}")
        row = evidence["by_function"][ident]
        index = row["first_failure_index"]
        rows[ident] = {**row, "first_failure": None if index is None else {
            "index": index, "packet": evidence["failures"][index]}}
    # Host.complete may be false solely because some requested roots were not
    # called in this session. Keep that fact; corpus coverage is aggregated by id.
    return {"rows": rows, "host_complete": False if evidence is None else evidence["complete"],
            "host_finalized": False if evidence is None else evidence["finalized"],
            "constructor_exclusions": payload["constructor_exclusions"],
            "configuration_error": payload["configuration_error"],
            "run_error": payload["run_error"], "finalization_error": payload["finalization_error"],
            "frames_requested": payload["frames_requested"], "frames_completed": payload["frames_completed"],
            "trajectory_id": None if evidence is None else evidence["trajectory_id"],
            "failure_packet_count": 0 if evidence is None else len(evidence["failures"]),
            "unresolved_packet_count": 0 if evidence is None else sum(
                item["status"] != "discharged" for item in evidence["obligations"]),
            "replay_totals": None if evidence is None else {
                key: evidence[key] for key in (
                    "open_segments", "unsupported", "mismatches", "unresolved_obligations")},
            "obligations": [] if evidence is None else evidence["obligations"]}


def plan_jobs(repo: Path, out: Path, identity: dict[str, Any],
              session_filter: list[str] | None = None) -> list[dict[str, Any]]:
    files = subprocess.run(["git", "ls-files", "-z", "--", "sessions/*.tje.json"], cwd=repo,
                           capture_output=True, check=True).stdout.decode().split("\0")
    if session_filter is not None:
        missing = set(session_filter) - set(files)
        if missing:
            raise CampaignError(f"session filter must name committed corpus inputs: {sorted(missing)}")
        files = session_filter
    jobs = []
    for relative in sorted(filter(None, files)):
        source = repo / relative
        recording = load_json(source)
        source_hash = digest(source)
        for revision in ("rev00", "rev02"):
            adapted = dict(recording)
            adapted["revision"] = revision
            copied = out / "inputs" / revision / source.name
            atomic_json(copied, adapted)
            job_id = f"{revision}--{source.name.removesuffix('.tje.json')}"
            job = {"id": job_id, "source_session": relative, "recorded_revision": recording["revision"],
                   "revision": revision, "frames": recording["frames"], "source_sha256": source_hash,
                   "input": str(copied), "input_sha256": digest(copied),
                   "report": str(out / "reports" / f"{job_id}.json"),
                   "log": str(out / "logs" / f"{job_id}.log"),
                   "receipt": str(out / "receipts" / f"{job_id}.json")}
            job["identity"] = canonical_hash({"campaign": identity, "job": job})
            jobs.append(job)
    if not jobs:
        raise CampaignError("no committed session input recordings")
    return jobs


def reusable(job: dict[str, Any], roots: list[str]) -> dict[str, Any] | None:
    try:
        receipt = load_json(Path(job["receipt"]))
        if receipt["identity"] != job["identity"]:
            return None
        if digest(Path(job["report"])) != receipt["report_sha256"] or digest(Path(job["log"])) != receipt["log_sha256"]:
            return None
        actual = analyze_worker(load_json(Path(job["report"])), roots)
        if receipt["analysis"] != actual:
            return None
        return receipt
    except (OSError, ValueError, KeyError):
        return None


def execute_job(job: dict[str, Any], worker: Path, env_script: Path, roms: dict[str, Path],
                roots: list[str]) -> tuple[int, list[str]]:
    Path(job["report"]).parent.mkdir(parents=True, exist_ok=True)
    Path(job["log"]).parent.mkdir(parents=True, exist_ok=True)
    command = worker_command(worker, env_script, "replay-environment", "--rom", str(roms[job["revision"]]),
                             "--session", job["input"], "--report", job["report"], "--function", *roots)
    with Path(job["log"]).open("wb") as output:
        result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT,
                                close_fds=True, check=False)
        output.flush()
        os.fsync(output.fileno())
    return result.returncode, command


def complete_job(job: dict[str, Any], exit_code: int, command: list[str], roots: list[str]) -> dict[str, Any]:
    payload = load_json(Path(job["report"]))
    analysis = analyze_worker(payload, roots)
    if payload["revision"] != job["revision"] or payload["frames_requested"] != job["frames"]:
        raise CampaignError("worker report belongs to another session/revision")
    if (exit_code == 0) != payload["complete"]:
        raise CampaignError("worker exit does not match its own complete flag")
    sync_file(Path(job["report"]))
    sync_directory(Path(job["log"]).parent)
    receipt = {"schema_version": 1, "identity": job["identity"], "job": job,
               "exit_code": exit_code, "command": command,
               "report_sha256": digest(Path(job["report"])), "log_sha256": digest(Path(job["log"])),
               "analysis": analysis}
    atomic_json(Path(job["receipt"]), receipt)
    return receipt


def summarize(receipts: list[dict[str, Any]], roots: list[str]) -> dict[str, Any]:
    functions = {ident: {revision: {key: 0 for key in COUNTERS} | {
        "constructor_exclusions": [], "first_failure": None, "body_bindings": []}
        for revision in ("rev00", "rev02")} for ident in roots}
    obligations: dict[str, list[dict[str, Any]]] = {}
    finalized = True
    scope_exclusions = False
    frames_complete = True
    runtime_errors = []
    global_evidence_errors = []
    finalization_errors = []
    for receipt in receipts:
        job = receipt["job"]
        analysis = receipt["analysis"]
        finalized &= analysis["host_finalized"]
        frames_complete &= analysis["frames_completed"] == analysis["frames_requested"]
        scope_exclusions |= bool(analysis["constructor_exclusions"] or analysis["configuration_error"])
        if analysis["run_error"]:
            runtime_errors.append({"job_id": job["id"], "error": analysis["run_error"]})
        totals = analysis["replay_totals"]
        if ((totals is not None and any(totals.values()))
                or analysis["failure_packet_count"] or analysis["unresolved_packet_count"]):
            global_evidence_errors.append({"job_id": job["id"], "totals": totals})
        if analysis["finalization_error"]:
            finalization_errors.append({"job_id": job["id"], "error": analysis["finalization_error"]})
            # A batch may leave a requested root uncalled in this one session;
            # that is corpus coverage debt, not a fabricated successful call.
            coverage_only = (
                analysis["host_finalized"] and totals is not None and not any(totals.values())
                and not analysis["constructor_exclusions"] and not analysis["configuration_error"]
                and not analysis["run_error"] and not analysis["failure_packet_count"]
                and not analysis["unresolved_packet_count"]
                and any(row["entered"] == 0 for row in analysis["rows"].values())
                and all(not any(row[key] for key in ("failed", "unsupported", "open", "provisional"))
                        and row["entered"] == row["reference_completed"] == row["checked"] == row["passed"]
                        for row in analysis["rows"].values()))
            if not coverage_only:
                global_evidence_errors.append({"job_id": job["id"], "finalization_error": analysis["finalization_error"]})
        for ident, row in analysis["rows"].items():
            target = functions[ident][job["revision"]]
            for key in COUNTERS:
                target[key] += row[key]
            if row.get("constructor_exclusion"):
                target["constructor_exclusions"].append({"job_id": job["id"], **row["constructor_exclusion"]})
            if row["first_failure"] is not None and target["first_failure"] is None:
                target["first_failure"] = {"job_id": job["id"], "report": job["report"], **row["first_failure"]}
            if row["body"] is not None:
                target["body_bindings"].append({"job_id": job["id"], "trajectory_id": analysis["trajectory_id"],
                                                 "body": row["body"], "effective_contract": row["effective_contract"]})
        for index, obligation in enumerate(analysis["obligations"]):
            obligations.setdefault(obligation["callee_inventory_id"], []).append({
                "job_id": job["id"], "report": job["report"], "report_sha256": receipt["report_sha256"],
                "pointer": f"/checks/replayed_environment/obligations/{index}",
                **obligation})
    eligible = (finalized and frames_complete and not scope_exclusions
                and not runtime_errors and not global_evidence_errors)
    zero_checked = {revision: [] for revision in ("rev00", "rev02")}
    for ident, revisions in functions.items():
        for revision, row in revisions.items():
            if row["checked"] == 0:
                zero_checked[revision].append(ident)
            row["eligible"] = (row["checked"] > 0
                               and row["entered"] == row["reference_completed"] == row["checked"] == row["passed"]
                               and not any(row[key] for key in ("failed", "unsupported", "open", "provisional"))
                               and not row["constructor_exclusions"])
            eligible &= row["eligible"]
    return {"schema_version": 1, "proof_tier": "ReplayedEnvironment", "complete": eligible,
            "functions": functions, "zero_checked": zero_checked,
            "constructor_exclusions_force_scope_incomplete": scope_exclusions,
            "all_host_reports_finalized": finalized, "all_input_frames_completed": frames_complete,
            "runtime_errors": runtime_errors, "global_evidence_errors": global_evidence_errors,
            "finalization_errors": finalization_errors,
            "obligations_by_callee": obligations,
            "identity_note": "Body/trajectory/invocation handles are scoped to each job; serialized descriptions never mint or import a Host proof."}


def run_campaign(args: argparse.Namespace) -> bool:
    if not 1 <= args.workers <= 10:
        raise CampaignError("worker count must be between one and ten")
    repo, out = args.repo.resolve(), args.out.resolve()
    build = load_json(args.build_receipt)
    worker = Path(build["worker"])
    if digest(worker) != build["binary_sha256"] or source_identity(repo) != build["source"]:
        raise CampaignError("worker/source does not match the controlled build receipt")
    catalogue = read_catalogue(worker, args.env_script)
    if canonical_hash(catalogue) != build["catalogue_sha256"]:
        raise CampaignError("actual body availability changed since the controlled build")
    roots = resolve_roots(catalogue, args.function, args.all_pending)
    session_filter = None
    if args.session:
        try:
            session_filter = sorted({
                (path if path.is_absolute() else repo / path).resolve().relative_to(repo).as_posix()
                for path in args.session
            })
        except ValueError as error:
            raise CampaignError("session filter must stay inside the selected repository") from error
    roms = {"rev00": args.rom_rev00.resolve(), "rev02": args.rom_rev02.resolve()}
    identity = {"source": build["source"], "binary_sha256": build["binary_sha256"],
                "catalogue": catalogue, "roots": roots,
                "session_filter": session_filter,
                "rom_sha256": {revision: digest(path) for revision, path in roms.items()}}
    manifest_path = out / "manifest.json"
    if manifest_path.exists() and load_json(manifest_path)["identity"] != identity:
        raise CampaignError("campaign identity changed; use a new output directory")
    jobs = plan_jobs(repo, out, identity, session_filter)
    atomic_json(manifest_path, {"schema_version": 1, "identity": identity, "jobs": jobs,
                                "workers": args.workers, "build_receipt": str(args.build_receipt.resolve())})
    completed = []
    remaining = []
    for job in jobs:
        receipt = reusable(job, roots)
        (completed if receipt is not None else remaining).append(receipt if receipt is not None else job)
    iterator = iter(sorted(remaining, key=lambda job: (-job["frames"], job["id"])))
    invalid_jobs = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        active = {}
        for _ in range(min(args.workers, len(remaining))):
            job = next(iterator)
            active[pool.submit(execute_job, job, worker, args.env_script, roms, roots)] = job
        while active:
            settled, _ = wait(active, return_when=FIRST_COMPLETED)
            for future in settled:
                job = active.pop(future)
                try:
                    code, command = future.result()
                    receipt = complete_job(job, code, command, roots)
                    completed.append(receipt)
                    print(f"completed {len(completed)}/{len(jobs)} {job['id']}", flush=True)
                except (OSError, ValueError, KeyError) as error:
                    failure = {"job_id": job["id"], "error": str(error),
                               "report": job["report"], "log": job["log"]}
                    invalid_jobs.append(failure)
                    atomic_json(out / "errors" / f"{job['id']}.json", failure)
                next_job = next(iterator, None)
                if next_job is not None:
                    active[pool.submit(execute_job, next_job, worker, args.env_script, roms, roots)] = next_job
    summary = summarize(completed, roots)
    summary.update(identity=identity, sessions=len(jobs), receipts=[job["receipt"] for job in jobs])
    summary["unfinished_or_invalid_jobs"] = invalid_jobs
    summary["scope_complete"] = summary["complete"] and not invalid_jobs and len(completed) == len(jobs)
    summary["scope_kind"] = "partial-session-scope" if session_filter is not None else "whole-committed-corpus"
    summary["whole_corpus_complete"] = summary["scope_complete"] and session_filter is None
    summary["complete"] = summary["whole_corpus_complete"]
    atomic_json(out / "result.json", summary)
    return summary["scope_complete"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    build = sub.add_parser("build")
    build.add_argument("--repo", type=Path, default=ROOT)
    build.add_argument("--target-dir", type=Path, required=True)
    build.add_argument("--out", type=Path, required=True)
    build.add_argument("--env-script", type=Path, default=ENV)
    run = sub.add_parser("run")
    run.add_argument("--repo", type=Path, default=ROOT)
    run.add_argument("--build-receipt", type=Path, required=True)
    run.add_argument("--rom-rev00", type=Path, required=True)
    run.add_argument("--rom-rev02", type=Path, required=True)
    run.add_argument("--out", type=Path, required=True)
    run.add_argument("--workers", type=int, choices=range(1, 11), default=10)
    run.add_argument("--env-script", type=Path, default=ENV)
    run.add_argument("--session", type=Path, action="append", default=[],
                     help="Repeatable committed-session filter, relative to --repo; always partial scope")
    selection = run.add_mutually_exclusive_group(required=True)
    selection.add_argument("--function", action="append", default=[])
    selection.add_argument("--all-pending", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "build":
            build_worker(args.repo.resolve(), args.target_dir.resolve(), args.out.resolve(), args.env_script)
            return 0
        return 0 if run_campaign(args) else 1
    except (CampaignError, OSError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"replay_campaign: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
