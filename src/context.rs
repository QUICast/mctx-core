#[cfg(feature = "metrics")]
use crate::metrics::{ContextMetricsSnapshot, MetricsSequence};
use crate::{MctxError, Publication, PublicationConfig, PublicationId, SendReport};
use socket2::Socket;
use std::net::UdpSocket;
#[cfg(feature = "metrics")]
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
#[cfg(feature = "metrics")]
use std::time::SystemTime;

#[cfg(feature = "metrics")]
#[derive(Debug, Default)]
struct ContextMetricsInner {
    sequence: MetricsSequence,
    publications_added: AtomicU64,
    publications_removed: AtomicU64,
    total_send_calls: AtomicU64,
    total_packets_sent: AtomicU64,
    total_bytes_sent: AtomicU64,
    total_send_errors: AtomicU64,
}

/// Small owner for a set of multicast publication sockets.
#[derive(Debug)]
pub struct Context {
    publications: Vec<Publication>,
    next_id: u64,
    #[cfg(feature = "metrics")]
    metrics: ContextMetricsInner,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    #[cfg(feature = "metrics")]
    fn record_send_success(&self, bytes_sent: usize) {
        let _update = self.metrics.sequence.write();
        self.metrics.total_send_calls.fetch_add(1, Relaxed);
        self.metrics.total_packets_sent.fetch_add(1, Relaxed);
        self.metrics
            .total_bytes_sent
            .fetch_add(bytes_sent as u64, Relaxed);
    }

    #[cfg(feature = "metrics")]
    fn record_send_error(&self) {
        let _update = self.metrics.sequence.write();
        self.metrics.total_send_calls.fetch_add(1, Relaxed);
        self.metrics.total_send_errors.fetch_add(1, Relaxed);
    }

    fn ensure_publication_config_is_unique(
        &self,
        config: &PublicationConfig,
    ) -> Result<(), MctxError> {
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        Ok(())
    }

    fn insert_publication(&mut self, publication: Publication) -> PublicationId {
        let id = publication.id();
        self.publications.push(publication);

        #[cfg(feature = "metrics")]
        {
            let _update = self.metrics.sequence.write();
            self.metrics.publications_added.fetch_add(1, Relaxed);
        }

        id
    }

    fn finish_publication_removal(&mut self, index: usize) -> Publication {
        let publication = self.publications.swap_remove(index);

        #[cfg(feature = "metrics")]
        {
            let _update = self.metrics.sequence.write();
            self.metrics.publications_removed.fetch_add(1, Relaxed);
        }

        publication
    }

    /// Creates an empty multicast sender context.
    pub fn new() -> Self {
        Self {
            publications: Vec::new(),
            next_id: 1,
            #[cfg(feature = "metrics")]
            metrics: ContextMetricsInner::default(),
        }
    }

    /// Returns the number of tracked publications.
    pub fn publication_count(&self) -> usize {
        self.publications.len()
    }

    /// Returns whether a publication ID exists in the context.
    pub fn contains_publication(&self, id: PublicationId) -> bool {
        self.publications
            .iter()
            .any(|publication| publication.id() == id)
    }

    /// Returns an immutable reference to one publication.
    pub fn get_publication(&self, id: PublicationId) -> Option<&Publication> {
        self.publications
            .iter()
            .find(|publication| publication.id() == id)
    }

    /// Returns a mutable reference to one publication.
    pub fn get_publication_mut(&mut self, id: PublicationId) -> Option<&mut Publication> {
        self.publications
            .iter_mut()
            .find(|publication| publication.id() == id)
    }

    /// Creates a new publication socket from configuration and stores it.
    pub fn add_publication(
        &mut self,
        config: PublicationConfig,
    ) -> Result<PublicationId, MctxError> {
        self.ensure_publication_config_is_unique(&config)?;

        let id = self.next_publication_id();
        let publication = Publication::new(id, config)?;
        Ok(self.insert_publication(publication))
    }

    /// Stores an existing socket as a publication after configuring it.
    pub fn add_publication_with_socket(
        &mut self,
        config: PublicationConfig,
        socket: Socket,
    ) -> Result<PublicationId, MctxError> {
        self.ensure_publication_config_is_unique(&config)?;

        let id = self.next_publication_id();
        let publication = Publication::new_with_socket(id, config, socket)?;
        Ok(self.insert_publication(publication))
    }

    /// Stores an existing standard-library UDP socket as a publication after configuring it.
    pub fn add_publication_with_udp_socket(
        &mut self,
        config: PublicationConfig,
        socket: UdpSocket,
    ) -> Result<PublicationId, MctxError> {
        self.ensure_publication_config_is_unique(&config)?;

        let id = self.next_publication_id();
        let publication = Publication::new_with_udp_socket(id, config, socket)?;
        Ok(self.insert_publication(publication))
    }

    /// Removes one publication and drops its socket.
    pub fn remove_publication(&mut self, id: PublicationId) -> bool {
        self.take_publication(id).is_some()
    }

    /// Extracts one publication from the context.
    pub fn take_publication(&mut self, id: PublicationId) -> Option<Publication> {
        let index = self
            .publications
            .iter()
            .position(|publication| publication.id() == id)?;

        Some(self.finish_publication_removal(index))
    }

    /// Returns all tracked publications.
    pub fn publications(&self) -> &[Publication] {
        &self.publications
    }

    /// Returns all tracked publications mutably.
    pub fn publications_mut(&mut self) -> &mut [Publication] {
        &mut self.publications
    }

    /// Sends one payload through the selected publication.
    pub fn send(&self, id: PublicationId, payload: &[u8]) -> Result<SendReport, MctxError> {
        let publication = self
            .get_publication(id)
            .ok_or(MctxError::PublicationNotFound)?;

        match publication.send(payload) {
            Ok(report) => {
                #[cfg(feature = "metrics")]
                self.record_send_success(report.bytes_sent);

                Ok(report)
            }
            Err(error) => {
                #[cfg(feature = "metrics")]
                self.record_send_error();

                Err(error)
            }
        }
    }

    /// Sends the same payload through every publication and pushes reports into `out`.
    ///
    /// If one publication fails, reports already written into `out` are preserved.
    pub fn send_all(&self, payload: &[u8], out: &mut Vec<SendReport>) -> Result<usize, MctxError> {
        let before = out.len();
        out.reserve(self.publications.len());

        for publication in &self.publications {
            match publication.send(payload) {
                Ok(report) => {
                    #[cfg(feature = "metrics")]
                    self.record_send_success(report.bytes_sent);

                    out.push(report);
                }
                Err(error) => {
                    #[cfg(feature = "metrics")]
                    self.record_send_error();

                    return Err(error);
                }
            }
        }

        Ok(out.len() - before)
    }

    /// Returns a snapshot of the context's current metrics.
    ///
    /// Counter fields such as `total_packets_sent` are cumulative for the
    /// lifetime of the context for send activity issued through `Context`
    /// methods. They are not recomputed from the currently active publications,
    /// and they do not decrease when a publication is removed.
    #[cfg(feature = "metrics")]
    pub fn metrics_snapshot(&self) -> ContextMetricsSnapshot {
        let (
            publications_added,
            publications_removed,
            total_send_calls,
            total_packets_sent,
            total_bytes_sent,
            total_send_errors,
        ) = self.metrics.sequence.read_consistent(|| {
            (
                self.metrics.publications_added.load(Relaxed),
                self.metrics.publications_removed.load(Relaxed),
                self.metrics.total_send_calls.load(Relaxed),
                self.metrics.total_packets_sent.load(Relaxed),
                self.metrics.total_bytes_sent.load(Relaxed),
                self.metrics.total_send_errors.load(Relaxed),
            )
        });

        ContextMetricsSnapshot {
            publications_added,
            publications_removed,
            active_publications: self.publications.len(),
            total_send_calls,
            total_packets_sent,
            total_bytes_sent,
            total_send_errors,
            captured_at: SystemTime::now(),
        }
    }

    fn next_publication_id(&mut self) -> PublicationId {
        let id = PublicationId(self.next_id);
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "metrics")]
    use crate::metrics::ContextMetricsSampler;
    use crate::test_support::{
        TEST_GROUP, multicast_test_result_or_skip, recv_payload, test_multicast_receiver,
    };
    use std::net::Ipv4Addr;

    #[test]
    fn context_send_reaches_a_local_receiver() {
        let (receiver, port) = test_multicast_receiver();
        let mut context = Context::new();
        let config = PublicationConfig::new(TEST_GROUP, port);
        let id = context.add_publication(config).unwrap();

        let Some(report) = multicast_test_result_or_skip(context.send(id, b"context hello")) else {
            return;
        };
        let payload = recv_payload(&receiver);

        assert_eq!(report.bytes_sent, b"context hello".len());
        assert_eq!(payload, b"context hello");
    }

    #[test]
    fn duplicate_publications_are_rejected() {
        let mut context = Context::new();
        let config = PublicationConfig::new(Ipv4Addr::new(239, 1, 2, 3), 5000);

        context.add_publication(config.clone()).unwrap();
        let result = context.add_publication(config);

        assert!(matches!(result, Err(MctxError::DuplicatePublication)));
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn context_metrics_track_successful_sends() {
        let (_receiver, port) = test_multicast_receiver();
        let mut context = Context::new();
        let id = context
            .add_publication(PublicationConfig::new(TEST_GROUP, port))
            .unwrap();
        let mut sampler = ContextMetricsSampler::new(&context);

        assert!(sampler.sample().is_none());
        if multicast_test_result_or_skip(context.send(id, b"metrics")).is_none() {
            return;
        }

        let snapshot = context.metrics_snapshot();
        let delta = sampler.sample().unwrap();

        assert_eq!(snapshot.publications_added, 1);
        assert_eq!(snapshot.publications_removed, 0);
        assert_eq!(snapshot.active_publications, 1);
        assert_eq!(snapshot.total_send_calls, 1);
        assert_eq!(snapshot.total_packets_sent, 1);
        assert_eq!(snapshot.total_bytes_sent, b"metrics".len() as u64);
        assert_eq!(snapshot.total_send_errors, 0);
        assert_eq!(delta.publications_added, 0);
        assert_eq!(delta.publications_removed, 0);
        assert_eq!(delta.send_calls, 1);
        assert_eq!(delta.packets_sent, 1);
        assert_eq!(delta.bytes_sent, b"metrics".len() as u64);
        assert_eq!(delta.send_errors, 0);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn context_metrics_totals_survive_publication_removal() {
        let (_receiver, port) = test_multicast_receiver();
        let mut context = Context::new();
        let id = context
            .add_publication(PublicationConfig::new(TEST_GROUP, port))
            .unwrap();

        if multicast_test_result_or_skip(context.send(id, b"lifetime")).is_none() {
            return;
        }
        let before_removal = context.metrics_snapshot();
        assert!(context.remove_publication(id));

        let after_removal = context.metrics_snapshot();

        assert_eq!(before_removal.total_packets_sent, 1);
        assert_eq!(before_removal.total_bytes_sent, b"lifetime".len() as u64);
        assert_eq!(after_removal.total_packets_sent, 1);
        assert_eq!(after_removal.total_bytes_sent, b"lifetime".len() as u64);
        assert_eq!(after_removal.active_publications, 0);
        assert_eq!(after_removal.publications_removed, 1);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn context_is_sync_with_metrics_enabled() {
        fn assert_sync<T: Sync>() {}

        assert_sync::<Context>();
    }
}
