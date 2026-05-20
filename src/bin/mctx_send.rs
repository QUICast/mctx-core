#[path = "common/send_args.rs"]
mod send_args;

use mctx_core::Context;
#[cfg(feature = "metrics")]
use mctx_core::jsonl::{
    MetricsJsonlOutputConfig, MetricsJsonlWriter, NETWORK_ARTIFACT_TYPE, header_json,
    infer_node_id_from_path, unix_timestamp_secs,
};
#[cfg(feature = "metrics")]
use mctx_core::{
    ContextMetricsDelta, ContextMetricsSampler, ContextMetricsSnapshot, PublicationConfig,
};
#[cfg(feature = "metrics")]
use serde_json::{Map, Value, json};
use std::env;
use std::error::Error;
#[cfg(feature = "metrics")]
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
#[cfg(feature = "metrics")]
use std::time::Instant;

#[cfg(feature = "metrics")]
const SENDER_PRODUCER: &str = "mctx-core/mctx_send";

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let parsed = match send_args::parse_send_cli_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            send_args::print_usage(&args[0], true);
            return Err(err.into());
        }
    };

    let config = parsed.build_config()?;
    let mut context = Context::new();
    let id = context.add_publication(config.clone())?;
    let interval = Duration::from_millis(parsed.interval_ms);

    #[cfg(feature = "metrics")]
    let summary_interval = summary_interval_from_env();
    #[cfg(feature = "metrics")]
    let summary_output = summary_output_from_env(&parsed, &config)?;
    #[cfg(feature = "metrics")]
    let mut metrics_sampler = ContextMetricsSampler::new(&context);
    #[cfg(feature = "metrics")]
    let _ = metrics_sampler.sample();
    #[cfg(feature = "metrics")]
    let mut metrics_writer: Option<MetricsJsonlWriter> = None;
    #[cfg(feature = "metrics")]
    let mut next_summary_at =
        summary_interval.map(|summary_interval| Instant::now() + summary_interval);

    for _ in 0..parsed.count {
        let report = context.send(id, parsed.payload.as_bytes())?;
        println!(
            "sent {} bytes to {} from {:?}",
            report.bytes_sent, report.destination, report.source_addr
        );

        #[cfg(feature = "metrics")]
        if let (Some(summary_interval), Some(deadline)) = (summary_interval, next_summary_at)
            && Instant::now() >= deadline
        {
            emit_metrics_summary(
                &context,
                &mut metrics_sampler,
                summary_output.as_ref(),
                &mut metrics_writer,
            )?;
            next_summary_at = Some(Instant::now() + summary_interval);
        }

        if !interval.is_zero() {
            thread::sleep(interval);
        }
    }

    #[cfg(feature = "metrics")]
    if summary_output.is_some() || summary_interval.is_some() {
        emit_metrics_summary(
            &context,
            &mut metrics_sampler,
            summary_output.as_ref(),
            &mut metrics_writer,
        )?;
    }

    Ok(())
}

#[cfg(feature = "metrics")]
fn summary_interval_from_env() -> Option<Duration> {
    let raw = env::var("MCTX_METRICS_SUMMARY_SECS").ok()?;
    let secs = raw.parse::<u64>().ok()?;
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

#[cfg(feature = "metrics")]
fn summary_file_from_env() -> Option<PathBuf> {
    let raw = env::var("MCTX_METRICS_SUMMARY_FILE").ok()?;
    if raw.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

#[cfg(feature = "metrics")]
fn metrics_node_id_from_env() -> Option<String> {
    env::var("MCTX_METRICS_NODE_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "metrics")]
fn metrics_flags_from_env() -> Result<Map<String, Value>, String> {
    let raw = match env::var("MCTX_METRICS_FLAGS_JSON") {
        Ok(raw) => raw,
        Err(_) => return Ok(Map::new()),
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }

    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|err| format!("MCTX_METRICS_FLAGS_JSON must be valid JSON: {err}"))?;

    match parsed {
        Value::Object(map) => Ok(map),
        _ => Err("MCTX_METRICS_FLAGS_JSON must be a JSON object".to_string()),
    }
}

#[cfg(feature = "metrics")]
fn sender_source_string(config: &PublicationConfig) -> String {
    config
        .source_addr
        .map(|source| source.to_string())
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(feature = "metrics")]
fn sender_interface_string(parsed: &send_args::SendCliArgs) -> String {
    match (parsed.interface, parsed.interface_index) {
        (Some(interface), _) => interface.to_string(),
        (None, Some(interface_index)) => interface_index.to_string(),
        (None, None) => "default".to_string(),
    }
}

#[cfg(feature = "metrics")]
fn build_sender_metrics_flags(
    parsed: &send_args::SendCliArgs,
    config: &PublicationConfig,
) -> Result<Map<String, Value>, String> {
    let mut flags = Map::new();
    flags.insert(
        "transport".to_string(),
        Value::String("udp-multicast".to_string()),
    );
    flags.insert("role".to_string(), Value::String("sender".to_string()));
    flags.insert(
        "multicast_group".to_string(),
        Value::String(config.group.to_string()),
    );
    flags.insert("multicast_port".to_string(), json!(config.dst_port));
    flags.insert(
        "multicast_source".to_string(),
        Value::String(sender_source_string(config)),
    );
    flags.insert(
        "multicast_interface".to_string(),
        Value::String(sender_interface_string(parsed)),
    );
    flags.insert("publish_interval_ms".to_string(), json!(parsed.interval_ms));
    flags.insert(
        "chunk_payload_bytes".to_string(),
        json!(parsed.payload.len()),
    );
    flags.insert(
        "pacing".to_string(),
        Value::String(
            if parsed.interval_ms > 0 {
                "fixed-interval"
            } else {
                "none"
            }
            .to_string(),
        ),
    );
    flags.insert("batch_send_enabled".to_string(), Value::Bool(false));
    flags.insert("loopback_enabled".to_string(), Value::Bool(config.loopback));
    flags.insert("ttl_or_hops".to_string(), json!(config.ttl));

    if let Some(source_port) = config.source_port {
        flags.insert("multicast_source_port".to_string(), json!(source_port));
    }

    if let Some(interface_index) = parsed.interface_index {
        flags.insert(
            "multicast_interface_index".to_string(),
            json!(interface_index),
        );
    }

    for (key, value) in metrics_flags_from_env()? {
        flags.entry(key).or_insert(value);
    }

    Ok(flags)
}

#[cfg(feature = "metrics")]
fn summary_output_from_env(
    parsed: &send_args::SendCliArgs,
    config: &PublicationConfig,
) -> Result<Option<MetricsJsonlOutputConfig>, String> {
    let Some(network_path) = summary_file_from_env() else {
        return Ok(None);
    };

    Ok(Some(MetricsJsonlOutputConfig {
        node_id: metrics_node_id_from_env()
            .unwrap_or_else(|| infer_node_id_from_path(&network_path)),
        flags: build_sender_metrics_flags(parsed, config)?,
        network_path,
    }))
}

#[cfg(feature = "metrics")]
fn emit_metrics_summary(
    context: &Context,
    metrics_sampler: &mut ContextMetricsSampler<'_>,
    output: Option<&MetricsJsonlOutputConfig>,
    writer: &mut Option<MetricsJsonlWriter>,
) -> Result<(), Box<dyn Error>> {
    let snapshot = context.metrics_snapshot();
    if let Some(delta) = metrics_sampler.sample_snapshot(snapshot.clone()) {
        if let Some(output) = output {
            write_metrics_summary_jsonl(&snapshot, &delta, output, writer)?;
        } else {
            print_metrics_summary(&snapshot, &delta);
        }
    }

    Ok(())
}

#[cfg(feature = "metrics")]
fn write_metrics_summary_jsonl(
    snapshot: &ContextMetricsSnapshot,
    delta: &ContextMetricsDelta,
    output: &MetricsJsonlOutputConfig,
    writer: &mut Option<MetricsJsonlWriter>,
) -> Result<(), std::io::Error> {
    let sample = json!({
        "ts": unix_timestamp_secs(snapshot.captured_at),
        "interval_secs": delta.interval_secs,
        "active_publications": snapshot.active_publications,
        "publications_added_total": snapshot.publications_added,
        "publications_added_delta": delta.publications_added,
        "publications_removed_total": snapshot.publications_removed,
        "publications_removed_delta": delta.publications_removed,
        "send_calls_total": snapshot.total_send_calls,
        "send_calls_delta": delta.send_calls,
        "packets_sent_total": snapshot.total_packets_sent,
        "packets_sent_delta": delta.packets_sent,
        "bytes_sent_total": snapshot.total_bytes_sent,
        "bytes_sent_delta": delta.bytes_sent,
        "send_errors_total": snapshot.total_send_errors,
        "send_errors_delta": delta.send_errors,
        "send_calls_per_sec": delta.send_calls_per_sec(),
        "packets_per_sec": delta.packets_per_sec(),
        "bytes_per_sec": delta.bytes_per_sec(),
        "send_errors_per_sec": delta.send_errors_per_sec(),
    });

    if writer.is_none() {
        let header = header_json(
            NETWORK_ARTIFACT_TYPE,
            SENDER_PRODUCER,
            &output.node_id,
            snapshot.captured_at,
            &output.flags,
        );
        *writer = Some(MetricsJsonlWriter::open(&output.network_path, &header)?);
    }

    writer
        .as_mut()
        .expect("metrics writer is initialized before append")
        .append_sample_row(&sample)
}

#[cfg(feature = "metrics")]
fn print_metrics_summary(snapshot: &ContextMetricsSnapshot, delta: &ContextMetricsDelta) {
    println!("[metrics]");
    println!("  interval_secs:              {:.3}", delta.interval_secs);
    println!(
        "  active_publications:        {}",
        snapshot.active_publications
    );
    println!(
        "  publications_added_total:   {}",
        snapshot.publications_added
    );
    println!("  publications_added_delta:   {}", delta.publications_added);
    println!(
        "  publications_removed_total: {}",
        snapshot.publications_removed
    );
    println!(
        "  publications_removed_delta: {}",
        delta.publications_removed
    );
    println!(
        "  send_calls_total:           {}",
        snapshot.total_send_calls
    );
    println!("  send_calls_delta:           {}", delta.send_calls);
    println!(
        "  packets_sent_total:         {}",
        snapshot.total_packets_sent
    );
    println!("  packets_sent_delta:         {}", delta.packets_sent);
    println!(
        "  bytes_sent_total:           {}",
        snapshot.total_bytes_sent
    );
    println!("  bytes_sent_delta:           {}", delta.bytes_sent);
    println!(
        "  send_errors_total:          {}",
        snapshot.total_send_errors
    );
    println!("  send_errors_delta:          {}", delta.send_errors);
    println!(
        "  send_calls_per_sec:         {:.3}",
        delta.send_calls_per_sec()
    );
    println!(
        "  packets_per_sec:            {:.3}",
        delta.packets_per_sec()
    );
    println!("  bytes_per_sec:              {:.3}", delta.bytes_per_sec());
    println!(
        "  send_errors_per_sec:        {:.3}",
        delta.send_errors_per_sec()
    );
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn sender_metrics_jsonl_uses_single_header_and_compact_samples() {
        let parent_name = format!(
            "mctx_send_metrics_node_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        );
        let parent = std::env::temp_dir().join(&parent_name);
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join("network_metrics.jsonl");

        let parsed = send_args::SendCliArgs {
            group: "ff3e::8000:1234".parse().unwrap(),
            dst_port: 5000,
            payload: "hello-v6".to_string(),
            count: 10,
            interval_ms: 25,
            source: Some("fd00::10".parse().unwrap()),
            bind_addr: None,
            source_port: Some(5500),
            interface: Some("fd00::10".parse().unwrap()),
            interface_index: None,
            ttl: Some(4),
            loopback: true,
        };
        let config = parsed.build_config().unwrap();
        let output = MetricsJsonlOutputConfig {
            node_id: infer_node_id_from_path(&path),
            flags: build_sender_metrics_flags(&parsed, &config).unwrap(),
            network_path: path.clone(),
        };

        let snapshot = ContextMetricsSnapshot {
            publications_added: 1,
            publications_removed: 0,
            active_publications: 1,
            total_send_calls: 10,
            total_packets_sent: 10,
            total_bytes_sent: 500,
            total_send_errors: 1,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(123),
        };
        let delta = ContextMetricsDelta {
            interval_secs: 2.0,
            publications_added: 1,
            publications_removed: 0,
            send_calls: 10,
            packets_sent: 10,
            bytes_sent: 500,
            send_errors: 1,
        };
        let mut writer = None;
        write_metrics_summary_jsonl(&snapshot, &delta, &output, &mut writer).unwrap();

        let later_snapshot = ContextMetricsSnapshot {
            total_send_calls: 12,
            total_packets_sent: 12,
            total_bytes_sent: 600,
            captured_at: SystemTime::UNIX_EPOCH + Duration::from_secs(124),
            ..snapshot.clone()
        };
        let later_delta = ContextMetricsDelta {
            interval_secs: 1.0,
            publications_added: 0,
            publications_removed: 0,
            send_calls: 2,
            packets_sent: 2,
            bytes_sent: 100,
            send_errors: 0,
        };

        write_metrics_summary_jsonl(&later_snapshot, &later_delta, &output, &mut writer).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let lines = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();

        assert_eq!(lines.len(), 3);

        let header: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header["artifact_type"], NETWORK_ARTIFACT_TYPE);
        assert_eq!(header["producer"], SENDER_PRODUCER);
        assert_eq!(header["node_id"], parent_name);

        for sample_line in &lines[1..] {
            let sample: Value = serde_json::from_str(sample_line).unwrap();
            assert!(sample.get("schema").is_none());
            assert!(sample.get("artifact_type").is_none());
            assert!(sample.get("node_id").is_none());
            assert!(sample.get("producer").is_none());
            assert!(sample.get("flags").is_none());
        }

        let sample = lines[1];
        assert!(sample.contains("\"publications_added_total\":1"));
        assert!(sample.contains("\"publications_added_delta\":1"));
        assert!(sample.contains("\"send_calls_total\":10"));
        assert!(sample.contains("\"send_calls_delta\":10"));
        assert!(sample.contains("\"packets_sent_total\":10"));
        assert!(sample.contains("\"packets_sent_delta\":10"));
        assert!(sample.contains("\"bytes_sent_total\":500"));
        assert!(sample.contains("\"bytes_sent_delta\":500"));
        assert!(sample.contains("\"send_errors_total\":1"));
        assert!(sample.contains("\"send_errors_delta\":1"));
        assert!(!sample.contains("\"artifact_type\":"));
        assert!(!sample.contains("\"node_id\":"));
        assert!(!sample.contains("\"flags\":"));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(parent);
    }
}
