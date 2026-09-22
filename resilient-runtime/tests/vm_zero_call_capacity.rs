#![cfg(feature = "vm")]

use resilient_runtime::vm::{Value, Vm, VmError};

#[test]
fn set_local_rejects_zero_call_capacity_without_panicking() {
    let mut vm = Vm::<4, 1, 0>::new();
    assert_eq!(
        vm.set_local(0, Value::Int(7)),
        Err(VmError::CallStackOverflow)
    );
}

#[test]
fn set_local_keeps_nonzero_capacity_bounds_behavior() {
    let mut vm = Vm::<4, 1>::new();
    assert_eq!(vm.set_local(0, Value::Int(7)), Ok(()));
    assert_eq!(
        vm.set_local(1, Value::Int(8)),
        Err(VmError::LocalsOutOfBounds)
    );
}
