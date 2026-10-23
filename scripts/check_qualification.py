#!/usr/bin/env python3
"""Check qualification evidence intake completeness, never certify a tool."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

COMMON = {"intended-use", "baseline", "requirements", "validation", "anomalies", "change-control", "independent-review"}
PROFILES = {
    "iso-26262": {"tool-confidence", "qualification-method", "safety-plan"},
    "do-178c": {"tool-qualification", "formal-methods-credit", "certification-plan"},
    "iec-62304": {"tool-validation", "risk-management", "software-classification"},
    "iec-61508": {"support-tool-assessment", "sil-methods", "functional-safety-plan"},
}


def text(value):
    return isinstance(value, str) and bool(value.strip())


def check(package, root):
    """Return actionable gaps; all artifact paths are relative to the package."""
    gaps = []
    if not isinstance(package, dict):
        return ["package must be an object"]
    if type(package.get("schema_version")) is not int or package["schema_version"] != 1:
        gaps.append("schema_version must be 1")
    for field in ("project", "intended_use", "owner"):
        if not text(package.get(field)):
            gaps.append(f"{field}: required nonempty text")
    baseline = package.get("baseline")
    if not isinstance(baseline, dict):
        baseline = {}
    for field in ("source_revision", "compiler_sha256", "target", "backend", "build_flags", "solver_version"):
        if not text(baseline.get(field)):
            gaps.append(f"baseline.{field}: required")
    if not re.fullmatch(r"[0-9a-f]{40}", str(baseline.get("source_revision", ""))):
        gaps.append("baseline.source_revision: expected full Git SHA")
    if not re.fullmatch(r"[0-9a-f]{64}", str(baseline.get("compiler_sha256", ""))):
        gaps.append("baseline.compiler_sha256: expected SHA-256")
    standards = package.get("standards")
    if not isinstance(standards, list) or not standards or any(not isinstance(s, str) for s in standards):
        gaps.append("standards: select one or more supported profiles")
        standards = []
    required = set(COMMON)
    for standard in standards:
        if standard not in PROFILES:
            gaps.append(f"unsupported standard: {standard}")
        else:
            required.update(PROFILES[standard])
    controls = package.get("controls")
    if not isinstance(controls, list):
        gaps.append("controls must be a list")
        controls = []
    seen = set()
    valid_controls = set()
    root = Path(root).resolve()
    for index, control in enumerate(controls):
        if not isinstance(control, dict):
            gaps.append(f"controls[{index}]: expected object")
            continue
        ident = control.get("id")
        if not text(ident):
            gaps.append(f"controls[{index}]: missing id")
            continue
        before = len(gaps)
        if ident in seen:
            gaps.append(f"{ident}: duplicate control")
        seen.add(ident)
        if control.get("status") != "reviewed":
            gaps.append(f"{ident}: evidence not reviewed")
        for field in ("owner", "reviewer", "review_record"):
            if not text(control.get(field)):
                gaps.append(f"{ident}: missing {field}")
        if control.get("owner") == control.get("reviewer"):
            gaps.append(f"{ident}: reviewer must differ from evidence owner")
        artifacts = control.get("artifacts")
        if not isinstance(artifacts, list) or not artifacts:
            gaps.append(f"{ident}: no evidence artifacts")
            artifacts = []
        for artifact in artifacts:
            if not isinstance(artifact, dict) or not text(artifact.get("path")):
                gaps.append(f"{ident}: invalid artifact")
                continue
            path = Path(artifact["path"])
            try:
                resolved = (root / path).resolve()
                if path.is_absolute() or not resolved.is_relative_to(root):
                    raise ValueError("artifact must stay within package directory")
                if not resolved.is_file():
                    raise ValueError("artifact file missing")
                digest = hashlib.sha256(resolved.read_bytes()).hexdigest()
                if digest != artifact.get("sha256"):
                    raise ValueError("artifact SHA-256 mismatch")
            except (OSError, ValueError, RuntimeError) as exc:
                gaps.append(f"{ident}: {artifact['path']}: {exc}")
        if len(gaps) == before:
            valid_controls.add(ident)
    for ident in sorted(required - seen):
        gaps.append(f"missing control: {ident}")
    traces = package.get("traceability")
    if not isinstance(traces, list) or not traces:
        gaps.append("traceability: at least one requirement-to-evidence record required")
        traces = []
    trace_ids = set()
    for index, trace in enumerate(traces):
        if not isinstance(trace, dict):
            gaps.append(f"traceability[{index}]: expected object")
            continue
        for field in ("requirement", "source", "test", "result"):
            if not text(trace.get(field)):
                gaps.append(f"traceability[{index}]: missing {field}")
        requirement = trace.get("requirement")
        if text(requirement):
            if requirement in trace_ids:
                gaps.append(f"duplicate requirement: {requirement}")
            trace_ids.add(requirement)
        if trace.get("result") != "pass":
            gaps.append(f"traceability[{index}]: result must be pass")
        if not text(trace.get("evidence_control")) or trace["evidence_control"] not in valid_controls:
            gaps.append(f"traceability[{index}]: evidence_control must reference reviewed evidence")
    issues = package.get("open_issues")
    if not isinstance(issues, list):
        gaps.append("open_issues: explicit list required")
    elif issues:
        gaps.append(f"open_issues: {len(issues)} unresolved; resolve before intake completion")
    return gaps


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    args = parser.parse_args()
    try:
        package = json.loads(args.package.read_text(encoding="utf-8"))
        gaps = check(package, args.package.parent)
    except (OSError, ValueError, RecursionError) as exc:
        gaps = [f"invalid package: {exc}"]
    print(json.dumps({"status": "gaps" if gaps else "intake-complete", "certified": False,
                      "scope": "Evidence inventory and integrity only; assessor acceptance is external.",
                      "gaps": gaps}, indent=2))
    return 1 if gaps else 0


if __name__ == "__main__":
    sys.exit(main())
