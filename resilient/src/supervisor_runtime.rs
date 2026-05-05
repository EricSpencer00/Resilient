//! RES-780: Supervisor runtime phase 1 — crash propagation and restart policies.
//!
//! This module extends the basic actor runtime (RES-332) with supervisor
//! support. When supervised actors crash, the supervisor's configured policy
//! determines whether to restart, escalate, or stop.
//!
//! **Data model**:
//! - `RestartPolicy`: permanent (restart always), transient (no restart),
//!   temporary (restart up to limit)
//! - `CrashEvent`: signals that an actor crashed and why
//! - `SupervisorState`: tracks strategy, children, restart history per child
//!
//! **Scheduler integration** (future PRs):
//! - When an actor fails, emit `CrashEvent` to supervisor
//! - Supervisor applies policy → restart, escalate, or stop
//! - Enforce limits on restart attempts

#![allow(dead_code)]

use std::collections::HashMap;

/// Restart policy for a supervised actor.
/// Determines behavior when the actor crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Actor is restarted indefinitely on crash.
    /// Typical for stateless workers that should be always-on.
    Permanent,
    /// Actor is not automatically restarted on crash.
    /// Supervisor will report the crash but not respawn.
    Transient,
    /// Actor can be restarted up to a bounded limit within a time window.
    /// After limit exceeded, supervisor escalates to its own supervisor.
    Temporary { max_restarts: u32, window_secs: u32 },
}

/// Reason an actor crashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashReason {
    /// Unhandled error from the actor's message handler.
    UnhandledError,
    /// Panic or assertion failure in the actor.
    Panic,
    /// Supervisor explicitly killed the actor.
    Killed,
    /// Timeout waiting for response.
    Timeout,
}

/// Signal that an actor crashed and should be handled by its supervisor.
#[derive(Debug, Clone, Copy)]
pub struct CrashEvent {
    /// PID of the actor that crashed.
    pub actor_pid: u64,
    /// Why the actor crashed.
    pub reason: CrashReason,
}

/// State for a supervised actor: policy, restart counts, and timestamps.
#[derive(Debug, Clone)]
pub struct SupervisedActorState {
    /// ID assigned by the supervisor (e.g., "worker_1").
    pub id: String,
    /// Restart policy to apply when this actor crashes.
    pub policy: RestartPolicy,
    /// Number of times this actor has been restarted in the current window.
    pub restart_count: u32,
    /// Unix timestamp of the last restart (seconds).
    pub last_restart_time: u64,
}

/// State for a supervisor actor.
/// Tracks strategy, supervised children, and restart history.
#[derive(Debug, Clone)]
pub struct SupervisorState {
    /// The strategy to apply (one_for_one, rest_for_one, all_for_one).
    pub strategy: String,
    /// Supervised children: id -> (child_pid, actor_state).
    pub children: HashMap<String, (u64, SupervisedActorState)>,
}

impl SupervisorState {
    /// Create a new supervisor state.
    pub fn new(strategy: &str) -> Self {
        Self {
            strategy: strategy.to_string(),
            children: HashMap::new(),
        }
    }

    /// Register a supervised child.
    pub fn register_child(
        &mut self,
        id: String,
        child_pid: u64,
        policy: RestartPolicy,
    ) -> Result<(), String> {
        if self.children.contains_key(&id) {
            return Err(format!("Child {} already supervised", id));
        }
        self.children.insert(
            id.clone(),
            (
                child_pid,
                SupervisedActorState {
                    id,
                    policy,
                    restart_count: 0,
                    last_restart_time: 0,
                },
            ),
        );
        Ok(())
    }

    /// Check if a child should be restarted based on its policy.
    /// Returns Ok(true) if restart is allowed, Ok(false) if not,
    /// or Err if restart limit exceeded.
    pub fn should_restart(&self, child_id: &str, now_secs: u64) -> Result<bool, String> {
        let (_, state) = match self.children.get(child_id) {
            Some(pair) => pair,
            None => return Err(format!("Unknown child {}", child_id)),
        };

        match state.policy {
            RestartPolicy::Permanent => Ok(true),
            RestartPolicy::Transient => Ok(false),
            RestartPolicy::Temporary {
                max_restarts,
                window_secs,
            } => {
                let window_elapsed =
                    now_secs.saturating_sub(state.last_restart_time) >= window_secs as u64;
                if window_elapsed {
                    // New window: reset count
                    Ok(true)
                } else if state.restart_count < max_restarts {
                    Ok(true)
                } else {
                    Err(format!(
                        "Child {} restart limit exceeded ({} in {} seconds)",
                        child_id, max_restarts, window_secs
                    ))
                }
            }
        }
    }

    /// Record that a child was restarted.
    pub fn record_restart(&mut self, child_id: &str, now_secs: u64) -> Result<(), String> {
        let (_, state) = match self.children.get_mut(child_id) {
            Some(pair) => pair,
            None => return Err(format!("Unknown child {}", child_id)),
        };

        match state.policy {
            RestartPolicy::Permanent => {
                state.restart_count = state.restart_count.saturating_add(1);
                state.last_restart_time = now_secs;
                Ok(())
            }
            RestartPolicy::Transient => {
                // Transient actors should not be restarted, so this is unexpected
                Err("Cannot restart transient actor".to_string())
            }
            RestartPolicy::Temporary { window_secs, .. } => {
                let window_elapsed =
                    now_secs.saturating_sub(state.last_restart_time) >= window_secs as u64;
                if window_elapsed {
                    // New window: reset count
                    state.restart_count = 1;
                } else {
                    state.restart_count = state.restart_count.saturating_add(1);
                }
                state.last_restart_time = now_secs;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supervisor_state_new_empty() {
        let sup = SupervisorState::new("one_for_one");
        assert_eq!(sup.strategy, "one_for_one");
        assert!(sup.children.is_empty());
    }

    #[test]
    fn register_child_succeeds() {
        let mut sup = SupervisorState::new("one_for_one");
        let result = sup.register_child("worker".to_string(), 42, RestartPolicy::Permanent);
        assert!(result.is_ok());
        assert!(sup.children.contains_key("worker"));
    }

    #[test]
    fn register_duplicate_child_errors() {
        let mut sup = SupervisorState::new("one_for_one");
        sup.register_child("worker".to_string(), 42, RestartPolicy::Permanent)
            .unwrap();
        let result = sup.register_child("worker".to_string(), 43, RestartPolicy::Permanent);
        assert!(result.is_err());
    }

    #[test]
    fn permanent_policy_always_restarts() {
        let mut sup = SupervisorState::new("one_for_one");
        sup.register_child("worker".to_string(), 42, RestartPolicy::Permanent)
            .unwrap();
        assert!(sup.should_restart("worker", 1000).unwrap());
        assert!(sup.should_restart("worker", 2000).unwrap());
    }

    #[test]
    fn transient_policy_never_restarts() {
        let mut sup = SupervisorState::new("one_for_one");
        sup.register_child("worker".to_string(), 42, RestartPolicy::Transient)
            .unwrap();
        assert!(!sup.should_restart("worker", 1000).unwrap());
    }

    #[test]
    fn temporary_policy_respects_limit() {
        let mut sup = SupervisorState::new("one_for_one");
        sup.register_child(
            "worker".to_string(),
            42,
            RestartPolicy::Temporary {
                max_restarts: 2,
                window_secs: 60,
            },
        )
        .unwrap();
        assert!(sup.should_restart("worker", 1000).unwrap());
        sup.record_restart("worker", 1000).unwrap();
        assert!(sup.should_restart("worker", 1001).unwrap());
        sup.record_restart("worker", 1001).unwrap();
        // Two restarts done, limit is 2
        assert!(sup.should_restart("worker", 1002).is_err());
    }

    #[test]
    fn temporary_policy_resets_after_window() {
        let mut sup = SupervisorState::new("one_for_one");
        sup.register_child(
            "worker".to_string(),
            42,
            RestartPolicy::Temporary {
                max_restarts: 2,
                window_secs: 60,
            },
        )
        .unwrap();
        assert!(sup.should_restart("worker", 1000).unwrap());
        sup.record_restart("worker", 1000).unwrap();
        sup.record_restart("worker", 1001).unwrap();
        // Limit exceeded at time 1001
        assert!(sup.should_restart("worker", 1001).is_err());
        // But after window (1000 + 60 = 1060), it resets
        assert!(sup.should_restart("worker", 1061).unwrap());
    }
}
