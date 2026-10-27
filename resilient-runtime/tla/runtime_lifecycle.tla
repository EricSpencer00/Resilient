---------------------- MODULE runtime_lifecycle ----------------------
(* Explicit mailbox/scheduler deregistration, extending the bounded
   cooperative runtime model without changing its existing transition set.
   Rust mapping: actor_runtime::deregister_actor removes the mailbox and
   removes the PID from both runnable and blocked scheduler collections.

   droppedCount is ghost accounting: it also counts messages discarded by
   explicit termination here, not only messages dropped during Crash.
   Restart counters retain the original model's historical interpretation.
*)
EXTENDS runtime

Deregister(p) ==
    /\ p \in actors
    /\ actors' = actors \ {p}
    /\ mailbox' = [mailbox EXCEPT ![p] = <<>>]
    /\ runnable' = SelectSeq(runnable, LAMBDA q : q # p)
    /\ blocked' = blocked \ {p}
    /\ droppedCount' = droppedCount + Len(mailbox[p])
    /\ UNCHANGED <<restartCount, sentCount, deliveredCount, crashCount>>

LifecycleNext == Next \/ (\E p \in Pids : Deregister(p))
LifecycleSpec == Init /\ [][LifecycleNext]_vars

InactiveActorsClean ==
    \A p \in Pids \ actors :
        /\ mailbox[p] = <<>>
        /\ p \notin RunnableSet
        /\ p \notin blocked

RunnableUnique == Len(runnable) = Cardinality(RunnableSet)

=============================================================================
