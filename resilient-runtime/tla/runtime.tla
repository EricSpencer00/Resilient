---------------------------- MODULE runtime ----------------------------
(*
RES-3930 Phase B2 — hand-written TLA+ spec of the actor scheduler.

Models the real state machine in `resilient/src/actor_runtime.rs`
(mailbox registry + cooperative Scheduler) and
`resilient/src/supervisor_runtime.rs` (crash/restart policy dispatch),
per the vocabulary mapping in `docs/TLA_ACTOR_MODEL.md`. Per that doc's
Q2 framing, this spec checks the scheduler implementation itself (a
fixed, already-shipped artifact), not arbitrary user programs.

State variable <-> Rust mapping:
  actors        <-> keys of MAILBOX_REGISTRY (registered/live PIDs)
  mailbox       <-> MAILBOX_REGISTRY: HashMap<ActorPid, VecDeque<Value>>
  runnable      <-> Scheduler.runnable: VecDeque<ActorPid> (kept as a
                     Seq here, not a set, so Next can assert FIFO pop
                     order per the `scheduler_pops_in_fifo_order` test)
  blocked       <-> Scheduler.blocked: HashSet<ActorPid>
  restartCount  <-> SupervisedActorState.restart_count

Cooperative, single-threaded execution (real scheduler: one actor runs
between yield points, no OS-thread interleaving) is reflected by Next
being one global, unparameterized-by-thread action — exactly the shape
noted in TLA_ACTOR_MODEL.md's "nondeterminism vs. determinism" section:
the internal FIFO pop order is deterministic like the Rust code, while
*which* action TLC explores next (spawn vs. send vs. receive vs. crash,
and from/to which actor) is the one axis of nondeterminism, matching
"message arrival timing from the external environment".
*)

EXTENDS Naturals, Sequences, FiniteSets, TLC

CONSTANTS
    p1, p2, p3,       \* the (small, per Q5) fixed set of actor identities
    Msgs,             \* finite set of message payloads
    MaxMailboxDepth,  \* mirrors DEFAULT_MAILBOX_CAPACITY, scaled down for TLC
    MaxRestarts,      \* mirrors RestartPolicy::Temporary { max_restarts, .. }
    MaxSends,         \* TLC-only bound on total Send() calls -- keeps the
                       \* sentCount/deliveredCount/droppedCount ghost counters
                       \* (unbounded in the real runtime) finite for exhaustive
                       \* search; not part of the Rust semantics being modeled
    MaxCrashes        \* TLC-only bound on total Crash() events -- a Permanent
                       \* policy actor restarts indefinitely in the real
                       \* runtime (no restart cap), so without this the state
                       \* space is infinite; not part of the Rust semantics

Pids == {p1, p2, p3}

\* Fixed per-actor restart policy for this model instance. One of each
\* policy so all three of supervisor_runtime.rs's RestartPolicy arms
\* (Permanent / Transient / Temporary) get exercised by TLC.
RestartPolicyOf ==
    [q \in Pids |->
        CASE q = p1 -> "Permanent"
          [] q = p2 -> "Temporary"
          [] q = p3 -> "Transient"]

ASSUME Msgs # {}
ASSUME MaxMailboxDepth \in Nat \ {0}
ASSUME MaxRestarts \in Nat

VARIABLES
    actors,         \* SUBSET Pids -- currently-registered actors
    mailbox,        \* [Pids -> Seq(Msgs)] -- per-actor bounded FIFO
    runnable,       \* Seq(Pids) -- FIFO runnable queue
    blocked,        \* SUBSET Pids -- blocked-on-receive set
    restartCount,   \* [Pids -> Nat] -- per-actor restart counter
    sentCount,      \* Nat -- total Send() calls (ghost, for NoLostMessages)
    deliveredCount, \* Nat -- total successful Receive() calls (ghost)
    droppedCount,   \* Nat -- total messages discarded on crash-drop (ghost)
    crashCount      \* Nat -- total Crash() events explored (ghost, TLC bound only)

vars == <<actors, mailbox, runnable, blocked, restartCount,
           sentCount, deliveredCount, droppedCount, crashCount>>

RunnableSet == {runnable[i] : i \in DOMAIN runnable}

RECURSIVE SumSet(_)
SumSet(S) ==
    IF S = {} THEN 0
    ELSE LET p == CHOOSE q \in S : TRUE
         IN Len(mailbox[p]) + SumSet(S \ {p})

TotalQueued == SumSet(Pids)

TypeOK ==
    /\ actors \subseteq Pids
    /\ mailbox \in [Pids -> Seq(Msgs)]
    /\ runnable \in Seq(Pids)
    /\ blocked \subseteq Pids
    /\ restartCount \in [Pids -> Nat]
    /\ sentCount \in Nat
    /\ deliveredCount \in Nat
    /\ droppedCount \in Nat
    /\ crashCount \in Nat

Init ==
    /\ actors = {}
    /\ mailbox = [p \in Pids |-> <<>>]
    /\ runnable = <<>>
    /\ blocked = {}
    /\ restartCount = [p \in Pids |-> 0]
    /\ sentCount = 0
    /\ deliveredCount = 0
    /\ droppedCount = 0
    /\ crashCount = 0

\* actor_runtime::register_actor -- allocate a fresh PID's mailbox and
\* mark it runnable. (fresh_pid's monotonic counter is abstracted away;
\* Pids is the fixed finite identity set TLC explores.)
Spawn(p) ==
    /\ p \notin actors
    /\ actors' = actors \cup {p}
    /\ mailbox' = [mailbox EXCEPT ![p] = <<>>]
    /\ runnable' = Append(runnable, p)
    /\ UNCHANGED <<blocked, restartCount, sentCount, deliveredCount, droppedCount, crashCount>>

\* actor_runtime::enqueue -- bounded FIFO append (WouldBlock when the
\* mailbox is at DEFAULT_MAILBOX_CAPACITY); on success, wakes the
\* target if it was blocked (mark_runnable call after the borrow ends).
Send(p, q, m) ==
    /\ p \in actors
    /\ q \in actors
    /\ sentCount < MaxSends
    /\ Len(mailbox[q]) < MaxMailboxDepth
    /\ mailbox' = [mailbox EXCEPT ![q] = Append(@, m)]
    /\ sentCount' = sentCount + 1
    /\ IF q \in blocked
       THEN /\ blocked' = blocked \ {q}
            /\ runnable' = Append(runnable, q)
       ELSE UNCHANGED <<blocked, runnable>>
    /\ UNCHANGED <<actors, restartCount, deliveredCount, droppedCount, crashCount>>

\* actor_runtime::dequeue driven by the scheduler's dispatch of the
\* FIFO head: pop one message, then re-append the actor to the back of
\* `runnable` (cooperative -- one actor's turn per Next, no interleaved
\* sub-steps within it, per TLA_ACTOR_MODEL.md).
Receive(p) ==
    /\ runnable # <<>>
    /\ Head(runnable) = p
    /\ mailbox[p] # <<>>
    /\ mailbox' = [mailbox EXCEPT ![p] = Tail(@)]
    /\ runnable' = Append(Tail(runnable), p)
    /\ deliveredCount' = deliveredCount + 1
    /\ UNCHANGED <<actors, blocked, restartCount, sentCount, droppedCount, crashCount>>

\* actor_runtime::actor_receive's WouldBlock path + mark_blocked --
\* empty mailbox moves the FIFO head from runnable to blocked.
Block(p) ==
    /\ runnable # <<>>
    /\ Head(runnable) = p
    /\ mailbox[p] = <<>>
    /\ runnable' = Tail(runnable)
    /\ blocked' = blocked \cup {p}
    /\ UNCHANGED <<actors, mailbox, restartCount, sentCount, deliveredCount, droppedCount, crashCount>>

\* supervisor_runtime::handle_crash_event -- policy-dispatched restart.
\* Permanent restarts unconditionally; Temporary restarts while under
\* MaxRestarts and otherwise falls through to the drop branch (mirrors
\* should_restart's Err-on-limit-exceeded -> handle_crash_event's
\* Ok(false)/Err both returning "don't restart"); Transient always
\* drops (should_restart's Ok(false) arm). Dropped/no-supervisor actors
\* are deregistered and their mailbox contents counted as lost-to-crash
\* (droppedCount), matching the documented "drop on crash" semantics.
Crash(p) ==
    /\ p \in actors
    /\ crashCount < MaxCrashes
    /\ crashCount' = crashCount + 1
    /\ CASE RestartPolicyOf[p] = "Permanent" ->
              /\ mailbox' = [mailbox EXCEPT ![p] = <<>>]
              /\ droppedCount' = droppedCount + Len(mailbox[p])
              /\ actors' = actors
              /\ runnable' = (IF p \in RunnableSet THEN runnable ELSE Append(runnable, p))
              /\ blocked' = blocked \ {p}
              /\ restartCount' = [restartCount EXCEPT ![p] = @ + 1]
              /\ UNCHANGED <<sentCount, deliveredCount>>
         [] RestartPolicyOf[p] = "Temporary" /\ restartCount[p] < MaxRestarts ->
              /\ mailbox' = [mailbox EXCEPT ![p] = <<>>]
              /\ droppedCount' = droppedCount + Len(mailbox[p])
              /\ actors' = actors
              /\ runnable' = (IF p \in RunnableSet THEN runnable ELSE Append(runnable, p))
              /\ blocked' = blocked \ {p}
              /\ restartCount' = [restartCount EXCEPT ![p] = @ + 1]
              /\ UNCHANGED <<sentCount, deliveredCount>>
         [] OTHER ->
              /\ actors' = actors \ {p}
              /\ droppedCount' = droppedCount + Len(mailbox[p])
              /\ mailbox' = [mailbox EXCEPT ![p] = <<>>]
              /\ runnable' = SelectSeq(runnable, LAMBDA q : q # p)
              /\ blocked' = blocked \ {p}
              /\ UNCHANGED <<restartCount, sentCount, deliveredCount>>

Next ==
    \/ \E p \in Pids : Spawn(p)
    \/ \E p \in Pids, q \in Pids, m \in Msgs : Send(p, q, m)
    \/ \E p \in Pids : Receive(p)
    \/ \E p \in Pids : Block(p)
    \/ \E p \in Pids : Crash(p)

Spec == Init /\ [][Next]_vars

----------------------------------------------------------------------
\* Properties (docs/TLA_ACTOR_MODEL.md "Invariants and temporal
\* properties to check"), five in number plus the accounting invariant
\* that backs the first one.

\* 1. No lost messages: every Send is eventually accounted for as
\*    delivered, dropped-on-crash, or still sitting in a mailbox. A
\*    scheduler bug that silently drops a message outside those three
\*    paths breaks this equation.
NoLostMessages == sentCount = deliveredCount + droppedCount + TotalQueued

\* 2. Mailbox bound respected -- matches
\*    enqueue_to_full_mailbox_returns_would_block's WouldBlock contract.
MailboxBound == \A p \in actors : Len(mailbox[p]) <= MaxMailboxDepth

\* 3. Deadlock-detector soundness: TLA+'s literal transcription of
\*    Scheduler::is_deadlocked() (runnable.is_empty() && !blocked.is_empty())
\*    must agree, on every reachable state, with the spec-level notion
\*    of "every registered actor is blocked and none runnable". Audited
\*    by hand against actor_runtime.rs's is_deadlocked (RunnableSet and
\*    blocked partition `actors`, so the two sides coincide whenever
\*    that partition invariant holds -- see ActorPartition below).
IsDeadlockedByCode == /\ runnable = <<>>
                      /\ blocked # {}
IsDeadlockedBySpec == /\ actors # {}
                      /\ runnable = <<>>
                      /\ \A p \in actors : p \in blocked
DeadlockDetectorSound == IsDeadlockedByCode <=> IsDeadlockedBySpec

\* Supporting structural invariant: every registered actor is in
\* exactly one of {runnable, blocked} between steps (no third,
\* "currently executing" state persists across Next, per the
\* cooperative single-global-action model).
ActorPartition == /\ actors = (RunnableSet \cup blocked)
                   /\ RunnableSet \cap blocked = {}

\* 4. Deadlock-freedom of the scheduler itself (not of user programs):
\*    every non-runnable state is accounted for by the deadlock
\*    predicate -- there's no silent hang where actors exist, nothing
\*    is runnable, yet not everyone is classified blocked.
DeadlockFreedom == (runnable = <<>> /\ actors # {}) => (blocked = actors)

\* 5. Restart convergence: a Temporary-policy actor's restart count
\*    never exceeds MaxRestarts (the Crash action structurally caps it,
\*    falling through to the drop branch once the limit is hit, mirroring
\*    should_restart's Err-on-exceeded), so it stabilizes rather than
\*    growing forever.
RestartConverges ==
    \A p \in Pids : (RestartPolicyOf[p] = "Temporary") => <>[](restartCount[p] <= MaxRestarts)

\* 6. Crash isolation: crashing one actor never mutates another actor's
\*    mailbox. Checked as an action-level property (not just an audit)
\*    over every step that is a Crash(p) transition.
CrashIsolation ==
    [][ \A p \in Pids : Crash(p) => (\A q \in Pids \ {p} : mailbox'[q] = mailbox[q]) ]_vars

=============================================================================
