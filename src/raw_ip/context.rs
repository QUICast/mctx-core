use crate::error::MctxError;
use crate::raw_ip::{RawIpPublication, RawIpPublicationId, RawIpSendReport, RawIpSocketConfig};
use std::collections::HashMap;

/// Owns ready-to-send generic raw-IP publications.
#[derive(Debug, Default)]
pub struct RawIpContext {
    publications: Vec<RawIpPublication>,
    publication_indices: HashMap<RawIpPublicationId, usize>,
    next_publication_id: u64,
}

impl RawIpContext {
    /// Creates an empty raw-IP context.
    pub fn new() -> Self {
        Self {
            publications: Vec::new(),
            publication_indices: HashMap::new(),
            next_publication_id: 1,
        }
    }

    /// Adds one configured raw-IP publication.
    pub fn add_publication(
        &mut self,
        config: RawIpSocketConfig,
    ) -> Result<RawIpPublicationId, MctxError> {
        if self
            .publications
            .iter()
            .any(|publication| publication.config() == &config)
        {
            return Err(MctxError::DuplicatePublication);
        }

        let id = RawIpPublicationId(self.next_publication_id);
        self.next_publication_id += 1;
        let publication = RawIpPublication::new(id, config)?;
        let index = self.publications.len();
        self.publications.push(publication);
        self.publication_indices.insert(id, index);
        Ok(id)
    }

    /// Returns the configured publication, if present.
    pub fn get_publication(&self, id: RawIpPublicationId) -> Option<&RawIpPublication> {
        self.publication_indices
            .get(&id)
            .and_then(|index| self.publications.get(*index))
    }

    /// Removes one publication and closes its raw sockets.
    pub fn remove_publication(&mut self, id: RawIpPublicationId) -> bool {
        let Some(index) = self.publication_indices.remove(&id) else {
            return false;
        };

        self.publications.swap_remove(index);
        if index < self.publications.len() {
            let moved_id = self.publications[index].id();
            self.publication_indices.insert(moved_id, index);
        }
        true
    }

    /// Transmits one complete caller-provided IPv4 or IPv6 datagram.
    pub fn send_ip_datagram(
        &self,
        id: RawIpPublicationId,
        ip_datagram: &[u8],
    ) -> Result<RawIpSendReport, MctxError> {
        self.get_publication(id)
            .ok_or(MctxError::PublicationNotFound)?
            .send_ip_datagram(ip_datagram)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_config_fails_before_privileged_socket_setup() {
        let mut context = RawIpContext::new();

        assert!(matches!(
            context.add_publication(RawIpSocketConfig::ipv4()),
            Err(MctxError::RawInterfaceRequired)
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unix_ipv6_requires_a_bound_source_before_socket_setup() {
        let mut context = RawIpContext::new();

        assert!(matches!(
            context.add_publication(RawIpSocketConfig::ipv6().with_interface_index(1)),
            Err(MctxError::RawPacketTransmitUnsupported(_))
        ));
    }
}
