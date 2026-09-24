#!/usr/bin/env python3
"""Run an end-to-end command and write verifiable evidence without saving logs."""

from __future__ import annotations

import argparse
import datetime as dt
import fcntl
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "resilient-e2e-evidence"
SCHEMA_VERSION = 1
FILES = ("manifest.json", "summary.md", "checksums.sha256", "run-metadata.json")
SECRET_FLAG = re.compile(
    r"(?i)^--?(?:token|secret|password|api[-_]?key|access[-_]?key)(?:=|$)"
)
SECRET_ASSIGNMENT = re.compile(
    r"(?i)^(?:[A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|API[_-]?KEY|ACCESS[_-]?KEY))="
)
ANSI_ESCAPE = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")
TEST_OUTCOME = re.compile(rb"^test\s+(.+?)\s+\.\.\.\s+(ok|FAILED|ignored|measured)$")
MAX_RESULT_LINE = 64 * 1024


class EvidenceError(Exception):
    """Raised when an evidence bundle cannot be created or verified."""


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode("utf-8")


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_output(*arguments: str) -> bytes:
    try:
        result = subprocess.run(
            ["git", *arguments],
            cwd=ROOT,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise EvidenceError("git metadata is unavailable") from error
    return result.stdout


def source_identity() -> dict[str, Any]:
    try:
        commit = git_output("rev-parse", "HEAD").decode("ascii").strip()
        tracked_diff = git_output("diff", "--binary", "HEAD")
        untracked_output = git_output(
            "ls-files", "--others", "--exclude-standard", "-z"
        )
    except UnicodeDecodeError as error:
        raise EvidenceError("git metadata is not valid UTF-8") from error

    untracked: list[dict[str, str]] = []
    for raw_path in sorted(p for p in untracked_output.split(b"\0") if p):
        relative = Path(os.fsdecode(raw_path))
        path = ROOT / relative
        if path.is_symlink():
            digest = sha256_bytes(os.readlink(path).encode("utf-8", "surrogateescape"))
        elif path.is_file():
            digest = sha256_file(path)
        else:
            continue
        untracked.append({"path": relative.as_posix(), "sha256": digest})

    dirty = bool(tracked_diff or untracked)
    state = {
        "tracked_diff_sha256": sha256_bytes(tracked_diff),
        "untracked": untracked,
    }
    worktree_hash = sha256_bytes(canonical_json(state)) if dirty else None
    return {
        "commit": commit,
        "dirty": dirty,
        "worktree_sha256": worktree_hash,
    }


def tool_version(executable: str) -> str:
    try:
        result = subprocess.run(
            [executable, "--version"],
            cwd=ROOT,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"
    output = result.stdout or result.stderr
    first_line = output.splitlines()[0] if output.splitlines() else ""
    return first_line[:200] if result.returncode == 0 and first_line else "unavailable"


def toolchain_versions() -> dict[str, str]:
    return {
        "cargo": tool_version("cargo"),
        "python": tool_version(sys.executable),
        "rustc": tool_version("rustc"),
    }


def input_hashes(values: list[str]) -> list[dict[str, str]]:
    records: dict[str, str] = {}
    for value in values:
        supplied = Path(value)
        candidate = supplied if supplied.is_absolute() else ROOT / supplied
        if candidate.is_symlink():
            raise EvidenceError("input files must not be symlinks")
        try:
            path = candidate.resolve(strict=True)
            relative = path.relative_to(ROOT)
        except (OSError, ValueError) as error:
            raise EvidenceError("each input must be a file inside the repository") from error
        if not path.is_file():
            raise EvidenceError("each input must be a regular file")
        name = relative.as_posix()
        if "\n" in name or "\r" in name:
            raise EvidenceError("input paths must not contain newlines")
        records[name] = sha256_file(path)
    return [{"path": name, "sha256": records[name]} for name in sorted(records)]


def run_id_for(identity: dict[str, Any]) -> str:
    return sha256_bytes(canonical_json(identity))


def summary_text(manifest: dict[str, Any]) -> str:
    source = manifest["source"]
    result = manifest["result"]
    lines = [
        "# End-to-end evidence",
        "",
        f"- Run ID: {manifest['run_id']}",
        f"- Result: {result['status']} (exit code {result['exit_code']})",
        f"- Source revision: {source['commit']}",
    ]
    if source["dirty"]:
        lines.append(f"- Working-tree digest: {source['worktree_sha256']}")
    lines.extend(
        [
            f"- Command: {shlex.join(manifest['command'])}",
            "",
            "## Toolchain",
            "",
        ]
    )
    for name, version in manifest["toolchain"].items():
        lines.append(f"- {name}: {version}")
    lines.extend(["", "## Inputs", ""])
    if manifest["inputs"]:
        for item in manifest["inputs"]:
            lines.append(f"- {item['path']}: {item['sha256']}")
    else:
        lines.append("- None declared")
    lines.extend(["", "## Outcome", ""])
    for outcome in result["outcomes"]:
        lines.append(f"- {outcome['name']}: {outcome['status']}")
    return "\n".join(lines) + "\n"


def checksums_text(directory: Path) -> str:
    entries = [
        (name, sha256_file(directory / name))
        for name in ("manifest.json", "summary.md")
    ]
    return "".join(f"{digest}  {name}\n" for name, digest in entries)


def verify_bundle(directory: Path) -> dict[str, Any]:
    if directory.is_symlink() or not directory.is_dir():
        raise EvidenceError("artifact path must be a directory")

    names = {item.name for item in directory.iterdir()}
    if names != set(FILES):
        raise EvidenceError("artifact bundle is missing files or contains unexpected files")
    for name in FILES:
        item = directory / name
        if item.is_symlink() or not item.is_file():
            raise EvidenceError("artifact entries must be regular files")

    try:
        manifest = json.loads((directory / "manifest.json").read_text("utf-8"))
        metadata = json.loads((directory / "run-metadata.json").read_text("utf-8"))
        checksums = (directory / "checksums.sha256").read_text("utf-8")
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EvidenceError("artifact metadata is malformed or incomplete") from error

    if not isinstance(manifest, dict):
        raise EvidenceError("manifest must be a JSON object")
    required = {
        "schema",
        "schema_version",
        "run_id",
        "source",
        "command",
        "toolchain",
        "inputs",
        "result",
        "evidence_sha256",
    }
    if set(manifest) != required:
        raise EvidenceError("manifest fields do not match the supported schema")
    if manifest["schema"] != SCHEMA or manifest["schema_version"] != SCHEMA_VERSION:
        raise EvidenceError("unsupported evidence schema")
    if not isinstance(manifest["source"], dict):
        raise EvidenceError("manifest source metadata is malformed")
    source = manifest["source"]
    if (
        set(source) != {"commit", "dirty", "worktree_sha256"}
        or not isinstance(source["commit"], str)
        or not source["commit"]
        or not isinstance(source["dirty"], bool)
        or (
            source["dirty"]
            and (
                not isinstance(source["worktree_sha256"], str)
                or not re.fullmatch(r"[0-9a-f]{64}", source["worktree_sha256"])
            )
        )
        or (not source["dirty"] and source["worktree_sha256"] is not None)
    ):
        raise EvidenceError("manifest source metadata is incomplete")

    command = manifest["command"]
    toolchain = manifest["toolchain"]
    inputs = manifest["inputs"]
    result = manifest["result"]
    if not isinstance(command, list) or not command or not all(
        isinstance(item, str) for item in command
    ):
        raise EvidenceError("manifest command is malformed")
    if not isinstance(toolchain, dict) or set(toolchain) != {"cargo", "python", "rustc"}:
        raise EvidenceError("manifest toolchain metadata is incomplete")
    if not all(isinstance(value, str) for value in toolchain.values()):
        raise EvidenceError("manifest toolchain metadata is malformed")
    if not isinstance(inputs, list):
        raise EvidenceError("manifest inputs must be a list")
    for item in inputs:
        if (
            not isinstance(item, dict)
            or set(item) != {"path", "sha256"}
            or not isinstance(item["path"], str)
            or not isinstance(item["sha256"], str)
            or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])
            or Path(item["path"]).is_absolute()
            or ".." in Path(item["path"]).parts
        ):
            raise EvidenceError("manifest input entry is malformed")
    if not isinstance(result, dict) or set(result) != {
        "status",
        "exit_code",
        "outcomes",
    }:
        raise EvidenceError("manifest result is incomplete")
    if (
        not isinstance(result["status"], str)
        or result["status"] not in {"passed", "failed"}
        or not isinstance(result["exit_code"], int)
        or isinstance(result["exit_code"], bool)
        or (result["status"] == "passed") != (result["exit_code"] == 0)
        or not isinstance(result["outcomes"], list)
        or not result["outcomes"]
    ):
        raise EvidenceError("manifest result is malformed")
    for outcome in result["outcomes"]:
        if (
            not isinstance(outcome, dict)
            or set(outcome) != {"name", "status"}
            or not isinstance(outcome["name"], str)
            or not isinstance(outcome["status"], str)
            or outcome["status"] not in {"passed", "failed", "ignored", "measured"}
        ):
            raise EvidenceError("manifest outcome is malformed")

    identity = {
        "source": source,
        "command": command,
        "toolchain": toolchain,
        "inputs": inputs,
    }
    expected_run_id = run_id_for(identity)
    if manifest["run_id"] != expected_run_id:
        raise EvidenceError("manifest run ID does not match its inputs")

    summary = (directory / "summary.md").read_text("utf-8")
    expected_summary = summary_text(manifest)
    if summary != expected_summary:
        raise EvidenceError("summary does not match the manifest")

    evidence = manifest["evidence_sha256"]
    if evidence != {"summary.md": sha256_file(directory / "summary.md")}:
        raise EvidenceError("evidence checksum is missing or does not match")

    if checksums != checksums_text(directory):
        raise EvidenceError("checksums file is missing, malformed, or does not match")

    if (
        not isinstance(metadata, dict)
        or set(metadata) != {"schema", "schema_version", "started_utc", "duration_ms"}
        or metadata["schema"] != "resilient-e2e-run-metadata"
        or metadata["schema_version"] != 1
        or not isinstance(metadata["started_utc"], str)
        or not isinstance(metadata["duration_ms"], int)
        or isinstance(metadata["duration_ms"], bool)
        or metadata["duration_ms"] < 0
    ):
        raise EvidenceError("volatile run metadata is malformed")

    return manifest


def output_path_for(value: str | None) -> Path:
    if value is None:
        return ROOT / "target" / "e2e-artifacts" / "latest"
    supplied = Path(value)
    candidate = supplied if supplied.is_absolute() else ROOT / supplied
    if candidate.is_symlink():
        raise EvidenceError("artifact output path must not be a symlink")
    return candidate.absolute()


def write_atomic(path: Path, content: bytes) -> None:
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        with temporary.open("wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def install_bundle(
    output: Path,
    manifest: dict[str, Any],
    summary: str,
    started_utc: str,
    duration_ms: int,
) -> None:
    parent = output.parent
    parent.mkdir(parents=True, exist_ok=True)
    lock_path = parent / f".{output.name}.lock"

    with lock_path.open("a+b") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        if output.exists():
            if output.is_symlink() or not output.is_dir():
                raise EvidenceError("artifact output must be a directory")
            if any(output.iterdir()):
                verify_bundle(output)
            else:
                output.rmdir()
        staging = Path(tempfile.mkdtemp(prefix=f".{output.name}.", dir=parent))
        try:
            summary_bytes = summary.encode("utf-8")
            manifest["evidence_sha256"] = {"summary.md": sha256_bytes(summary_bytes)}
            (staging / "summary.md").write_bytes(summary_bytes)
            (staging / "manifest.json").write_bytes(canonical_json(manifest))
            metadata = {
                "schema": "resilient-e2e-run-metadata",
                "schema_version": 1,
                "started_utc": started_utc,
                "duration_ms": duration_ms,
            }
            (staging / "run-metadata.json").write_bytes(canonical_json(metadata))
            (staging / "checksums.sha256").write_text(
                checksums_text(staging), encoding="utf-8"
            )

            if output.exists():
                backup = parent / f".{output.name}.old.{os.getpid()}"
                os.replace(output, backup)
                try:
                    os.replace(staging, output)
                except OSError:
                    os.replace(backup, output)
                    raise
                shutil.rmtree(backup)
            else:
                os.replace(staging, output)
        finally:
            if staging.exists():
                shutil.rmtree(staging)


def reject_secret_arguments(command: list[str]) -> None:
    if any(
        SECRET_FLAG.search(argument) or SECRET_ASSIGNMENT.search(argument)
        for argument in command
    ):
        raise EvidenceError(
            "secret-like command arguments are not recorded; pass secrets via the environment"
        )


def execute_command(command: list[str]) -> tuple[int, list[dict[str, str]]]:
    try:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except FileNotFoundError:
        return 127, []
    except OSError:
        return 126, []

    outcomes: list[dict[str, str]] = []
    outcomes_lock = threading.Lock()

    def relay(stream: Any, destination: Any) -> None:
        while True:
            line = stream.readline(MAX_RESULT_LINE + 1)
            if not line:
                return
            if len(line) > MAX_RESULT_LINE and not line.endswith(b"\n"):
                while line and not line.endswith(b"\n"):
                    line = stream.readline(MAX_RESULT_LINE + 1)
                continue
            try:
                destination.buffer.write(line)
                destination.flush()
            except (BrokenPipeError, OSError):
                pass

            normalized = ANSI_ESCAPE.sub(b"", line.strip())
            match = TEST_OUTCOME.fullmatch(normalized)
            if match:
                name = match.group(1).decode("utf-8", "replace")
                raw_status = match.group(2).decode("ascii")
                status = {
                    "ok": "passed",
                    "FAILED": "failed",
                    "ignored": "ignored",
                    "measured": "measured",
                }[raw_status]
                with outcomes_lock:
                    outcomes.append({"name": name, "status": status})

    threads = [
        threading.Thread(target=relay, args=(process.stdout, sys.stdout)),
        threading.Thread(target=relay, args=(process.stderr, sys.stderr)),
    ]
    for thread in threads:
        thread.start()
    exit_code = process.wait()
    for thread in threads:
        thread.join()
    outcomes.sort(key=lambda item: (item["name"], item["status"]))
    return exit_code, outcomes


def run_command(args: argparse.Namespace) -> int:
    command = list(args.command)
    if command and command[0] == "--":
        command.pop(0)
    if not command:
        raise EvidenceError("provide a command after --")
    reject_secret_arguments(command)

    source = source_identity()
    inputs = input_hashes(args.input)
    toolchain = toolchain_versions()
    identity = {
        "source": source,
        "command": command,
        "toolchain": toolchain,
        "inputs": inputs,
    }
    run_id = run_id_for(identity)
    output = output_path_for(args.output)
    started = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    start_time = time.monotonic()

    exit_code, outcomes = execute_command(command)
    duration_ms = max(0, int((time.monotonic() - start_time) * 1000))
    status = "passed" if exit_code == 0 else "failed"
    outcomes.append({"name": "command", "status": status})
    outcomes.sort(key=lambda item: (item["name"], item["status"]))
    manifest = {
        "schema": SCHEMA,
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        **identity,
        "result": {
            "status": status,
            "exit_code": exit_code,
            "outcomes": outcomes,
        },
        "evidence_sha256": {},
    }
    summary = summary_text(manifest)
    install_bundle(output, manifest, summary, started, duration_ms)

    print(f"E2E evidence written to {output}")
    try:
        verify_bundle(output)
    except EvidenceError as error:
        raise EvidenceError(f"created evidence failed verification: {error}") from error
    print(f"E2E evidence verified: {run_id}")
    return exit_code


def verify_command(args: argparse.Namespace) -> int:
    path = Path(args.artifact)
    if not path.is_absolute():
        path = ROOT / path
    try:
        manifest = verify_bundle(path)
    except EvidenceError as error:
        print(f"e2e-evidence: {error}", file=sys.stderr)
        return 1
    print(f"verified {manifest['result']['status']} E2E evidence {manifest['run_id']}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Run E2E commands and write repeatable evidence bundles."
    )
    commands = parser.add_subparsers(dest="action", required=True)
    run = commands.add_parser("run", help="run a command and write evidence")
    run.add_argument("--output", help="artifact directory (default: target/e2e-artifacts/latest)")
    run.add_argument(
        "--input",
        action="append",
        default=[],
        help="repository-relative configuration or fixture file to hash",
    )
    run.add_argument("command", nargs=argparse.REMAINDER)
    verify = commands.add_parser("verify", help="validate an evidence bundle")
    verify.add_argument("artifact", help="artifact directory to verify")
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        if args.action == "run":
            return run_command(args)
        return verify_command(args)
    except EvidenceError as error:
        print(f"e2e-evidence: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
