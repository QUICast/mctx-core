#[cfg(feature = "metrics")]
use crate::metrics::ContextMetricsSnapshot;
use crate::{MctxError, Publication, PublicationConfig, PublicationId, SendReport};
use socket2::Socket;
use std::net::UdpSocket;

/// Small owner for a set of multicast publication sockets.
#[derive(Debug)]
pub struct Context {
    publications: Vec<Publication>,
    next_id: u64,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    /// Creates an empty multicast sender context.
    pub fn new() -> Self {
        Self {
            publications: Vec::new(),
            next_id: 1,
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
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == &config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        let id = self.next_publication_id();
        let publication = Publication::new(id, config)?;
        self.publications.push(publication);
        Ok(id)
    }

    /// Stores an existing socket as a publication after configuring it.
    pub fn add_publication_with_socket(
        &mut self,
        config: PublicationConfig,
        socket: Socket,
    ) -> Result<PublicationId, MctxError> {
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == &config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        let id = self.next_publication_id();
        let publication = Publication::new_with_socket(id, config, socket)?;
        self.publications.push(publication);
        Ok(id)
    }

    /// Stores an existing standard-library UDP socket as a publication after configuring it.
    pub fn add_publication_with_udp_socket(
        &mut self,
        config: PublicationConfig,
        socket: UdpSocket,
    ) -> Result<PublicationId, MctxError> {
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == &config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        let id = self.next_publication_id();
        let publication = Publication::new_with_udp_socket(id, config, socket)?;
        self.publications.push(publication);
        Ok(id)
    }

    /// Removes one publication and drops its socket.
    pub fn remove_publication(&mut self, id: PublicationId) -> bool {
        let Some(index) = self
            .publications
            .iter()
            .position(|publication| publication.id() == id)
        else {
            return false;
        };

        self.publications.swap_remove(index);
        true
    }

    /// Extracts one publication from the context.
    pub fn take_publication(&mut self, id: PublicationId) -> Option<Publication> {
        let index = self
            .publications
            .iter()
            .position(|publication| publication.id() == id)?;

        Some(self.publications.swap_remove(index))
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

        publication.send(payload)
    }

    /// Sends the same payload through every publication and pushes reports into `out`.
    ///
    /// If one publication fails, reports already written into `out` are preserved.
    pub fn send_all(&self, payload: &[u8], out: &mut Vec<SendReport>) -> Result<usize, MctxError> {
        let before = out.len();

        for publication in &self.publications {
            out.push(publication.send(payload)?);
        }

        Ok(out.len() - before)
    }

    /// Returns a metrics snapshot aggregated across all publications.
    #[cfg(feature = "metrics")]
    pub fn metrics_snapshot(&self) -> ContextMetricsSnapshot {
        let mut snapshot = ContextMetricsSnapshot {
            publication_count: self.publications.len(),
            ..ContextMetricsSnapshot::default()
        };

        for publication in &self.publications {
            let publication_metrics = publication.metrics_snapshot();
            snapshot.send_calls += publication_metrics.send_calls;
            snapshot.packets_sent += publication_metrics.packets_sent;
            snapshot.bytes_sent += publication_metrics.bytes_sent;
            snapshot.send_errors += publication_metrics.send_errors;
        }

        snapshot
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
    use crate::test_support::{TEST_GROUP, recv_payload, test_multicast_receiver};
    use std::net::Ipv4Addr;

    #[test]
    fn context_send_reaches_a_local_receiver() {
        let (receiver, port) = test_multicast_receiver();
        let mut context = Context::new();
        let config = PublicationConfig::new(TEST_GROUP, port);
        let id = context.add_publication(config).unwrap();

        let report = context.send(id, b"context hello").unwrap();
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
        let sampler = ContextMetricsSampler::new(&context);

        context.send(id, b"metrics").unwrap();

        let delta = sampler.delta();
        assert_eq!(delta.publication_count_change, 0);
        assert_eq!(delta.send_calls, 1);
        assert_eq!(delta.packets_sent, 1);
        assert_eq!(delta.bytes_sent, b"metrics".len() as u64);
        assert_eq!(delta.send_errors, 0);
    }
}
