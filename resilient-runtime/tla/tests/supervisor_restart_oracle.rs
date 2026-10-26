//! Emits bounded observations from the real supervisor implementation.
#[path = "../../../resilient/src/supervisor_runtime.rs"]
mod supervisor;

use supervisor::{RestartPolicy, SupervisorState};

fn main() {
    for budget in 0..=2 {
        for window in 0..=2 {
            for count in 0..=budget {
                for last in 0..=3 {
                    for now in 0..=3 {
                        let mut supervisor = SupervisorState::new("one_for_one");
                        supervisor
                            .register_child(
                                "worker".into(),
                                42,
                                RestartPolicy::Temporary {
                                    max_restarts: budget,
                                    window_secs: window,
                                },
                            )
                            .expect("register child");
                        let state = &mut supervisor.children.get_mut("worker").expect("child").1;
                        state.restart_count = count;
                        state.last_restart_time = last;
                        let allowed = supervisor.should_restart("worker", now) == Ok(true);
                        let recorded = supervisor.record_restart("worker", now).is_ok();
                        let state = &supervisor.children["worker"].1;
                        println!(
                            "{budget},{window},{count},{last},{now},{allowed},{recorded},{},{}",
                            state.restart_count, state.last_restart_time
                        );
                    }
                }
            }
        }
    }
}
