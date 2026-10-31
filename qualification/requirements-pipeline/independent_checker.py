"""Independent finite-profile translation checks; no imports from the generator."""
import re


class CorrespondenceError(ValueError):
    pass


def insist(condition, message):
    if not condition:
        raise CorrespondenceError("independent correspondence: " + message)


def source_model(source):
    insist(len(source) <= 65536, "source exceeds profile limit")
    declarations, rows, properties, identifiers = {}, {}, [], set()
    grammars = [
        ("transition", "When state is $from and event is $event , next state shall be $to ."),
        ("after", "After event $event , state shall be $to ."),
        ("from", "From state $from , event $event shall lead to state $to ."),
        ("eventually", "Assuming fair event $event , state $to shall eventually be reached ."),
    ]
    for number, line in enumerate(source.decode("utf-8").splitlines(), 1):
        words = re.findall(r"[A-Za-z_][A-Za-z_0-9]*|[^\s]", line)
        if not words:
            continue
        insist(len(words) > 3 and words[1] == ":" and words[-1] == ".", f"invalid sentence at line {number}")
        label = words[0]
        if label in ("States", "Events", "Initial"):
            insist(label not in declarations, "duplicate declaration")
            values = words[2:-1:2]
            insist(words[3:-1:2] == [","] * (len(values) - 1), "invalid declaration separators")
            insist(1 <= len(values) <= (1 if label == "Initial" else 16), "declaration size outside profile")
            insist(len(set(values)) == len(values), "duplicate domain symbol")
            insist(all(re.fullmatch(r"[a-z][a-z0-9_]{0,31}", value) for value in values), "invalid domain symbol")
            declarations[label] = values
            continue
        insist(re.fullmatch(r"[A-Z][A-Z0-9_]{0,31}", label) and label not in identifiers, "invalid or duplicate requirement ID")
        identifiers.add(label)
        for kind, grammar in grammars:
            pattern = grammar.split()
            sentence = words[2:]
            if len(pattern) != len(sentence):
                continue
            fields = {}
            for expected, actual in zip(pattern, sentence):
                if expected.startswith("$"):
                    fields[expected[1:]] = actual
                elif expected != actual:
                    break
            else:
                if kind == "transition":
                    key = (fields["from"], fields["event"])
                    insist(key not in rows, "duplicate transition pair")
                    rows[key] = dict(fields, id=label)
                else:
                    arguments = [fields[k] for k in (("from", "event", "to") if kind == "from" else ("event", "to"))]
                    properties.append({"id": label, "kind": kind, "arguments": arguments})
                break
        else:
            raise CorrespondenceError(f"independent correspondence: unknown sentence at line {number}")
    insist(set(declarations) == {"States", "Events", "Initial"}, "missing declarations")
    states, events = declarations["States"], declarations["Events"]
    initial = declarations["Initial"][0]
    insist(initial in states, "initial state outside domain")
    domain = {(state, event) for state in states for event in events}
    insist(set(rows) == domain and all(r["to"] in states for r in rows.values()), "transition domain mismatch")
    insist(properties, "missing properties")
    fair = set()
    for prop in properties:
        args = prop["arguments"]
        domains = (states, events, states) if prop["kind"] == "from" else (events, states)
        insist(all(a in d for a, d in zip(args, domains)), "property outside domain")
        if prop["kind"] == "eventually":
            fair.add(args[0])
    insist(len(fair) <= 1, "different fairness assumptions")
    reached = {initial}
    for _ in states:
        reached |= {r["to"] for r in rows.values() if r["from"] in reached}
    insist(reached == set(states), "unreachable state")
    return {"profile": "finite-controller-v1", "states": states, "events": events, "initial": initial,
            "transitions": [rows[s, e] for s in states for e in events], "properties": properties}


def check_code(model, data):
    tokens = re.findall(r"[A-Za-z_][A-Za-z_0-9]*|[0-9]+|&&|==|->|[^\s]", data.decode("utf-8"))
    header = "fn step ( int state , int event ) -> int {".split()
    insist(tokens[:len(header)] == header, "unsupported function interface")
    cursor, table = len(header), {}
    while cursor < len(tokens) and tokens[cursor] == "if":
        expected = "if state == $s && event == $e { return $t ; }".split()
        branch = tokens[cursor:cursor + len(expected)]
        values = []
        for actual, template in zip(branch, expected):
            if template.startswith("$"):
                insist(actual.isascii() and actual.isdigit(), "nonliteral generated branch")
                values.append(int(actual))
            else:
                insist(actual == template, "unsupported generated code or hidden effects")
        insist(len(branch) == len(expected) and len(values) == 3, "incomplete branch")
        state, event, target = values
        insist((state, event) not in table, "duplicate generated guard")
        table[state, event] = target
        cursor += len(expected)
    insist(tokens[cursor:] == ["return", "-", "1", ";", "}"], "unsupported fallback or hidden effects")
    expected_table = {(model["states"].index(r["from"]), model["events"].index(r["event"])):
                      model["states"].index(r["to"]) for r in model["transitions"]}
    insist(table == expected_table, "generated code transition mismatch")


def check_tla(model, files):
    states, events = model["states"], model["events"]
    sids, eids = {s: i for i, s in enumerate(states)}, {e: i for i, e in enumerate(events)}
    initial = sids[model["initial"]]
    expected = ["---- MODULE Controller ----", "EXTENDS Integers", "VARIABLES state, previous, event",
                "vars == <<state, previous, event>>", f"Init == /\\ state = {initial} /\\ previous = {initial} /\\ event = -1"]
    relation = {(r["from"], r["event"]): r["to"] for r in model["transitions"]}
    for event, event_id in eids.items():
        clauses = [f"state = {state_id} -> {sids[relation[state, event]]}" for state, state_id in sids.items()]
        expected.append(f"Event{event_id} == /\\ previous' = state /\\ event' = {event_id} /\\ state' = CASE " + " [] ".join(clauses))
    expected.append("Next == " + " \\/ ".join(f"Event{i}" for i in eids.values()))
    assumptions = {eids[p["arguments"][0]] for p in model["properties"] if p["kind"] == "eventually"}
    expected.append("Spec == Init /\\ [][Next]_vars" + "".join(f" /\\ WF_vars(Event{i})" for i in sorted(assumptions)))
    expected.append(f"TypeOK == /\\ state \\in 0..{len(states)-1} /\\ previous \\in 0..{len(states)-1} /\\ event \\in -1..{len(events)-1}")
    config = ["SPECIFICATION Spec", "INVARIANT TypeOK"]
    for i, prop in enumerate(model["properties"]):
        args = prop["arguments"]
        if prop["kind"] == "after":
            formula = f"event = {eids[args[0]]} => state = {sids[args[1]]}"
        elif prop["kind"] == "from":
            formula = f"(previous = {sids[args[0]]} /\\ event = {eids[args[1]]}) => state = {sids[args[2]]}"
        else:
            formula = f"<>(state = {sids[args[1]]})"
        expected.append(f"Property{i} == {formula}")
        config.append(("PROPERTY" if prop["kind"] == "eventually" else "INVARIANT") + f" Property{i}")
    expected.append("====")
    for name, lines in (("Controller.tla", expected), ("Controller.cfg", config)):
        insist(" ".join(files[name].decode("utf-8").split()) == " ".join(" ".join(lines).split()), f"{name} differs from independently derived semantics")


def check(source, model, files):
    independent = source_model(source)
    insist(model == independent, "generator IR differs from original requirements")
    check_code(independent, files["implementation.rz"])
    check_tla(independent, files)
    return {"transition_pairs": len(independent["transitions"]), "properties": len(independent["properties"]),
            "scope": "separate finite-profile source/artifact checker; not a general refinement proof"}
