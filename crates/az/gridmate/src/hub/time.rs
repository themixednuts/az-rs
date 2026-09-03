//! Strong protocol-time values owned by Amazon Hub.

use crate::protocol_time::define_protocol_time;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// Difference between the Unix epoch and the Amazon Hub synchronized-clock
/// epoch (`2000-01-01T00:00:00Z`).
const SYNCED_TIMESTAMP_EPOCH_FROM_UNIX: Duration = Duration::from_hours(262_968);

define_protocol_time!(
    /// `Amazon::Hub::SyncedTimestamp`, carried as an unsigned nanosecond count.
    SyncedTimestamp
);

/// Failure to establish the process's Amazon Hub synchronized clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SyncedClockError {
    #[error("system time precedes the Amazon Hub epoch (2000-01-01T00:00:00Z)")]
    BeforeEpoch,
}

/// Process-local producer for [`SyncedTimestamp`].
///
/// The wall clock is sampled once. Subsequent values advance from a monotonic
/// [`Instant`], so operating-system clock corrections cannot move a running
/// Hub clock backwards. Servers use one instance for registration timestamps
/// and later clock-synchronization messages.
#[derive(Debug, Clone)]
pub struct SyncedClock {
    anchor: SyncedTimestamp,
    started_at: Instant,
}

impl SyncedClock {
    /// Establish a synchronized clock from the current system wall time.
    ///
    /// # Errors
    ///
    /// Returns [`SyncedClockError::BeforeEpoch`] if the system wall clock reads
    /// earlier than the Amazon Hub epoch (`2000-01-01T00:00:00Z`) — either
    /// before the Unix epoch, or in the 30 years between the two epochs.
    pub fn system() -> Result<Self, SyncedClockError> {
        Self::from_system_time_at(SystemTime::now(), Instant::now())
    }

    /// Establish a synchronized clock from an explicit wall-clock sample.
    ///
    /// This is useful when a host already owns the authoritative system-time
    /// sample. The resulting clock still advances monotonically from creation.
    ///
    /// # Errors
    ///
    /// Returns [`SyncedClockError::BeforeEpoch`] if `system_time` is earlier
    /// than the Amazon Hub epoch (`2000-01-01T00:00:00Z`) — either before the
    /// Unix epoch, or in the 30 years between the two epochs.
    pub fn from_system_time(system_time: SystemTime) -> Result<Self, SyncedClockError> {
        Self::from_system_time_at(system_time, Instant::now())
    }

    /// Return the current synchronized timestamp.
    #[must_use]
    pub fn now(&self) -> SyncedTimestamp {
        self.at_elapsed(self.started_at.elapsed())
    }

    #[must_use]
    fn at_elapsed(&self, elapsed: Duration) -> SyncedTimestamp {
        let elapsed = SyncedTimestamp::from_std(elapsed).as_nanoseconds();
        SyncedTimestamp::from_nanoseconds(self.anchor.as_nanoseconds().saturating_add(elapsed))
    }

    fn from_system_time_at(
        system_time: SystemTime,
        started_at: Instant,
    ) -> Result<Self, SyncedClockError> {
        let since_unix = system_time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SyncedClockError::BeforeEpoch)?;
        let since_hub_epoch = since_unix
            .checked_sub(SYNCED_TIMESTAMP_EPOCH_FROM_UNIX)
            .ok_or(SyncedClockError::BeforeEpoch)?;
        Ok(Self {
            anchor: SyncedTimestamp::from_std(since_hub_epoch),
            started_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::serialize::Marshaler;
    use crate::serialize::buffer::{CARRIER_ENDIAN, ReadBuffer, WriteBuffer};

    use super::*;

    #[test]
    fn synced_timestamp_preserves_the_fixed_u64_wire_shape() {
        let value = SyncedTimestamp::from_nanoseconds(0x1112_1314_1516_1718);
        let mut wb = WriteBuffer::new(CARRIER_ENDIAN);
        value.marshal(&mut wb);

        let bytes = wb.into_vec();
        assert_eq!(bytes.len(), 8);
        let mut rb = ReadBuffer::new(CARRIER_ENDIAN, &bytes);
        let decoded = SyncedTimestamp::unmarshal(&mut rb).expect("decode synchronized timestamp");
        assert_eq!(decoded, value);
        assert_eq!(rb.left(), 0);
    }

    #[test]
    fn synchronized_clock_uses_the_2000_utc_epoch() {
        // The sub-second part is a multiple of 100 ns: Windows SystemTime is
        // FILETIME-backed, so finer precision truncates at construction.
        let live_sample =
            UNIX_EPOCH + Duration::from_secs(1_786_349_471) + Duration::from_nanos(979_279_900);
        let clock = SyncedClock::from_system_time_at(live_sample, Instant::now())
            .expect("live wall-clock sample");

        assert_eq!(clock.anchor.as_nanoseconds(), 839_664_671_979_279_900);
    }

    #[test]
    fn synchronized_clock_advances_from_its_monotonic_anchor() {
        let clock = SyncedClock::from_system_time_at(
            UNIX_EPOCH + SYNCED_TIMESTAMP_EPOCH_FROM_UNIX + Duration::from_secs(7),
            Instant::now(),
        )
        .expect("clock after Hub epoch");

        assert_eq!(
            clock.at_elapsed(Duration::from_millis(250)),
            SyncedTimestamp::from_nanoseconds(7_250_000_000)
        );
    }

    #[test]
    fn synchronized_clock_rejects_pre_epoch_wall_time() {
        assert_eq!(
            SyncedClock::from_system_time(UNIX_EPOCH).unwrap_err(),
            SyncedClockError::BeforeEpoch
        );
    }
}
