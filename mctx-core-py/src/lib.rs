use mctx_core::{
    Context, MctxError, OutgoingInterface, PublicationAddressFamily, PublicationConfig,
    PublicationId, SendReport,
};
use pyo3::exceptions::{
    PyBlockingIOError, PyLookupError, PyNotImplementedError, PyOSError, PyRuntimeError,
    PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::cell::RefCell;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

#[derive(Debug, Clone)]
struct SharedContext {
    inner: Rc<RefCell<Context>>,
}

impl SharedContext {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Context::new())),
        }
    }
}

fn invalid_argument(message: impl Into<String>) -> PyErr {
    PyValueError::new_err(message.into())
}

fn borrow_error(kind: &'static str) -> PyErr {
    PyRuntimeError::new_err(format!(
        "mctx_core {kind} is already borrowed by another operation"
    ))
}

fn parse_ip_addr(raw: &str, field: &'static str) -> PyResult<IpAddr> {
    raw.parse()
        .map_err(|_| invalid_argument(format!("invalid {field} IP address: {raw}")))
}

fn parse_optional_ip_addr(raw: Option<&str>, field: &'static str) -> PyResult<Option<IpAddr>> {
    raw.map(|value| parse_ip_addr(value, field)).transpose()
}

fn parse_socket_addr(raw: &str, field: &'static str) -> PyResult<SocketAddr> {
    raw.parse()
        .map_err(|_| invalid_argument(format!("invalid {field} socket address: {raw}")))
}

fn publication_not_found(publication_id: u64) -> PyErr {
    PyLookupError::new_err(format!("mctx_core publication {publication_id} not found"))
}

#[allow(clippy::too_many_arguments)]
fn build_publication_config(
    group: &str,
    dst_port: u16,
    source: Option<&str>,
    source_port: Option<u16>,
    bind: Option<&str>,
    interface: Option<&str>,
    interface_index: Option<u32>,
    ttl: u32,
    loopback: bool,
) -> PyResult<PublicationConfig> {
    if bind.is_some() && (source.is_some() || source_port.is_some()) {
        return Err(invalid_argument(
            "`bind` cannot be combined with `source` or `source_port`",
        ));
    }

    if interface.is_some() && interface_index.is_some() {
        return Err(invalid_argument(
            "`interface` and `interface_index` are mutually exclusive",
        ));
    }

    let group = parse_ip_addr(group, "group")?;
    let source_addr = parse_optional_ip_addr(source, "source")?;
    let interface_addr = parse_optional_ip_addr(interface, "interface")?;

    let mut config = PublicationConfig::new(group, dst_port)
        .with_ttl(ttl)
        .with_loopback(loopback);

    if let Some(bind_addr) = bind {
        config = config.with_bind_addr(parse_socket_addr(bind_addr, "bind")?);
    } else {
        if let Some(source_addr) = source_addr {
            config = config.with_source_addr(source_addr);
        }

        if let Some(source_port) = source_port {
            config = config.with_source_port(source_port);
        }
    }

    if let Some(interface_addr) = interface_addr {
        config = match interface_addr {
            IpAddr::V4(interface) => config.with_outgoing_interface(interface),
            IpAddr::V6(interface) => config.with_outgoing_interface(interface),
        };
    }

    if let Some(interface_index) = interface_index {
        config = config.with_ipv6_interface_index(interface_index);
    }

    Ok(config)
}

fn addr_to_tuple(addr: SocketAddr) -> (String, u16) {
    (addr.ip().to_string(), addr.port())
}

fn opt_addr_to_tuple(addr: Option<SocketAddr>) -> Option<(String, u16)> {
    addr.map(addr_to_tuple)
}

fn family_name(family: PublicationAddressFamily) -> &'static str {
    match family {
        PublicationAddressFamily::Ipv4 => "ipv4",
        PublicationAddressFamily::Ipv6 => "ipv6",
    }
}

fn mctx_error_to_py(err: MctxError) -> PyErr {
    match err {
        MctxError::InvalidDestinationPort
        | MctxError::InvalidMulticastGroup
        | MctxError::InvalidSourcePort
        | MctxError::InvalidSourceAddress
        | MctxError::InvalidInterfaceAddress
        | MctxError::InvalidIpv6InterfaceIndex
        | MctxError::InvalidRawBindAddress
        | MctxError::SourceAddressFamilyMismatch
        | MctxError::OutgoingInterfaceFamilyMismatch
        | MctxError::RawBindAddressFamilyMismatch
        | MctxError::Ipv6SourceInterfaceMismatch { .. }
        | MctxError::Ipv6ScopedMulticastRequiresInterface
        | MctxError::ExistingSocketAddressFamilyMismatch
        | MctxError::ExistingSocketPortMismatch { .. }
        | MctxError::ExistingSocketAddressMismatch { .. }
        | MctxError::DuplicatePublication
        | MctxError::InvalidRawIpDatagram
        | MctxError::InvalidRawMulticastDestination
        | MctxError::RawDatagramSourceMismatch { .. }
        | MctxError::RawInterfaceRequired
        | MctxError::RawUnsupportedLinkType(_) => PyValueError::new_err(err.to_string()),
        MctxError::PublicationNotFound => PyLookupError::new_err(err.to_string()),
        MctxError::InterfaceDiscoveryFailed(_) => PyRuntimeError::new_err(err.to_string()),
        MctxError::SendFailed(io_err) | MctxError::RawSendFailed(io_err)
            if io_err.kind() == std::io::ErrorKind::WouldBlock =>
        {
            PyBlockingIOError::new_err(io_err.to_string())
        }
        MctxError::RawPacketTransmitUnsupported(_) => {
            PyNotImplementedError::new_err(err.to_string())
        }
        MctxError::SocketCreateFailed(io_err)
        | MctxError::SocketOptionFailed(io_err)
        | MctxError::SocketBindFailed(io_err)
        | MctxError::SocketConnectFailed(io_err)
        | MctxError::SocketLocalAddrFailed(io_err)
        | MctxError::SendFailed(io_err)
        | MctxError::RawSocketCreateFailed(io_err)
        | MctxError::RawSocketBindFailed(io_err)
        | MctxError::RawSendFailed(io_err) => PyOSError::new_err(io_err.to_string()),
    }
}

fn with_context<T>(shared: &SharedContext, f: impl FnOnce(&Context) -> PyResult<T>) -> PyResult<T> {
    let context = shared
        .inner
        .try_borrow()
        .map_err(|_| borrow_error("context"))?;
    f(&context)
}

fn with_context_mut<T>(
    shared: &SharedContext,
    f: impl FnOnce(&mut Context) -> PyResult<T>,
) -> PyResult<T> {
    let mut context = shared
        .inner
        .try_borrow_mut()
        .map_err(|_| borrow_error("context"))?;
    f(&mut context)
}

fn with_publication<T>(
    shared: &SharedContext,
    id: PublicationId,
    f: impl FnOnce(&mctx_core::Publication) -> PyResult<T>,
) -> PyResult<T> {
    with_context(shared, |context| {
        let publication = context
            .get_publication(id)
            .ok_or_else(|| publication_not_found(id.0))?;
        f(publication)
    })
}

fn send_report_to_py(py: Python<'_>, report: SendReport) -> PyResult<Py<PySendReport>> {
    Py::new(py, PySendReport::from(report))
}

#[pyclass(
    module = "mctx_core._mctx_core",
    name = "Context",
    unsendable,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyContext {
    shared: SharedContext,
}

#[pymethods]
impl PyContext {
    #[new]
    fn new() -> Self {
        Self {
            shared: SharedContext::new(),
        }
    }

    fn publication_count(&self) -> PyResult<usize> {
        with_context(&self.shared, |context| Ok(context.publication_count()))
    }

    #[pyo3(signature = (group, dst_port, source=None, source_port=None, bind=None, interface=None, interface_index=None, ttl=1, loopback=true))]
    #[allow(clippy::too_many_arguments)]
    fn add_publication(
        &self,
        py: Python<'_>,
        group: &str,
        dst_port: u16,
        source: Option<&str>,
        source_port: Option<u16>,
        bind: Option<&str>,
        interface: Option<&str>,
        interface_index: Option<u32>,
        ttl: u32,
        loopback: bool,
    ) -> PyResult<Py<PyPublication>> {
        let config = build_publication_config(
            group,
            dst_port,
            source,
            source_port,
            bind,
            interface,
            interface_index,
            ttl,
            loopback,
        )?;

        let id = with_context_mut(&self.shared, |context| {
            context.add_publication(config).map_err(mctx_error_to_py)
        })?;

        Py::new(
            py,
            PyPublication {
                shared: self.shared.clone(),
                id,
            },
        )
    }

    fn get_publication(&self, py: Python<'_>, publication_id: u64) -> PyResult<Py<PyPublication>> {
        let id = PublicationId(publication_id);
        with_context(&self.shared, |context| {
            if context.contains_publication(id) {
                Ok(())
            } else {
                Err(publication_not_found(publication_id))
            }
        })?;

        Py::new(
            py,
            PyPublication {
                shared: self.shared.clone(),
                id,
            },
        )
    }

    fn remove_publication(&self, publication_id: u64) -> PyResult<bool> {
        with_context_mut(&self.shared, |context| {
            Ok(context.remove_publication(PublicationId(publication_id)))
        })
    }

    fn send(
        &self,
        py: Python<'_>,
        publication_id: u64,
        payload: &[u8],
    ) -> PyResult<Py<PySendReport>> {
        let report = with_context(&self.shared, |context| {
            context
                .send(PublicationId(publication_id), payload)
                .map_err(mctx_error_to_py)
        })?;

        send_report_to_py(py, report)
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Context(publication_count={})",
            self.publication_count()?
        ))
    }
}

#[pyclass(
    module = "mctx_core._mctx_core",
    name = "Publication",
    unsendable,
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PyPublication {
    shared: SharedContext,
    id: PublicationId,
}

#[pymethods]
impl PyPublication {
    #[getter]
    fn id(&self) -> u64 {
        self.id.0
    }

    #[getter]
    fn group(&self) -> PyResult<String> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().group.to_string())
        })
    }

    #[getter]
    fn dst_port(&self) -> PyResult<u16> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().dst_port)
        })
    }

    #[getter]
    fn family(&self) -> PyResult<&'static str> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(family_name(publication.config().family()))
        })
    }

    #[getter]
    fn configured_source_addr(&self) -> PyResult<Option<String>> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().source_addr.map(|ip| ip.to_string()))
        })
    }

    #[getter]
    fn source_port(&self) -> PyResult<Option<u16>> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().source_port)
        })
    }

    #[getter]
    fn outgoing_interface(&self) -> PyResult<Option<String>> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(match publication.config().outgoing_interface {
                Some(OutgoingInterface::Ipv4Addr(interface)) => Some(interface.to_string()),
                Some(OutgoingInterface::Ipv6Addr(interface)) => Some(interface.to_string()),
                Some(OutgoingInterface::Ipv6Index(_)) | None => None,
            })
        })
    }

    #[getter]
    fn outgoing_interface_index(&self) -> PyResult<Option<u32>> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(match publication.config().outgoing_interface {
                Some(OutgoingInterface::Ipv6Index(index)) => Some(index),
                Some(OutgoingInterface::Ipv4Addr(_))
                | Some(OutgoingInterface::Ipv6Addr(_))
                | None => None,
            })
        })
    }

    #[getter]
    fn ttl(&self) -> PyResult<u32> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().ttl)
        })
    }

    #[getter]
    fn loopback(&self) -> PyResult<bool> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.config().loopback)
        })
    }

    #[getter]
    fn destination(&self) -> PyResult<(String, u16)> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(addr_to_tuple(publication.destination()))
        })
    }

    fn local_addr(&self) -> PyResult<(String, u16)> {
        with_publication(&self.shared, self.id, |publication| {
            publication
                .local_addr()
                .map(addr_to_tuple)
                .map_err(mctx_error_to_py)
        })
    }

    fn source_addr(&self) -> PyResult<String> {
        with_publication(&self.shared, self.id, |publication| {
            publication
                .source_addr()
                .map(|ip| ip.to_string())
                .map_err(mctx_error_to_py)
        })
    }

    fn announce_tuple(&self) -> PyResult<(String, String, u16)> {
        with_publication(&self.shared, self.id, |publication| {
            publication
                .announce_tuple()
                .map(|(source, group, port)| (source.to_string(), group.to_string(), port))
                .map_err(mctx_error_to_py)
        })
    }

    fn send(&self, py: Python<'_>, payload: &[u8]) -> PyResult<Py<PySendReport>> {
        let report = with_publication(&self.shared, self.id, |publication| {
            publication.send(payload).map_err(mctx_error_to_py)
        })?;

        send_report_to_py(py, report)
    }

    fn remove(&self) -> PyResult<bool> {
        with_context_mut(&self.shared, |context| {
            Ok(context.remove_publication(self.id))
        })
    }

    #[cfg(unix)]
    fn fileno(&self) -> PyResult<i32> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.as_raw_fd())
        })
    }

    #[cfg(windows)]
    fn socket_handle(&self) -> PyResult<usize> {
        with_publication(&self.shared, self.id, |publication| {
            Ok(publication.as_raw_socket() as usize)
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        Ok(format!(
            "Publication(id={}, group={:?}, port={}, family={:?})",
            self.id(),
            self.group()?,
            self.dst_port()?,
            self.family()?,
        ))
    }
}

#[pyclass(
    module = "mctx_core._mctx_core",
    name = "SendReport",
    skip_from_py_object
)]
#[derive(Debug, Clone)]
struct PySendReport {
    publication_id: u64,
    destination_addr: String,
    destination_port: u16,
    local_addr: Option<(String, u16)>,
    source_addr: Option<String>,
    bytes_sent: usize,
}

impl From<SendReport> for PySendReport {
    fn from(report: SendReport) -> Self {
        Self {
            publication_id: report.publication_id.0,
            destination_addr: report.destination.ip().to_string(),
            destination_port: report.destination.port(),
            local_addr: opt_addr_to_tuple(report.local_addr),
            source_addr: report.source_addr.map(|ip| ip.to_string()),
            bytes_sent: report.bytes_sent,
        }
    }
}

#[pymethods]
impl PySendReport {
    #[getter]
    fn publication_id(&self) -> u64 {
        self.publication_id
    }

    #[getter]
    fn destination(&self) -> (String, u16) {
        (self.destination_addr.clone(), self.destination_port)
    }

    #[getter]
    fn destination_addr(&self) -> &str {
        &self.destination_addr
    }

    #[getter]
    fn destination_port(&self) -> u16 {
        self.destination_port
    }

    #[getter]
    fn local_addr(&self) -> Option<(String, u16)> {
        self.local_addr.clone()
    }

    #[getter]
    fn source_addr(&self) -> Option<&str> {
        self.source_addr.as_deref()
    }

    #[getter]
    fn bytes_sent(&self) -> usize {
        self.bytes_sent
    }

    fn __repr__(&self) -> String {
        format!(
            "SendReport(publication_id={}, destination=({}, {}), local_addr={:?}, source_addr={:?}, bytes_sent={})",
            self.publication_id,
            self.destination_addr,
            self.destination_port,
            self.local_addr,
            self.source_addr,
            self.bytes_sent,
        )
    }
}

#[pymodule]
fn _mctx_core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyContext>()?;
    module.add_class::<PyPublication>()?;
    module.add_class::<PySendReport>()?;
    Ok(())
}
