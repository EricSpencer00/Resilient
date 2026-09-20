//! Regression coverage for the compiler's packed local-slot capacity.

use std::fmt::Write;

use crate::bytecode::CompileError;
use crate::compiler::compile;

fn program_with_top_level_lets(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        writeln!(source, "let local_{index} = {index};").expect("writing a String cannot fail");
    }
    source.push_str(&format!("local_{}\n", count - 1));
    source
}

fn program_with_function_locals(count: usize) -> String {
    let mut source = String::from("fn big() -> int {\n");
    for index in 0..count {
        writeln!(source, "let local_{index} = {index};").expect("writing a String cannot fail");
    }
    writeln!(
        source,
        "return local_{count_minus_one};",
        count_minus_one = count - 1
    )
    .expect("writing a String cannot fail");
    source.push_str("}\nbig()\n");
    source
}

#[test]
fn last_packed_local_slot_is_representable() {
    let source = program_with_top_level_lets(8192);
    let (program, errors) = crate::parse(&source);
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    assert!(compile(&program).is_ok());
}

#[test]
fn first_local_after_packed_capacity_is_rejected() {
    let source = program_with_top_level_lets(8193);
    let (program, errors) = crate::parse(&source);
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    assert!(matches!(
        compile(&program),
        Err(CompileError::TooManyLocals)
    ));
}

#[test]
fn function_local_after_packed_capacity_is_rejected() {
    let source = program_with_function_locals(8193);
    let (program, errors) = crate::parse(&source);
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    assert!(matches!(
        compile(&program),
        Err(CompileError::TooManyLocals)
    ));
}
