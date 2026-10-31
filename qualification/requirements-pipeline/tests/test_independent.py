"""Generator fault injection against unchanged, independently authored requirements."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import pipeline

SOURCE = b"""States: safe, running.
Events: start, stop.
Initial: safe.
R1: When state is safe and event is start, next state shall be running.
R2: When state is safe and event is stop, next state shall be safe.
R3: When state is running and event is start, next state shall be running.
R4: When state is running and event is stop, next state shall be safe.
S1: After event stop, state shall be safe.
L1: Assuming fair event stop, state safe shall eventually be reached.
"""


class IndependentCorrespondence(unittest.TestCase):
    def test_consistently_wrong_generator_translation_is_rejected(self):
        original = pipeline.parse(SOURCE)
        for field in ("transition", "initial", "encoding", "property", "fairness"):
            with self.subTest(field=field):
                wrong = copy.deepcopy(original)
                if field == "transition":
                    wrong["transitions"][2]["to"] = "safe"
                elif field == "initial":
                    wrong["initial"] = "running"
                elif field == "encoding":
                    wrong["states"].reverse()
                elif field == "property":
                    wrong["properties"][0]["arguments"][1] = "running"
                else:
                    wrong["properties"][1]["arguments"][0] = "start"
                with patch.object(pipeline, "parse", return_value=wrong):
                    with self.assertRaisesRegex(pipeline.Failure, "independent"):
                        pipeline.artifacts(SOURCE)

    def test_generated_artifact_mutants_are_rejected(self):
        from independent_checker import check, CorrespondenceError
        model, files = pipeline.artifacts(SOURCE)
        mutations = [
            ("implementation.rz", b"return 1;", b"return 0;"),
            ("implementation.rz", b"state == 0 && event == 0", b"state == 1 && event == 0"),
            ("implementation.rz", b"    return -1;", b'    println("effect"); return -1;'),
            ("Controller.tla", b"Property0 == event = 1 => state = 0", b"Property0 == TRUE"),
            ("Controller.tla", b"WF_vars(Event1)", b"WF_vars(Event0)"),
            ("Controller.tla", b"state = 0 -> 1", b"state = 0 -> 0"),
            ("Controller.cfg", b"INVARIANT Property0\n", b""),
        ]
        for name, before, after in mutations:
            with self.subTest(artifact=name, before=before):
                self.assertIn(before, files[name])
                wrong = dict(files)
                wrong[name] = wrong[name].replace(before, after, 1)
                with self.assertRaises(CorrespondenceError):
                    check(SOURCE, model, wrong)

    def test_independent_checker_does_not_reuse_generator_parser(self):
        from independent_checker import check
        model, files = pipeline.artifacts(SOURCE)
        with patch.object(pipeline, "parse", side_effect=AssertionError("generator parser called")):
            result = check(SOURCE, model, files)
        self.assertEqual(result["transition_pairs"], 4)
        self.assertEqual(result["properties"], 2)

    def test_other_controller_shapes_and_orderings(self):
        from independent_checker import check
        for states, events in [(1, 1), (2, 3), (5, 2), (16, 16)]:
            with self.subTest(states=states, events=events):
                lines = ["States: " + ", ".join(f"s{i}" for i in range(states)) + ".",
                         "Events: " + ", ".join(f"e{i}" for i in range(events)) + ".", "Initial: s0."]
                for state in range(states):
                    for event in range(events):
                        target = (state + 1) % states
                        lines.append(f"R{state}_{event}: When state is s{state} and event is e{event}, next state shall be s{target}.")
                lines.append(f"S1: From state s0, event e0 shall lead to state s{1 % states}.")
                source = ("\n".join(lines) + "\n").encode()
                model, files = pipeline.artifacts(source)
                result = check(source, model, files)
                self.assertEqual(result["transition_pairs"], states * events)


if __name__ == "__main__":
    unittest.main()
