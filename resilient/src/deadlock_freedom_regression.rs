use super::build;
use crate::parse;

#[test]
fn actor_cycle_through_while_bodies_is_not_certified_as_acyclic() {
    let src = r#"
        actor A {
            receive tick(int n) {
                while n > 0 { send(B); }
            }
        }
        actor B {
            receive tick(int n) {
                while n > 0 { send(A); }
            }
        }
    "#;
    let (program, errors) = parse(src);
    assert!(errors.is_empty(), "parse errors: {errors:?}");

    let graph = build(&program);
    assert!(graph.edges["A"].contains("B"));
    assert!(graph.edges["B"].contains("A"));
}

#[test]
fn actor_sends_inside_other_nested_forms_are_collected() {
    let src = r#"
        actor A {
            receive tick(int n) {
                for i in 0..1 { send(B); }
                match n {
                    0 => send(B),
                    _ => { send(B); }
                }
                try { send(B); } catch Timeout { send(B); }
            }
        }
        actor B {
            receive tick(int n) {}
        }
    "#;
    let (program, errors) = parse(src);
    assert!(errors.is_empty(), "parse errors: {errors:?}");

    let graph = build(&program);
    assert!(graph.edges["A"].contains("B"));
}
