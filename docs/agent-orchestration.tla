-------------------------- MODULE AgentOrchestration --------------------------
EXTENDS Naturals, FiniteSets, Sequences

(*
This model specifies the safety contract for Resilient's agent
orchestration. It abstracts away GitHub and shell details and keeps only
the state transitions that matter for correctness.

Model constants:
  Agents  - set of agent identities
  Issues  - set of issue identities
  Files   - set of repository file identities

Suggested TLC constraints:
  Agents = {a1, a2}
  Issues = {i1, i2, i3}
  Files  = {f1, f2, f3}
*)

CONSTANTS Agents, Issues, Files

ASSUME Agents # {} /\ Issues # {} /\ Files # {}

States == {
    "open",
    "dispatched",
    "running",
    "guardrail_failed",
    "guardrail_passed",
    "synced",
    "ready",
    "merged",
    "abandoned"
}

VARIABLES
    state,          \* issue -> lifecycle state
    owner,          \* issue -> agent or None
    prOpen,         \* issue -> whether a PR exists and is still live
    claims,         \* issue -> claimed file set
    guardrailOk,    \* issue -> local guardrail passed
    syncedOk,       \* issue -> branch synced through agents/integration
    ciOk,           \* issue -> required CI checks passed
    handoff         \* issue -> durable handoff event count

None == "none"

TypeOK ==
    /\ state \in [Issues -> States]
    /\ owner \in [Issues -> Agents \cup {None}]
    /\ prOpen \in [Issues -> BOOLEAN]
    /\ claims \in [Issues -> SUBSET Files]
    /\ guardrailOk \in [Issues -> BOOLEAN]
    /\ syncedOk \in [Issues -> BOOLEAN]
    /\ ciOk \in [Issues -> BOOLEAN]
    /\ handoff \in [Issues -> Nat]

Init ==
    /\ state = [i \in Issues |-> "open"]
    /\ owner = [i \in Issues |-> None]
    /\ prOpen = [i \in Issues |-> FALSE]
    /\ claims = [i \in Issues |-> {}]
    /\ guardrailOk = [i \in Issues |-> FALSE]
    /\ syncedOk = [i \in Issues |-> FALSE]
    /\ ciOk = [i \in Issues |-> FALSE]
    /\ handoff = [i \in Issues |-> 0]

LiveIssues == {i \in Issues : prOpen[i] /\ state[i] \notin {"merged", "abandoned"}}

NoClaimOverlap ==
    \A i, j \in LiveIssues :
        i # j => claims[i] \cap claims[j] = {}

ReadyRequiresGuardrail ==
    \A i \in Issues :
        state[i] = "ready" => guardrailOk[i]

MergeRequiresFullGate ==
    \A i \in Issues :
        state[i] = "merged" => guardrailOk[i] /\ syncedOk[i] /\ ciOk[i]

MergedReleasesClaims ==
    \A i \in Issues :
        state[i] = "merged" => claims[i] = {}

Dispatch(i, a, fs) ==
    /\ state[i] = "open"
    /\ a \in Agents
    /\ fs \subseteq Files
    /\ \A j \in LiveIssues : fs \cap claims[j] = {}
    /\ state' = [state EXCEPT ![i] = "dispatched"]
    /\ owner' = [owner EXCEPT ![i] = a]
    /\ prOpen' = [prOpen EXCEPT ![i] = TRUE]
    /\ claims' = [claims EXCEPT ![i] = fs]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<guardrailOk, syncedOk, ciOk>>

Start(i) ==
    /\ state[i] = "dispatched"
    /\ state' = [state EXCEPT ![i] = "running"]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, prOpen, claims, guardrailOk, syncedOk, ciOk>>

GuardrailFail(i) ==
    /\ state[i] = "running"
    /\ state' = [state EXCEPT ![i] = "guardrail_failed"]
    /\ guardrailOk' = [guardrailOk EXCEPT ![i] = FALSE]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, prOpen, claims, syncedOk, ciOk>>

GuardrailPass(i) ==
    /\ state[i] \in {"running", "guardrail_failed"}
    /\ state' = [state EXCEPT ![i] = "guardrail_passed"]
    /\ guardrailOk' = [guardrailOk EXCEPT ![i] = TRUE]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, prOpen, claims, syncedOk, ciOk>>

Sync(i) ==
    /\ state[i] = "guardrail_passed"
    /\ guardrailOk[i]
    /\ state' = [state EXCEPT ![i] = "synced"]
    /\ syncedOk' = [syncedOk EXCEPT ![i] = TRUE]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, prOpen, claims, guardrailOk, ciOk>>

MarkReady(i) ==
    /\ state[i] = "synced"
    /\ guardrailOk[i]
    /\ syncedOk[i]
    /\ state' = [state EXCEPT ![i] = "ready"]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, prOpen, claims, guardrailOk, syncedOk, ciOk>>

CiPass(i) ==
    /\ state[i] \in {"synced", "ready"}
    /\ ciOk' = [ciOk EXCEPT ![i] = TRUE]
    /\ UNCHANGED <<state, owner, prOpen, claims, guardrailOk, syncedOk, handoff>>

Merge(i) ==
    /\ state[i] = "ready"
    /\ guardrailOk[i] /\ syncedOk[i] /\ ciOk[i]
    /\ state' = [state EXCEPT ![i] = "merged"]
    /\ prOpen' = [prOpen EXCEPT ![i] = FALSE]
    /\ claims' = [claims EXCEPT ![i] = {}]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, guardrailOk, syncedOk, ciOk>>

Abandon(i) ==
    /\ state[i] \notin {"merged", "abandoned"}
    /\ state' = [state EXCEPT ![i] = "abandoned"]
    /\ prOpen' = [prOpen EXCEPT ![i] = FALSE]
    /\ claims' = [claims EXCEPT ![i] = {}]
    /\ handoff' = [handoff EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owner, guardrailOk, syncedOk, ciOk>>

Next ==
    \/ \E i \in Issues, a \in Agents, fs \in SUBSET Files : Dispatch(i, a, fs)
    \/ \E i \in Issues : Start(i)
    \/ \E i \in Issues : GuardrailFail(i)
    \/ \E i \in Issues : GuardrailPass(i)
    \/ \E i \in Issues : Sync(i)
    \/ \E i \in Issues : MarkReady(i)
    \/ \E i \in Issues : CiPass(i)
    \/ \E i \in Issues : Merge(i)
    \/ \E i \in Issues : Abandon(i)

Spec == Init /\ [][Next]_<<state, owner, prOpen, claims, guardrailOk, syncedOk, ciOk, handoff>>

Safety ==
    TypeOK
    /\ NoClaimOverlap
    /\ ReadyRequiresGuardrail
    /\ MergeRequiresFullGate
    /\ MergedReleasesClaims

=============================================================================
