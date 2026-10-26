------------------------- MODULE timer_identity -------------------------
EXTENDS Naturals, FiniteSets

CONSTANTS Slots, MaxGeneration, CheckGeneration, PreserveGeneration,
          RetireOnExhaustion
ASSUME /\ Slots # {}
       /\ MaxGeneration \in Nat \ {0}
       /\ CheckGeneration \in BOOLEAN
       /\ PreserveGeneration \in BOOLEAN
       /\ RetireOnExhaustion \in BOOLEAN

VARIABLES generation, allocated, issued, revoked, lastAccepted, lastRevoked
vars == <<generation, allocated, issued, revoked, lastAccepted, lastRevoked>>
Handles == Slots \X (1..MaxGeneration)
Valid(handle) == /\ handle[1] \in allocated
                 /\ (~CheckGeneration \/ generation[handle[1]] = handle[2])

Init == /\ generation = [slot \in Slots |-> 0]
        /\ allocated = {}
        /\ issued = {}
        /\ revoked = {}
        /\ lastAccepted = FALSE
        /\ lastRevoked = FALSE

Allocate(slot) ==
    /\ slot \notin allocated
    /\ generation[slot] < MaxGeneration \/ ~RetireOnExhaustion
    /\ LET next == IF generation[slot] < MaxGeneration
                   THEN generation[slot] + 1 ELSE 1
       IN /\ generation' = [generation EXCEPT ![slot] = next]
          /\ issued' = issued \cup {<<slot, next>>}
    /\ allocated' = allocated \cup {slot}
    /\ UNCHANGED revoked
    /\ lastAccepted' = FALSE
    /\ lastRevoked' = FALSE

Release(handle) ==
    /\ handle \in issued
    /\ Valid(handle)
    /\ allocated' = allocated \ {handle[1]}
    /\ revoked' = revoked \cup {<<handle[1], generation[handle[1]]>>}
    /\ lastAccepted' = TRUE
    /\ lastRevoked' = (handle \in revoked)
    /\ UNCHANGED <<generation, issued>>

Request(handle) ==
    /\ handle \in issued
    /\ lastAccepted' = Valid(handle)
    /\ lastRevoked' = (handle \in revoked)
    /\ UNCHANGED <<generation, allocated, issued, revoked>>

ResetAll ==
    /\ allocated' = {}
    /\ revoked' = revoked \cup {<<slot, generation[slot]>> : slot \in allocated}
    /\ generation' = IF PreserveGeneration THEN generation
                      ELSE [slot \in Slots |-> 0]
    /\ UNCHANGED issued
    /\ lastAccepted' = FALSE
    /\ lastRevoked' = FALSE

Next == (\E slot \in Slots : Allocate(slot))
        \/ (\E handle \in Handles : Release(handle) \/ Request(handle))
        \/ ResetAll

TypeOK == /\ generation \in [Slots -> 0..MaxGeneration]
          /\ allocated \subseteq Slots
          /\ issued \subseteq Handles
          /\ revoked \subseteq issued
          /\ lastAccepted \in BOOLEAN
          /\ lastRevoked \in BOOLEAN
NoResurrection == {handle \in issued : Valid(handle)} \cap revoked = {}
RejectRevoked == ~(lastAccepted /\ lastRevoked)
Spec == Init /\ [][Next]_vars
=============================================================================
