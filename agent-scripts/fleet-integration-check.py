#!/usr/bin/env python3
"""Verify a fleet PR's integration without pushing code or changing checkout."""

import argparse
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import urlsplit

REPOSITORY = "EricSpencer00/Resilient"
REMOTE = f"https://github.com/{REPOSITORY}.git"
TRUSTED = {"OWNER", "MEMBER", "COLLABORATOR"}


class Refusal(RuntimeError):
    pass


def run(*command, cwd=None):
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True,
                          text=True, timeout=60).stdout.strip()


def git(root, *arguments):
    return run("git", *arguments, cwd=root)


def root_and_lease():
    root = Path(run("git", "rev-parse", "--show-toplevel")).resolve()
    lease = Path(git(root, "rev-parse", "--git-path", "fleet-lease.json"))
    if not lease.is_absolute():
        lease = root / lease
    return root, lease


def canonical_origin(root):
    try:
        origin = git(root, "config", "--get", "remote.origin.url")
    except subprocess.CalledProcessError:
        return False
    # Transport rewrites and SSH host aliases must not hide the project path.
    scp = None if "://" in origin else re.fullmatch(r"(?:[^@/]+@)?[^/:]+:(.+)", origin)
    if scp:
        path = scp.group(1)
    else:
        parsed = urlsplit(origin)
        path = parsed.path.lstrip("/")
    path = path.rstrip("/").lower()
    if path.endswith(".git"):
        path = path[:-4]
    return path == REPOSITORY.lower()


def mode(root, lease):
    return "fleet" if lease.exists() or canonical_origin(root) else "legacy"


def require(condition, message):
    if not condition:
        raise Refusal(message)


def metadata(root, number):
    data = json.loads(run("gh", "api", f"repos/{REPOSITORY}/pulls/{number}", cwd=root))
    require(isinstance(data, dict), "invalid PR metadata")
    require(data.get("number") == number and data.get("state") == "open", "PR is not open")
    require(data.get("author_association") in TRUSTED, "PR author lacks trusted association")
    head, base = data.get("head"), data.get("base")
    require(isinstance(head, dict) and isinstance(base, dict), "missing PR refs")
    head_repo, base_repo = head.get("repo"), base.get("repo")
    require(isinstance(head_repo, dict) and isinstance(base_repo, dict)
            and head_repo.get("full_name") == REPOSITORY
            and base_repo.get("full_name") == REPOSITORY, "cross-repository PR refused")
    require(base.get("ref") == "main", "PR must target main")
    require(data.get("html_url") == f"https://github.com/{REPOSITORY}/pull/{number}", "unexpected PR URL")
    for ref in (head, base):
        require(isinstance(ref.get("sha"), str)
                and re.fullmatch(r"[0-9a-f]{40}", ref["sha"]), "invalid PR commit ID")
    return data


def verify(root, lease_path, number, expected_base=None, expected_head=None):
    require(lease_path.is_file(), "fleet lease missing; use fleet checkout before promotion")
    lease = json.loads(lease_path.read_text())
    require(isinstance(lease, dict) and lease.get("repo") == "Resilient"
            and lease.get("main") == "main", "lease does not identify Resilient main")
    require(isinstance(lease.get("path"), str)
            and Path(lease["path"]).resolve() == root, "workspace does not match lease path")
    require(isinstance(lease.get("lease_id"), str) and lease["lease_id"], "lease ID missing")
    branch = git(root, "rev-parse", "--abbrev-ref", "HEAD")
    require(branch not in {"HEAD", "main", "agents/integration"}, "use a named feature branch")
    require(not git(root, "status", "--porcelain", "--untracked-files=all"), "workspace is not clean")

    before = metadata(root, number)
    require((expected_base is None) == (expected_head is None), "both expected commit IDs are required")
    if expected_base is not None:
        require(before["base"]["sha"] == expected_base and before["head"]["sha"] == expected_head,
                "verification context changed after the guardrail; rerun all gates")
    changesets = json.loads(run("fleet", "changesets", "--json", cwd=root))
    require(isinstance(changesets, list), "invalid fleet changeset response")
    matches = [item for item in changesets if isinstance(item, dict)
               and item.get("pr_url") == before["html_url"]]
    require(len(matches) == 1, "registered changeset missing or ambiguous")
    change = matches[0]
    require(change.get("repo") == "Resilient" and change.get("lease_id") == lease["lease_id"],
            "changeset does not belong to this lease")
    require(change.get("status") in {"pr_open", "tested_green"}, "changeset is not an open submission")
    require(isinstance(change.get("id"), str)
            and re.fullmatch(r"c_[0-9a-f]{10,64}", change["id"]), "invalid changeset ID")
    head_ref = f"fleet/{change['id']}"
    require(before["head"].get("ref") == head_ref, "PR branch does not match registered changeset")

    try:
        configured = git(root, "config", "--get", "remote.fleet-verified.url")
    except subprocess.CalledProcessError:
        configured = None
    if configured is None:
        git(root, "remote", "add", "fleet-verified", REMOTE)
    else:
        require(configured == REMOTE, "fleet-verified remote is configured differently")
    main_ref = "refs/remotes/fleet-verified/main"
    pr_ref = f"refs/remotes/fleet-verified/pr-{number}"
    git(root, "fetch", "--no-tags", "fleet-verified",
        f"+refs/heads/main:{main_ref}", f"+refs/heads/{head_ref}:{pr_ref}")
    main_sha, head_sha = git(root, "rev-parse", main_ref), git(root, "rev-parse", pr_ref)
    require(main_sha == before["base"]["sha"] and head_sha == before["head"]["sha"],
            "PR or main changed during fetch; retry from a fresh submission")
    ancestry = subprocess.run(["git", "merge-base", "--is-ancestor", main_sha, head_sha], cwd=root)
    require(ancestry.returncode == 0, "fleet PR is not based on current main; sync and resubmit through fleet")
    local_sha = git(root, "rev-parse", "HEAD")
    tree = git(root, "rev-parse", "HEAD^{tree}")
    require(tree == git(root, "rev-parse", f"{pr_ref}^{{tree}}"), "workspace tree differs from PR head")

    after = metadata(root, number)
    require(all(after[side][field] == before[side][field]
                for side in ("head", "base") for field in ("ref", "sha")),
            "PR metadata changed during verification")
    require(git(root, "rev-parse", "HEAD") == local_sha
            and not git(root, "status", "--porcelain", "--untracked-files=all"),
            "workspace changed during verification")
    return {"repository": REPOSITORY, "remote": "fleet-verified", "changeset": change["id"], "base_sha": main_sha,
            "head_sha": head_sha, "tree_sha": tree}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", action="store_true", help="print fleet or legacy; no mutations")
    parser.add_argument("--pr", type=int)
    parser.add_argument("--expected-base")
    parser.add_argument("--expected-head")
    args = parser.parse_args()
    try:
        root, lease = root_and_lease()
        if args.mode:
            print(mode(root, lease))
            return
        require(args.pr is not None and args.pr > 0, "a positive --pr is required")
        require(mode(root, lease) == "fleet", "workspace is not on the fleet integration route")
        print(json.dumps(verify(root, lease, args.pr, args.expected_base, args.expected_head), sort_keys=True))
    except (OSError, ValueError, Refusal, subprocess.SubprocessError) as error:
        if isinstance(error, subprocess.CalledProcessError):
            detail = f"{Path(error.cmd[0]).name} command failed (exit {error.returncode})"
        else:
            detail = str(error)
        parser.exit(1, f"Fleet integration refused: {detail}\n")


if __name__ == "__main__":
    main()
