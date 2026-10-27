//! Delay ownership and driver time are independent of peripheral expiry.

use super::{Registry, Status, TimerError, TimerId, TimerState, identity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    Waiting { remaining_us: u64 },
    Cancelled,
}

#[derive(Clone, Copy)]
pub(super) struct Ticket {
    id: TimerId,
}

pub(super) fn advance(slot: &mut TimerState, elapsed_us: u32) {
    if let Phase::Waiting { remaining_us } = slot.delay {
        slot.delay = Phase::Waiting {
            remaining_us: remaining_us.saturating_sub(u64::from(elapsed_us)),
        };
    }
}

pub(super) fn ensure_idle(slot: &TimerState) -> Result<(), TimerError> {
    if slot.delay != Phase::Idle {
        Err(TimerError::InvalidState)
    } else {
        Ok(())
    }
}

pub(super) fn cancel(slot: &mut TimerState) {
    if matches!(slot.delay, Phase::Waiting { .. }) {
        slot.delay = Phase::Cancelled;
    }
}

pub(super) fn begin(registry: &mut Registry, id: TimerId, ms: u32) -> Result<Ticket, TimerError> {
    let slot = identity::slot(registry, id)?;
    ensure_idle(slot)?;
    if slot.status == Status::Running {
        return Err(TimerError::InvalidState);
    }
    slot.delay = Phase::Waiting {
        remaining_us: u64::from(ms).saturating_mul(1_000),
    };
    slot.status = Status::Running;
    slot.accum_us = 0;
    slot.elapsed_us = 0;
    Ok(Ticket { id })
}

pub(super) fn poll(registry: &mut Registry, ticket: Ticket) -> Result<bool, TimerError> {
    let slot = identity::slot(registry, ticket.id)?;
    match slot.delay {
        Phase::Cancelled => {
            slot.delay = Phase::Idle;
            slot.status = Status::Stopped;
            Err(TimerError::InvalidState)
        }
        Phase::Waiting { .. } if slot.status == Status::Stopped => {
            slot.delay = Phase::Idle;
            Err(TimerError::InvalidState)
        }
        Phase::Waiting { remaining_us: 0 } => {
            slot.delay = Phase::Idle;
            slot.status = Status::Stopped;
            Ok(true)
        }
        Phase::Waiting { .. } => Ok(false),
        Phase::Idle => Err(TimerError::InvalidState),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timer::TimerConfig;

    fn configured() -> (Registry, TimerId) {
        let mut registry = Registry::empty();
        let id = identity::allocate(&mut registry, TimerConfig::one_shot(1_000), 1_000).unwrap();
        (registry, id)
    }

    #[test]
    fn expired_peripheral_does_not_freeze_a_delay() {
        let (mut registry, id) = configured();
        let ticket = begin(&mut registry, id, 5).unwrap();
        identity::slot(&mut registry, id).unwrap().status = Status::Expired;
        assert!(matches!(
            begin(&mut registry, id, 1),
            Err(TimerError::InvalidState)
        ));
        advance(identity::slot(&mut registry, id).unwrap(), 1_000);
        assert_eq!(poll(&mut registry, ticket), Ok(false));
        advance(identity::slot(&mut registry, id).unwrap(), 4_000);
        assert_eq!(poll(&mut registry, ticket), Ok(true));
        assert_eq!(
            identity::slot(&mut registry, id).unwrap().delay,
            Phase::Idle
        );
    }

    #[test]
    fn cancellation_reservation_is_released_only_by_its_waiter() {
        let (mut registry, id) = configured();
        let ticket = begin(&mut registry, id, 5).unwrap();
        assert_eq!(
            ensure_idle(identity::slot(&mut registry, id).unwrap()),
            Err(TimerError::InvalidState)
        );
        assert!(matches!(
            begin(&mut registry, id, 1),
            Err(TimerError::InvalidState)
        ));
        cancel(identity::slot(&mut registry, id).unwrap());
        identity::slot(&mut registry, id).unwrap().status = Status::Stopped;
        assert!(matches!(
            begin(&mut registry, id, 1),
            Err(TimerError::InvalidState)
        ));
        assert_eq!(poll(&mut registry, ticket), Err(TimerError::InvalidState));
        assert_eq!(
            ensure_idle(identity::slot(&mut registry, id).unwrap()),
            Ok(())
        );
        assert!(begin(&mut registry, id, 1).is_ok());
    }

    #[test]
    fn peripheral_counter_saturation_cannot_complete_or_freeze_a_delay() {
        let (mut registry, id) = configured();
        let ticket = begin(&mut registry, id, 1).unwrap();
        identity::slot(&mut registry, id).unwrap().elapsed_us = u64::MAX;
        assert_eq!(poll(&mut registry, ticket), Ok(false));
        advance(identity::slot(&mut registry, id).unwrap(), 1_000);
        assert_eq!(poll(&mut registry, ticket), Ok(true));
    }

    #[test]
    fn maximum_millisecond_duration_is_reachable_without_wrap() {
        let (mut registry, id) = configured();
        let ticket = begin(&mut registry, id, u32::MAX).unwrap();
        for _ in 0..999 {
            advance(identity::slot(&mut registry, id).unwrap(), u32::MAX);
            assert_eq!(poll(&mut registry, ticket), Ok(false));
        }
        advance(identity::slot(&mut registry, id).unwrap(), u32::MAX);
        assert_eq!(poll(&mut registry, ticket), Ok(true));
    }
}
