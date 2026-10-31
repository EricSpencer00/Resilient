#!/usr/bin/env python3
"""Bounded controlled requirements, explicit-state checking and executable conformance."""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import shutil
import subprocess
import sys


class Failure(Exception):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def require(condition, message):
    if not condition:
        raise Failure(message)


def parse(source):
    require(len(source) <= 65536, "requirements: maximum source size is 64 KiB")
    text = source.decode("utf-8")
    declarations, transitions, properties, identifiers = {}, [], [], set()
    name = r"([a-z][a-z0-9_]{0,31})"
    patterns = [
        ("transition", rf"When state is {name} and event is {name}, next state shall be {name}\."),
        ("after", rf"After event {name}, state shall be {name}\."),
        ("from", rf"From state {name}, event {name} shall lead to state {name}\."),
        ("eventually", rf"Assuming fair event {name}, state {name} shall eventually be reached\."),
    ]
    for line_number, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line:
            continue
        declaration = re.fullmatch(r"(States|Events|Initial): (.+)\.", line)
        if declaration:
            key, value = declaration.groups()
            require(key not in declarations, f"requirements:{line_number}: duplicate {key}")
            values = value.split(", ")
            require(all(re.fullmatch(name, item) for item in values), f"requirements:{line_number}: invalid name")
            require(len(values) == len(set(values)), f"requirements:{line_number}: duplicate name")
            require(1 <= len(values) <= (1 if key == "Initial" else 16), "requirements: declaration size outside profile")
            declarations[key] = values
            continue
        requirement = re.fullmatch(r"([A-Z][A-Z0-9_]{0,31}): (.+)", line)
        require(requirement is not None, f"requirements:{line_number}: unsupported sentence")
        identifier, sentence = requirement.groups()
        require(identifier not in identifiers, f"requirements:{line_number}: duplicate identifier")
        identifiers.add(identifier)
        for kind, pattern in patterns:
            match = re.fullmatch(pattern, sentence)
            if match:
                values = match.groups()
                if kind == "transition":
                    transitions.append(dict(zip(("from", "event", "to"), values), id=identifier))
                else:
                    properties.append({"id": identifier, "kind": kind, "arguments": list(values)})
                break
        else:
            raise Failure(f"requirements:{line_number}: unsupported sentence")
    require(set(declarations) == {"States", "Events", "Initial"}, "requirements: missing declarations")
    states, events, initial = declarations["States"], declarations["Events"], declarations["Initial"][0]
    require(initial in states, "requirements: unknown initial state")
    table = {}
    for row in transitions:
        require(row["from"] in states and row["to"] in states and row["event"] in events, "requirements: unknown transition symbol")
        key = (row["from"], row["event"])
        require(key not in table, "requirements: conflicting or duplicate transition")
        table[key] = row
    require(len(table) == len(states) * len(events), "requirements: transition table must be total")
    for prop in properties:
        args = prop["arguments"]
        domains = (states, events, states) if prop["kind"] == "from" else (events, states)
        require(all(arg in domain for arg, domain in zip(args, domains)), "requirements: unknown property symbol")
    require(properties, "requirements: at least one independent property is required")
    fair_events = {p["arguments"][0] for p in properties if p["kind"] == "eventually"}
    require(len(fair_events) <= 1, "requirements: this profile permits one shared fair-event assumption")
    reachable = {initial}
    while True:
        expanded = reachable | {r["to"] for r in transitions if r["from"] in reachable}
        if expanded == reachable:
            break
        reachable = expanded
    require(reachable == set(states), "requirements: declared state is unreachable")
    return {"profile": "finite-controller-v1", "states": states, "events": events, "initial": initial,
            "transitions": [table[s, e] for s in states for e in events], "properties": properties}


def artifacts(source):
    model = parse(source)
    states, events = model["states"], model["events"]
    si, ei = states.index, events.index
    rows = model["transitions"]
    code = ["fn step(int state, int event) -> int {"]
    for row in rows:
        code.append(f'    if state == {si(row["from"])} && event == {ei(row["event"])} {{ return {si(row["to"])}; }}')
    code.extend(["    return -1;", "}", ""])
    tla = ["---- MODULE Controller ----", "EXTENDS Integers", "VARIABLES state, previous, event",
           "vars == <<state, previous, event>>",
           f'Init == /\\ state = {si(model["initial"])} /\\ previous = {si(model["initial"])} /\\ event = -1']
    for index, event in enumerate(events):
        cases = [f'state = {si(r["from"])} -> {si(r["to"])}' for r in rows if r["event"] == event]
        tla.append(f"Event{index} == /\\ previous' = state /\\ event' = {index} /\\ state' = CASE " + " [] ".join(cases))
    tla.append("Next == " + " \\/ ".join(f"Event{i}" for i in range(len(events))))
    fairness = sorted({ei(p["arguments"][0]) for p in model["properties"] if p["kind"] == "eventually"})
    tla.append("Spec == Init /\\ [][Next]_vars" + "".join(f" /\\ WF_vars(Event{i})" for i in fairness))
    tla.append(f"TypeOK == /\\ state \\in 0..{len(states)-1} /\\ previous \\in 0..{len(states)-1} /\\ event \\in -1..{len(events)-1}")
    cfg = ["SPECIFICATION Spec", "INVARIANT TypeOK"]
    for index, prop in enumerate(model["properties"]):
        args, kind = prop["arguments"], prop["kind"]
        if kind == "after":
            expression = f"event = {ei(args[0])} => state = {si(args[1])}"
        elif kind == "from":
            expression = f"(previous = {si(args[0])} /\\ event = {ei(args[1])}) => state = {si(args[2])}"
        else:
            expression = f"<>(state = {si(args[1])})"
        tla.append(f"Property{index} == {expression}")
        cfg.append(("PROPERTY " if kind == "eventually" else "INVARIANT ") + f"Property{index}")
    tla.extend(["====", ""])
    files = {"requirements.txt": source, "model.json": encoded(model), "implementation.rz": "\n".join(code).encode(),
             "Controller.tla": "\n".join(tla).encode(), "Controller.cfg": ("\n".join(cfg) + "\n").encode()}
    files["manifest.json"] = encoded({"profile": model["profile"], "generator_sha256": digest(Path(__file__).read_bytes()),
                                      "artifacts": {name: digest(data) for name, data in files.items()}})
    return model, files


def validate(bundle):
    model, expected = artifacts((bundle / "requirements.txt").read_bytes())
    require({p.name for p in bundle.iterdir()} == set(expected), "artifact mismatch: bundle inventory")
    for name, data in expected.items():
        require((bundle / name).is_file() and not (bundle / name).is_symlink() and (bundle / name).read_bytes() == data,
                f"artifact mismatch: {name}")
    return model, expected


def executable(value):
    found = shutil.which(value)
    require(found is not None, f"executable not available: {value}")
    return str(Path(found).resolve())


def run(command, output, label, env=None, timeout=60):
    (output / f"{label}-command.json").write_bytes(encoded(command))
    try:
        result = subprocess.run(command, cwd=output, env=env, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        (output / f"{label}.log").write_bytes((error.stdout or b"") + (error.stderr or b""))
        raise Failure(f"{label}: timed out") from error
    (output / f"{label}.log").write_bytes(result.stdout + result.stderr)
    (output / f"{label}-stdout.txt").write_bytes(result.stdout)
    (output / f"{label}-stderr.txt").write_bytes(result.stderr)
    require(result.returncode == 0, f"{label}: executable exited {result.returncode}; see {label}.log")
    return result.stdout.decode("utf-8")


def check_model(args, model, output, report):
    report["model_check"] = "failed"
    jar = args.jar.resolve()
    require(jar.is_file(), "TLC jar is unavailable")
    java = executable("java")
    report["tlc_jar_sha256"] = digest(jar.read_bytes())
    run([java, "-version"], output, "java-version")
    run([java, "-Xmx512m", "-cp", str(jar), "tlc2.TLC", "-workers", "1", "-fp", "0", "-cleanup",
         "-dump", "states.dump", "Controller"], output, "tlc", timeout=args.timeout_seconds)
    log = (output / "tlc.log").read_text()
    require("Model checking completed. No error has been found." in log, "TLC did not confirm completed checking")
    dump = (output / "states.dump").read_text()
    observed = set()
    for block in re.split(r"(?m)^State \d+:.*\n", dump)[1:]:
        values = dict(re.findall(r"/\\ (state|previous|event) = (-?\d+)", block))
        require(set(values) == {"state", "previous", "event"}, "unrecognized TLC state dump")
        observed.add(tuple(int(values[key]) for key in ("previous", "event", "state")))
    si, ei = model["states"].index, model["events"].index
    expected = {(si(r["from"]), ei(r["event"]), si(r["to"])) for r in model["transitions"]}
    initial = si(model["initial"])
    require(observed == expected | {(initial, -1, initial)}, "model transition mismatch or incomplete TLC state dump")
    report.update(model_check="passed", model_transitions=len(expected))


def check_runtime(args, model, output, report):
    rz = executable(args.rz)
    report["compiler_sha256"] = digest(Path(rz).read_bytes())
    report["compiler_path"] = rz
    candidate = args.implementation.read_bytes() if args.implementation else (output / "implementation.rz").read_bytes()
    (output / "candidate.rz").write_bytes(candidate)
    report["candidate_sha256"] = digest(candidate)
    si, ei = model["states"].index, model["events"].index
    rows = model["transitions"]
    harness = candidate + b"\n" + ("\n".join(f'println(step({si(r["from"])}, {ei(r["event"])}));' for r in rows) + "\n").encode()
    (output / "harness.rz").write_bytes(harness)
    expected = [str(si(row["to"])) for row in rows]
    environment = {k: v for k, v in os.environ.items() if not k.startswith("RESILIENT_")}
    report["runtime_observations"] = 0
    report["observations"] = {}
    for mode in ("interpreter", "match", "direct"):
        env = dict(environment, RESILIENT_DISPATCH=mode if mode != "interpreter" else "match")
        command = [rz, "--no-cache"] + ([] if mode == "interpreter" else ["--vm"]) + ["harness.rz"]
        actual = run(command, output, mode, env, args.timeout_seconds).splitlines()
        if actual and actual[-1] == "Program executed successfully":
            actual.pop()
        report["observations"][mode] = [{"requirement": row["id"], "from": row["from"], "event": row["event"], "expected": expected[i], "actual": actual[i] if i < len(actual) else None} for i, row in enumerate(rows)]
        require(actual == expected, f"transition mismatch: {mode}: expected {expected}, observed {actual}")
        report["runtime_observations"] += len(actual)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    generate = sub.add_parser("generate")
    generate.add_argument("source", type=Path)
    generate.add_argument("--output", type=Path, required=True)
    for command in ("check-runtime", "verify"):
        child = sub.add_parser(command)
        child.add_argument("bundle", type=Path)
        child.add_argument("--rz", required=True)
        child.add_argument("--timeout-seconds", type=float, default=60)
        child.add_argument("--implementation", type=Path)
        child.add_argument("--output", type=Path, required=True)
        if command == "verify":
            child.add_argument("--jar", type=Path, required=True)
    args = parser.parse_args()
    report = None
    try:
        if args.command == "generate":
            _, files = artifacts(args.source.read_bytes())
        else:
            require(0 < args.timeout_seconds <= 60, "timeout must be positive and at most 60 seconds")
            model, files = validate(args.bundle)
        output = args.output.resolve()
        require(not output.exists(), "output already exists; use a fresh evidence directory")
        output.mkdir(parents=True)
        for name, data in files.items():
            (output / name).write_bytes(data)
        if args.command != "generate":
            report = {"status": "failed", "certified": False, "model_check": "not-run", "runtime_observations": 0,
                      "scope": "finite-controller-v1 instance conformance; human intent and target deployment not verified",
                      "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine()},
                      "python": sys.version, "generator_sha256": digest(Path(__file__).read_bytes())}
            if args.command == "verify":
                check_model(args, model, output, report)
            check_runtime(args, model, output, report)
            report["status"] = "passed"
        return 0
    except (Failure, OSError, UnicodeError, ValueError) as error:
        if report is not None:
            report["error"] = str(error)
        print(str(error), file=sys.stderr)
        return 1
    finally:
        if report is not None:
            (output / "report.json").write_bytes(encoded(report))
            (output / "SHA256SUMS").write_text("".join(f"{digest(p.read_bytes())}  {p.name}\n" for p in sorted(output.iterdir()) if p.is_file() and p.name != "SHA256SUMS"))


if __name__ == "__main__":
    sys.exit(main())
