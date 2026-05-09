pub mod config;
pub mod context;
pub mod error;
#[cfg(feature = "metrics")]
pub mod metrics;
mod platform;
pub mod publication;
pub mod report;
#[cfg(test)]
mod test_support;
#[cfg(feature = "tokio")]
pub mod tokio_adapter;

pub use config::{
    Ipv6MulticastScope, OutgoingInterface, PublicationAddressFamily, PublicationConfig,
    is_ipv6_ssm_group,
};
pub use context::Context;
pub use error::MctxError;
#[cfg(feature = "metrics")]
pub use metrics::{
    ContextMetricsDelta, ContextMetricsSampler, ContextMetricsSnapshot, PublicationMetricsDelta,
    PublicationMetricsSampler, PublicationMetricsSnapshot,
};
pub use publication::{Publication, PublicationId, PublicationParts};
pub use report::SendReport;
#[cfg(feature = "tokio")]
pub use tokio_adapter::{TokioPublication, TokioSendError};
