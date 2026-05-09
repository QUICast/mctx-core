#[cfg(feature = "metrics")]
use serde_json::{Map, Value, json};
#[cfg(feature = "metrics")]
use std::fs;
#[cfg(feature = "metrics")]
use std::fs::OpenOptions;
#[cfg(feature = "metrics")]
use std::io::Write;
#[cfg(feature = "metrics")]
use std::path::{Path, PathBuf};
#[cfg(feature = "metrics")]
use std::time::{SystemTime, UNIX_EPOCH};

/// Canonical Heimdall JSONL schema version used by metrics writers.
#[cfg(feature = "metrics")]
pub const HEIMDALL_JSONL_SCHEMA: &str = "heimdall-jsonl-v1";

/// Canonical sender-side network artifact type.
#[cfg(feature = "metrics")]
pub const NETWORK_ARTIFACT_TYPE: &str = "mctx-network";

/// Canonical process hardware artifact type.
#[cfg(feature = "metrics")]
pub const HARDWARE_ARTIFACT_TYPE: &str = "process-hardware";

/// Common JSONL output configuration for one metrics file.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsJsonlOutputConfig {
    pub network_path: PathBuf,
    pub node_id: String,
    pub flags: Map<String, Value>,
}

/// Infers a `node_id` from a metrics file path.
///
/// Resolution order:
/// 1. parent directory name
/// 2. file stem
/// 3. `"unknown"`
#[cfg(feature = "metrics")]
pub fn infer_node_id_from_path(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .filter(|stem| !stem.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Converts a system time to a Unix timestamp in fractional seconds.
#[cfg(feature = "metrics")]
pub fn unix_timestamp_secs(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

/// Builds the canonical header object for a Heimdall JSONL file.
#[cfg(feature = "metrics")]
pub fn header_json(
    artifact_type: &'static str,
    producer: &'static str,
    node_id: &str,
    created_at: SystemTime,
    flags: &Map<String, Value>,
) -> Value {
    json!({
        "schema": HEIMDALL_JSONL_SCHEMA,
        "artifact_type": artifact_type,
        "node_id": node_id,
        "producer": producer,
        "created_at": unix_timestamp_secs(created_at),
        "flags": Value::Object(flags.clone()),
    })
}

/// Returns the first non-empty line from a JSONL file, if any.
#[cfg(feature = "metrics")]
pub fn first_non_empty_line(path: &Path) -> Result<Option<String>, std::io::Error> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    Ok(contents
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string))
}

/// Validates the existing first non-empty JSONL line as a Heimdall header.
#[cfg(feature = "metrics")]
pub fn validate_existing_header(path: &Path) -> Result<Option<Value>, std::io::Error> {
    let Some(line) = first_non_empty_line(path)? else {
        return Ok(None);
    };

    let parsed: Value = serde_json::from_str(&line).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("existing JSONL header is invalid JSON: {err}"),
        )
    })?;

    let schema = parsed.get("schema").and_then(Value::as_str);
    let artifact_type = parsed.get("artifact_type").and_then(Value::as_str);
    let node_id = parsed.get("node_id").and_then(Value::as_str);
    let producer = parsed.get("producer").and_then(Value::as_str);
    let flags = parsed.get("flags");

    if schema != Some(HEIMDALL_JSONL_SCHEMA)
        || artifact_type.is_none()
        || node_id.is_none()
        || producer.is_none()
        || !matches!(flags, Some(Value::Object(_)))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "existing JSONL file does not start with a Heimdall header object",
        ));
    }

    Ok(Some(parsed))
}

/// Ensures a JSONL file has exactly one header at the top before samples.
#[cfg(feature = "metrics")]
pub fn ensure_single_header(path: &Path, header: &Value) -> Result<(), std::io::Error> {
    match validate_existing_header(path)? {
        Some(_) => Ok(()),
        None => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, header).map_err(std::io::Error::other)?;
            file.write_all(b"\n")?;
            Ok(())
        }
    }
}

/// Appends one compact sample row to a JSONL file with a canonical header.
#[cfg(feature = "metrics")]
pub fn append_jsonl_sample_row(
    path: &Path,
    header: &Value,
    sample: &Value,
) -> Result<(), std::io::Error> {
    ensure_single_header(path, header)?;

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, sample).map_err(std::io::Error::other)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    #[test]
    fn node_id_inference_prefers_parent_dir_over_file_stem() {
        let path = PathBuf::from("/tmp/sender-0001/network-metrics.jsonl");
        assert_eq!(infer_node_id_from_path(&path), "sender-0001");
    }

    #[test]
    fn writes_single_header_then_compact_samples() {
        let parent_name = format!(
            "mctx_jsonl_header_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        );
        let parent = std::env::temp_dir().join(&parent_name);
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join("network.jsonl");

        let mut flags = Map::new();
        flags.insert("role".to_string(), Value::String("sender".to_string()));
        let header = header_json(
            NETWORK_ARTIFACT_TYPE,
            "mctx-core/test",
            &infer_node_id_from_path(&path),
            SystemTime::UNIX_EPOCH + Duration::from_secs(10),
            &flags,
        );
        let sample1 = json!({"ts": 11.0, "interval_secs": 1.0, "packets_sent_total": 5});
        let sample2 = json!({"ts": 12.0, "interval_secs": 1.0, "packets_sent_total": 10});

        append_jsonl_sample_row(&path, &header, &sample1).unwrap();
        append_jsonl_sample_row(&path, &header, &sample2).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        let lines = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();

        assert_eq!(lines.len(), 3);

        let parsed_header: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed_header["schema"], HEIMDALL_JSONL_SCHEMA);
        assert_eq!(parsed_header["artifact_type"], NETWORK_ARTIFACT_TYPE);
        assert_eq!(parsed_header["node_id"], parent_name);
        assert!(parsed_header["flags"].is_object());

        for sample_line in &lines[1..] {
            let sample: Value = serde_json::from_str(sample_line).unwrap();
            assert!(sample.get("schema").is_none());
            assert!(sample.get("artifact_type").is_none());
            assert!(sample.get("node_id").is_none());
            assert!(sample.get("producer").is_none());
            assert!(sample.get("flags").is_none());
        }

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(parent);
    }

    #[test]
    fn invalid_first_line_header_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_invalid_header_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        fs::write(&path, "{\"ts\":1.0,\"interval_secs\":1.0}\n").unwrap();

        let err = validate_existing_header(&path).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = fs::remove_file(path);
    }
}
