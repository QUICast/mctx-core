# Metrics

Enable metrics with:

```bash
cargo test --features metrics
```

The `metrics` feature adds send counters without changing the default API
surface:

- `Publication::metrics_snapshot()`
- `Context::metrics_snapshot()`
- `PublicationMetricsSampler`
- `ContextMetricsSampler`

Tracked counters:

- send calls
- packets sent
- bytes sent
- send errors
