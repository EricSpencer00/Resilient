#!/usr/bin/env bash
# sweep-drafts.sh — drive every abandoned draft PR to a terminal state
# (merged or closed) autonomously, with no human in the loop.
#
# Why this exists
# ---------------
# Heavy CI is deferred on draft PRs (RES-3862) to save Actions minutes,
# so a draft's required checks are cheap "skipped-green" placeholders,
# NOT a real build. That is a big cost win, but it means a draft nobody
# marks ready would sit forever with meaningless green checks. This
# sweeper is the other half of that trade: it promotes idle drafts so a
# REAL CI run happens exactly once, lands the ones that still pass, and
# closes the ones that cannot — so no draft is ever "left dead".
#
# Policy (every threshold is env-overridable)
# -------------------------------------------
#   `no-sweep` label            -> skip (explicit opt-out for WIP).
#   idle < IDLE_HOURS (24)      -> skip (an agent may still be working).
#   empty diff (0 +/- lines)    -> close (nothing to land).
#   idle > STALE_DAYS (14)      -> close (presumed abandoned; a real CI
#                                  run on multi-week-stale work almost
#                                  always fails, and we will not spend
#                                  ~40 CI-min per doomed draft).
#   otherwise (idle in window)  -> PROMOTE, capped at MAX_PROMOTE (3) per
#                                  run so a backlog cannot spike CI spend:
#         update-branch (a fresh SHA has no checks, so the skipped-green
#                        placeholder can never fool auto-merge)
#         -> mark ready
#         -> rerun the required workflows so a REAL (non-draft) run
#            executes even though a bot-driven ready transition is
#            otherwise held by GitHub's GITHUB_TOKEN recursion guard
#         -> arm native auto-merge (gates on the REAL required checks)
#         -> label `sweep-promoted`.
#
# Reap (every run): for each open non-draft PR labeled `sweep-promoted`:
#         mergeable == CONFLICTING, or a REQUIRED check == FAILURE  -> close.
#         (A skipped-green check reports SUCCESS, never FAILURE, so it can
#          never trigger a false close — only a real failing run closes.)
#         all green / still pending -> leave (auto-merge lands it / CI runs).
#
# Every close keeps the branch and leaves the linked issue open, so the
# work can be re-picked-up. Nothing is force-pushed or admin-merged; the
# real required checks are always the merge gate.
#
# Usage:
#   agent-scripts/sweep-drafts.sh            # dry-run (report only)
#   agent-scripts/sweep-drafts.sh --go       # apply
#   agent-scripts/sweep-drafts.sh --only 248 # restrict to a single PR

set -euo pipefail

GO=0
ONLY=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --go) GO=1; shift ;;
        --only) ONLY="${2:-}"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

IDLE_HOURS="${IDLE_HOURS:-24}"
STALE_DAYS="${STALE_DAYS:-14}"
MAX_PROMOTE="${MAX_PROMOTE:-3}"
STALE_HOURS=$(( STALE_DAYS * 24 ))

# Workflows that own branch-protection required checks. Rerunning these
# forces a real (non-draft) run so auto-merge gates on genuine results.
REQUIRED_WORKFLOWS=("CI" "embedded" "size-gate")

# Branch-protection required check names, per
# `gh api repos/.../branches/main/protection`. Only a FAILURE on one of
# these closes a PR — non-required checks (CodeQL, ai-threats, perf) are
# advisory and never trigger a close.
REQUIRED_CHECKS_JSON='["build / test / clippy","build / test with --features z3","board hygiene","resilient-runtime-cortex-m-demo (thumbv7em-none-eabihf)","resilient-runtime (riscv32imac-unknown-none-elf)","resilient-runtime (thumbv6m-none-eabi)","cortex-m demo .text budget check"]'

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

say() { echo "$@"; }

hours_idle() {
    python3 -c "import sys,datetime as d; t=d.datetime.fromisoformat(sys.argv[1].replace('Z','+00:00')); print(int((d.datetime.now(d.timezone.utc)-t).total_seconds()//3600))" "$1"
}

has_label() {
    # has_label <labels-json> <name>  -> prints True/False
    echo "$1" | python3 -c "import sys,json; print(any(l['name']==sys.argv[1] for l in json.load(sys.stdin)))" "$2"
}

ensure_labels() {
    gh label create "sweep-promoted" --color "1D76DB" \
        --description "Promoted from draft by sweep-drafts.sh; awaiting real CI" \
        >/dev/null 2>&1 || true
}

close_pr() {
    local pr="$1" reason="$2"
    say "  CLOSE #$pr — $reason"
    if (( GO )); then
        gh pr comment "$pr" --body "🧹 Auto-closed by the draft sweeper: ${reason}. The branch is preserved and the linked issue stays open — reopen this PR or re-pick the ticket to retry." >/dev/null 2>&1 || true
        gh pr close "$pr" >/dev/null 2>&1 || say "    (close failed for #$pr)"
    fi
}

force_real_ci() {
    # Rerun the latest run of each required workflow on this SHA so a real
    # (non-draft) build executes. GITHUB_TOKEN reruns clear the
    # action_required hold — verified by auto-rerun-held-ci.yml.
    local branch="$1" sha="$2" wf rid
    for wf in "${REQUIRED_WORKFLOWS[@]}"; do
        rid=$(gh run list --branch "$branch" --workflow "$wf" --limit 15 \
                --json databaseId,headSha \
                --jq "[.[] | select(.headSha==\"$sha\")][0].databaseId" 2>/dev/null || true)
        if [[ -n "$rid" && "$rid" != "null" ]]; then
            if gh run rerun "$rid" >/dev/null 2>&1; then
                say "    reran '$wf' (run $rid)"
            else
                say "    could not rerun '$wf' (run $rid)"
            fi
        else
            say "    no '$wf' run yet for ${sha:0:8} — hourly auto-rerun will start it"
        fi
    done
}

promote_pr() {
    local pr="$1" branch="$2"
    say "  PROMOTE #$pr ($branch)"
    if (( GO == 0 )); then return; fi
    gh pr update-branch "$pr" >/dev/null 2>&1 || true

    # update-branch can reveal that the branch conflicts with current main.
    # A conflicting draft is a loser — close it while it is still a draft so
    # we neither spend a real CI run that can never merge nor leave it parked
    # ready+armed. Give GitHub a moment to recompute mergeability first.
    local m
    for _ in 1 2 3 4 5; do
        m=$(gh pr view "$pr" --json mergeable --jq .mergeable 2>/dev/null || echo UNKNOWN)
        [[ "$m" != "UNKNOWN" ]] && break
        sleep 3
    done
    if [[ "$m" == "CONFLICTING" ]]; then
        say "    conflicts with main after update-branch — closing instead of promoting"
        close_pr "$pr" "conflicts with main that could not be auto-resolved"
        return
    fi

    local sha
    sha=$(gh pr view "$pr" --json headRefOid --jq .headRefOid 2>/dev/null || true)
    gh pr ready "$pr" >/dev/null 2>&1 || true
    [[ -n "$sha" ]] && force_real_ci "$branch" "$sha"
    if gh pr merge "$pr" --squash --auto --delete-branch >/dev/null 2>&1; then
        say "    armed native auto-merge (gates on the real required checks)"
    else
        say "    (could not arm auto-merge for #$pr)"
    fi
    gh pr edit "$pr" --add-label "sweep-promoted" >/dev/null 2>&1 || true
    gh pr comment "$pr" --body "🚀 Promoted from draft by the sweeper: a real CI run is starting now. This PR auto-merges if the required checks pass, and is auto-closed if it cannot land." >/dev/null 2>&1 || true
}

required_failed_count() {
    local pr="$1"
    gh pr view "$pr" --json statusCheckRollup --jq "
        ${REQUIRED_CHECKS_JSON} as \$req
        | [.statusCheckRollup[]
            | select(.name as \$n | \$req | index(\$n))
            | select(.conclusion==\"FAILURE\" or .conclusion==\"CANCELLED\" or .conclusion==\"TIMED_OUT\")
          ] | length" 2>/dev/null || echo 0
}

reap_promoted() {
    say "== reap: promoted PRs that turned out to be losers =="
    local rows pr isdraft merge
    rows=$(gh pr list --state open --label "sweep-promoted" --base main --limit 100 \
            --json number,isDraft --jq '.[] | "\(.number)\t\(.isDraft)"' 2>/dev/null || true)
    if [[ -z "$rows" ]]; then say "  (none)"; return; fi
    while IFS=$'\t' read -r pr isdraft; do
        [[ -z "$pr" ]] && continue
        [[ "$isdraft" == "true" ]] && continue
        merge=$(gh pr view "$pr" --json mergeable --jq .mergeable 2>/dev/null || echo UNKNOWN)
        if [[ "$merge" == "CONFLICTING" ]]; then
            close_pr "$pr" "conflicts with main that could not be auto-resolved"
            continue
        fi
        if [[ "$(required_failed_count "$pr")" != "0" ]]; then
            close_pr "$pr" "a required CI check failed on the real (post-promotion) run"
            continue
        fi
        say "  #$pr — green or pending, leaving for auto-merge"
    done <<< "$rows"
}

main() {
    (( GO )) && ensure_labels
    say "sweep-drafts: GO=$GO IDLE_HOURS=$IDLE_HOURS STALE_DAYS=$STALE_DAYS MAX_PROMOTE=$MAX_PROMOTE"
    reap_promoted

    say "== scan open drafts =="
    local nums
    if [[ -n "$ONLY" ]]; then
        nums="$ONLY"
    else
        nums=$(gh pr list --state open --draft --base main --limit 100 \
                --json number --jq '.[].number' 2>/dev/null || true)
    fi
    if [[ -z "$nums" ]]; then say "  (no open drafts)"; return; fi

    # Collect promote candidates as "idle<TAB>pr<TAB>branch" so we can
    # promote the freshest (most likely to still land) first, under the cap.
    local candidates="" pr info idle add del labels branch
    for pr in $nums; do
        info=$(gh pr view "$pr" --json number,updatedAt,additions,deletions,labels,headRefName 2>/dev/null || true)
        [[ -z "$info" ]] && continue
        labels=$(echo "$info" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["labels"]))')
        if [[ "$(has_label "$labels" "no-sweep")" == "True" ]]; then
            say "  #$pr — has no-sweep label, skipping"; continue
        fi
        idle=$(hours_idle "$(echo "$info" | python3 -c 'import sys,json;print(json.load(sys.stdin)["updatedAt"])')")
        add=$(echo "$info" | python3 -c 'import sys,json;print(json.load(sys.stdin)["additions"])')
        del=$(echo "$info" | python3 -c 'import sys,json;print(json.load(sys.stdin)["deletions"])')
        branch=$(echo "$info" | python3 -c 'import sys,json;print(json.load(sys.stdin)["headRefName"])')

        if (( idle < IDLE_HOURS )); then
            say "  #$pr — idle ${idle}h < ${IDLE_HOURS}h, likely active, skipping"; continue
        fi
        if (( add == 0 && del == 0 )); then
            close_pr "$pr" "empty draft (no changes)"; continue
        fi
        if (( idle > STALE_HOURS )); then
            close_pr "$pr" "stale draft (idle ${idle}h > ${STALE_DAYS}d), presumed abandoned"; continue
        fi
        candidates+="${idle}	${pr}	${branch}"$'\n'
    done

    if [[ -n "$candidates" ]]; then
        say "== promote (freshest first, cap ${MAX_PROMOTE}) =="
        local n=0
        while IFS=$'\t' read -r idle pr branch; do
            [[ -z "$pr" ]] && continue
            if (( n >= MAX_PROMOTE )); then
                say "  #$pr — deferred (hit per-run cap of ${MAX_PROMOTE}); next run picks it up"
                continue
            fi
            promote_pr "$pr" "$branch"
            n=$(( n + 1 ))
        done < <(echo "$candidates" | sort -n)
    fi
    say "sweep complete."
}

main
