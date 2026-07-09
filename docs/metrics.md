# Metrics

Metrics are optional and sit outside the core send API.

## Enabling Metrics

```bash
cargo test --features metrics
```

The current JSONL writer lives in `mctx_send` when built with
`--features metrics`.

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

Samplers measure elapsed intervals with a monotonic clock, so wall-clock
adjustments do not distort rates. The snapshot `captured_at` field remains a
`SystemTime` because it is used for exported timestamps. Direct
`delta_since()` calls compare those wall-clock timestamps; callers with their
own monotonic duration can use `delta_since_duration()`. Likewise, `sample()`
uses the sampler's monotonic clock, while `sample_snapshot()` respects the
supplied snapshots' `captured_at` values. Callers that capture snapshots with
their own monotonic clock can use `sample_snapshot_at()`.

Counter updates use a sequence guard, so a concurrent snapshot observes each
send as one coherent update rather than splitting calls, packets, and bytes
across intervals.

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

`mctx_send` can emit periodic or final sender summaries when built with
`--features metrics`.

Environment variables:

- `MCTX_METRICS_SUMMARY_SECS=<n>`: emit a periodic delta summary every `n`
  seconds
- `MCTX_METRICS_SUMMARY_FILE=<path>`: write single-header Heimdall JSONL
  instead of printing summaries
- `MCTX_METRICS_NODE_ID=<id>`: override the JSONL header `node_id`
- `MCTX_METRICS_FLAGS_JSON=<json>`: merge extra JSON object fields into the
  header `flags` map

If `MCTX_METRICS_SUMMARY_FILE` is set, `mctx_send` always writes a final sample
row at exit when a delta is available, even if no periodic interval elapsed.

## JSONL Schema

Metrics JSONL files use a single-header format:

1. The first non-empty line is the header object.
2. Remaining lines are compact sample rows.
3. Exactly one header is written per file.
4. Sample rows do not repeat schema, artifact type, node ID, producer, or
   flags.

Network output uses `artifact_type = "mctx-network"`.

The canonical header shape is:

```json
{
  "schema": "heimdall-jsonl-v1",
  "artifact_type": "mctx-network",
  "node_id": "sender-0001",
  "producer": "mctx-core/mctx_send",
  "created_at": 1779999999.123,
  "flags": {
    "transport": "udp-multicast",
    "role": "sender"
  }
}
```

The canonical sample shape is:

```json
{
  "ts": 1779999999.456,
  "interval_secs": 1.0,
  "active_publications": 1,
  "publications_added_total": 1,
  "publications_added_delta": 0,
  "publications_removed_total": 0,
  "publications_removed_delta": 0,
  "send_calls_total": 100,
  "send_calls_delta": 10,
  "packets_sent_total": 100,
  "packets_sent_delta": 10,
  "bytes_sent_total": 64000,
  "bytes_sent_delta": 6400,
  "send_errors_total": 0,
  "send_errors_delta": 0,
  "send_calls_per_sec": 10.0,
  "packets_per_sec": 10.0,
  "bytes_per_sec": 6400.0,
  "send_errors_per_sec": 0.0
}
```

There is no support for headerless files anymore. Existing files without a
valid first-line Heimdall header are rejected by the JSONL helper before any
sample row is appended.

Requested headers are validated before a file is created. A stateful writer
holds an exclusive advisory writer lock for its lifetime, so a second writer
fails instead of racing the header or interleaving rows. Each JSON object and
its newline are serialized into one buffer before being appended.

## Node ID Resolution

`node_id` is resolved in this order:

1. `MCTX_METRICS_NODE_ID`
2. parent directory name of `MCTX_METRICS_SUMMARY_FILE`
3. file stem of `MCTX_METRICS_SUMMARY_FILE`
4. `"unknown"`

## Flags

`flags` is a free-form JSON object. `mctx_send` populates it with sender-facing
configuration details such as:

- `transport`
- `role`
- `multicast_source`
- `multicast_interface`
- `multicast_group`
- `multicast_port`
- `publish_interval_ms`
- `chunk_payload_bytes`
- `pacing`
- `batch_send_enabled`

Callers can add experiment labels or integrity/pacing metadata through
`MCTX_METRICS_FLAGS_JSON`. Built-in flags win on key conflicts so core sender
identity fields stay stable.
