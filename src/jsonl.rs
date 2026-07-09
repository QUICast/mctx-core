#[cfg(feature = "metrics")]
use fs2::FileExt;
#[cfg(feature = "metrics")]
use serde_json::{Map, Value, json};
#[cfg(feature = "metrics")]
use std::fs;
#[cfg(feature = "metrics")]
use std::fs::File;
#[cfg(feature = "metrics")]
use std::fs::OpenOptions;
#[cfg(feature = "metrics")]
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
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

/// Stateful JSONL writer that validates or writes the canonical header once,
/// then appends compact sample rows without rescanning the whole file.
#[cfg(feature = "metrics")]
#[derive(Debug)]
pub struct MetricsJsonlWriter {
    file: File,
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

/// Returns the first non-empty line from a valid canonical JSONL file, if any.
///
/// Existing non-empty files without a valid single header and valid sample rows
/// are rejected rather than exposed as headerless input.
#[cfg(feature = "metrics")]
pub fn first_non_empty_line(path: &Path) -> Result<Option<String>, std::io::Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    validate_existing_header_reader(BufReader::new(file))
        .map(|header| header.map(|header| header.line))
}

/// Validates the existing first non-empty JSONL line as a Heimdall header.
#[cfg(feature = "metrics")]
pub fn validate_existing_header(path: &Path) -> Result<Option<Value>, std::io::Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    validate_existing_header_reader(BufReader::new(file))
        .map(|header| header.map(|header| header.value))
}

#[cfg(feature = "metrics")]
struct ValidatedJsonlHeader {
    value: Value,
    line: String,
}

#[cfg(feature = "metrics")]
fn validate_existing_header_reader(
    reader: impl BufRead,
) -> Result<Option<ValidatedJsonlHeader>, std::io::Error> {
    let mut non_empty_line_index = 0usize;
    let mut parsed_header = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        non_empty_line_index += 1;
        let parsed = serde_json::from_str::<Value>(&line).map_err(|err| {
            let message = if non_empty_line_index == 1 {
                format!("existing JSONL header is invalid JSON: {err}")
            } else {
                format!(
                    "existing JSONL sample line {} is invalid JSON: {err}",
                    non_empty_line_index
                )
            };
            std::io::Error::new(std::io::ErrorKind::InvalidData, message)
        })?;

        if non_empty_line_index == 1 {
            validate_header_object(&parsed).map_err(|message| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("existing JSONL header is invalid: {message}"),
                )
            })?;

            parsed_header = Some(ValidatedJsonlHeader {
                value: parsed,
                line,
            });
            continue;
        }

        if is_header_object(&parsed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "existing JSONL file contains more than one Heimdall header object",
            ));
        }

        validate_sample_row(&parsed).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "existing JSONL sample line {} is invalid: {}",
                    non_empty_line_index, err
                ),
            )
        })?;
    }

    Ok(parsed_header)
}

/// Ensures a JSONL file has exactly one header at the top before samples.
#[cfg(feature = "metrics")]
pub fn ensure_single_header(path: &Path, header: &Value) -> Result<(), std::io::Error> {
    open_jsonl_append_file(path, header).map(|_| ())
}

#[cfg(feature = "metrics")]
fn open_jsonl_append_file(path: &Path, header: &Value) -> Result<File, std::io::Error> {
    validate_header_object(header).map_err(|message| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("requested JSONL header is invalid: {message}"),
        )
    })?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    file.try_lock_exclusive().map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "failed to acquire exclusive JSONL writer lock for {}: {error}",
                path.display()
            ),
        )
    })?;

    file.seek(SeekFrom::Start(0))?;
    let existing =
        validate_existing_header_reader(BufReader::new(&mut file))?.map(|header| header.value);
    match existing {
        Some(existing) => {
            if !headers_are_compatible(&existing, header) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "existing JSONL header does not match the requested schema metadata",
                ));
            }

            Ok(file)
        }
        None => {
            write_json_line(&mut file, header)?;
            Ok(file)
        }
    }
}

#[cfg(feature = "metrics")]
fn write_json_line(file: &mut File, value: &Value) -> Result<(), std::io::Error> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)
}

/// Appends one compact sample row to a JSONL file with a canonical header.
#[cfg(feature = "metrics")]
pub fn append_jsonl_sample_row(
    path: &Path,
    header: &Value,
    sample: &Value,
) -> Result<(), std::io::Error> {
    validate_sample_row(sample)?;
    let mut file = open_jsonl_append_file(path, header)?;
    write_json_line(&mut file, sample)
}

#[cfg(feature = "metrics")]
impl MetricsJsonlWriter {
    /// Opens or creates a single-header JSONL file for repeated sample appends.
    pub fn open(path: &Path, header: &Value) -> Result<Self, std::io::Error> {
        Ok(Self {
            file: open_jsonl_append_file(path, header)?,
        })
    }

    /// Appends one compact sample row without rescanning the existing file.
    pub fn append_sample_row(&mut self, sample: &Value) -> Result<(), std::io::Error> {
        validate_sample_row(sample)?;
        write_json_line(&mut self.file, sample)
    }
}

#[cfg(feature = "metrics")]
fn headers_are_compatible(existing: &Value, expected: &Value) -> bool {
    let comparable_keys = ["schema", "artifact_type", "node_id", "producer", "flags"];
    comparable_keys
        .into_iter()
        .all(|key| existing.get(key) == expected.get(key))
}

#[cfg(feature = "metrics")]
fn is_header_object(value: &Value) -> bool {
    validate_header_object(value).is_ok()
}

#[cfg(feature = "metrics")]
fn validate_header_object(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "header must be a JSON object".to_string())?;
    if object.get("schema").and_then(Value::as_str) != Some(HEIMDALL_JSONL_SCHEMA) {
        return Err(format!("schema must be `{HEIMDALL_JSONL_SCHEMA}`"));
    }

    for field in ["artifact_type", "node_id", "producer"] {
        if object
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!("{field} must be a non-empty string"));
        }
    }

    if !object
        .get("created_at")
        .and_then(Value::as_f64)
        .is_some_and(f64::is_finite)
    {
        return Err("created_at must be a finite JSON number".to_string());
    }
    if !matches!(object.get("flags"), Some(Value::Object(_))) {
        return Err("flags must be a JSON object".to_string());
    }

    Ok(())
}

#[cfg(feature = "metrics")]
fn validate_sample_row(sample: &Value) -> Result<(), std::io::Error> {
    let Some(sample_object) = sample.as_object() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "JSONL sample rows must be JSON objects",
        ));
    };

    for reserved_key in ["schema", "artifact_type", "node_id", "producer", "flags"] {
        if sample_object.contains_key(reserved_key) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "JSONL sample rows must not contain reserved header field `{reserved_key}`"
                ),
            ));
        }
    }

    for required_number in ["ts", "interval_secs"] {
        if !sample_object
            .get(required_number)
            .and_then(Value::as_f64)
            .is_some_and(f64::is_finite)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("JSONL sample field `{required_number}` must be a finite number"),
            ));
        }
    }

    if sample_object["interval_secs"].as_f64().unwrap_or(-1.0) < 0.0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "JSONL sample field `interval_secs` must not be negative",
        ));
    }

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
        assert_eq!(
            first_non_empty_line(&path).unwrap().as_deref(),
            Some(lines[0])
        );

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
        assert_eq!(
            first_non_empty_line(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn existing_header_without_created_at_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_missing_created_at_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        fs::write(
            &path,
            r#"{"schema":"heimdall-jsonl-v1","artifact_type":"mctx-network","node_id":"sender-a","producer":"mctx-core/test","flags":{}}
"#,
        )
        .unwrap();

        let err = validate_existing_header(&path).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_requested_header_is_rejected_before_file_creation() {
        let path = std::env::temp_dir().join(format!(
            "mctx_invalid_requested_header_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));

        let err = MetricsJsonlWriter::open(&path, &json!({"not": "a header"})).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!path.exists());
    }

    #[test]
    fn mismatched_existing_header_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_mismatched_header_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header1 = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });
        let header2 = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-b",
            "producer": "mctx-core/test",
            "created_at": 2.0,
            "flags": {"role": "sender"},
        });

        append_jsonl_sample_row(&path, &header1, &json!({"ts": 1.0, "interval_secs": 1.0}))
            .unwrap();
        let err =
            append_jsonl_sample_row(&path, &header2, &json!({"ts": 2.0, "interval_secs": 1.0}))
                .unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn additional_header_line_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_extra_header_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });
        let contents = format!(
            "{}\n{}\n{}\n",
            serde_json::to_string(&header).unwrap(),
            serde_json::to_string(&json!({"ts": 1.0, "interval_secs": 1.0})).unwrap(),
            serde_json::to_string(&header).unwrap(),
        );
        fs::write(&path, contents).unwrap();

        let err = validate_existing_header(&path).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn sample_rows_with_reserved_header_fields_are_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_reserved_sample_fields_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });
        let err = append_jsonl_sample_row(
            &path,
            &header,
            &json!({"ts": 1.0, "interval_secs": 1.0, "schema": HEIMDALL_JSONL_SCHEMA}),
        )
        .unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn existing_sample_rows_with_reserved_header_fields_are_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mctx_reserved_existing_sample_fields_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });
        let contents = format!(
            "{}\n{}\n",
            serde_json::to_string(&header).unwrap(),
            serde_json::to_string(
                &json!({"ts": 1.0, "interval_secs": 1.0, "schema": HEIMDALL_JSONL_SCHEMA})
            )
            .unwrap(),
        );
        fs::write(&path, contents).unwrap();

        let err = validate_existing_header(&path).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn stateful_writer_appends_without_repeating_the_header() {
        let path = std::env::temp_dir().join(format!(
            "mctx_stateful_writer_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });

        let mut writer = MetricsJsonlWriter::open(&path, &header).unwrap();
        writer
            .append_sample_row(&json!({"ts": 1.0, "interval_secs": 1.0}))
            .unwrap();
        writer
            .append_sample_row(&json!({"ts": 2.0, "interval_secs": 1.0}))
            .unwrap();
        drop(writer);

        let contents = fs::read_to_string(&path).unwrap();
        let lines = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert!(is_header_object(&serde_json::from_str(lines[0]).unwrap()));
        assert!(!is_header_object(&serde_json::from_str(lines[1]).unwrap()));
        assert!(!is_header_object(&serde_json::from_str(lines[2]).unwrap()));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn second_stateful_writer_cannot_race_the_active_writer() {
        let path = std::env::temp_dir().join(format!(
            "mctx_locked_writer_{}.jsonl",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos()
        ));
        let header = json!({
            "schema": HEIMDALL_JSONL_SCHEMA,
            "artifact_type": NETWORK_ARTIFACT_TYPE,
            "node_id": "sender-a",
            "producer": "mctx-core/test",
            "created_at": 1.0,
            "flags": {"role": "sender"},
        });
        let writer = MetricsJsonlWriter::open(&path, &header).unwrap();

        let err = MetricsJsonlWriter::open(&path, &header).unwrap_err();

        assert!(matches!(
            err.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
        ));
        drop(writer);
        MetricsJsonlWriter::open(&path, &header).unwrap();
        let _ = fs::remove_file(path);
    }
}
