# Demo Binaries

One-shot send:

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello
```

Burst send with count and delay:

```bash
cargo run --bin mctx_send -- 239.1.2.3 5000 hello 100 10
```

Tokio variant:

```bash
cargo run --features tokio --bin mctx_tokio_send -- 239.1.2.3 5000 hello 100 10
```

Argument order:

`<group> <port> <payload> [count] [interval_ms]`
