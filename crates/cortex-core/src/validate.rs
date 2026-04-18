//! Runtime JSON-Schema validator for events.
//!
//! Schemas under `schemas/` are embedded at compile time so validation works
//! without any on-disk lookup in downstream crates.

use crate::events::Kind;
use jsonschema::Validator as JsonValidator;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::collections::HashMap;

/// Schema validation failure.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ValidationError {
    /// A JSON pointer-style error path from the underlying validator.
    #[error("{path}: {message}")]
    Schema {
        /// Instance pointer into the document.
        path: String,
        /// Validator message.
        message: String,
    },
    /// Envelope's declared `kind` is unknown.
    #[error("unknown kind: {0}")]
    UnknownKind(String),
    /// Envelope is missing `kind`.
    #[error("envelope missing `kind`")]
    MissingKind,
    /// Envelope is missing `payload`.
    #[error("envelope missing `payload`")]
    MissingPayload,
}

const ENVELOPE_SCHEMA: &str = include_str!("../schemas/envelope.schema.json");
const CONTEXT_SCHEMA: &str = include_str!("../schemas/context.schema.json");
const TURN_SCHEMA: &str = include_str!("../schemas/kinds/turn.schema.json");
const TOOL_CALL_SCHEMA: &str = include_str!("../schemas/kinds/tool_call.schema.json");
const AGENT_CALL_SCHEMA: &str = include_str!("../schemas/kinds/agent_call.schema.json");
const MEMORY_SCHEMA: &str = include_str!("../schemas/kinds/memory.schema.json");
const DECISION_SCHEMA: &str = include_str!("../schemas/kinds/decision.schema.json");
const ANALYSIS_SCHEMA: &str = include_str!("../schemas/kinds/analysis.schema.json");
const LAW_VIOLATION_SCHEMA: &str = include_str!("../schemas/kinds/law_violation.schema.json");
const ARTIFACT_SCHEMA: &str = include_str!("../schemas/kinds/artifact.schema.json");

static VALIDATOR: Lazy<Validator> = Lazy::new(|| {
    Validator::new().expect("embedded schemas must always compile")
});

/// Compiled schema set used to validate envelopes + per-kind payloads.
pub struct Validator {
    envelope: JsonValidator,
    /// Map kind-stem → compiled schema.
    kinds: HashMap<&'static str, JsonValidator>,
}

impl Validator {
    /// Compile every embedded schema. Expensive — call once and share.
    pub fn new() -> Result<Self, String> {
        // The envelope schema `$ref`s the context schema by relative URI. We
        // pre-register it so the validator resolves in-process.
        let context_uri = "context.schema.json";
        let context_value: Value = serde_json::from_str(CONTEXT_SCHEMA)
            .map_err(|e| format!("context schema parse: {e}"))?;

        let envelope_value: Value = serde_json::from_str(ENVELOPE_SCHEMA)
            .map_err(|e| format!("envelope schema parse: {e}"))?;
        let envelope = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .with_resource(
                context_uri,
                jsonschema::Resource::from_contents(context_value)
                    .map_err(|e| format!("context resource: {e}"))?,
            )
            .build(&envelope_value)
            .map_err(|e| format!("envelope schema compile: {e}"))?;

        let kinds: HashMap<&'static str, JsonValidator> = [
            ("turn", compile(TURN_SCHEMA)?),
            ("tool_call", compile(TOOL_CALL_SCHEMA)?),
            ("agent_call", compile(AGENT_CALL_SCHEMA)?),
            ("memory", compile(MEMORY_SCHEMA)?),
            ("decision", compile(DECISION_SCHEMA)?),
            ("analysis", compile(ANALYSIS_SCHEMA)?),
            ("law_violation", compile(LAW_VIOLATION_SCHEMA)?),
            ("artifact", compile(ARTIFACT_SCHEMA)?),
        ]
        .into_iter()
        .collect();

        Ok(Validator { envelope, kinds })
    }

    /// Return the shared, lazily-compiled validator.
    pub fn shared() -> &'static Validator {
        &VALIDATOR
    }

    /// Validate just the envelope shape (no payload inspection).
    pub fn validate_envelope(&self, value: &Value) -> Result<(), Vec<ValidationError>> {
        collect(&self.envelope, value)
    }

    /// Validate an envelope plus the matching per-kind payload.
    pub fn validate_event(&self, value: &Value) -> Result<(), Vec<ValidationError>> {
        let mut errors = collect(&self.envelope, value).err().unwrap_or_default();

        let kind_val = value.get("kind").and_then(|k| k.as_str());
        let payload = value.get("payload");

        match (kind_val, payload) {
            (Some(k), Some(p)) => {
                if let Some(schema) = self.kinds.get(k) {
                    if let Err(mut payload_errors) = collect(schema, p) {
                        // Re-root the path so callers can see it lives under `payload.*`.
                        for e in &mut payload_errors {
                            if let ValidationError::Schema { path, .. } = e {
                                *path = if path == "/" || path.is_empty() {
                                    "/payload".into()
                                } else {
                                    format!("/payload{}", path)
                                };
                            }
                        }
                        errors.append(&mut payload_errors);
                    }
                } else {
                    errors.push(ValidationError::UnknownKind(k.to_string()));
                }
            }
            (None, _) => errors.push(ValidationError::MissingKind),
            (_, None) => errors.push(ValidationError::MissingPayload),
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Validate an event against the shared [`Validator`].
pub fn validate_event(value: &Value) -> Result<(), Vec<ValidationError>> {
    Validator::shared().validate_event(value)
}

/// Validate an envelope's shape against the shared [`Validator`] (no payload check).
pub fn validate_envelope(value: &Value) -> Result<(), Vec<ValidationError>> {
    Validator::shared().validate_envelope(value)
}

/// Compile a JSON Schema draft 2020-12 document.
fn compile(src: &str) -> Result<JsonValidator, String> {
    let value: Value = serde_json::from_str(src).map_err(|e| format!("schema parse: {e}"))?;
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&value)
        .map_err(|e| format!("schema compile: {e}"))
}

fn collect(schema: &JsonValidator, value: &Value) -> Result<(), Vec<ValidationError>> {
    let mut out = Vec::new();
    for err in schema.iter_errors(value) {
        out.push(ValidationError::Schema {
            path: err.instance_path.to_string(),
            message: err.to_string(),
        });
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

/// Shorthand: return the schema-file stem for a [`Kind`].
pub fn schema_stem_for(kind: Kind) -> &'static str {
    kind.schema_stem()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_envelope() -> Value {
        json!({
            "event_id": "01HXYZABCDEF0123456789ABCD",
            "schema_version": "1",
            "occurred_at": "2026-04-17T12:34:56.789Z",
            "session_id": "01HXYZABCDEF0123456789ABCD",
            "stream": "live",
            "tool": "claude-code",
            "kind": "tool_call",
            "context": { "platform": "linux" },
            "payload": {
                "tool_name": "Bash",
                "input": { "command": "ls" },
                "outcome": "success"
            },
            "content_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        })
    }

    #[test]
    fn valid_event_passes() {
        validate_event(&fresh_envelope()).expect("valid event should pass");
    }

    #[test]
    fn missing_required_field_fails() {
        let mut v = fresh_envelope();
        v.as_object_mut().unwrap().remove("content_hash");
        let errors = validate_event(&v).unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn unknown_kind_fails() {
        let mut v = fresh_envelope();
        v["kind"] = json!("nonsense_kind");
        let errors = validate_event(&v).unwrap_err();
        assert!(errors.iter().any(|e| matches!(e, ValidationError::Schema { .. })));
    }

    #[test]
    fn payload_shape_mismatch_fails() {
        let mut v = fresh_envelope();
        v["payload"] = json!({ "tool_name": "Bash" });
        let errors = validate_event(&v).unwrap_err();
        assert!(errors.iter().any(|e| match e {
            ValidationError::Schema { path, .. } => path.starts_with("/payload"),
            _ => false,
        }));
    }

    #[test]
    fn bad_content_hash_fails() {
        let mut v = fresh_envelope();
        v["content_hash"] = json!("md5:abc");
        let errors = validate_event(&v).unwrap_err();
        assert!(!errors.is_empty());
    }
}
