# Metrics

Metrics are optional and sit outside the core send API.

## Enabling Metrics

```bash
cargo test --features metrics
```

## Model

The metrics system is split into three layers:

### Snapshot

A snapshot is a point-in-time view.

Counter fields in a snapshot are cumulative.

For `ContextMetricsSnapshot`, publication add/remove counters and send
call/packet/byte/error counters are true context-lifetime totals for send
activity issued through `Context` methods. They are not recomputed from the
currently active publications, and they do not decrease when a publication is
removed.

Gauge-like fields in a snapshot reflect current state only:

- `active_publications`

At the publication level, snapshot counters remain cumulative for the lifetime
of that `Publication` object.

### Delta

A delta is computed between two snapshots of the same metric type.

Delta fields represent only the change over the sampled interval:

- publications added during the interval
- publications removed during the interval
- send calls during the interval
- packets sent during the interval
- bytes sent during the interval
- send errors during the interval

### Sampler

A sampler stores the previous snapshot and computes deltas across repeated
samples.

The first call to `sample()` returns `None` because a delta requires two
snapshots.

## Cumulative Totals

At the context level, these snapshot fields are cumulative totals:

- `publications_added`
- `publications_removed`
- `total_send_calls`
- `total_packets_sent`
- `total_bytes_sent`
- `total_send_errors`

At the publication level, these snapshot fields are cumulative totals for the
lifetime of the publication object:

- `send_calls`
- `packets_sent`
- `bytes_sent`
- `send_errors`

## Rates

Delta types expose average interval rates such as:

- `send_calls_per_sec()`
- `packets_per_sec()`
- `bytes_per_sec()`
- `send_errors_per_sec()`

These are computed from delta counters divided by the sampled interval.

## CLI and JSONL

Unlike `mcrx_recv`, the current sender demo binaries do not yet emit periodic
metrics summaries or JSONL output.

That means there is no sender-side JSONL schema change for downstream
consumers such as Heimdall today.

If a sender JSONL emitter is added later, it should follow the same explicit
pattern as the receiver side and prefer `*_total` and `*_delta` counter names
instead of ambiguous bare counter keys.
