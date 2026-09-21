use resilient_runtime::vm::loader::{LoaderError, load_and_run};
use resilient_runtime::vm::serde::{self, DecodeError};
use resilient_runtime::vm::{Instr, Value};

#[test]
fn flat_loader_rejects_unreachable_out_of_range_branch_targets() {
    for branch in [
        Instr::Jump(u32::MAX),
        Instr::JumpIfFalse(u32::MAX),
        Instr::JumpIfTrue(u32::MAX),
    ] {
        let program = [Instr::PushConst(Value::Int(7)), Instr::Return, branch];
        let mut blob = [0u8; 64];
        let len = serde::encode(&program, &mut blob).expect("fixture should fit");

        assert_eq!(
            load_and_run::<4, 4, 0>(&blob[..len]),
            Err(LoaderError::DecodeFailed(DecodeError::InvalidJumpTarget {
                target: u32::MAX,
                code_len: program.len(),
            }))
        );
    }
}
