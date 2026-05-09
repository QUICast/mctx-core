# Demo Binaries

The demo binaries use explicit flags for sender source and outgoing interface:

- `--source <ip>`: exact sender source/local bind IP
- `--source-port <port>`: exact sender UDP source port
- `--bind <ip:port>`: exact sender local bind IP and port
- `--interface <ip>`: outgoing multicast interface selected by local IP
- `--interface-index <idx>`: outgoing IPv6 multicast interface selected by interface index

## Basic IPv4 Send

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello
```

## IPv4 Send with Explicit Source

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello --source 192.168.1.10 --source-port 5001
```

## Same-Host IPv6 SSM-Style Send

```bash
cargo run --bin mctx_send -- ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1
```

## Cross-Machine IPv6 SSM-Style Send

```bash
cargo run --bin mctx_send -- ff3e::8000:1234 5000 hello-v6 --source fd00::10
```

## Link-Local IPv6 Send

```bash
cargo run --bin mctx_send -- ff32::8000:1234 5000 hello-v6 --source fe80::1234 --interface-index 7
```

## Burst Send

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello 100 10
```

## Tokio Variant

```bash
cargo run --features tokio --bin mctx_tokio_send -- ff31::8000:1234 5000 hello-v6 --source ::1 --interface ::1
```
