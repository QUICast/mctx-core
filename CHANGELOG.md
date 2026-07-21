# Changelog

## Unreleased

### Fixed

- Made Python asyncio sends safe when a publication is removed while waiting
  for write readiness, including selector loops that never emit another event.
- Preserved native OS error numbers on Python `OSError` and
  `BlockingIOError` exceptions.

### Added

- Added optional `raw-route-egress` support for unbound, unconnected
  route-selected raw IPv4 publications on Linux and macOS.
- Added source-preserving route-selected IPv6 multicast egress on Linux using
  cached NETLINK_ROUTE resolution, route/link invalidation, and AF_PACKET.
- Added explicit-interface full-header IPv6 multicast egress on macOS through
  Ethernet BPF injection.
- Added detailed IPv6 raw-egress capability reporting that distinguishes
  unsupported, local-source-only, explicit full-header, and route-selected
  full-header modes.
- Added transactional `RawContext::replace_publication`, which preserves the
  publication ID and leaves the original publication untouched on failure.
- Added ignored privileged Linux two-veth tests covering IPv4 and byte-exact
  IPv6 route changes, transient no-route errors, and recovery through one
  publication ID.
- Exposed IPv6 destination and local scope IDs through additive Python
  publication/report properties without changing existing address tuples.
- Added Python regressions for publication removal, unsupported writer
  readiness, invalid polling intervals, and native error numbers.

### Changed

- Linux explicit IPv6 raw multicast now consistently uses AF_PACKET for local
  and remote header sources, avoiding conditional base-header reconstruction.
- Full-header IPv6 backends reject conflicting hop-limit overrides and do not
  provide same-host IP multicast loopback.
- Documented macOS route-selected IPv6, Windows IPv6 raw transmit, iOS, and
  Android as explicitly unsupported rather than claiming unverified fidelity.
- Marked the PyO3 `cdylib` as non-testable by Cargo directly; installed-package
  tests remain the authoritative binding runtime suite.
- Extended binding CI to exercise both the declared Python 3.9 minimum and
  Python 3.12 across Linux, macOS, and Windows.

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
