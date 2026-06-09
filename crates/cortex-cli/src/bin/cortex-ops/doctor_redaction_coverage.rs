//! Phase15e §4 — `cortex-ops doctor-redaction-coverage`.
//!
//! Samples 100 recent envelopes from the `cortex.events.raw` Synap stream
//! and runs `PATTERN_CATALOG_V1` against each payload JSON.  Any pattern
//! match is reported as an `unredacted-candidate` with field path + byte
//! offset + a truncated preview (first 16 chars + sha256 hash of the full
//! match so the operator can cross-reference without logging the secret).
//!
//! Exit 0 = zero candidates found.
//! Exit 1 = transport / runtime error.
//! Exit 2 = at least one unredacted-candidate found.

use std::process::ExitCode;

use cortex_core::redact::PATTERN_CATALOG_V1;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use synap_sdk::{SynapClient, SynapConfig};

const STREAM_RAW: &str = "cortex.events.raw";
const SAMPLE_SIZE: usize = 100;

#[derive(Debug, Clone, Serialize)]
struct Candidate {
    envelope_offset: u64,
    field_path: String,
    class: &'static str,
    byte_offset: usize,
    length: usize,
    preview: String,
}

pub fn run(synap_url: Option<String>, stream: Option<String>, json: bool) -> ExitCode {
    let synap_url = synap_url
        .or_else(|| std::env::var("CORTEX_SYNAP_URL").ok())
        .or_else(|| std::env::var("SYNAP_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17003".to_string());
    let stream_name = stream.unwrap_or_else(|| STREAM_RAW.to_string());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let events = match runtime.block_on(async {
        let client = SynapClient::new(SynapConfig::new(&synap_url))?;
        let streams = client.stream();

        let stats = streams.stats(&stream_name).await?;
        let start = stats
            .max_offset
            .saturating_sub((SAMPLE_SIZE as u64).saturating_sub(1));

        streams
            .consume(&stream_name, Some(start), Some(SAMPLE_SIZE))
            .await
            .map_err(anyhow::Error::from)
    }) {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "ERROR: sample Synap stream {stream_name} at {synap_url}: {e}"
            );
            return ExitCode::from(1);
        }
    };

    let sampled = events.len();
    let mut candidates: Vec<Candidate> = Vec::new();

    for event in &events {
        scan_value(&event.data, "", event.offset, &mut candidates);
    }

    if json {
        let report = serde_json::json!({
            "stream": stream_name,
            "sampled": sampled,
            "candidates": candidates,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!(
            "doctor redaction-coverage: stream={stream_name} synap={synap_url} sampled={sampled}"
        );
        if candidates.is_empty() {
            println!("OK: 0 unredacted candidates");
        } else {
            println!(
                "WARN: {} unredacted-candidate(s) found",
                candidates.len()
            );
            for c in &candidates {
                println!(
                    "  unredacted-candidate  class={}  envelope_offset={}  field={}  byte_offset={}  len={}  preview={}",
                    c.class,
                    c.envelope_offset,
                    c.field_path,
                    c.byte_offset,
                    c.length,
                    c.preview,
                );
            }
        }
    }

    if candidates.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

fn scan_value(value: &Value, path: &str, envelope_offset: u64, out: &mut Vec<Candidate>) {
    match value {
        Value::String(s) => {
            for p in PATTERN_CATALOG_V1.iter() {
                for m in p.regex.find_iter(s) {
                    out.push(Candidate {
                        envelope_offset,
                        field_path: path.to_string(),
                        class: p.class,
                        byte_offset: m.start(),
                        length: m.len(),
                        preview: make_preview(&s[m.start()..m.end()]),
                    });
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let child = if path.is_empty() {
                    format!("[{i}]")
                } else {
                    format!("{path}[{i}]")
                };
                scan_value(item, &child, envelope_offset, out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let child = if path.is_empty() {
                    k.to_string()
                } else {
                    format!("{path}.{k}")
                };
                scan_value(v, &child, envelope_offset, out);
            }
        }
        _ => {}
    }
}

/// First 16 chars of the matched text, followed by the first 8 hex digits of
/// sha256(full match) so the operator can identify duplicates without the
/// full secret appearing in logs.
fn make_preview(text: &str) -> String {
    let trunc: String = text.chars().take(16).collect();
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    let digest = h.finalize();
    let hash8: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    format!("{trunc}...<{hash8}>")
}
