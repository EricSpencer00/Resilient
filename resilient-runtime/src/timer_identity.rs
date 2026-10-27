//! Slot reuse must never make an old timer capability valid again.

use super::{Registry, Status, TimerConfig, TimerError, TimerId, TimerState};

pub(super) const INDEX_BITS: u32 = u8::BITS;
pub(super) const MAX_GENERATION: u64 = u64::MAX >> INDEX_BITS;

pub(super) fn next_generation(current: u64) -> Option<u64> {
    current
        .checked_add(1)
        .filter(|next| *next <= MAX_GENERATION)
}

pub(super) fn allocate(
    registry: &mut Registry,
    config: TimerConfig,
    period_us: u32,
) -> Result<TimerId, TimerError> {
    for (index, slot) in registry.slots.iter_mut().enumerate() {
        if slot.status != Status::Free {
            continue;
        }
        let Some(generation) = next_generation(slot.generation) else {
            continue;
        };
        *slot = TimerState {
            generation,
            delay: super::delay::Phase::Idle,
            status: Status::Stopped,
            mode: config.mode,
            period_us,
            accum_us: 0,
            count: 0,
            elapsed_us: 0,
            callback: None,
        };
        return Ok(TimerId((generation << INDEX_BITS) | index as u64));
    }
    Err(TimerError::Exhausted)
}

pub(super) fn slot(registry: &mut Registry, id: TimerId) -> Result<&mut TimerState, TimerError> {
    registry
        .slots
        .get_mut(id.index() as usize)
        .filter(|slot| slot.status != Status::Free && slot.generation == id.generation())
        .ok_or(TimerError::InvalidId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_generation_is_usable_but_cannot_wrap() {
        assert_eq!(next_generation(MAX_GENERATION - 1), Some(MAX_GENERATION));
        assert_eq!(next_generation(MAX_GENERATION), None);
        assert_eq!(next_generation(u64::MAX), None);
    }

    #[test]
    fn generation_bits_cannot_overlap_the_public_index() {
        let encoded = (MAX_GENERATION << INDEX_BITS) | 255;
        let handle = TimerId(encoded);
        assert_eq!(handle.index(), 255);
        assert_eq!(handle.generation(), MAX_GENERATION);
    }

    #[test]
    fn allocator_skips_a_retired_slot() {
        let mut registry = Registry::empty();
        registry.slots[0].generation = MAX_GENERATION;
        let handle = allocate(&mut registry, TimerConfig::periodic(1_000), 1_000).unwrap();
        assert_eq!(handle.index(), 1);
        assert_eq!(handle.generation(), 1);
        assert_eq!(registry.slots[0].status, Status::Free);
        assert_eq!(registry.slots[0].generation, MAX_GENERATION);
    }

    #[test]
    fn allocator_fails_closed_when_all_slots_are_retired() {
        let mut registry = Registry::empty();
        for slot in &mut registry.slots {
            slot.generation = MAX_GENERATION;
        }
        assert_eq!(
            allocate(&mut registry, TimerConfig::periodic(1_000), 1_000),
            Err(TimerError::Exhausted)
        );
        assert!(
            registry
                .slots
                .iter()
                .all(|slot| slot.status == Status::Free && slot.generation == MAX_GENERATION)
        );
    }
}
