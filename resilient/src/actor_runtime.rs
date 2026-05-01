//! RES-332: actor runtime — mailbox + PID infrastructure (PR 1/5).
//!
//! Resilient already has a host-side actor verifier (`crate::verifier_actors`)
//! and a parsed `actor Counter { … }` declaration, but the runtime
//! plumbing that an interpreter would need to actually `spawn` /
//! `send` / `receive` doesn't exist yet. This module ships the
//! data-model layer that PRs 2-5 build on:
//!
//! * `ActorPid` — opaque, 1-indexed handle. Stable for the lifetime
//!   of the process. The `Value::ActorPid(u64)` enum variant (added
//!   in `lib.rs`) wraps this so user code can pass PIDs around.
//!
//! * `MAILBOX_REGISTRY` — thread-local `HashMap<u64, VecDeque<Value>>`
//!   mapping each live PID to its bounded mailbox. Default capacity
//!   matches the spec's `BOUNDED_MAILBOX_CAPACITY` (8). `enqueue`
//!   returns `MailboxError::WouldBlock` when full so PR 2's `send`
//!   builtin can decide between yielding the sender vs an immediate
//!   error.
//!
//! * `Scheduler` — runnable / blocked sets + the next-PID counter.
//!   This PR exposes the data model only; PR 3 adds the `step()`
//!   loop, PR 4 adds deadlock detection.
//!
//! ## Spec authority
//!
//! `docs/superpowers/specs/2026-04-30-actor-scaffolding-design.md`
//! describes the 5-PR sequence. The semantic contract (FIFO per pair,
//! atomic receive, bounded mailbox, drop-on-crash, legal self-send)
//! is locked in by `2026-04-30-actor-message-semantics.md`.
//!
//! ## Feature isolation
//!
//! All actor-runtime logic lives in this module. PR 2 will add a
//! single `Value::ActorPid(u64)` arm to `Value` and three calls to
//! `register_builtins` in `lib.rs`. PRs 3-4 will modify the
//! interpreter's outer loop. PR 5 lands the ping-pong example.

// PR 2-5 of RES-332 consume the surface laid down here; PR 1 sets it up
// without yet calling into it from non-test sites, so dead-code lint
// would otherwise fire on every helper.
#![allow(dead_code)]

use crate::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

// ---------------------------------------------------------------------------
// PID
// ---------------------------------------------------------------------------

/// Opaque, process-stable actor handle.
///
/// `0` is reserved as the sentinel "not-yet-assigned" / "no actor"
/// value; legal PIDs start at `1`. The `u64` width gives us room for
/// every actor a long-running embedded program could plausibly
/// spawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorPid(pub u64);

impl ActorPid {
    /// The reserved no-actor sentinel. Useful as a default when an
    /// API has to construct a value before the real PID is known.
    pub const NONE: ActorPid = ActorPid(0);

    /// True for the reserved zero handle.
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for ActorPid {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "ActorPid({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// Mailbox
// ---------------------------------------------------------------------------

/// Default mailbox capacity per the actor-semantics spec
/// (`BOUNDED_MAILBOX_CAPACITY = 8`). PR 2 will expose
/// `spawn_bounded(fn, n)` for callers that need a different bound.
pub const DEFAULT_MAILBOX_CAPACITY: usize = 8;

/// Outcome of a mailbox enqueue / dequeue.
#[derive(Debug, Clone, PartialEq)]
pub enum MailboxError {
    /// PID does not (or no longer) exists in the registry.
    NotLive(ActorPid),
    /// `enqueue` saw a mailbox at capacity. PR 2's `send` decides
    /// whether the sender yields and retries or fails immediately.
    WouldBlock(ActorPid),
}

impl std::fmt::Display for MailboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            MailboxError::NotLive(pid) => write!(f, "actor PID {} is not live", pid.0),
            MailboxError::WouldBlock(pid) => {
                write!(f, "mailbox for PID {} is at capacity", pid.0)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Per-thread scheduler state. PR 3 will add the `step()` /
/// `next_runnable()` driver; this PR exposes the data model so PR 2's
/// builtins can register actors and inspect their state.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// FIFO list of runnable PIDs. The scheduler pops from the front
    /// and pushes to the back when an actor yields back voluntarily
    /// or after a `receive` re-readies it.
    runnable: VecDeque<ActorPid>,
    /// Set of PIDs that are currently blocked on `receive` with an
    /// empty mailbox. PR 4's deadlock check fires when this set is
    /// non-empty AND `runnable` is empty.
    blocked: HashSet<ActorPid>,
    /// Monotonic next-PID counter. Always >= 1 (zero is reserved).
    next_pid: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh PID. Each call increments the internal
    /// counter; PIDs are never reused within a process run.
    pub fn fresh_pid(&mut self) -> ActorPid {
        self.next_pid += 1;
        ActorPid(self.next_pid)
    }

    /// Mark `pid` as runnable. Idempotent — calling on an
    /// already-runnable PID is a no-op (the PID stays in its existing
    /// queue position rather than getting bumped to the back).
    pub fn mark_runnable(&mut self, pid: ActorPid) {
        if !self.runnable.contains(&pid) {
            self.blocked.remove(&pid);
            self.runnable.push_back(pid);
        }
    }

    /// Mark `pid` as blocked on `receive`. The next `send` to this
    /// PID will pull it back into `runnable` (PR 2's responsibility).
    pub fn mark_blocked(&mut self, pid: ActorPid) {
        self.runnable.retain(|p| *p != pid);
        self.blocked.insert(pid);
    }

    /// Pop the next runnable PID for the scheduler to dispatch.
    /// Returns `None` when the runnable queue is empty.
    pub fn pop_runnable(&mut self) -> Option<ActorPid> {
        self.runnable.pop_front()
    }

    /// True when there is no runnable actor but at least one blocked
    /// actor. PR 4's deadlock detector fires on this condition.
    pub fn is_deadlocked(&self) -> bool {
        self.runnable.is_empty() && !self.blocked.is_empty()
    }

    /// Snapshot of currently-blocked PIDs, sorted ascending. Used by
    /// PR 4's deadlock diagnostic.
    pub fn blocked_pids(&self) -> Vec<ActorPid> {
        let mut out: Vec<ActorPid> = self.blocked.iter().copied().collect();
        out.sort();
        out
    }
}

// ---------------------------------------------------------------------------
// Thread-local registry + scheduler
// ---------------------------------------------------------------------------

thread_local! {
    /// Mailbox registry — keyed by PID. Each entry is a bounded
    /// FIFO of unread `Value`s. PR 1 exposes `register_actor`,
    /// `enqueue`, `dequeue` against this thread-local; PR 3's
    /// scheduler loop drives the lifecycle.
    static MAILBOX_REGISTRY: RefCell<HashMap<ActorPid, VecDeque<Value>>> =
        RefCell::new(HashMap::new());
    /// Per-thread scheduler. PRs 3-4 add the `step()` loop and
    /// deadlock check.
    static SCHEDULER: RefCell<Scheduler> = RefCell::new(Scheduler::new());
    /// Maps each live PID to its body `Value::Function`. PR 3's
    /// `Scheduler::step` reads this to dispatch the actor's frame;
    /// PR 2's `actor_spawn` writes it.
    static ACTOR_FN_REGISTRY: RefCell<HashMap<ActorPid, Value>> =
        RefCell::new(HashMap::new());
    /// The PID of the actor whose frame is currently executing on this
    /// thread. PR 3's scheduler sets this before calling the actor fn;
    /// `actor_receive` reads it so user code can write `receive()` with
    /// no arguments.
    static CURRENT_ACTOR_PID: RefCell<Option<ActorPid>> = const { RefCell::new(None) };
}

/// Allocate a fresh PID, register an empty mailbox for it, and mark
/// it as runnable. PR 2's `spawn` builtin calls this immediately
/// before enqueueing the actor's initial frame.
pub fn register_actor() -> ActorPid {
    let pid = SCHEDULER.with(|s| s.borrow_mut().fresh_pid());
    MAILBOX_REGISTRY.with(|m| {
        m.borrow_mut()
            .insert(pid, VecDeque::with_capacity(DEFAULT_MAILBOX_CAPACITY));
    });
    SCHEDULER.with(|s| s.borrow_mut().mark_runnable(pid));
    pid
}

/// Append `msg` to `pid`'s mailbox. Returns `WouldBlock` when the
/// mailbox is at capacity, `NotLive` when the PID is unknown.
/// On success, if the target actor is currently `blocked` on
/// `receive`, transitions it back to `runnable` so the scheduler
/// will pick it up.
pub fn enqueue(pid: ActorPid, msg: Value) -> Result<(), MailboxError> {
    MAILBOX_REGISTRY.with(|m| {
        let mut reg = m.borrow_mut();
        let mailbox = reg.get_mut(&pid).ok_or(MailboxError::NotLive(pid))?;
        if mailbox.len() >= DEFAULT_MAILBOX_CAPACITY {
            return Err(MailboxError::WouldBlock(pid));
        }
        mailbox.push_back(msg);
        Ok(())
    })?;
    // Outside the borrow — wake the actor if it was blocked.
    SCHEDULER.with(|s| s.borrow_mut().mark_runnable(pid));
    Ok(())
}

/// Pop the oldest message from `pid`'s mailbox. Returns
/// `Ok(Some(msg))` on success, `Ok(None)` when the mailbox is empty
/// (PR 2's `receive` will mark the actor blocked and yield in this
/// case), and `Err(NotLive)` when the PID is unknown.
pub fn dequeue(pid: ActorPid) -> Result<Option<Value>, MailboxError> {
    MAILBOX_REGISTRY.with(|m| {
        let mut reg = m.borrow_mut();
        let mailbox = reg.get_mut(&pid).ok_or(MailboxError::NotLive(pid))?;
        Ok(mailbox.pop_front())
    })
}

/// Inspect the mailbox depth for `pid`. Used by tests and PR 4's
/// deadlock-diagnostic preflight.
pub fn mailbox_len(pid: ActorPid) -> Result<usize, MailboxError> {
    MAILBOX_REGISTRY.with(|m| {
        let reg = m.borrow();
        let mailbox = reg.get(&pid).ok_or(MailboxError::NotLive(pid))?;
        Ok(mailbox.len())
    })
}

/// Remove `pid` from the registry — terminates the actor for
/// scheduling purposes. PR 5's example will exercise this through
/// the `done` actor lifecycle path.
pub fn deregister_actor(pid: ActorPid) -> Result<(), MailboxError> {
    let removed = MAILBOX_REGISTRY.with(|m| m.borrow_mut().remove(&pid));
    if removed.is_none() {
        return Err(MailboxError::NotLive(pid));
    }
    SCHEDULER.with(|s| {
        let mut sched = s.borrow_mut();
        sched.runnable.retain(|p| *p != pid);
        sched.blocked.remove(&pid);
    });
    Ok(())
}

/// Mark the calling actor as blocked on receive. PR 3's scheduler
/// will resume it when a message lands or a deadlock is detected.
pub fn mark_blocked(pid: ActorPid) {
    SCHEDULER.with(|s| s.borrow_mut().mark_blocked(pid));
}

// ---------------------------------------------------------------------------
// PR 2: spawn / send / receive
// ---------------------------------------------------------------------------

/// Allocate a new actor running `fn_value`, return `Value::ActorPid`.
/// Stores the function body in `ACTOR_FN_REGISTRY` for PR 3's scheduler.
pub fn actor_spawn(fn_value: Value) -> Result<Value, String> {
    let pid = register_actor();
    ACTOR_FN_REGISTRY.with(|r| r.borrow_mut().insert(pid, fn_value));
    Ok(Value::ActorPid(pid.0))
}

/// Enqueue `msg` into `pid_raw`'s mailbox. Maps `MailboxError` to a
/// human-readable `String` so it fits the `RResult<Value>` builtin API.
pub fn actor_send(pid_raw: u64, msg: Value) -> Result<(), String> {
    enqueue(ActorPid(pid_raw), msg).map_err(|e| e.to_string())
}

/// Dequeue the next message for the currently-executing actor.
/// Reads `CURRENT_ACTOR_PID` (set by PR 3's `Scheduler::step`).
/// Returns `Err("WouldBlock:<pid>")` when the mailbox is empty so the
/// scheduler can mark the actor blocked and retry later.
pub fn actor_receive() -> Result<Value, String> {
    let pid = CURRENT_ACTOR_PID
        .with(|c| *c.borrow())
        .ok_or_else(|| "receive() called outside of an actor context".to_string())?;
    match dequeue(pid).map_err(|e| e.to_string())? {
        Some(msg) => Ok(msg),
        None => {
            mark_blocked(pid);
            Err(format!("WouldBlock:{}", pid.0))
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler helpers (used by PR 3's Scheduler::step)
// ---------------------------------------------------------------------------

/// Set the currently-executing actor's PID. Called by PR 3's
/// `Scheduler::step` before dispatching each actor frame; cleared
/// (pass `None`) after the frame returns.
pub fn set_current_actor(pid: Option<ActorPid>) {
    CURRENT_ACTOR_PID.with(|c| *c.borrow_mut() = pid);
}

/// Retrieve the function body registered for `pid`. Returns `None`
/// when the PID is unknown or `actor_spawn` was not called for it.
pub fn get_actor_fn(pid: ActorPid) -> Option<Value> {
    ACTOR_FN_REGISTRY.with(|r| r.borrow().get(&pid).cloned())
}

/// Reset every thread-local for a fresh test run. Test-only — calling
/// this from production code would corrupt any in-flight scheduling.
#[cfg(test)]
pub fn reset_for_test() {
    MAILBOX_REGISTRY.with(|m| m.borrow_mut().clear());
    SCHEDULER.with(|s| *s.borrow_mut() = Scheduler::new());
    ACTOR_FN_REGISTRY.with(|r| r.borrow_mut().clear());
    CURRENT_ACTOR_PID.with(|c| *c.borrow_mut() = None);
}

// ---------------------------------------------------------------------------
// Typecheck pass entry-point (no-op for PR 1).
// ---------------------------------------------------------------------------

/// Wired into `<EXTENSION_PASSES>` in `typechecker.rs` so PRs 2-5 can
/// progressively add validation (e.g. "first arg of `spawn` must be a
/// fn-value with no parameters") without further core-file edits. For
/// PR 1 there's nothing to check.
pub fn check(_program: &crate::Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> ActorPid {
        reset_for_test();
        register_actor()
    }

    #[test]
    fn register_actor_allocates_fresh_pid() {
        reset_for_test();
        let p1 = register_actor();
        let p2 = register_actor();
        assert_ne!(p1, p2);
        assert_eq!(p1.0 + 1, p2.0);
        assert!(!p1.is_none());
    }

    #[test]
    fn enqueue_dequeue_round_trips() {
        let pid = fresh();
        enqueue(pid, Value::Int(42)).expect("enqueue should succeed");
        let v = dequeue(pid).expect("dequeue should succeed");
        match v {
            Some(Value::Int(n)) => assert_eq!(n, 42),
            other => panic!("expected Some(Int(42)), got {:?}", other),
        }
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let pid = fresh();
        let v = dequeue(pid).expect("dequeue should succeed");
        assert!(v.is_none(), "fresh actor's mailbox should be empty");
    }

    #[test]
    fn enqueue_to_unknown_pid_errors() {
        reset_for_test();
        let bogus = ActorPid(999);
        let err = enqueue(bogus, Value::Int(1)).expect_err("enqueue to bogus PID should fail");
        match err {
            MailboxError::NotLive(p) => assert_eq!(p, bogus),
            other => panic!("expected NotLive, got {:?}", other),
        }
    }

    #[test]
    fn enqueue_to_full_mailbox_returns_would_block() {
        let pid = fresh();
        for i in 0..DEFAULT_MAILBOX_CAPACITY {
            enqueue(pid, Value::Int(i as i64)).expect("under capacity");
        }
        let err = enqueue(pid, Value::Int(99)).expect_err("over capacity");
        match err {
            MailboxError::WouldBlock(p) => assert_eq!(p, pid),
            other => panic!("expected WouldBlock, got {:?}", other),
        }
    }

    #[test]
    fn fifo_ordering_is_preserved() {
        let pid = fresh();
        for i in 0..5 {
            enqueue(pid, Value::Int(i as i64)).unwrap();
        }
        for expected in 0..5 {
            match dequeue(pid).unwrap() {
                Some(Value::Int(n)) => assert_eq!(n, expected),
                other => panic!("expected Int({}), got {:?}", expected, other),
            }
        }
    }

    #[test]
    fn mailbox_len_reports_depth() {
        let pid = fresh();
        assert_eq!(mailbox_len(pid).unwrap(), 0);
        enqueue(pid, Value::Int(1)).unwrap();
        enqueue(pid, Value::Int(2)).unwrap();
        assert_eq!(mailbox_len(pid).unwrap(), 2);
        let _ = dequeue(pid).unwrap();
        assert_eq!(mailbox_len(pid).unwrap(), 1);
    }

    #[test]
    fn deregister_removes_actor() {
        let pid = fresh();
        deregister_actor(pid).expect("deregister should succeed");
        let err = enqueue(pid, Value::Int(1)).expect_err("enqueue after deregister should fail");
        assert!(matches!(err, MailboxError::NotLive(_)));
    }

    #[test]
    fn mark_blocked_then_send_marks_runnable() {
        // PR 3-4 contract: when a blocked actor receives a message,
        // it transitions back to runnable so the scheduler picks it
        // up on the next step.
        let pid = fresh();
        mark_blocked(pid);
        SCHEDULER.with(|s| {
            assert!(s.borrow().blocked_pids().contains(&pid));
        });
        enqueue(pid, Value::Int(1)).unwrap();
        SCHEDULER.with(|s| {
            let sched = s.borrow();
            assert!(!sched.blocked_pids().contains(&pid));
        });
    }

    #[test]
    fn deadlock_when_only_blocked_actors_remain() {
        let p1 = fresh();
        let p2 = register_actor();
        mark_blocked(p1);
        mark_blocked(p2);
        // Drain the runnable queue (initially p1 and p2 were marked
        // runnable on register; mark_blocked moved them to blocked).
        SCHEDULER.with(|s| {
            assert!(s.borrow().is_deadlocked(), "both actors blocked → deadlock");
        });
    }

    #[test]
    fn scheduler_pops_in_fifo_order() {
        reset_for_test();
        let p1 = register_actor();
        let p2 = register_actor();
        let p3 = register_actor();
        SCHEDULER.with(|s| {
            let mut sched = s.borrow_mut();
            assert_eq!(sched.pop_runnable(), Some(p1));
            assert_eq!(sched.pop_runnable(), Some(p2));
            assert_eq!(sched.pop_runnable(), Some(p3));
            assert_eq!(sched.pop_runnable(), None);
        });
    }

    // -----------------------------------------------------------------------
    // PR 2 tests: actor_spawn / actor_send / actor_receive
    // -----------------------------------------------------------------------

    /// Minimal placeholder value for spawn tests — actor_spawn accepts
    /// any Value; the function body is only executed by PR 3's scheduler.
    fn stub_fn() -> Value {
        Value::Int(0)
    }

    #[test]
    fn actor_spawn_returns_pid_value() {
        reset_for_test();
        let result = actor_spawn(stub_fn()).expect("spawn should succeed");
        match result {
            Value::ActorPid(id) => assert!(id > 0, "PID must be non-zero"),
            other => panic!("expected ActorPid, got {:?}", other),
        }
    }

    #[test]
    fn actor_spawn_stores_fn_in_registry() {
        reset_for_test();
        let pid_val = actor_spawn(stub_fn()).expect("spawn should succeed");
        let Value::ActorPid(id) = pid_val else {
            panic!("expected ActorPid");
        };
        let stored = get_actor_fn(ActorPid(id));
        assert!(stored.is_some(), "fn body must be in ACTOR_FN_REGISTRY");
    }

    #[test]
    fn actor_send_enqueues_message() {
        reset_for_test();
        let Value::ActorPid(id) = actor_spawn(stub_fn()).unwrap() else {
            panic!("expected ActorPid");
        };
        actor_send(id, Value::Int(99)).expect("send should succeed");
        assert_eq!(
            mailbox_len(ActorPid(id)).unwrap(),
            1,
            "mailbox should have 1 message"
        );
    }

    #[test]
    fn actor_send_to_unknown_pid_errors() {
        reset_for_test();
        let err = actor_send(9999, Value::Int(1)).expect_err("send to bogus PID should fail");
        assert!(
            err.contains("not live"),
            "error should mention 'not live', got: {}",
            err
        );
    }

    #[test]
    fn actor_receive_dequeues_message() {
        reset_for_test();
        let Value::ActorPid(id) = actor_spawn(stub_fn()).unwrap() else {
            panic!("expected ActorPid");
        };
        actor_send(id, Value::Int(42)).unwrap();
        set_current_actor(Some(ActorPid(id)));
        let msg = actor_receive().expect("receive should succeed");
        set_current_actor(None);
        match msg {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn actor_receive_empty_mailbox_returns_would_block() {
        reset_for_test();
        let Value::ActorPid(id) = actor_spawn(stub_fn()).unwrap() else {
            panic!("expected ActorPid");
        };
        set_current_actor(Some(ActorPid(id)));
        let err = actor_receive().expect_err("receive on empty mailbox should error");
        set_current_actor(None);
        assert!(
            err.starts_with("WouldBlock:"),
            "error should be WouldBlock, got: {}",
            err
        );
        // Actor must be marked blocked after WouldBlock.
        SCHEDULER.with(|s| {
            assert!(
                s.borrow().blocked_pids().contains(&ActorPid(id)),
                "actor should be marked blocked"
            );
        });
    }

    #[test]
    fn actor_receive_without_context_errors() {
        reset_for_test();
        set_current_actor(None);
        let err = actor_receive().expect_err("receive outside actor context should error");
        assert!(err.contains("outside of an actor context"), "got: {}", err);
    }
}
