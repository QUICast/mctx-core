# Changelog

## [0.3.0] - 2026-07-10

### Added

- Optional `raw-ip` support for transmitting caller-supplied complete IPv4 or
  IPv6 datagrams toward an explicitly pinned interface. The API is intended
  for higher-level control traffic such as ICMP Packet Too Big and does not add
  AMT or ICMP policy to `mctx-core`.
- Explicit compile-time capability reporting for raw IPv4 and IPv6 transmit on
  Linux, macOS, Windows, and unsupported targets.
- Strict complete-datagram validation, typed raw-IP configuration failures,
  cross-platform compile coverage, and an ignored privileged Linux namespace
  test for complete ICMPv4 and ICMPv6 error packets.

### Changed

- Declared Rust 1.88 as the minimum supported toolchain and enabled all optional
  features for docs.rs builds.
- Bounded per-publication raw-IP protocol socket reuse to 16 entries. Existing
  UDP multicast and `raw-packets` APIs and behavior remain unchanged.
