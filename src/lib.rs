pub mod config;
pub mod context;
pub mod error;
#[cfg(feature = "metrics")]
pub mod jsonl;
#[cfg(feature = "metrics")]
pub mod metrics;
mod platform;
pub mod publication;
#[cfg(feature = "raw-packets")]
pub mod raw;
#[cfg(feature = "raw-ip")]
pub mod raw_ip;
pub mod report;
#[cfg(any(
    all(
        feature = "raw-ip",
        any(target_os = "linux", target_os = "macos", windows)
    ),
    all(feature = "raw-packets", any(target_os = "macos", windows))
))]
mod socket_cache;
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
#[cfg(feature = "raw-packets")]
pub use raw::{
    RawContext, RawIpv6EgressCapabilities, RawIpv6EgressCapability, RawPublication,
    RawPublicationConfig, RawPublicationId, RawSendReport, RawValidationMode,
    raw_ipv6_egress_capabilities,
};
#[cfg(feature = "raw-route-egress")]
pub use raw::{
    RawEgressMode, RawRouteEgressCapabilities, RawRouteEgressCapability,
    raw_route_egress_capabilities,
};
#[cfg(feature = "raw-ip")]
pub use raw_ip::{
    RawIpCapabilities, RawIpCapability, RawIpContext, RawIpPublication, RawIpPublicationConfig,
    RawIpPublicationId, RawIpSendReport, RawIpSocketConfig, raw_ip_capabilities,
};
pub use report::SendReport;
#[cfg(feature = "tokio")]
pub use tokio_adapter::{TokioPublication, TokioSendError};
