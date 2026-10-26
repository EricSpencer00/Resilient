use super::{RestartPolicy, SupervisorState};

fn temporary(limit: u32, window: u32) -> SupervisorState {
    let mut supervisor = SupervisorState::new("one_for_one");
    supervisor
        .register_child(
            "worker".into(),
            42,
            RestartPolicy::Temporary {
                max_restarts: limit,
                window_secs: window,
            },
        )
        .expect("register child");
    supervisor
}

fn snapshot(supervisor: &SupervisorState) -> (u32, u64) {
    let state = &supervisor.children["worker"].1;
    (state.restart_count, state.last_restart_time)
}

#[test]
fn zero_budget_never_authorizes_or_records_even_after_reset() {
    for window in [0, 1, 60] {
        let mut supervisor = temporary(0, window);
        for now in [0, 1, 59, 60, 1000, u64::MAX] {
            assert!(supervisor.should_restart("worker", now).is_err());
            assert!(supervisor.record_restart("worker", now).is_err());
            assert_eq!(snapshot(&supervisor), (0, 0));
        }
    }
}

#[test]
fn direct_recording_cannot_exceed_the_current_budget() {
    let mut supervisor = temporary(2, 60);
    supervisor
        .record_restart("worker", 1000)
        .expect("first restart");
    supervisor
        .record_restart("worker", 1001)
        .expect("second restart");
    assert!(supervisor.record_restart("worker", 1002).is_err());
    assert_eq!(snapshot(&supervisor), (2, 1001));
}

#[test]
fn exact_idle_window_boundary_resets_count_to_one() {
    let mut supervisor = temporary(1, 60);
    supervisor
        .record_restart("worker", 1000)
        .expect("first restart");
    assert!(supervisor.record_restart("worker", 1059).is_err());
    assert_eq!(snapshot(&supervisor), (1, 1000));
    supervisor
        .record_restart("worker", 1060)
        .expect("elapsed window");
    assert_eq!(snapshot(&supervisor), (1, 1060));
}

#[test]
fn clock_rollback_does_not_reset_an_exhausted_budget() {
    let mut supervisor = temporary(1, 60);
    supervisor
        .record_restart("worker", 1000)
        .expect("first restart");
    assert!(supervisor.should_restart("worker", 900).is_err());
    assert!(supervisor.record_restart("worker", 900).is_err());
    assert_eq!(snapshot(&supervisor), (1, 1000));
}

#[test]
fn reset_window_is_measured_from_the_last_recording() {
    let mut supervisor = temporary(2, 60);
    supervisor
        .record_restart("worker", 1000)
        .expect("first restart");
    supervisor
        .record_restart("worker", 1050)
        .expect("remaining budget");
    assert!(supervisor.record_restart("worker", 1060).is_err());
    supervisor
        .record_restart("worker", 1110)
        .expect("idle window elapsed");
    assert_eq!(snapshot(&supervisor), (1, 1110));
}

#[test]
fn positive_budget_with_zero_window_keeps_immediate_reset_semantics() {
    let mut supervisor = temporary(1, 0);
    supervisor
        .record_restart("worker", 0)
        .expect("first restart");
    supervisor
        .record_restart("worker", 0)
        .expect("immediate reset");
    assert_eq!(snapshot(&supervisor), (1, 0));
}

#[test]
fn permanent_counter_still_saturates_and_transient_recording_is_rejected() {
    let mut supervisor = SupervisorState::new("one_for_one");
    supervisor
        .register_child("worker".into(), 42, RestartPolicy::Permanent)
        .unwrap();
    supervisor
        .children
        .get_mut("worker")
        .unwrap()
        .1
        .restart_count = u32::MAX;
    supervisor
        .record_restart("worker", u64::MAX)
        .expect("permanent restart");
    assert_eq!(snapshot(&supervisor), (u32::MAX, u64::MAX));
    supervisor.children.get_mut("worker").unwrap().1.policy = RestartPolicy::Transient;
    assert!(!supervisor.should_restart("worker", 0).unwrap());
    assert!(supervisor.record_restart("worker", 0).is_err());
    assert_eq!(snapshot(&supervisor), (u32::MAX, u64::MAX));
}

#[test]
fn unknown_child_cannot_mutate_another_childs_state() {
    let mut supervisor = temporary(1, 60);
    assert!(supervisor.should_restart("missing", 1000).is_err());
    assert!(supervisor.record_restart("missing", 1000).is_err());
    assert_eq!(snapshot(&supervisor), (0, 0));
}
