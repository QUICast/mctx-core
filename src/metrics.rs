use crate::{Context, Publication};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Default)]
pub(crate) struct MetricsSequence {
    value: AtomicU64,
}

pub(crate) struct MetricsWriteGuard<'a> {
    sequence: &'a MetricsSequence,
}

impl MetricsSequence {
    pub(crate) fn write(&self) -> MetricsWriteGuard<'_> {
        let mut current = self.value.load(Ordering::Relaxed);
        loop {
            if current % 2 == 1 {
                std::hint::spin_loop();
                current = self.value.load(Ordering::Relaxed);
                continue;
            }

            match self.value.compare_exchange_weak(
                current,
                current.wrapping_add(1),
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return MetricsWriteGuard { sequence: self },
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) fn read_consistent<T>(&self, read: impl Fn() -> T) -> T {
        loop {
            let before = self.value.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }

            let snapshot = read();
            // Keep all counter reads between the two sequence observations.
            std::sync::atomic::fence(Ordering::AcqRel);
            let after = self.value.load(Ordering::Relaxed);
            if before == after {
                return snapshot;
            }
        }
    }
}

impl Drop for MetricsWriteGuard<'_> {
    fn drop(&mut self) {
        self.sequence.value.fetch_add(1, Ordering::Release);
    }
}

fn rate_per_sec(count: u64, interval_secs: f64) -> f64 {
    if interval_secs > 0.0 {
        count as f64 / interval_secs
    } else {
        0.0
    }
}

/// A point-in-time snapshot of cumulative publication metrics.
///
/// Counter fields in this snapshot are cumulative from the lifetime of the
/// publication and can be compared against an earlier snapshot to compute
/// deltas and rates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationMetricsSnapshot {
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
    pub captured_at: SystemTime,
}

/// The difference between two cumulative publication metrics snapshots.
///
/// This contains only counter-based deltas over the sampled interval.
#[derive(Debug, Clone, PartialEq)]
pub struct PublicationMetricsDelta {
    pub interval_secs: f64,
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

impl PublicationMetricsSnapshot {
    /// Computes the counter deltas between this snapshot and an earlier one.
    ///
    /// Returns `None` if:
    /// - `earlier` was captured after `self`
    /// - any cumulative counter appears to have moved backwards
    pub fn delta_since(&self, earlier: &Self) -> Option<PublicationMetricsDelta> {
        let duration = self.captured_at.duration_since(earlier.captured_at).ok()?;
        self.delta_since_duration(earlier, duration)
    }

    /// Computes counter deltas using a caller-supplied monotonic interval.
    pub fn delta_since_duration(
        &self,
        earlier: &Self,
        duration: Duration,
    ) -> Option<PublicationMetricsDelta> {
        Some(PublicationMetricsDelta {
            interval_secs: duration.as_secs_f64(),
            send_calls: self.send_calls.checked_sub(earlier.send_calls)?,
            packets_sent: self.packets_sent.checked_sub(earlier.packets_sent)?,
            bytes_sent: self.bytes_sent.checked_sub(earlier.bytes_sent)?,
            send_errors: self.send_errors.checked_sub(earlier.send_errors)?,
        })
    }
}

impl PublicationMetricsDelta {
    /// Returns the average send call count per second over the sampled interval.
    pub fn send_calls_per_sec(&self) -> f64 {
        rate_per_sec(self.send_calls, self.interval_secs)
    }

    /// Returns the average packets sent per second over the sampled interval.
    pub fn packets_per_sec(&self) -> f64 {
        rate_per_sec(self.packets_sent, self.interval_secs)
    }

    /// Returns the average bytes sent per second over the sampled interval.
    pub fn bytes_per_sec(&self) -> f64 {
        rate_per_sec(self.bytes_sent, self.interval_secs)
    }

    /// Returns the average send error count per second over the sampled interval.
    pub fn send_errors_per_sec(&self) -> f64 {
        rate_per_sec(self.send_errors, self.interval_secs)
    }
}

/// Tracks successive publication metrics snapshots and computes deltas between them.
#[derive(Debug, Clone)]
pub struct PublicationMetricsSampler<'a> {
    publication: &'a Publication,
    previous: Option<PublicationMetricsSnapshot>,
    previous_sampled_at: Option<Instant>,
}

impl<'a> PublicationMetricsSampler<'a> {
    pub fn new(publication: &'a Publication) -> Self {
        Self {
            publication,
            previous: None,
            previous_sampled_at: None,
        }
    }

    pub fn snapshot(&self) -> PublicationMetricsSnapshot {
        self.publication.metrics_snapshot()
    }

    pub fn sample(&mut self) -> Option<PublicationMetricsDelta> {
        let current = self.snapshot();
        self.sample_snapshot_at(current, Instant::now())
    }

    /// Computes a delta from a caller-supplied snapshot using its wall-clock
    /// `captured_at` timestamp. This clears the monotonic baseline used by
    /// `sample()` and `sample_snapshot_at()`.
    pub fn sample_snapshot(
        &mut self,
        current: PublicationMetricsSnapshot,
    ) -> Option<PublicationMetricsDelta> {
        let delta = self
            .previous
            .as_ref()
            .and_then(|previous| current.delta_since(previous));
        self.previous = Some(current);
        self.previous_sampled_at = None;
        delta
    }

    /// Computes a delta from a caller-supplied snapshot and monotonic capture
    /// instant. The first call after `sample_snapshot()` establishes a new
    /// monotonic baseline and returns `None`.
    pub fn sample_snapshot_at(
        &mut self,
        current: PublicationMetricsSnapshot,
        sampled_at: Instant,
    ) -> Option<PublicationMetricsDelta> {
        let delta = match (&self.previous, self.previous_sampled_at) {
            (Some(previous), Some(previous_sampled_at)) => sampled_at
                .checked_duration_since(previous_sampled_at)
                .and_then(|duration| current.delta_since_duration(previous, duration)),
            _ => None,
        };
        self.previous = Some(current);
        self.previous_sampled_at = Some(sampled_at);
        delta
    }

    pub fn reset(&mut self) {
        self.previous = None;
        self.previous_sampled_at = None;
    }

    pub fn previous(&self) -> Option<&PublicationMetricsSnapshot> {
        self.previous.as_ref()
    }

    /// Convenience alias for `sample()`.
    pub fn delta(&mut self) -> Option<PublicationMetricsDelta> {
        self.sample()
    }
}

/// A point-in-time snapshot of cumulative context metrics.
///
/// Counter fields in this snapshot are cumulative from the lifetime of the
/// context for send activity issued through `Context` methods and can be
/// compared against an earlier snapshot to compute deltas and rates.
///
/// Gauge-like fields such as `active_publications` represent the current state
/// at the moment the snapshot was taken and should not be interpreted as
/// cumulative counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMetricsSnapshot {
    pub publications_added: u64,
    pub publications_removed: u64,
    pub active_publications: usize,
    pub total_send_calls: u64,
    pub total_packets_sent: u64,
    pub total_bytes_sent: u64,
    pub total_send_errors: u64,
    pub captured_at: SystemTime,
}

/// The difference between two cumulative context metrics snapshots.
///
/// This contains only counter-based deltas over the sampled interval.
/// Gauge-like values such as active publication counts are intentionally not
/// included here; callers should inspect those directly from the latest
/// snapshot instead.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMetricsDelta {
    pub interval_secs: f64,
    pub publications_added: u64,
    pub publications_removed: u64,
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

impl ContextMetricsSnapshot {
    /// Computes the counter deltas between this snapshot and an earlier one.
    ///
    /// Returns `None` if:
    /// - `earlier` was captured after `self`
    /// - any cumulative counter appears to have moved backwards
    ///
    /// The resulting delta contains only counter-based values and the elapsed
    /// interval in seconds. Gauge-like values such as active publication counts
    /// should be read directly from the latest snapshot instead.
    pub fn delta_since(&self, earlier: &Self) -> Option<ContextMetricsDelta> {
        let duration = self.captured_at.duration_since(earlier.captured_at).ok()?;
        self.delta_since_duration(earlier, duration)
    }

    /// Computes counter deltas using a caller-supplied monotonic interval.
    pub fn delta_since_duration(
        &self,
        earlier: &Self,
        duration: Duration,
    ) -> Option<ContextMetricsDelta> {
        Some(ContextMetricsDelta {
            interval_secs: duration.as_secs_f64(),
            publications_added: self
                .publications_added
                .checked_sub(earlier.publications_added)?,
            publications_removed: self
                .publications_removed
                .checked_sub(earlier.publications_removed)?,
            send_calls: self
                .total_send_calls
                .checked_sub(earlier.total_send_calls)?,
            packets_sent: self
                .total_packets_sent
                .checked_sub(earlier.total_packets_sent)?,
            bytes_sent: self
                .total_bytes_sent
                .checked_sub(earlier.total_bytes_sent)?,
            send_errors: self
                .total_send_errors
                .checked_sub(earlier.total_send_errors)?,
        })
    }
}

impl ContextMetricsDelta {
    /// Returns the average send call count per second over the sampled interval.
    pub fn send_calls_per_sec(&self) -> f64 {
        rate_per_sec(self.send_calls, self.interval_secs)
    }

    /// Returns the average packets sent per second over the sampled interval.
    pub fn packets_per_sec(&self) -> f64 {
        rate_per_sec(self.packets_sent, self.interval_secs)
    }

    /// Returns the average bytes sent per second over the sampled interval.
    pub fn bytes_per_sec(&self) -> f64 {
        rate_per_sec(self.bytes_sent, self.interval_secs)
    }

    /// Returns the average send error count per second over the sampled interval.
    pub fn send_errors_per_sec(&self) -> f64 {
        rate_per_sec(self.send_errors, self.interval_secs)
    }
}

/// Tracks successive context metrics snapshots and computes deltas between them.
#[derive(Debug, Clone)]
pub struct ContextMetricsSampler<'a> {
    context: &'a Context,
    previous: Option<ContextMetricsSnapshot>,
    previous_sampled_at: Option<Instant>,
}

impl<'a> ContextMetricsSampler<'a> {
    pub fn new(context: &'a Context) -> Self {
        Self {
            context,
            previous: None,
            previous_sampled_at: None,
        }
    }

    pub fn snapshot(&self) -> ContextMetricsSnapshot {
        self.context.metrics_snapshot()
    }

    pub fn sample(&mut self) -> Option<ContextMetricsDelta> {
        let current = self.snapshot();
        self.sample_snapshot_at(current, Instant::now())
    }

    /// Computes a delta from a caller-supplied snapshot using its wall-clock
    /// `captured_at` timestamp. This clears the monotonic baseline used by
    /// `sample()` and `sample_snapshot_at()`.
    pub fn sample_snapshot(
        &mut self,
        current: ContextMetricsSnapshot,
    ) -> Option<ContextMetricsDelta> {
        let delta = self
            .previous
            .as_ref()
            .and_then(|previous| current.delta_since(previous));
        self.previous = Some(current);
        self.previous_sampled_at = None;
        delta
    }

    /// Computes a delta from a caller-supplied snapshot and monotonic capture
    /// instant. The first call after `sample_snapshot()` establishes a new
    /// monotonic baseline and returns `None`.
    pub fn sample_snapshot_at(
        &mut self,
        current: ContextMetricsSnapshot,
        sampled_at: Instant,
    ) -> Option<ContextMetricsDelta> {
        let delta = match (&self.previous, self.previous_sampled_at) {
            (Some(previous), Some(previous_sampled_at)) => sampled_at
                .checked_duration_since(previous_sampled_at)
                .and_then(|duration| current.delta_since_duration(previous, duration)),
            _ => None,
        };
        self.previous = Some(current);
        self.previous_sampled_at = Some(sampled_at);
        delta
    }

    pub fn reset(&mut self) {
        self.previous = None;
        self.previous_sampled_at = None;
    }

    pub fn previous(&self) -> Option<&ContextMetricsSnapshot> {
        self.previous.as_ref()
    }

    /// Convenience alias for `sample()`.
    pub fn delta(&mut self) -> Option<ContextMetricsDelta> {
        self.sample()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn publication_snapshot(
        send_calls: u64,
        packets_sent: u64,
        bytes_sent: u64,
        send_errors: u64,
        captured_at: SystemTime,
    ) -> PublicationMetricsSnapshot {
        PublicationMetricsSnapshot {
            send_calls,
            packets_sent,
            bytes_sent,
            send_errors,
            captured_at,
        }
    }

    #[test]
    fn context_delta_since_uses_lifetime_total_fields() {
        let earlier = ContextMetricsSnapshot {
            publications_added: 1,
            publications_removed: 0,
            active_publications: 1,
            total_send_calls: 10,
            total_packets_sent: 8,
            total_bytes_sent: 800,
            total_send_errors: 2,
            captured_at: SystemTime::UNIX_EPOCH,
        };
        let later = ContextMetricsSnapshot {
            publications_added: 2,
            publications_removed: 1,
            active_publications: 1,
            total_send_calls: 14,
            total_packets_sent: 11,
            total_bytes_sent: 1250,
            total_send_errors: 3,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
        };

        let delta = later.delta_since(&earlier).unwrap();

        assert_eq!(delta.interval_secs, 2.0);
        assert_eq!(delta.publications_added, 1);
        assert_eq!(delta.publications_removed, 1);
        assert_eq!(delta.send_calls, 4);
        assert_eq!(delta.packets_sent, 3);
        assert_eq!(delta.bytes_sent, 450);
        assert_eq!(delta.send_errors, 1);
        assert_eq!(delta.packets_per_sec(), 1.5);
        assert_eq!(delta.bytes_per_sec(), 225.0);
    }

    #[test]
    fn publication_delta_since_uses_interval_and_rates() {
        let earlier = publication_snapshot(4, 3, 300, 1, SystemTime::UNIX_EPOCH);
        let later = publication_snapshot(
            7,
            5,
            620,
            2,
            SystemTime::UNIX_EPOCH + Duration::from_secs(4),
        );

        let delta = later.delta_since(&earlier).unwrap();

        assert_eq!(delta.interval_secs, 4.0);
        assert_eq!(delta.send_calls, 3);
        assert_eq!(delta.packets_sent, 2);
        assert_eq!(delta.bytes_sent, 320);
        assert_eq!(delta.send_errors, 1);
        assert_eq!(delta.send_calls_per_sec(), 0.75);
        assert_eq!(delta.packets_per_sec(), 0.5);
        assert_eq!(delta.bytes_per_sec(), 80.0);
        assert_eq!(delta.send_errors_per_sec(), 0.25);
    }

    #[test]
    fn context_sampler_uses_monotonic_interval_when_wall_clock_moves_backwards() {
        let context = Context::new();
        let mut sampler = ContextMetricsSampler::new(&context);
        let sampled_at = Instant::now();
        let earlier = ContextMetricsSnapshot {
            publications_added: 1,
            publications_removed: 0,
            active_publications: 1,
            total_send_calls: 1,
            total_packets_sent: 1,
            total_bytes_sent: 10,
            total_send_errors: 0,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        };
        let later = ContextMetricsSnapshot {
            total_send_calls: 2,
            total_packets_sent: 2,
            total_bytes_sent: 20,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(5),
            ..earlier.clone()
        };

        assert!(sampler.sample_snapshot_at(earlier, sampled_at).is_none());
        let delta = sampler
            .sample_snapshot_at(later, sampled_at + Duration::from_secs(2))
            .unwrap();

        assert_eq!(delta.interval_secs, 2.0);
        assert_eq!(delta.packets_sent, 1);
        assert_eq!(delta.bytes_sent, 10);
    }

    #[test]
    fn context_sampler_uses_supplied_snapshot_timestamps() {
        let context = Context::new();
        let mut sampler = ContextMetricsSampler::new(&context);
        let earlier = ContextMetricsSnapshot {
            publications_added: 1,
            publications_removed: 0,
            active_publications: 1,
            total_send_calls: 1,
            total_packets_sent: 1,
            total_bytes_sent: 10,
            total_send_errors: 0,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(10),
        };
        let later = ContextMetricsSnapshot {
            total_send_calls: 2,
            total_packets_sent: 2,
            total_bytes_sent: 20,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(15),
            ..earlier.clone()
        };

        assert!(sampler.sample_snapshot(earlier).is_none());
        let delta = sampler.sample_snapshot(later).unwrap();

        assert_eq!(delta.interval_secs, 5.0);
        assert_eq!(delta.packets_sent, 1);
    }
}
