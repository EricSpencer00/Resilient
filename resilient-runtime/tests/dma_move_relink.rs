use resilient_runtime::dma::{DmaChain, DmaDescriptor, DmaWidth};

fn descriptor(source: usize, dest: usize) -> DmaDescriptor {
    DmaDescriptor::new(source, dest, 4, DmaWidth::Word).expect("aligned descriptor")
}

#[test]
fn start_relinks_descriptors_after_chain_move() {
    let mut populated = DmaChain::<2>::new();
    populated
        .append(descriptor(0x1000, 0x2000))
        .expect("first descriptor");
    populated
        .append(descriptor(0x1004, 0x2004))
        .expect("second descriptor");

    let mut empty = DmaChain::<2>::new();
    core::mem::swap(&mut populated, &mut empty);

    let transfer = populated.start();
    let first = transfer.descriptor(0).expect("first descriptor");
    let second = transfer.descriptor(1).expect("second descriptor");
    assert_eq!(transfer.head_ptr(), first as *const _);
    assert_eq!(first.next, second as *const _);
    assert!(second.next.is_null());
}

#[test]
fn direct_head_pointer_relinks_after_chain_move() {
    let mut populated = DmaChain::<2>::new();
    populated
        .append(descriptor(0x3000, 0x4000))
        .expect("first descriptor");
    populated
        .append(descriptor(0x3004, 0x4004))
        .expect("second descriptor");

    let mut empty = DmaChain::<2>::new();
    core::mem::swap(&mut populated, &mut empty);

    let head = populated.head_ptr();
    let first = populated.get(0).expect("first descriptor");
    let second = populated.get(1).expect("second descriptor");
    assert_eq!(head, first as *const _);
    assert_eq!(first.next, second as *const _);
    assert!(second.next.is_null());
}
