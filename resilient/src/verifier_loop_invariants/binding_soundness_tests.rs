#![cfg(feature = "z3")]

use crate::typechecker::TypeChecker;
use crate::{Lexer, Parser};

fn certificates(source: &str) -> usize {
    let mut parser = Parser::new(Lexer::new(source));
    let program = parser.parse_program();
    assert!(parser.errors.is_empty(), "{:?}", parser.errors);
    let mut checker = TypeChecker::new()
        .with_warn_unverified(false)
        .with_proof_capture(true);
    super::verify_and_capture(&mut checker, &program);
    checker.loop_invariant_certificate_count()
}

#[test]
fn reassignment_invalidates_entry_constant() {
    assert_eq!(
        certificates("let i = 0; i = -1; while i < 1 invariant i >= 0 { i = i + 1; }"),
        0
    );
}

#[test]
fn branch_writes_do_not_restore_entry_constants() {
    assert_eq!(
        certificates("let i = 0; if true { i = -1; } while i < 1 invariant i >= 0 { i = i + 1; }"),
        0
    );
}

#[test]
fn prior_loop_writes_invalidate_following_loop_entry() {
    assert_eq!(
        certificates(
            "let i = 0; while i > -1 { i = i - 1; } while i < 1 invariant i >= 0 { i = i + 1; }"
        ),
        0
    );
}

#[test]
fn nested_loop_cannot_assume_first_outer_iteration_constants() {
    assert_eq!(
        certificates(
            "let i = 0; while i > -2 { while i < 0 invariant i >= 0 { i = i + 1; } i = i - 1; }"
        ),
        0
    );
}

#[test]
fn unchanged_counter_still_has_a_proof() {
    assert_eq!(
        certificates("let i = 0; while i < 3 invariant i >= 0 { i = i + 1; }"),
        1
    );
}

#[test]
fn sibling_loops_keep_only_unchanged_bindings() {
    assert_eq!(
        certificates(
            "let i = 0; let j = 0; while i < 3 invariant i >= 0 { i = i + 1; } while j < 3 invariant j >= 0 { j = j + 1; }"
        ),
        2
    );
}

#[test]
fn branch_local_literals_do_not_escape_to_other_branches() {
    assert_eq!(
        certificates(
            "let i = 0; if true { i = -1; } else { while i < 3 invariant i >= 0 { i = i + 1; } }"
        ),
        1
    );
}

#[test]
fn finite_loop_writes_invalidate_later_entry_state() {
    assert_eq!(
        certificates(
            "let i = 0; for x in [-1] { i = x; } while i < 1 invariant i >= 0 { i = i + 1; }"
        ),
        0
    );
}

#[test]
fn recovery_path_mutations_cannot_restore_old_constants() {
    assert_eq!(
        certificates(
            "let i = 0; try { i = -1; } catch Timeout { i = -2; } while i < 1 invariant i >= 0 { i = i + 1; }"
        ),
        0
    );
}

#[test]
fn handler_proof_is_independent_of_successful_try_path() {
    assert_eq!(
        certificates(
            "let i = -1; try { let i = 0; } catch Timeout { while i < 1 invariant i >= 0 { i = i + 1; } }"
        ),
        0
    );
}

#[test]
fn declaration_in_repeated_body_reestablishes_local_constant() {
    assert_eq!(
        certificates(
            "let n = 0; while n < 2 { let i = 0; while i < 2 invariant i >= 0 { i = i + 1; } n = n + 1; }"
        ),
        1
    );
}
