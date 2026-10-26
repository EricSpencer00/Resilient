----------------------- MODULE supervisor_restart -----------------------
(* Temporary restart authorization and recording, including idle-window
   reset and saturating subtraction when the supplied clock rolls back.
   This models SupervisorState's public API, not actor execution. *)
EXTENDS Naturals

CONSTANTS MaxBudget, MaxWindow, MaxTime
ASSUME /\ MaxBudget \in Nat /\ MaxWindow \in Nat /\ MaxTime \in Nat

VARIABLES budget, window, count, last, now
vars == <<budget, window, count, last, now>>

Elapsed == IF now >= last THEN now - last ELSE 0
CanRestart == budget > 0 /\ (Elapsed >= window \/ count < budget)
AfterCount == IF Elapsed >= window THEN 1 ELSE count + 1
RecordedCount == IF CanRestart THEN AfterCount ELSE count
RecordedLast == IF CanRestart THEN now ELSE last

Init ==
    /\ budget \in 0..MaxBudget
    /\ window \in 0..MaxWindow
    /\ count \in 0..budget
    /\ last \in 0..MaxTime
    /\ now \in 0..MaxTime

ClockJump ==
    /\ now' \in 0..MaxTime
    /\ UNCHANGED <<budget, window, count, last>>

RecordRestart ==
    /\ CanRestart
    /\ count' = AfterCount
    /\ last' = now
    /\ UNCHANGED <<budget, window, now>>

Next == ClockJump \/ RecordRestart
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ budget \in 0..MaxBudget
    /\ window \in 0..MaxWindow
    /\ count \in 0..MaxBudget
    /\ last \in 0..MaxTime
    /\ now \in 0..MaxTime
BudgetBound == count <= budget
ZeroBudgetDisabled == budget = 0 => ~CanRestart

=============================================================================
