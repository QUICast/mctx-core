use mctx_core::PublicationConfig;
use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SendCliArgs {
    pub group: IpAddr,
    pub dst_port: u16,
    pub payload: String,
    pub count: u64,
    pub interval_ms: u64,
    pub source: Option<IpAddr>,
    pub bind_addr: Option<SocketAddr>,
    pub source_port: Option<u16>,
    pub interface: Option<IpAddr>,
    pub interface_index: Option<u32>,
    pub ttl: Option<u32>,
    pub loopback: bool,
}

impl SendCliArgs {
    pub(crate) fn build_config(&self) -> Result<PublicationConfig, String> {
        let mut config = PublicationConfig::new(self.group, self.dst_port);

        if let Some(bind_addr) = self.bind_addr {
            config = config.with_bind_addr(bind_addr);
        } else {
            if let Some(source) = self.source {
                config = config.with_source_addr(source);
            }

            if let Some(source_port) = self.source_port {
                config = config.with_source_port(source_port);
            }
        }

        if let Some(interface) = self.interface {
            config = match interface {
                IpAddr::V4(interface) => config.with_outgoing_interface(interface),
                IpAddr::V6(interface) => config.with_outgoing_interface(interface),
            };
        }

        if let Some(interface_index) = self.interface_index {
            config = config.with_ipv6_interface_index(interface_index);
        }

        if let Some(ttl) = self.ttl {
            config = config.with_ttl(ttl);
        }

        if !self.loopback {
            config = config.with_loopback(false);
        }

        config.validate().map_err(|err| err.to_string())?;
        Ok(config)
    }
}

pub(crate) fn parse_send_cli_args(args: &[String]) -> Result<SendCliArgs, String> {
    if args.len() < 4 {
        return Err("missing required arguments".to_string());
    }

    let group = args[1]
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid multicast group: {err}"))?;
    let dst_port = args[2]
        .parse::<u16>()
        .map_err(|err| format!("invalid destination port: {err}"))?;
    let payload = args[3].clone();

    let mut parsed = SendCliArgs {
        group,
        dst_port,
        payload,
        count: 1,
        interval_ms: 0,
        source: None,
        bind_addr: None,
        source_port: None,
        interface: None,
        interface_index: None,
        ttl: None,
        loopback: true,
    };

    let mut index = 4;

    if let Some(value) = args.get(index)
        && !value.starts_with('-')
    {
        parsed.count = value
            .parse::<u64>()
            .map_err(|err| format!("invalid send count: {err}"))?;
        index += 1;
    }

    if let Some(value) = args.get(index)
        && !value.starts_with('-')
    {
        parsed.interval_ms = value
            .parse::<u64>()
            .map_err(|err| format!("invalid interval_ms: {err}"))?;
        index += 1;
    }

    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                index += 1;
                parsed.source = Some(parse_value(args, index, "--source")?);
                index += 1;
            }
            "--bind" => {
                index += 1;
                parsed.bind_addr = Some(parse_value(args, index, "--bind")?);
                index += 1;
            }
            "--source-port" => {
                index += 1;
                parsed.source_port = Some(parse_value(args, index, "--source-port")?);
                index += 1;
            }
            "--interface" => {
                index += 1;
                parsed.interface = Some(parse_value(args, index, "--interface")?);
                index += 1;
            }
            "--interface-index" => {
                index += 1;
                parsed.interface_index = Some(parse_value(args, index, "--interface-index")?);
                index += 1;
            }
            "--ttl" => {
                index += 1;
                parsed.ttl = Some(parse_value(args, index, "--ttl")?);
                index += 1;
            }
            "--no-loopback" => {
                parsed.loopback = false;
                index += 1;
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    if parsed.bind_addr.is_some() && (parsed.source.is_some() || parsed.source_port.is_some()) {
        return Err("--bind cannot be combined with --source or --source-port".to_string());
    }

    if parsed.interface.is_some() && parsed.interface_index.is_some() {
        return Err("--interface and --interface-index are mutually exclusive".to_string());
    }

    if matches!(parsed.group, IpAddr::V4(_)) && parsed.interface_index.is_some() {
        return Err("--interface-index is only valid for IPv6 multicast".to_string());
    }

    if let Some(source) = parsed.source
        && !same_family_ip(parsed.group, source)
    {
        return Err("--source must match the multicast group address family".to_string());
    }

    if let Some(bind_addr) = parsed.bind_addr
        && !same_family_ip(parsed.group, bind_addr.ip())
    {
        return Err("--bind must match the multicast group address family".to_string());
    }

    if let Some(interface) = parsed.interface
        && !same_family_ip(parsed.group, interface)
    {
        return Err("--interface must match the multicast group address family".to_string());
    }

    Ok(parsed)
}

pub(crate) fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!(
        "  {program} <group> <dst_port> <payload> [count] [interval_ms] [--source <ip>] [--source-port <port>] [--bind <ip:port>] [--interface <ip>] [--interface-index <idx>] [--ttl <ttl>] [--no-loopback]"
    );
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {program} 239.1.2.3 5000 hello");
    eprintln!("  {program} 239.1.2.3 5000 hello 100 10 --source 192.168.1.10");
    eprintln!("  {program} ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1");
    eprintln!("  {program} ff3e::8000:1234 5000 hello-v6 --source fd00::10");
    eprintln!("  {program} ff32::8000:1234 5000 hello-v6 --source fe80::1234 --interface-index 7");
    eprintln!();
    eprintln!("Notes:");
    eprintln!("  - use --source to pin the exact sender source IP");
    eprintln!("  - use --interface to choose the outgoing multicast interface by local IP");
    eprintln!("  - use --interface-index for IPv6 when you want an explicit interface index");
    eprintln!(
        "  - for IPv6 SSM-style testing, choose groups in ff3x::/32 such as ff31::8000:1234 or ff3e::8000:1234"
    );
    eprintln!("  - ff31::/16 is interface-local and works well for same-host tests");
    eprintln!("  - ff32::/16 is link-local; send from a fe80:: source on the same link");
    eprintln!(
        "  - ff3e::/16 is global scope; keep destination scope_id at 0 and rely on the bound source plus multicast interface selection"
    );
}

fn parse_value<T>(args: &[String], index: usize, flag: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = args
        .get(index)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    value
        .parse::<T>()
        .map_err(|err| format!("invalid value for {flag}: {err}"))
}

fn same_family_ip(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV6};

    #[test]
    fn parses_ipv6_source_and_interface_flags() {
        let args = vec![
            "mctx_send".to_string(),
            "ff31::8000:1234".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--source".to_string(),
            "::1".to_string(),
            "--interface".to_string(),
            "::1".to_string(),
        ];

        let parsed = parse_send_cli_args(&args).unwrap();

        assert_eq!(
            parsed.group,
            IpAddr::V6("ff31::8000:1234".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(parsed.dst_port, 5000);
        assert_eq!(parsed.source, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert_eq!(parsed.interface, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn parses_ipv6_interface_index() {
        let args = vec![
            "mctx_send".to_string(),
            "ff3e::8000:1234".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--interface-index".to_string(),
            "7".to_string(),
        ];

        let parsed = parse_send_cli_args(&args).unwrap();

        assert_eq!(parsed.interface_index, Some(7));
    }

    #[test]
    fn rejects_bind_with_source_flags() {
        let args = vec![
            "mctx_send".to_string(),
            "239.1.2.3".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--bind".to_string(),
            "192.168.1.10:5001".to_string(),
            "--source".to_string(),
            "192.168.1.10".to_string(),
        ];

        let result = parse_send_cli_args(&args);

        assert!(
            result
                .unwrap_err()
                .contains("--bind cannot be combined with --source or --source-port")
        );
    }

    #[test]
    fn rejects_ipv4_group_with_interface_index() {
        let args = vec![
            "mctx_send".to_string(),
            "239.1.2.3".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--interface-index".to_string(),
            "7".to_string(),
        ];

        let result = parse_send_cli_args(&args);

        assert!(
            result
                .unwrap_err()
                .contains("--interface-index is only valid for IPv6 multicast")
        );
    }

    #[test]
    fn build_config_uses_bind_addr() {
        let parsed = SendCliArgs {
            group: IpAddr::V6("ff3e::8000:1234".parse().unwrap()),
            dst_port: 5000,
            payload: "hello".to_string(),
            count: 1,
            interval_ms: 0,
            source: None,
            bind_addr: Some(SocketAddr::V6(SocketAddrV6::new(
                "fd00::10".parse().unwrap(),
                5001,
                0,
                0,
            ))),
            source_port: None,
            interface: None,
            interface_index: None,
            ttl: None,
            loopback: true,
        };

        let config = parsed.build_config().unwrap();

        assert_eq!(
            config.source_addr,
            Some(IpAddr::V6("fd00::10".parse().unwrap()))
        );
        assert_eq!(config.source_port, Some(5001));
    }

    #[test]
    fn build_config_uses_ipv4_interface() {
        let parsed = SendCliArgs {
            group: IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3)),
            dst_port: 5000,
            payload: "hello".to_string(),
            count: 1,
            interval_ms: 0,
            source: None,
            bind_addr: None,
            source_port: None,
            interface: Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))),
            interface_index: None,
            ttl: Some(4),
            loopback: false,
        };

        let config = parsed.build_config().unwrap();

        assert_eq!(
            config.outgoing_interface,
            Some(Ipv4Addr::new(192, 168, 1, 10).into())
        );
        assert_eq!(config.ttl, 4);
        assert!(!config.loopback);
    }
}
