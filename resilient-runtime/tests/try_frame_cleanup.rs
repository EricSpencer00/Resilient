#![cfg(feature = "vm")]

use resilient_runtime::vm::serde::{
    DecodedFunctionMeta, EncodeFunctionDef, decode_program, encode_program,
};
use resilient_runtime::vm::{CatchArm, FunctionDef, Instr, TryHandlerEntry, Value, Vm, VmError};

#[test]
fn decoded_callee_try_frame_cannot_leak_into_later_failure() {
    // The callee returns before its EnterTry is exited. Its handler target is
    // valid within that callee, so this is a decodable artifact rather than a
    // malformed reference. The later failing call must still be uncaught.
    let leaked_try = [
        Instr::EnterTry(0),
        Instr::PushConst(Value::Int(7)),
        Instr::Return,
        Instr::PushConst(Value::Int(99)),
        Instr::Return,
    ];
    let failing = [Instr::PushConst(Value::Int(0)), Instr::Return];
    let main = [
        Instr::EnterTry(1),
        Instr::Call(0),
        Instr::PushConst(Value::Int(7)),
        Instr::Eq,
        Instr::JumpIfTrue(7),
        Instr::PushConst(Value::Int(42)),
        Instr::Return,
        Instr::Call(1),
        Instr::Return,
    ];
    let encoded_functions = [
        EncodeFunctionDef {
            code: &leaked_try,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
            capture_count: 0,
        },
        EncodeFunctionDef {
            code: &failing,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: Some(1),
            capture_count: 0,
        },
    ];
    let mut handler = TryHandlerEntry::EMPTY;
    handler.arms[0] = Some(CatchArm {
        variant: 1,
        handler_pc: 3,
    });

    let mut blob = [0u8; 256];
    let len = encode_program(
        &main,
        &encoded_functions,
        &[handler, TryHandlerEntry::EMPTY],
        &mut blob,
    )
    .expect("reproduction artifact should encode");
    let mut out_main = [Instr::Return; 12];
    let mut out_meta = [DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
        capture_count: 0,
    }; 2];
    let mut out_code = [Instr::Return; 16];
    let mut out_handlers = [TryHandlerEntry::EMPTY; 2];
    let counts = decode_program(
        &blob[..len],
        &mut out_main,
        &mut out_meta,
        &mut out_code,
        &mut out_handlers,
    )
    .expect("valid references should decode");
    assert_eq!(counts.func_count, 2);

    let functions = [
        FunctionDef {
            code: &out_code
                [out_meta[0].offset as usize..(out_meta[0].offset + out_meta[0].len) as usize],
            arity: out_meta[0].arity,
            local_count: out_meta[0].local_count,
            postcheck: out_meta[0].postcheck,
            fails_variant: out_meta[0].fails_variant,
            capture_count: out_meta[0].capture_count,
        },
        FunctionDef {
            code: &out_code
                [out_meta[1].offset as usize..(out_meta[1].offset + out_meta[1].len) as usize],
            arity: out_meta[1].arity,
            local_count: out_meta[1].local_count,
            postcheck: out_meta[1].postcheck,
            fails_variant: out_meta[1].fails_variant,
            capture_count: out_meta[1].capture_count,
        },
    ];
    let mut vm = Vm::<8, 1, 3, 2, 0, 8>::new();
    assert_eq!(
        vm.run_with_tries(
            &functions,
            &out_handlers[..counts.try_count],
            &out_main[..counts.main_len],
        ),
        Err(VmError::CheckedFailure(1)),
    );
}
