use mctx_core::{RawContext, RawPublicationConfig, RawValidationMode};
use std::env;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawSendCliArgs {
    group: IpAddr,
    dst_port: u16,
    payload: String,
    count: u64,
    interval_ms: u64,
    source: IpAddr,
    bind_addr: Option<IpAddr>,
    source_port: u16,
    interface: Option<IpAddr>,
    interface_index: Option<u32>,
    ttl: Option<u8>,
    loopback: bool,
    allow_any_destination: bool,
    route_selected_egress: bool,
    quiet: bool,
}

impl RawSendCliArgs {
    fn build_config(&self) -> Result<RawPublicationConfig, String> {
        let mut config = match self.group {
            IpAddr::V4(_) => RawPublicationConfig::ipv4(),
            IpAddr::V6(_) => RawPublicationConfig::ipv6(),
        };

        if self.route_selected_egress {
            #[cfg(feature = "raw-route-egress")]
            {
                config = config.with_route_selected_egress();
            }
            #[cfg(not(feature = "raw-route-egress"))]
            {
                return Err(
                    "--route-selected-egress requires the raw-route-egress Cargo feature"
                        .to_string(),
                );
            }
        } else {
            config = config.with_bind_addr(self.bind_addr.unwrap_or(self.source));

            if let Some(interface) = self.interface {
                config = match interface {
                    IpAddr::V4(interface) => config.with_outgoing_interface(interface),
                    IpAddr::V6(interface) => config.with_outgoing_interface(interface),
                };
            }

            if let Some(interface_index) = self.interface_index {
                config = config.with_ipv6_interface_index(interface_index);
            }
        }

        if let Some(ttl) = self.ttl.filter(|_| !self.route_selected_egress) {
            config = config.with_ttl(ttl);
        }

        if !self.loopback {
            config = config.with_loopback(false);
        }

        if self.allow_any_destination {
            config = config.with_validation_mode(RawValidationMode::AllowAnyDestination);
        }

        config.validate().map_err(|err| err.to_string())?;
        Ok(config)
    }

    fn build_datagram(&self) -> Result<Vec<u8>, String> {
        let ttl_or_hops = self.ttl.unwrap_or(1);
        match (self.source, self.group) {
            (IpAddr::V4(source), IpAddr::V4(group)) => Ok(build_ipv4_udp_datagram(
                source,
                group,
                self.source_port,
                self.dst_port,
                self.payload.as_bytes(),
                ttl_or_hops,
            )?),
            (IpAddr::V6(source), IpAddr::V6(group)) => Ok(build_ipv6_udp_datagram(
                source,
                group,
                self.source_port,
                self.dst_port,
                self.payload.as_bytes(),
                ttl_or_hops,
            )?),
            _ => Err("--source must match the multicast group address family".to_string()),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let parsed = match parse_raw_send_cli_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            print_usage(&args[0]);
            return Err(err.into());
        }
    };

    let config = parsed.build_config()?;
    let datagram = parsed.build_datagram()?;
    let mut context = RawContext::new();
    let id = context.add_publication(config)?;
    let interval = Duration::from_millis(parsed.interval_ms);

    for packet_index in 0..parsed.count {
        let report = context.send_raw(id, &datagram)?;
        if !parsed.quiet {
            println!(
                "sent {} raw bytes proto {:?} to {:?} from {:?} via ifindex {:?}",
                report.bytes_sent,
                report.ip_protocol,
                report.destination_ip,
                report.source_ip,
                report.outgoing_interface_index
            );
        }

        if packet_index + 1 < parsed.count && !interval.is_zero() {
            thread::sleep(interval);
        }
    }

    Ok(())
}

fn parse_raw_send_cli_args(args: &[String]) -> Result<RawSendCliArgs, String> {
    if args.len() < 4 {
        return Err("missing required arguments".to_string());
    }

    let group = args[1]
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid group: {err}"))?;
    let dst_port = parse_port(&args[2], "dst_port")?;
    let payload = args[3].clone();

    let mut parsed = RawSendCliArgs {
        group,
        dst_port,
        payload,
        count: 1,
        interval_ms: 0,
        source: match group {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        },
        bind_addr: None,
        source_port: 4000,
        interface: None,
        interface_index: None,
        ttl: None,
        loopback: true,
        allow_any_destination: false,
        route_selected_egress: false,
        quiet: false,
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

    let mut source = None;

    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                index += 1;
                source = Some(parse_ip_value(args, index, "--source")?);
                index += 1;
            }
            "--source-port" => {
                index += 1;
                parsed.source_port = parse_port(
                    args.get(index)
                        .ok_or_else(|| "missing value for --source-port".to_string())?,
                    "--source-port",
                )?;
                index += 1;
            }
            "--bind" => {
                index += 1;
                parsed.bind_addr = Some(parse_ip_value(args, index, "--bind")?);
                index += 1;
            }
            "--interface" => {
                index += 1;
                parsed.interface = Some(parse_ip_value(args, index, "--interface")?);
                index += 1;
            }
            "--interface-index" => {
                index += 1;
                parsed.interface_index = Some(parse_u32_value(args, index, "--interface-index")?);
                index += 1;
            }
            "--ttl" => {
                index += 1;
                parsed.ttl = Some(parse_u8_value(args, index, "--ttl")?);
                index += 1;
            }
            "--no-loopback" => {
                parsed.loopback = false;
                index += 1;
            }
            "--allow-any-destination" => {
                parsed.allow_any_destination = true;
                index += 1;
            }
            "--route-selected-egress" => {
                parsed.route_selected_egress = true;
                index += 1;
            }
            "--quiet" => {
                parsed.quiet = true;
                index += 1;
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    let source = source.ok_or_else(|| "--source is required for raw send".to_string())?;
    parsed.source = source;

    if !same_family_ip(parsed.group, parsed.source) {
        return Err("--source must match the multicast group address family".to_string());
    }

    if let Some(interface) = parsed.interface
        && !same_family_ip(parsed.group, interface)
    {
        return Err("--interface must match the multicast group address family".to_string());
    }

    if let Some(bind_addr) = parsed.bind_addr
        && !same_family_ip(parsed.group, bind_addr)
    {
        return Err("--bind must match the multicast group address family".to_string());
    }

    if parsed.interface.is_some() && parsed.interface_index.is_some() {
        return Err("--interface and --interface-index are mutually exclusive".to_string());
    }

    if matches!(parsed.group, IpAddr::V4(_)) && parsed.interface_index.is_some() {
        return Err("--interface-index is only valid for IPv6 raw send".to_string());
    }

    if parsed.route_selected_egress
        && (parsed.bind_addr.is_some()
            || parsed.interface.is_some()
            || parsed.interface_index.is_some())
    {
        return Err(
            "--route-selected-egress cannot be combined with --bind, --interface, or --interface-index"
                .to_string(),
        );
    }

    Ok(parsed)
}

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!(
        "  {program} <group> <dst_port> <payload> [count] [interval_ms] --source <ip> [--bind <local-ip>] [--source-port <port>] [--interface <ip>] [--interface-index <idx>] [--route-selected-egress] [--ttl <ttl>] [--no-loopback] [--allow-any-destination] [--quiet]"
    );
    eprintln!();
    eprintln!("Examples:");
    eprintln!(
        "  {program} 239.255.12.34 5000 hello-raw 5 100 --source 192.168.1.20 --source-port 4000"
    );
    eprintln!(
        "  {program} 232.1.2.3 5000 hello-ssm 5 100 --source 192.168.1.20 --source-port 4000"
    );
    eprintln!(
        "  {program} 232.1.2.3 5000 hello-routed 5 100 --source 198.51.100.10 --source-port 4000 --route-selected-egress"
    );
    eprintln!(
        "  {program} ff3e::8000:1234 5000 hello-v6-routed 5 100 --source 2001:db8::10 --source-port 4000 --route-selected-egress --no-loopback"
    );
    eprintln!(
        "  {program} ff32::8000:1234 5000 hello-v6 5 100 --source fe80::1234 --source-port 4000 --interface-index 7 --no-loopback"
    );
    eprintln!(
        "  {program} ff3e::8000:1234 5000 hello-v6 5 100 --source fd00::10 --source-port 4000 --interface fd00::10"
    );
    eprintln!();
    eprintln!("Notes:");
    eprintln!(
        "  - this binary builds a complete UDP-in-IP datagram and sends it through the raw API"
    );
    eprintln!(
        "  - --source sets the source IP encoded into the IP header and is used as the local bind unless --bind is provided"
    );
    eprintln!("  - --bind selects a distinct local egress address for remote-source forwarding");
    eprintln!("  - --source-port defaults to 4000");
    eprintln!("  - use --interface or --interface-index when you need to force egress selection");
    eprintln!(
        "  - route-selected IPv4 follows OS routing; Linux IPv6 follows the main table with route/link invalidation"
    );
    eprintln!(
        "  - Linux AF_PACKET and macOS BPF preserve complete IPv6 headers; they do not provide same-host IP loopback"
    );
    eprintln!("  - Windows currently supports raw IPv4 only");
}

fn parse_ip_value(args: &[String], index: usize, flag: &str) -> Result<IpAddr, String> {
    args.get(index)
        .ok_or_else(|| format!("missing value for {flag}"))?
        .parse::<IpAddr>()
        .map_err(|err| format!("invalid value for {flag}: {err}"))
}

fn parse_u32_value(args: &[String], index: usize, flag: &str) -> Result<u32, String> {
    args.get(index)
        .ok_or_else(|| format!("missing value for {flag}"))?
        .parse::<u32>()
        .map_err(|err| format!("invalid value for {flag}: {err}"))
}

fn parse_u8_value(args: &[String], index: usize, flag: &str) -> Result<u8, String> {
    args.get(index)
        .ok_or_else(|| format!("missing value for {flag}"))?
        .parse::<u8>()
        .map_err(|err| format!("invalid value for {flag}: {err}"))
}

fn parse_port(value: &str, field: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|err| format!("invalid {field}: {err}"))?;

    if port == 0 {
        return Err(format!("{field} must not be 0"));
    }

    Ok(port)
}

fn same_family_ip(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

fn build_ipv4_udp_datagram(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    ttl: u8,
) -> Result<Vec<u8>, String> {
    let total_len = 20usize
        .checked_add(8)
        .and_then(|len| len.checked_add(payload.len()))
        .ok_or_else(|| "IPv4 datagram length overflow".to_string())?;
    let total_len_u16 =
        u16::try_from(total_len).map_err(|_| "IPv4 datagram is too large".to_string())?;

    let mut datagram = vec![0u8; total_len];
    datagram[0] = 0x45;
    datagram[2..4].copy_from_slice(&total_len_u16.to_be_bytes());
    datagram[8] = ttl;
    datagram[9] = 17;
    datagram[12..16].copy_from_slice(&source.octets());
    datagram[16..20].copy_from_slice(&destination.octets());

    let udp_len = u16::try_from(8 + payload.len())
        .map_err(|_| "UDP payload is too large for IPv4".to_string())?;
    datagram[20..22].copy_from_slice(&source_port.to_be_bytes());
    datagram[22..24].copy_from_slice(&destination_port.to_be_bytes());
    datagram[24..26].copy_from_slice(&udp_len.to_be_bytes());
    datagram[26..28].copy_from_slice(&0u16.to_be_bytes());
    datagram[28..].copy_from_slice(payload);

    let udp_checksum = udp_checksum_v4(source, destination, &datagram[20..]);
    datagram[26..28].copy_from_slice(&udp_checksum.to_be_bytes());

    let ipv4_checksum = ipv4_header_checksum(&datagram[..20]);
    datagram[10..12].copy_from_slice(&ipv4_checksum.to_be_bytes());

    Ok(datagram)
}

fn build_ipv6_udp_datagram(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    hop_limit: u8,
) -> Result<Vec<u8>, String> {
    let payload_len = 8usize
        .checked_add(payload.len())
        .ok_or_else(|| "IPv6 UDP payload length overflow".to_string())?;
    let payload_len_u16 =
        u16::try_from(payload_len).map_err(|_| "IPv6 UDP payload is too large".to_string())?;
    let total_len = 40usize
        .checked_add(payload_len)
        .ok_or_else(|| "IPv6 datagram length overflow".to_string())?;

    let mut datagram = vec![0u8; total_len];
    datagram[0] = 0x60;
    datagram[4..6].copy_from_slice(&payload_len_u16.to_be_bytes());
    datagram[6] = 17;
    datagram[7] = hop_limit;
    datagram[8..24].copy_from_slice(&source.octets());
    datagram[24..40].copy_from_slice(&destination.octets());
    datagram[40..42].copy_from_slice(&source_port.to_be_bytes());
    datagram[42..44].copy_from_slice(&destination_port.to_be_bytes());
    datagram[44..46].copy_from_slice(&payload_len_u16.to_be_bytes());
    datagram[46..48].copy_from_slice(&0u16.to_be_bytes());
    datagram[48..].copy_from_slice(payload);

    let udp_checksum = udp_checksum_v6(source, destination, &datagram[40..]);
    datagram[46..48].copy_from_slice(&udp_checksum.to_be_bytes());

    Ok(datagram)
}

fn udp_checksum_v4(source: Ipv4Addr, destination: Ipv4Addr, udp_packet: &[u8]) -> u16 {
    let mut pseudo = Vec::with_capacity(12 + udp_packet.len() + (udp_packet.len() % 2));
    pseudo.extend_from_slice(&source.octets());
    pseudo.extend_from_slice(&destination.octets());
    pseudo.push(0);
    pseudo.push(17);
    pseudo.extend_from_slice(&(udp_packet.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(udp_packet);
    normalize_udp_checksum(ones_complement_checksum(&pseudo))
}

fn udp_checksum_v6(source: Ipv6Addr, destination: Ipv6Addr, udp_packet: &[u8]) -> u16 {
    let mut pseudo = Vec::with_capacity(40 + udp_packet.len() + (udp_packet.len() % 2));
    pseudo.extend_from_slice(&source.octets());
    pseudo.extend_from_slice(&destination.octets());
    pseudo.extend_from_slice(&(udp_packet.len() as u32).to_be_bytes());
    pseudo.extend_from_slice(&[0, 0, 0, 17]);
    pseudo.extend_from_slice(udp_packet);
    normalize_udp_checksum(ones_complement_checksum(&pseudo))
}

fn normalize_udp_checksum(checksum: u16) -> u16 {
    if checksum == 0 { 0xffff } else { checksum }
}

fn ipv4_header_checksum(header: &[u8]) -> u16 {
    ones_complement_checksum(header)
}

fn ones_complement_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;

    for chunk in bytes.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }

    if !bytes.len().is_multiple_of(2) {
        sum += (bytes[bytes.len() - 1] as u32) << 8;
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }

    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_raw_cli_requires_source() {
        let args = vec![
            "mctx_raw_send".to_string(),
            "239.255.12.34".to_string(),
            "5000".to_string(),
            "hello".to_string(),
        ];

        let err = parse_raw_send_cli_args(&args).unwrap_err();
        assert!(err.contains("--source is required"));
    }

    #[test]
    fn parse_raw_cli_accepts_ipv6_interface_index() {
        let args = vec![
            "mctx_raw_send".to_string(),
            "ff31::8000:1234".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--source".to_string(),
            "::1".to_string(),
            "--interface-index".to_string(),
            "7".to_string(),
        ];

        let parsed = parse_raw_send_cli_args(&args).unwrap();
        assert_eq!(parsed.interface_index, Some(7));
    }

    #[test]
    fn build_ipv4_datagram_sets_expected_addresses_and_ports() {
        let datagram = build_ipv4_udp_datagram(
            Ipv4Addr::new(10, 1, 2, 3),
            Ipv4Addr::new(239, 1, 2, 3),
            4000,
            5000,
            b"hello",
            8,
        )
        .unwrap();

        assert_eq!(&datagram[12..16], &Ipv4Addr::new(10, 1, 2, 3).octets());
        assert_eq!(&datagram[16..20], &Ipv4Addr::new(239, 1, 2, 3).octets());
        assert_eq!(u16::from_be_bytes([datagram[20], datagram[21]]), 4000);
        assert_eq!(u16::from_be_bytes([datagram[22], datagram[23]]), 5000);
    }

    #[test]
    fn build_ipv6_datagram_sets_expected_addresses_and_ports() {
        let datagram = build_ipv6_udp_datagram(
            "::1".parse().unwrap(),
            "ff31::8000:1234".parse().unwrap(),
            4000,
            5000,
            b"hello",
            8,
        )
        .unwrap();

        assert_eq!(&datagram[8..24], &Ipv6Addr::LOCALHOST.octets());
        assert_eq!(
            &datagram[24..40],
            &"ff31::8000:1234".parse::<Ipv6Addr>().unwrap().octets()
        );
        assert_eq!(u16::from_be_bytes([datagram[40], datagram[41]]), 4000);
        assert_eq!(u16::from_be_bytes([datagram[42], datagram[43]]), 5000);
        assert_ne!(u16::from_be_bytes([datagram[46], datagram[47]]), 0);
    }

    #[test]
    fn build_config_uses_source_as_default_bind_addr() {
        let parsed = RawSendCliArgs {
            group: IpAddr::V4(Ipv4Addr::new(239, 255, 12, 34)),
            dst_port: 5000,
            payload: "hello".to_string(),
            count: 1,
            interval_ms: 0,
            source: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            bind_addr: None,
            source_port: 4000,
            interface: None,
            interface_index: None,
            ttl: Some(4),
            loopback: true,
            allow_any_destination: false,
            route_selected_egress: false,
            quiet: false,
        };

        let config = parsed.build_config().unwrap();
        assert_eq!(
            config.bind_addr,
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)))
        );
    }

    #[test]
    fn build_config_can_bind_locally_for_a_distinct_datagram_source() {
        let parsed = RawSendCliArgs {
            group: IpAddr::V6("ff3e::8000:1234".parse().unwrap()),
            dst_port: 5000,
            payload: "hello".to_string(),
            count: 1,
            interval_ms: 0,
            source: IpAddr::V6("2001:db8::10".parse().unwrap()),
            bind_addr: Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            source_port: 4000,
            interface: Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            interface_index: None,
            ttl: Some(4),
            loopback: true,
            allow_any_destination: false,
            route_selected_egress: false,
            quiet: true,
        };

        let config = parsed.build_config().unwrap();
        assert_eq!(config.bind_addr, Some(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[cfg(feature = "raw-route-egress")]
    #[test]
    fn route_selected_cli_config_does_not_bind_the_header_source() {
        let args = vec![
            "mctx_raw_send".to_string(),
            "232.1.2.3".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--source".to_string(),
            "198.51.100.10".to_string(),
            "--route-selected-egress".to_string(),
            "--ttl".to_string(),
            "12".to_string(),
        ];

        let parsed = parse_raw_send_cli_args(&args).unwrap();
        let config = parsed.build_config().unwrap();

        assert_eq!(config.egress_mode, mctx_core::RawEgressMode::RouteSelected);
        assert_eq!(config.bind_addr, None);
        assert_eq!(config.outgoing_interface, None);
        assert_eq!(config.ttl, None);
    }

    #[cfg(feature = "raw-route-egress")]
    #[test]
    fn route_selected_cli_accepts_wider_scope_ipv6() {
        let args = vec![
            "mctx_raw_send".to_string(),
            "ff3e::8000:1234".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--source".to_string(),
            "2001:db8::10".to_string(),
            "--route-selected-egress".to_string(),
            "--no-loopback".to_string(),
        ];

        let parsed = parse_raw_send_cli_args(&args).unwrap();
        let config = parsed.build_config().unwrap();

        assert_eq!(config.egress_mode, mctx_core::RawEgressMode::RouteSelected);
        assert_eq!(
            config.family,
            Some(mctx_core::PublicationAddressFamily::Ipv6)
        );
        assert_eq!(config.bind_addr, None);
        assert_eq!(config.outgoing_interface, None);
        assert_eq!(config.loopback, Some(false));
    }

    #[test]
    fn route_selected_cli_rejects_explicit_selectors() {
        let args = vec![
            "mctx_raw_send".to_string(),
            "232.1.2.3".to_string(),
            "5000".to_string(),
            "hello".to_string(),
            "--source".to_string(),
            "198.51.100.10".to_string(),
            "--route-selected-egress".to_string(),
            "--interface".to_string(),
            "192.0.2.10".to_string(),
        ];

        let error = parse_raw_send_cli_args(&args).unwrap_err();
        assert!(error.contains("cannot be combined"));
    }
}
