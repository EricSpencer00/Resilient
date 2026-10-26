#!/usr/bin/env python3
"""Hermetic fleet-readiness regressions using real Git and local bare remotes."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
REAL_GIT = shutil.which("git")
REPOSITORY = "EricSpencer00/Resilient"
REMOTE = f"https://github.com/{REPOSITORY}.git"
COPIES = ("fleet-integration-check.py", "sync-integration.sh", "ready-or-bail.sh",
          "verify-scope.sh", "check-overlaps.sh", "claims-ref.sh", "github-rest-fallback.sh")

FAKE = r'''
import copy,json,os,pathlib,sys
state_file=pathlib.Path(os.environ['FLEET_TEST_STATE'])
state=json.loads(state_file.read_text())
args=sys.argv[1:]
tool=pathlib.Path(sys.argv[0]).name
def event(value):
 with open(os.environ['FLEET_TEST_EVENTS'],'a') as output: output.write(value+'\n')
if tool=='fleet' and args==['changesets','--json']:
 event('fleet:get'); print(json.dumps(state['changesets']))
elif tool=='cargo':
 event('cargo:'+args[0]); sys.exit(1 if state.get('cargo_failure')==args[0] else 0)
elif tool=='git':
 if state.get('claims_unavailable') and args[:1] in (['fetch'],['ls-remote']) and any('agent-claims' in arg for arg in args):
  event('claims:unavailable'); sys.exit(128)
 os.execv(os.environ['FLEET_TEST_REAL_GIT'],[os.environ['FLEET_TEST_REAL_GIT'],*args])
elif tool=='gh' and args[:1]==['api'] and '/pulls/' in args[1]:
 if '--jq' in args:
  query=args[args.index('--jq')+1]
  if 'body' in query: print('Refs #99')
  else: sys.exit('unexpected REST field')
 else:
  state['reads']=state.get('reads',0)+1; state_file.write_text(json.dumps(state))
  result=copy.deepcopy(state['pr'])
  if state['reads']>state.get('race_after',999):
   for side,values in state.get('race',{}).items(): result[side].update(values)
  event('gh:get'); print(json.dumps(result))
elif tool=='gh' and args[:2]==['pr','list']:
 if state.get('pr_list_failure'): sys.exit(17)
 if state.get('omit_open_prs'): print('[]'); sys.exit(0)
 own={'number':42,'headRefName':state['pr']['head']['ref'], 'headRefOid':state['pr']['head']['sha'],'files':[{'path':'value.txt'}]}
 print(json.dumps([own]+state.get('other_prs',[])))
elif tool=='gh' and args[:2]==['pr','edit'] and '--add-label' in args:
 event('label:'+args[args.index('--add-label')+1])
elif tool=='gh' and args[:2]==['pr','ready']:
 event('ready')
elif tool=='gh' and args[:2]==['pr','comment']:
 event('comment')
else:
 sys.exit('unexpected isolated tool invocation: '+tool+' '+repr(args))
'''


class FleetIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fleet integration ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "work"
        self.remote = self.root / "remote.git"
        self.state_file = self.root / "state.json"
        self.events_file = self.root / "events.log"
        self.push_file = self.root / "push-attempt"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        # Isolate synthetic fixture commits from user identities/signing configuration.
        self.env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1",
                    "GIT_TERMINAL_PROMPT": "0", "GIT_ALLOW_PROTOCOL": "file",
                    "PYTHONDONTWRITEBYTECODE": "1"}
        self.git("init", "--bare", "--initial-branch=main", str(self.remote), cwd=self.root)
        self.git("init", "--initial-branch=main", str(self.repo), cwd=self.root)
        self.git("config", "user.name", "fleet-fixture")
        self.git("config", "user.email", "fleet-fixture@example.invalid")
        self.git("remote", "add", "origin", str(self.remote))
        self.git("config", f"url.{self.remote}.insteadOf", REMOTE)
        scripts = self.repo / "agent-scripts"
        scripts.mkdir()
        for filename in COPIES:
            shutil.copy2(SCRIPTS / filename, scripts / filename)
        # The copied guardrail must not recursively run its own enclosing test driver.
        (scripts / "agent-handoff.sh").write_text("#!/bin/sh\nexit 0\n")
        (scripts / "agent-handoff.sh").chmod(0o755)
        (self.repo / "value.txt").write_text("base\n")
        (self.repo / "file-claims.json").write_text('{"claims": {}}')
        self.git("add", ".")
        self.git("commit", "-m", "base")
        base = self.git("rev-parse", "HEAD")
        self.git("push", "origin", "HEAD:refs/heads/main")
        self.git("push", "origin", "HEAD:refs/heads/agent-claims")
        self.git("push", "origin", "HEAD:refs/heads/agents/integration")
        self.git("switch", "-c", "topic")
        (self.repo / "value.txt").write_text("feature\n")
        self.git("commit", "-am", "feature")
        head = self.git("rev-parse", "HEAD")
        self.change_id = "c_1234567890"
        self.git("push", "origin", f"HEAD:refs/heads/fleet/{self.change_id}")
        self.git("remote", "remove", "origin")
        self.lease_file = self.repo / ".git" / "fleet-lease.json"
        self.lease = {"repo": "Resilient", "main": "main", "lease_id": "l_fixture", "path": str(self.repo)}
        self.lease_file.write_text(json.dumps(self.lease))
        url = f"https://github.com/{REPOSITORY}/pull/42"
        self.state = {
            "pr": {"number": 42, "state": "open", "author_association": "OWNER", "html_url": url,
                   "head": {"ref": f"fleet/{self.change_id}", "sha": head, "repo": {"full_name": REPOSITORY}},
                   "base": {"ref": "main", "sha": base, "repo": {"full_name": REPOSITORY}}},
            "changesets": [{"id": self.change_id, "repo": "Resilient", "lease_id": "l_fixture",
                            "status": "pr_open", "pr_url": url}],
        }
        for tool in ("gh", "fleet", "cargo", "git"):
            path = self.bin / tool
            path.write_text(f"#!{sys.executable}\n" + FAKE)
            path.chmod(0o755)
        hook = self.repo / ".git/hooks/pre-push"
        hook.write_text('#!/bin/sh\nprintf attempted > "$FLEET_TEST_PUSH"\nexit 1\n')
        hook.chmod(0o755)
        self.env.update({"PATH": str(self.bin) + os.pathsep + os.environ["PATH"],
                         "FLEET_TEST_STATE": str(self.state_file),
                         "FLEET_TEST_EVENTS": str(self.events_file), "FLEET_TEST_PUSH": str(self.push_file),
                         "FLEET_TEST_REAL_GIT": REAL_GIT})

    def git(self, *args, cwd=None):
        return subprocess.run([REAL_GIT, *args], cwd=cwd or self.repo, env=self.env,
                              check=True, text=True, capture_output=True).stdout.strip()

    def invoke(self, ready=False, arguments=()):
        self.state_file.write_text(json.dumps(self.state))
        before = self.git("rev-parse", "HEAD")
        script = "ready-or-bail.sh" if ready else "sync-integration.sh"
        result = subprocess.run(["bash", str(self.repo / "agent-scripts" / script), "--pr", "42", *arguments],
                                cwd=self.repo, env=self.env, text=True, capture_output=True, timeout=30)
        self.assertFalse(self.push_file.exists(), "integration attempted a Git push")
        self.assertEqual(self.git("rev-parse", "HEAD"), before, "integration changed checkout")
        events = self.events_file.read_text().splitlines() if self.events_file.exists() else []
        return result, events

    def refuse(self, ready=False):
        result, events = self.invoke(ready=ready)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertNotIn("ready", events)
        self.assertFalse(any(event.startswith("label:") for event in events), events)
        return result, events

    def test_registered_submission_without_origin_is_verified_without_push(self):
        result, events = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("label:integration-synced", events)
        self.assertIn("no direct code push", result.stdout)

    def test_dry_run_does_not_stamp_labels(self):
        result, events = self.invoke(arguments=("--no-push",))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(any(event.startswith("label:") for event in events))

    def test_full_ready_path_runs_all_cargo_gates_before_mutations(self):
        result, events = self.invoke(ready=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for gate in ("fmt", "clippy", "test"):
            self.assertIn("cargo:" + gate, events)
            self.assertLess(events.index("cargo:" + gate), events.index("label:integration-synced"))
        self.assertLess(events.index("label:integration-synced"), events.index("ready"))
        self.assertIn("label:agent-vetted", events)

    def test_failed_cargo_gate_cannot_mark_ready(self):
        self.state["cargo_failure"] = "test"
        _, events = self.refuse(ready=True)
        self.assertIn("cargo:test", events)

    def test_dirty_and_untracked_sources_are_rejected(self):
        (self.repo / "untracked.rs").write_text("uncommitted source")
        self.refuse()

    def test_tracked_uncommitted_source_is_rejected(self):
        (self.repo / "value.txt").write_text("uncommitted source\n")
        self.refuse()

    def test_fetched_head_must_match_github_metadata(self):
        self.state["pr"]["head"]["sha"] = "b" * 40
        self.refuse()

    def test_fetched_main_must_match_github_metadata(self):
        self.state["pr"]["base"]["sha"] = "e" * 40
        self.refuse()

    def test_non_main_target_is_rejected(self):
        self.state["pr"]["base"]["ref"] = "development"
        self.refuse()

    def test_closed_pr_is_rejected(self):
        self.state["pr"]["state"] = "closed"
        self.refuse()

    def test_different_committed_tree_is_rejected(self):
        (self.repo / "value.txt").write_text("different\n")
        self.git("commit", "-am", "different local tree")
        self.refuse()

    def test_wrong_or_ambiguous_changeset_is_rejected(self):
        self.state["changesets"] *= 2
        self.refuse()

    def test_changeset_from_another_lease_is_rejected(self):
        self.state["changesets"][0]["lease_id"] = "l_other"
        self.refuse()

    def test_missing_changeset_is_rejected(self):
        self.state["changesets"] = []
        self.refuse()

    def test_foreign_pr_is_rejected(self):
        self.state["pr"]["head"]["repo"]["full_name"] = "outsider/Resilient"
        self.refuse()

    def test_untrusted_author_is_rejected(self):
        self.state["pr"]["author_association"] = "CONTRIBUTOR"
        self.refuse()

    def test_head_change_between_observations_is_rejected(self):
        self.state.update({"race_after": 1, "race": {"head": {"sha": "f" * 40}}})
        self.refuse()

    def test_head_change_after_tests_cannot_mark_ready(self):
        self.state.update({"race_after": 2, "race": {"head": {"sha": "f" * 40}}})
        _, events = self.refuse(ready=True)
        self.assertIn("cargo:test", events)

    def test_main_change_between_observations_is_rejected(self):
        self.state.update({"race_after": 1, "race": {"base": {"sha": "e" * 40}}})
        self.refuse()

    def test_unrelated_repository_metadata_does_not_invalidate_source(self):
        self.state.update({"race_after": 1, "race": {"head": {"repo": {"full_name": REPOSITORY, "description": "changed"}}}})
        result, _ = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_stale_main_ancestry_is_rejected(self):
        self.git("switch", "main")
        (self.repo / "value.txt").write_text("new main\n")
        self.git("commit", "-am", "main advanced")
        main = self.git("rev-parse", "HEAD")
        self.git("fetch", str(self.repo), "refs/heads/main:refs/heads/main", cwd=self.remote)
        self.git("switch", "topic")
        self.state["pr"]["base"]["sha"] = main
        self.refuse()

    def test_overlap_uses_verified_base_without_origin(self):
        self.state["other_prs"] = [{"number": 43, "headRefName": "other", "headRefOid": "a" * 40,
                                    "files": [{"path": "value.txt"}]}]
        result, events = self.refuse(ready=True)
        self.assertIn("cargo:test", events)
        self.assertIn("overlap", result.stdout)

    def test_canonical_checkout_missing_lease_cannot_fall_back_to_push(self):
        self.lease_file.unlink()
        self.git("remote", "add", "origin", REMOTE)
        result, _ = self.refuse()
        self.assertIn("fleet lease missing", result.stderr)

    def test_wrong_lease_path_is_rejected(self):
        self.lease["path"] = str(self.root / "another")
        self.lease_file.write_text(json.dumps(self.lease))
        self.refuse()

    def test_missing_fleet_executable_is_rejected(self):
        (self.bin / "fleet").unlink()
        (self.bin / "python3").symlink_to(sys.executable)
        self.env["PATH"] = str(self.bin) + os.pathsep + "/usr/bin:/bin"
        self.assertIsNone(shutil.which("fleet", path=self.env["PATH"]))
        self.refuse()

    def test_malformed_lease_cannot_fall_back_to_legacy_push(self):
        self.lease_file.write_text("{not json")
        self.refuse()

    def test_ssh_host_alias_cannot_hide_the_canonical_project(self):
        self.lease_file.unlink()
        self.git("remote", "add", "origin", "git@github-eric:EricSpencer00/Resilient.git")
        result, _ = self.refuse()
        self.assertIn("fleet lease missing", result.stderr)

    def publish_claims(self, text):
        metadata = self.root / "metadata"
        self.git("clone", "--branch", "agent-claims", str(self.remote), str(metadata), cwd=self.root)
        self.git("config", "user.name", "claims-fixture", cwd=metadata)
        self.git("config", "user.email", "claims-fixture@example.invalid", cwd=metadata)
        (metadata / "file-claims.json").write_text(text)
        self.git("commit", "-am", "claims fixture", cwd=metadata)
        self.git("fetch", str(metadata), "refs/heads/agent-claims:refs/heads/agent-claims", cwd=self.remote)

    def test_missing_registry_on_existing_claims_ref_is_rejected(self):
        metadata = self.root / "metadata"
        self.git("clone", "--branch", "agent-claims", str(self.remote), str(metadata), cwd=self.root)
        self.git("config", "user.name", "claims-fixture", cwd=metadata)
        self.git("config", "user.email", "claims-fixture@example.invalid", cwd=metadata)
        self.git("rm", "file-claims.json", cwd=metadata)
        self.git("commit", "-m", "missing registry", cwd=metadata)
        self.git("fetch", str(metadata), "refs/heads/agent-claims:refs/heads/agent-claims", cwd=self.remote)
        result, _ = self.refuse(ready=True)
        self.assertIn("current claims registry missing", result.stdout)

    def test_confirmed_absent_claims_ref_is_empty(self):
        self.git("update-ref", "-d", "refs/heads/agent-claims", cwd=self.remote)
        result, events = self.invoke(ready=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("ready", events)

    def test_unavailable_pr_list_cannot_be_treated_as_no_overlap(self):
        self.state["pr_list_failure"] = True
        result, _ = self.refuse(ready=True)
        self.assertIn("overlap verification could not complete", result.stdout)

    def test_missing_expected_pr_cannot_be_treated_as_no_overlap(self):
        self.state["omit_open_prs"] = True
        result, _ = self.refuse(ready=True)
        self.assertIn("expected PR head missing", result.stdout)

    def test_non_extension_claim_conflict_blocks_readiness(self):
        self.publish_claims(json.dumps({"claims": {"value.txt": {"branch": "other", "claimed_at": "fixture"}}}))
        self.state["other_prs"] = [{"number":43,"headRefName":"other","headRefOid":"a"*40,"files":[]}]
        result, _ = self.refuse(ready=True)
        self.assertIn("CLAIMED", result.stdout)
        self.assertIn("overlap detected", result.stdout)

    def test_malformed_claims_metadata_blocks_readiness(self):
        self.publish_claims("{not json")
        result, _ = self.refuse(ready=True)
        self.assertIn("invalid claims metadata", result.stdout)

    def test_unavailable_claims_cannot_use_an_empty_fallback(self):
        self.state["claims_unavailable"] = True
        result, _ = self.refuse(ready=True)
        self.assertIn("fresh claims metadata unavailable", result.stdout)

    def test_tool_remote_collision_is_not_overwritten(self):
        self.git("remote", "add", "fleet-verified", str(self.remote))
        result, _ = self.refuse()
        self.assertIn("configured differently", result.stderr)
        self.assertEqual(self.git("config", "--get", "remote.fleet-verified.url"), str(self.remote))

    def test_noncanonical_unmanaged_repository_retains_legacy_mode(self):
        self.state_file.write_text(json.dumps(self.state))
        self.lease_file.unlink()
        self.git("remote", "add", "origin", str(self.remote))
        result = subprocess.run([sys.executable, str(self.repo / "agent-scripts/fleet-integration-check.py"), "--mode"],
                                cwd=self.repo, env=self.env, text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "legacy")


if __name__ == "__main__":
    unittest.main()
