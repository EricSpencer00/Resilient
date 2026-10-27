--------------------------- MODULE timer_delay ---------------------------
EXTENDS Naturals
CONSTANTS Period, Duration, AdvanceAfterExpiry, AcknowledgeCancellation,
          PreserveReservation
ASSUME /\ Period \in Nat \ {0}
       /\ Duration \in Nat \ {0}
       /\ Period < Duration
       /\ AdvanceAfterExpiry \in BOOLEAN
       /\ AcknowledgeCancellation \in BOOLEAN
       /\ PreserveReservation \in BOOLEAN

VARIABLES status, phase, remaining, elapsed, peripheralElapsed, fires, result
vars == <<status, phase, remaining, elapsed, peripheralElapsed, fires, result>>

Init == /\ status = "running"
        /\ phase = "waiting"
        /\ remaining = Duration
        /\ elapsed = 0
        /\ peripheralElapsed = 0
        /\ fires = 0
        /\ result = "pending"

Tick ==
    /\ phase = "waiting"
    /\ elapsed < Duration
    /\ elapsed' = elapsed + 1
    /\ remaining' = IF (AdvanceAfterExpiry \/ status = "running") /\ remaining > 0
                     THEN remaining - 1 ELSE remaining
    /\ peripheralElapsed' = IF status = "running"
                            THEN peripheralElapsed + 1 ELSE peripheralElapsed
    /\ status' = IF status = "running" /\ peripheralElapsed + 1 >= Period
                 THEN "expired" ELSE status
    /\ fires' = IF status = "running" /\ peripheralElapsed + 1 >= Period
                THEN fires + 1 ELSE fires
    /\ UNCHANGED <<phase, result>>

Complete ==
    /\ phase = "waiting"
    /\ remaining = 0
    /\ phase' = "idle"
    /\ status' = "stopped"
    /\ result' = "ok"
    /\ UNCHANGED <<remaining, elapsed, peripheralElapsed, fires>>

Cancel ==
    /\ phase = "waiting"
    /\ phase' = IF PreserveReservation THEN "cancelled" ELSE "idle"
    /\ status' = "stopped"
    /\ UNCHANGED <<remaining, elapsed, peripheralElapsed, fires, result>>

Acknowledge ==
    /\ AcknowledgeCancellation
    /\ phase = "cancelled"
    /\ phase' = "idle"
    /\ result' = "cancelled"
    /\ remaining' = 0
    /\ UNCHANGED <<status, elapsed, peripheralElapsed, fires>>

Next == Tick \/ Complete \/ Cancel \/ Acknowledge
Spec == Init /\ [][Next]_vars /\ WF_vars(Tick) /\ WF_vars(Complete) /\ WF_vars(Acknowledge)

TypeOK == /\ status \in {"running", "expired", "stopped"}
          /\ phase \in {"waiting", "cancelled", "idle"}
          /\ remaining \in 0..Duration
          /\ elapsed \in 0..Duration
          /\ peripheralElapsed \in 0..Period
          /\ fires \in 0..1
          /\ result \in {"pending", "ok", "cancelled"}
NoLostReservation == (result = "pending") => (phase # "idle")
NoPrematureSuccess == (result = "ok") => (remaining = 0 /\ elapsed >= Duration)
EventuallyReturned == <> (result # "pending")
=============================================================================
