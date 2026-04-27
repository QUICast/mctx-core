use crate::{Context, Publication};

/// Publication metrics at one point in time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublicationMetricsSnapshot {
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

/// Publication metrics change between two snapshots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublicationMetricsDelta {
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

impl PublicationMetricsDelta {
    pub fn between(before: PublicationMetricsSnapshot, after: PublicationMetricsSnapshot) -> Self {
        Self {
            send_calls: after.send_calls.saturating_sub(before.send_calls),
            packets_sent: after.packets_sent.saturating_sub(before.packets_sent),
            bytes_sent: after.bytes_sent.saturating_sub(before.bytes_sent),
            send_errors: after.send_errors.saturating_sub(before.send_errors),
        }
    }
}

/// Convenience sampler for one publication.
#[derive(Debug)]
pub struct PublicationMetricsSampler<'a> {
    publication: &'a Publication,
    baseline: PublicationMetricsSnapshot,
}

impl<'a> PublicationMetricsSampler<'a> {
    pub fn new(publication: &'a Publication) -> Self {
        Self {
            publication,
            baseline: publication.metrics_snapshot(),
        }
    }

    pub fn snapshot(&self) -> PublicationMetricsSnapshot {
        self.publication.metrics_snapshot()
    }

    pub fn delta(&self) -> PublicationMetricsDelta {
        PublicationMetricsDelta::between(self.baseline, self.snapshot())
    }
}

/// Context metrics at one point in time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextMetricsSnapshot {
    pub publication_count: usize,
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

/// Context metrics change between two snapshots.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContextMetricsDelta {
    pub publication_count_change: i64,
    pub send_calls: u64,
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub send_errors: u64,
}

impl ContextMetricsDelta {
    pub fn between(before: ContextMetricsSnapshot, after: ContextMetricsSnapshot) -> Self {
        Self {
            publication_count_change: after.publication_count as i64
                - before.publication_count as i64,
            send_calls: after.send_calls.saturating_sub(before.send_calls),
            packets_sent: after.packets_sent.saturating_sub(before.packets_sent),
            bytes_sent: after.bytes_sent.saturating_sub(before.bytes_sent),
            send_errors: after.send_errors.saturating_sub(before.send_errors),
        }
    }
}

/// Convenience sampler for one context.
#[derive(Debug)]
pub struct ContextMetricsSampler<'a> {
    context: &'a Context,
    baseline: ContextMetricsSnapshot,
}

impl<'a> ContextMetricsSampler<'a> {
    pub fn new(context: &'a Context) -> Self {
        Self {
            context,
            baseline: context.metrics_snapshot(),
        }
    }

    pub fn snapshot(&self) -> ContextMetricsSnapshot {
        self.context.metrics_snapshot()
    }

    pub fn delta(&self) -> ContextMetricsDelta {
        ContextMetricsDelta::between(self.baseline, self.snapshot())
    }
}
