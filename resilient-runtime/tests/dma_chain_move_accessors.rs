use resilient_runtime::dma::{DmaChain, DmaDescriptor, DmaWidth};
use std::boxed::Box;

#[repr(align(4))]
struct Aligned([u8; 16]);

static SOURCE: Aligned = Aligned([0; 16]);
static DESTINATION: Aligned = Aligned([0; 16]);

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
fn moved_chain_accessors_hide_stale_descriptor_links() {
    let mut chain: DmaChain<2> = DmaChain::new();
    chain.append(descriptor(0)).unwrap();
    chain.append(descriptor(4)).unwrap();

    let moved = Box::new(chain);

    assert_eq!(moved.len(), 2);
    assert!(moved.get(0).is_none());
    assert!(moved.get(1).is_none());
    assert!(moved.head_ptr().is_null());
}
