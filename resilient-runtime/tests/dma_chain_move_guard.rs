use resilient_runtime::dma::{DmaChain, DmaDescriptor, DmaError, DmaWidth};

#[repr(align(4))]
struct Aligned([u8; 32]);

static SOURCE: Aligned = Aligned([0; 32]);
static DESTINATION: Aligned = Aligned([0; 32]);

fn descriptor(offset: usize) -> DmaDescriptor {
    DmaDescriptor::new(
        SOURCE.0.as_ptr() as usize + offset,
        DESTINATION.0.as_ptr() as usize + offset,
        4,
        DmaWidth::Word,
    )
    .unwrap()
}

#[test]
fn moved_populated_chain_fails_closed_before_dma_start() {
    let mut chain: DmaChain<2> = DmaChain::new();
    chain.append(descriptor(0)).unwrap();
    chain.append(descriptor(4)).unwrap();

    let mut moved = chain;
    let transfer = moved.start();

    assert!(matches!(
        transfer.error(),
        Some(DmaError::ChainMoved { .. })
    ));
    assert!(transfer.head_ptr().is_null());
    assert_eq!(transfer.descriptor_count(), 0);
    assert_eq!(transfer.total_bytes(), 0);
    assert!(transfer.descriptor(0).is_none());
}

#[test]
fn append_after_chain_move_returns_typed_error() {
    let mut chain: DmaChain<2> = DmaChain::new();
    chain.append(descriptor(0)).unwrap();

    let mut moved = chain;
    assert!(matches!(
        moved.append(descriptor(4)),
        Err(DmaError::ChainMoved { .. })
    ));
}
