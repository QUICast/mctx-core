use crate::{MctxError, Publication, SendReport};
use std::io;
#[cfg(not(unix))]
use std::time::Duration;
use thiserror::Error;

/// Errors returned by the Tokio adapter.
#[derive(Debug, Error)]
pub enum TokioSendError {
    /// Waiting for Tokio readiness failed.
    #[error("MCTX: tokio readiness failed: {0}")]
    Readiness(io::Error),

    /// The underlying multicast sender returned an error.
    #[error(transparent)]
    Send(#[from] MctxError),
}

/// Thin Tokio wrapper around an owned publication.
///
/// On Unix this uses `tokio::io::unix::AsyncFd` to wait for write readiness.
/// On other platforms it falls back to an async sleep-and-retry loop.
#[derive(Debug)]
pub struct TokioPublication {
    #[cfg(unix)]
    inner: tokio::io::unix::AsyncFd<Publication>,
    #[cfg(not(unix))]
    inner: Publication,
    #[cfg(not(unix))]
    poll_interval: Duration,
}

impl TokioPublication {
    /// Wraps an owned publication for use with Tokio.
    pub fn new(publication: Publication) -> io::Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self {
                inner: tokio::io::unix::AsyncFd::new(publication)?,
            })
        }

        #[cfg(not(unix))]
        {
            Ok(Self {
                inner: publication,
                poll_interval: Duration::from_millis(10),
            })
        }
    }

    /// Returns a shared reference to the wrapped publication.
    pub fn publication(&self) -> &Publication {
        #[cfg(unix)]
        {
            self.inner.get_ref()
        }

        #[cfg(not(unix))]
        {
            &self.inner
        }
    }

    /// Consumes the adapter and returns the wrapped publication.
    pub fn into_publication(self) -> Publication {
        #[cfg(unix)]
        {
            self.inner.into_inner()
        }

        #[cfg(not(unix))]
        {
            self.inner
        }
    }

    /// Overrides the async poll interval used on platforms without `AsyncFd`.
    #[cfg(not(unix))]
    pub fn with_poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Waits for socket readiness and sends one payload.
    pub async fn send(&self, payload: &[u8]) -> Result<SendReport, TokioSendError> {
        #[cfg(unix)]
        {
            loop {
                let mut readiness = self
                    .inner
                    .writable()
                    .await
                    .map_err(TokioSendError::Readiness)?;

                match self.inner.get_ref().send(payload) {
                    Ok(report) => return Ok(report),
                    Err(error) if error.is_would_block() => readiness.clear_ready(),
                    Err(error) => return Err(TokioSendError::Send(error)),
                }
            }
        }

        #[cfg(not(unix))]
        {
            loop {
                match self.inner.send(payload) {
                    Ok(report) => return Ok(report),
                    Err(error) if error.is_would_block() => {
                        tokio::time::sleep(self.poll_interval).await
                    }
                    Err(error) => return Err(TokioSendError::Send(error)),
                }
            }
        }
    }
}

#[cfg(all(test, feature = "tokio"))]
mod tests {
    use super::*;
    use crate::test_support::{TEST_GROUP, recv_payload, test_multicast_receiver};
    use crate::{Context, PublicationConfig};

    #[tokio::test]
    async fn tokio_publication_sends_a_packet() {
        let (receiver, port) = test_multicast_receiver();
        let mut context = Context::new();
        let id = context
            .add_publication(PublicationConfig::new(TEST_GROUP, port))
            .unwrap();

        let publication = context.take_publication(id).unwrap();
        let publication = TokioPublication::new(publication).unwrap();

        publication.send(b"tokio hello").await.unwrap();
        let payload = recv_payload(&receiver);

        assert_eq!(payload, b"tokio hello");
    }
}
