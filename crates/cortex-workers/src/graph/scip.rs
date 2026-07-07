//! Phase27d §2.1-§2.3 — SCIP ingestion (precise cross-file references).
//!
//! `rust-analyzer scip .` emits a **binary protobuf** index (`index.scip`),
//! not JSON as the original task wording assumed — see
//! `.rulebook/tasks/phase27d_scip-precise-extraction/design.md` (captured
//! from real `rust-analyzer 1.96.0` output via `protoc --decode_raw`) for
//! the authoritative field-number map. This module hand-rolls the small
//! subset of the protobuf wire format the SCIP `Index` message needs
//! (varint + length-delimited decoding, ~60 lines) instead of depending on
//! `prost` + a generated `scip.proto` — the schema surface we consume is
//! tiny and fixed, so a full protobuf codegen dependency is not worth the
//! extra build step for the bootstrap/CI path (§2.4).
//!
//! Three pieces:
//!
//! 1. **Parser** ([`parse_scip_bytes`]) — decodes the wire format into
//!    [`ScipIndex`] / [`ScipDocument`] / [`ScipOccurrence`] /
//!    [`ScipSymbolInfo`], plus the SCIP symbol-string grammar
//!    ([`parse_symbol`]).
//! 2. **Two-pass resolver** ([`scip_to_patch`]) — pass 1 collects every
//!    definition into a qualified-name index and emits `:Symbol` +
//!    `DEFINES` upserts; pass 2 walks references, resolves each to its
//!    exact target (or a `:ScipExternal` stub, §2.3), and emits `CALLS`
//!    / `REFERENCES` edges tagged [`EdgeConfidence::Extracted`].
//! 3. **External stubs** — any reference whose qualified name is not
//!    defined inside this index (i.e. it belongs to a different crate)
//!    resolves to a `:ScipExternal` node keyed on the full package
//!    identity + descriptors, so an edge is never left dangling.
//!
//! Resolution key: definitions and references are matched by their
//! **qualified name** (see [`ParsedSymbol::qualified_name`]), not by raw
//! SCIP symbol-string equality. This is required by design.md's DEFINES
//! note: rust-analyzer emits a struct's own type symbol as `Type#` but
//! reuses `impl#[Type]` (no trailing member) for bare `Self`/type
//! references inside the impl block. Both collapse to the same
//! qualified name (`Type`), so keying the resolver on qualified name
//! unifies them for free instead of needing a special-cased symbol
//! rewrite.

use std::collections::BTreeMap;
use std::collections::HashMap;

use serde_json::Value;
use thiserror::Error;

use super::analyzer::pending_artifact_id;
use super::identity::{artifact_natural_key, symbol_natural_key};
use super::patch::{EdgeConfidence, EdgeOp, GraphPatch, NodeOp};

/// Failure modes raised while decoding a SCIP protobuf index.
#[derive(Debug, Error)]
pub enum ScipError {
    /// The buffer ended in the middle of a varint, tag, or
    /// length-delimited field.
    #[error("unexpected end of SCIP protobuf input")]
    UnexpectedEof,
    /// A varint continued past 64 bits without terminating — the input
    /// is not valid protobuf.
    #[error("varint exceeds 64 bits")]
    VarintTooLong,
    /// A protobuf wire type outside the four this parser understands
    /// (varint, 64-bit, length-delimited, 32-bit).
    #[error("unsupported protobuf wire type {0}")]
    UnsupportedWireType(u8),
    /// A field was expected to have a specific wire type (e.g. `symbol`
    /// is always length-delimited) but the input carried another.
    #[error("expected protobuf wire type {expected} but found {actual}")]
    UnexpectedWireType {
        /// Wire type this parser needed to decode the field.
        expected: u8,
        /// Wire type actually present in the input.
        actual: u8,
    },
    /// An `Occurrence.range` (or `enclosing_range`) packed-varint field
    /// decoded to a length other than 3 (same-line shorthand) or 4
    /// (full `[start_line, start_char, end_line, end_char]`).
    #[error("occurrence range has {0} packed varints (expected 3 or 4)")]
    InvalidRange(usize),
    /// An `Occurrence` was missing its required `range` field.
    #[error("occurrence is missing its required range field")]
    MissingRange,
}

/// Minimal hand-rolled protobuf wire-format cursor over a byte slice.
/// Only understands the four wire types SCIP's schema uses.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn read_u8(&mut self) -> Result<u8, ScipError> {
        let byte = *self.buf.get(self.pos).ok_or(ScipError::UnexpectedEof)?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_varint(&mut self) -> Result<u64, ScipError> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        loop {
            if shift >= 64 {
                return Err(ScipError::VarintTooLong);
            }
            let byte = self.read_u8()?;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(result)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], ScipError> {
        let end = self.pos.checked_add(len).ok_or(ScipError::UnexpectedEof)?;
        if end > self.buf.len() {
            return Err(ScipError::UnexpectedEof);
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Reads a `(field_number, wire_type)` tag.
    fn read_tag(&mut self) -> Result<(u32, u8), ScipError> {
        let v = self.read_varint()?;
        Ok(((v >> 3) as u32, (v & 0x7) as u8))
    }

    /// Skips a field's value given its wire type, without interpreting it.
    fn skip_field(&mut self, wire_type: u8) -> Result<(), ScipError> {
        match wire_type {
            0 => {
                self.read_varint()?;
            }
            1 => {
                self.read_bytes(8)?;
            }
            2 => {
                let len = self.read_varint()? as usize;
                self.read_bytes(len)?;
            }
            5 => {
                self.read_bytes(4)?;
            }
            other => return Err(ScipError::UnsupportedWireType(other)),
        }
        Ok(())
    }
}

/// Reads a length-delimited (wire type 2) field's payload bytes.
fn read_length_delimited<'a>(r: &mut Reader<'a>, wire_type: u8) -> Result<&'a [u8], ScipError> {
    if wire_type != 2 {
        return Err(ScipError::UnexpectedWireType {
            expected: 2,
            actual: wire_type,
        });
    }
    let len = r.read_varint()? as usize;
    r.read_bytes(len)
}

/// Reads a varint (wire type 0) field's value.
fn read_varint_field(r: &mut Reader, wire_type: u8) -> Result<u64, ScipError> {
    if wire_type != 0 {
        return Err(ScipError::UnexpectedWireType {
            expected: 0,
            actual: wire_type,
        });
    }
    r.read_varint()
}

fn utf8_owned(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A parsed SCIP index — the top-level `Index` protobuf message.
#[derive(Debug, Clone, Default)]
pub struct ScipIndex {
    /// Name of the tool that produced the index (e.g. `"rust-analyzer"`).
    pub tool_name: String,
    /// Version string of the producing tool.
    pub tool_version: String,
    /// `file://` URI of the indexed project root.
    pub project_root: String,
    /// Every source document the index covers.
    pub documents: Vec<ScipDocument>,
}

/// One indexed source file (`Document` message).
#[derive(Debug, Clone, Default)]
pub struct ScipDocument {
    /// Repo-relative path, normalized to forward slashes (design.md
    /// gotcha #1 — rust-analyzer emits `\\`-separated paths on Windows).
    pub relative_path: String,
    /// Language identifier (e.g. `"rust"`).
    pub language: String,
    /// Every occurrence (definition or reference) in this document.
    pub occurrences: Vec<ScipOccurrence>,
    /// Every `SymbolInformation` entry (doc comments, display names)
    /// this document declares.
    pub symbols: Vec<ScipSymbolInfo>,
}

/// A `(start_line, start_char)`–`(end_line, end_char)` span inside a
/// document, decoded from SCIP's packed-varint range encoding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScipRange {
    /// Zero-indexed start line.
    pub start_line: u32,
    /// Zero-indexed start column (UTF-16 code units per SCIP spec).
    pub start_char: u32,
    /// Zero-indexed end line.
    pub end_line: u32,
    /// Zero-indexed end column.
    pub end_char: u32,
}

/// One occurrence of a symbol (definition or reference) inside a
/// document.
#[derive(Debug, Clone)]
pub struct ScipOccurrence {
    /// Span of this occurrence itself.
    pub range: ScipRange,
    /// Raw SCIP symbol string (see [`parse_symbol`]).
    pub symbol: String,
    /// Bitfield of `SymbolRole` flags (`Definition = 1`, `Import = 2`,
    /// `WriteAccess = 4`, `ReadAccess = 8`, `Generated = 16`, `Test =
    /// 32`). Use [`is_definition`] rather than comparing for equality —
    /// this is a bitfield, not an enum (design.md gotcha #3).
    pub roles: u32,
    /// Span of the definition's full body (only present on definition
    /// occurrences), used by the resolver to find which definition
    /// encloses a given reference.
    pub enclosing_range: Option<ScipRange>,
}

/// Metadata about one declared symbol (`SymbolInformation` message).
#[derive(Debug, Clone, Default)]
pub struct ScipSymbolInfo {
    /// Raw SCIP symbol string this metadata describes.
    pub symbol: String,
    /// Human-readable display name (e.g. `"Worker"`, `"new"`).
    pub display_name: String,
    /// For `local N` symbols, the symbol string of the containing
    /// definition (function/method) — `None` for global symbols.
    pub enclosing_symbol: Option<String>,
    /// Doc-comment text attached to the symbol, if any.
    pub documentation: Vec<String>,
}

/// Bitfield check for the `Definition` role (design.md gotcha #3 — this
/// is a bitfield, so callers must never test `roles == 1`).
#[must_use]
pub fn is_definition(roles: u32) -> bool {
    roles & 1 != 0
}

/// Decodes a packed-varint range field. SCIP shortens same-line ranges
/// to 3 elements (`[line, start_char, end_char]`); a 4-element range is
/// `[start_line, start_char, end_line, end_char]` (design.md gotcha #2).
fn decode_range(buf: &[u8]) -> Result<ScipRange, ScipError> {
    let mut r = Reader::new(buf);
    let mut values = Vec::new();
    while !r.is_empty() {
        values.push(r.read_varint()?);
    }
    match values.as_slice() {
        [line, start_char, end_char] => Ok(ScipRange {
            start_line: *line as u32,
            start_char: *start_char as u32,
            end_line: *line as u32,
            end_char: *end_char as u32,
        }),
        [start_line, start_char, end_line, end_char] => Ok(ScipRange {
            start_line: *start_line as u32,
            start_char: *start_char as u32,
            end_line: *end_line as u32,
            end_char: *end_char as u32,
        }),
        other => Err(ScipError::InvalidRange(other.len())),
    }
}

fn parse_tool_info(buf: &[u8]) -> Result<(String, String), ScipError> {
    let mut r = Reader::new(buf);
    let mut name = String::new();
    let mut version = String::new();
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            1 => name = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            2 => version = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok((name, version))
}

fn parse_metadata(buf: &[u8]) -> Result<(String, String, String), ScipError> {
    let mut r = Reader::new(buf);
    let mut tool_name = String::new();
    let mut tool_version = String::new();
    let mut project_root = String::new();
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            2 => {
                let sub = read_length_delimited(&mut r, wire_type)?;
                let (name, version) = parse_tool_info(sub)?;
                tool_name = name;
                tool_version = version;
            }
            3 => project_root = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok((tool_name, tool_version, project_root))
}

fn parse_occurrence(buf: &[u8]) -> Result<ScipOccurrence, ScipError> {
    let mut r = Reader::new(buf);
    let mut range: Option<ScipRange> = None;
    let mut symbol = String::new();
    let mut roles: u32 = 0;
    let mut enclosing_range: Option<ScipRange> = None;
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            1 => range = Some(decode_range(read_length_delimited(&mut r, wire_type)?)?),
            2 => symbol = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            3 => roles = read_varint_field(&mut r, wire_type)? as u32,
            7 => {
                enclosing_range = Some(decode_range(read_length_delimited(&mut r, wire_type)?)?);
            }
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok(ScipOccurrence {
        range: range.ok_or(ScipError::MissingRange)?,
        symbol,
        roles,
        enclosing_range,
    })
}

fn parse_symbol_information(buf: &[u8]) -> Result<ScipSymbolInfo, ScipError> {
    let mut r = Reader::new(buf);
    let mut symbol = String::new();
    let mut display_name = String::new();
    let mut enclosing_symbol: Option<String> = None;
    let mut documentation = Vec::new();
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            1 => symbol = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            3 => documentation.push(utf8_owned(read_length_delimited(&mut r, wire_type)?)),
            6 => display_name = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            8 => enclosing_symbol = Some(utf8_owned(read_length_delimited(&mut r, wire_type)?)),
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok(ScipSymbolInfo {
        symbol,
        display_name,
        enclosing_symbol,
        documentation,
    })
}

/// Normalizes a SCIP `relative_path` to forward slashes (design.md
/// gotcha #1 — rust-analyzer emits `\\`-separated paths on Windows).
fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn parse_document(buf: &[u8]) -> Result<ScipDocument, ScipError> {
    let mut r = Reader::new(buf);
    let mut relative_path = String::new();
    let mut language = String::new();
    let mut occurrences = Vec::new();
    let mut symbols = Vec::new();
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            1 => relative_path = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            2 => occurrences.push(parse_occurrence(read_length_delimited(&mut r, wire_type)?)?),
            3 => symbols.push(parse_symbol_information(read_length_delimited(
                &mut r, wire_type,
            )?)?),
            4 => language = utf8_owned(read_length_delimited(&mut r, wire_type)?),
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok(ScipDocument {
        relative_path: normalize_relative_path(&relative_path),
        language,
        occurrences,
        symbols,
    })
}

/// Decodes a SCIP protobuf index (`rust-analyzer scip .`'s `index.scip`
/// output) into the in-memory [`ScipIndex`] model.
pub fn parse_scip_bytes(bytes: &[u8]) -> Result<ScipIndex, ScipError> {
    let mut r = Reader::new(bytes);
    let mut tool_name = String::new();
    let mut tool_version = String::new();
    let mut project_root = String::new();
    let mut documents = Vec::new();
    while !r.is_empty() {
        let (field, wire_type) = r.read_tag()?;
        match field {
            1 => {
                let sub = read_length_delimited(&mut r, wire_type)?;
                let (name, version, root) = parse_metadata(sub)?;
                tool_name = name;
                tool_version = version;
                project_root = root;
            }
            2 => {
                let sub = read_length_delimited(&mut r, wire_type)?;
                documents.push(parse_document(sub)?);
            }
            _ => r.skip_field(wire_type)?,
        }
    }
    Ok(ScipIndex {
        tool_name,
        tool_version,
        project_root,
        documents,
    })
}

// ---------------------------------------------------------------------
// Symbol grammar
// ---------------------------------------------------------------------

/// A parsed SCIP symbol string — either a document-scoped `local N` or a
/// fully qualified global symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSymbol {
    /// `local N` — scoped to the document it appears in. Never becomes
    /// a graph node (design.md gotcha #4).
    Local(u32),
    /// `<scheme> <manager> <package> <descriptors>`. `package` may be
    /// one or more space-separated tokens (`"name version"` for the
    /// indexed crate itself, `"name url"` for a cross-crate stdlib
    /// reference — design.md's dependency-descriptor case).
    Global {
        /// Symbol scheme, always `"rust-analyzer"` for this indexer.
        scheme: String,
        /// Package manager, always `"cargo"` for this indexer.
        manager: String,
        /// Package identity (name, or `"name version"` / `"name url"`).
        package: String,
        /// The descriptor suffix (e.g. `"storage/Store#"`,
        /// `"impl#[Worker]new()."`).
        descriptors: String,
    },
}

/// Classification of a descriptor string's trailing marker, per the
/// SCIP suffix convention (design.md): `/` namespace, `#` type, `().`
/// method/function, `.` term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescriptorKind {
    Namespace,
    Type,
    Method,
    Term,
    Unknown,
}

/// Classifies a descriptors string by its trailing SCIP suffix marker.
fn classify_descriptors(descriptors: &str) -> DescriptorKind {
    if descriptors.ends_with("().") {
        DescriptorKind::Method
    } else if descriptors.ends_with('#') {
        DescriptorKind::Type
    } else if descriptors.ends_with('/') {
        DescriptorKind::Namespace
    } else if descriptors.ends_with('.') {
        DescriptorKind::Term
    } else {
        DescriptorKind::Unknown
    }
}

/// Strips a leading `[...]` bracket group (non-nested — SCIP's bracketed
/// type-argument lists never contain further `[`/`]`), repeating until
/// none remain. Used to skip generic-argument groups that trail an
/// `impl#[Type]` owner marker (e.g. the `[`Add<Self>`]` trait-arg group
/// in `impl#[usize][`Add<Self>`]add().`).
fn strip_leading_bracket_groups(mut s: &str) -> &str {
    while let Some(rest) = s.strip_prefix('[') {
        match rest.find(']') {
            Some(idx) => s = &rest[idx + 1..],
            None => break,
        }
    }
    s
}

/// Builds a clean dotted/qualified display name from a descriptors
/// string, e.g. `"storage/Store#"` → `"storage::Store"`,
/// `"impl#[Worker]new()."` → `"Worker::new"` (an `impl#[Type]` scope is
/// treated as owner `Type`, per design.md's DEFINES note).
fn qualified_name_from_descriptors(descriptors: &str) -> String {
    let segments: Vec<&str> = descriptors.split('/').collect();
    let last_idx = segments.len().saturating_sub(1);
    let mut parts: Vec<String> = Vec::new();
    for (i, segment) in segments.iter().enumerate() {
        if i != last_idx {
            if !segment.is_empty() {
                parts.push((*segment).to_string());
            }
            continue;
        }
        if let Some(rest) = segment.strip_prefix("impl#[") {
            if let Some(close) = rest.find(']') {
                parts.push(rest[..close].to_string());
                let remainder = strip_leading_bracket_groups(&rest[close + 1..]);
                if !remainder.is_empty() {
                    if let Some(name) = strip_descriptor_suffix(remainder) {
                        parts.push(name);
                    }
                }
                continue;
            }
        }
        if let Some(name) = strip_descriptor_suffix(segment) {
            parts.push(name);
        }
    }
    parts.join("::")
}

/// Strips the trailing SCIP suffix marker from one descriptor segment,
/// returning the bare name (empty names are filtered out by the caller).
fn strip_descriptor_suffix(segment: &str) -> Option<String> {
    let name = if let Some(stripped) = segment.strip_suffix("().") {
        stripped
    } else if let Some(stripped) = segment.strip_suffix(['#', '/', '.']) {
        stripped
    } else {
        segment
    };
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

impl ParsedSymbol {
    /// Clean dotted/qualified display name for this symbol. Locals use
    /// a `local#N` placeholder — callers must skip locals before they
    /// reach graph emission (design.md gotcha #4), so this form is only
    /// ever seen in diagnostics.
    #[must_use]
    pub fn qualified_name(&self) -> String {
        match self {
            ParsedSymbol::Local(n) => format!("local#{n}"),
            ParsedSymbol::Global { descriptors, .. } => {
                qualified_name_from_descriptors(descriptors)
            }
        }
    }
}

/// Parses a raw SCIP symbol string into its grammar components.
///
/// Handles `local N` and the global
/// `<scheme> <manager> <package...> <descriptors>` form, including the
/// cross-crate case where `package` itself space-splits into a name and
/// a URL (design.md's dependency-descriptor note). Descriptors never
/// contain unescaped spaces in rust-analyzer's output, so the last
/// whitespace-separated token is always the full descriptor suffix.
#[must_use]
pub fn parse_symbol(symbol: &str) -> ParsedSymbol {
    if let Some(rest) = symbol.strip_prefix("local ") {
        if let Ok(n) = rest.trim().parse::<u32>() {
            return ParsedSymbol::Local(n);
        }
    }
    let tokens: Vec<&str> = symbol.split(' ').collect();
    if tokens.len() < 4 {
        return ParsedSymbol::Global {
            scheme: String::new(),
            manager: String::new(),
            package: String::new(),
            descriptors: symbol.to_string(),
        };
    }
    let last = tokens.len() - 1;
    ParsedSymbol::Global {
        scheme: tokens[0].to_string(),
        manager: tokens[1].to_string(),
        package: tokens[2..last].join(" "),
        descriptors: tokens[last].to_string(),
    }
}

// ---------------------------------------------------------------------
// Two-pass resolver (§2.2) + external stubs (§2.3)
// ---------------------------------------------------------------------

/// A definition collected during pass 1, keyed by qualified name in the
/// resolver's index.
struct DefinedSymbol {
    natural_key: String,
    enclosing_range: Option<ScipRange>,
    kind: DescriptorKind,
}

/// Where a reference resolved to.
enum ResolvedRefTarget<'a> {
    /// Resolved to a definition captured in pass 1 (same document or
    /// elsewhere in the index).
    Internal(&'a DefinedSymbol),
    /// Not defined anywhere in this index — a different crate's symbol
    /// (or, defensively, a malformed reference). Stubbed as
    /// `:ScipExternal` so the edge never dangles (§2.3).
    External {
        natural_key: String,
        qualified_name: String,
        kind: DescriptorKind,
    },
}

fn upsert_node(
    patch: &mut GraphPatch,
    label: &str,
    natural_key: &str,
    fill_props: impl FnOnce(&mut BTreeMap<String, Value>),
) {
    if patch
        .nodes
        .iter()
        .any(|n| n.label == label && n.natural_key == natural_key)
    {
        return;
    }
    let mut props = BTreeMap::new();
    fill_props(&mut props);
    let mut node = NodeOp::with_identity(label, natural_key);
    node.props = props;
    patch.nodes.push(node);
}

/// Upserts the `:Artifact` node for `(repo, path)`, using the canonical
/// `repo|path|content_hash` key when `content_hash_for` resolves the
/// file, or the deterministic `pending|repo|path` sentinel otherwise —
/// mirroring [`super::analyzer::patch_builder`]'s honest fallback for an
/// unknown content hash rather than inventing a placeholder value.
fn upsert_artifact(
    patch: &mut GraphPatch,
    repo: &str,
    path: &str,
    content_hash_for: &dyn Fn(&str) -> Option<String>,
) -> String {
    match content_hash_for(path) {
        Some(hash) => {
            let key = artifact_natural_key(repo, path, &hash);
            upsert_node(patch, "Artifact", &key, |props| {
                props.insert("repo".into(), Value::String(repo.to_string()));
                props.insert("path".into(), Value::String(path.to_string()));
                props.insert("content_hash".into(), Value::String(hash));
            });
            key
        }
        None => {
            let key = pending_artifact_id(repo, path);
            upsert_node(patch, "Artifact", &key, |props| {
                props.insert("repo".into(), Value::String(repo.to_string()));
                props.insert("path".into(), Value::String(path.to_string()));
            });
            key
        }
    }
}

fn upsert_symbol_node(patch: &mut GraphPatch, key: &str, qualified_name: &str, source_file: &str) {
    upsert_node(patch, "Symbol", key, |props| {
        props.insert("name".into(), Value::String(qualified_name.to_string()));
        props.insert(
            "qualified_name".into(),
            Value::String(qualified_name.to_string()),
        );
        props.insert("source_file".into(), Value::String(source_file.to_string()));
    });
}

/// Builds an [`EdgeOp`] tagged with the SCIP analyzer's provenance and
/// [`EdgeConfidence::Extracted`] (score + weight `1.0` — SCIP edges are
/// exact, not heuristic).
fn make_scip_edge(
    edge_type: &str,
    from_label: &str,
    from_key: &str,
    to_label: &str,
    to_key: &str,
) -> EdgeOp {
    let mut props = BTreeMap::new();
    props.insert("analyzer".into(), Value::String("scip".into()));
    EdgeOp {
        edge_type: edge_type.to_string(),
        from_label: from_label.to_string(),
        from_key: from_key.to_string(),
        to_label: to_label.to_string(),
        to_key: to_key.to_string(),
        props,
        weight: Some(1.0),
        ..Default::default()
    }
    .with_confidence(EdgeConfidence::Extracted, Some(1.0))
}

fn range_contains(outer: &ScipRange, inner: &ScipRange) -> bool {
    (outer.start_line, outer.start_char) <= (inner.start_line, inner.start_char)
        && (inner.end_line, inner.end_char) <= (outer.end_line, outer.end_char)
}

fn span_size(r: &ScipRange) -> (u32, u32) {
    (r.end_line.saturating_sub(r.start_line), r.end_char)
}

/// Finds the tightest pass-1 definition (by qualified name, restricted
/// to the given document's definitions) whose `enclosing_range`
/// contains `ref_range`. Falls back to `None` when no definition in the
/// document encloses the reference (e.g. a top-level `use` statement),
/// letting the caller anchor the edge on the containing `:Artifact`
/// instead.
fn find_enclosing_definition<'a>(
    ref_range: &ScipRange,
    doc_def_names: &[String],
    global_index: &'a HashMap<String, DefinedSymbol>,
) -> Option<&'a DefinedSymbol> {
    let mut best: Option<(&DefinedSymbol, (u32, u32))> = None;
    for name in doc_def_names {
        let Some(def) = global_index.get(name) else {
            continue;
        };
        let Some(enclosing) = &def.enclosing_range else {
            continue;
        };
        if !range_contains(enclosing, ref_range) {
            continue;
        }
        let span = span_size(enclosing);
        let keep = match &best {
            None => true,
            Some((_, best_span)) => span < *best_span,
        };
        if keep {
            best = Some((def, span));
        }
    }
    best.map(|(def, _)| def)
}

/// Resolves a reference's [`ParsedSymbol`] against the pass-1 index:
/// internal when the qualified name was defined somewhere in this SCIP
/// index (same document or elsewhere), otherwise an external stub
/// identity (§2.3) — a reference whose package differs from the
/// indexed crate's is, by construction, never present in `global_index`
/// (only this crate's own definitions are captured in pass 1), so
/// "not found" and "different package" coincide in practice.
fn resolve_reference_target<'a>(
    parsed: &ParsedSymbol,
    global_index: &'a HashMap<String, DefinedSymbol>,
) -> ResolvedRefTarget<'a> {
    let qualified_name = parsed.qualified_name();
    if let Some(def) = global_index.get(&qualified_name) {
        return ResolvedRefTarget::Internal(def);
    }
    match parsed {
        ParsedSymbol::Global {
            scheme,
            manager,
            package,
            descriptors,
        } => ResolvedRefTarget::External {
            natural_key: format!("scip_external|{scheme}|{manager}|{package}|{descriptors}"),
            kind: classify_descriptors(descriptors),
            qualified_name,
        },
        ParsedSymbol::Local(n) => ResolvedRefTarget::External {
            // Callers filter locals out before calling; stub defensively
            // rather than panic if that invariant is ever violated.
            natural_key: format!("scip_external|local|{n}"),
            kind: DescriptorKind::Unknown,
            qualified_name,
        },
    }
}

/// Converts a parsed [`ScipIndex`] into a [`GraphPatch`] of precise
/// `:Symbol` nodes and `DEFINES` / `CALLS` / `REFERENCES` edges
/// (phase27d §2.2-§2.3).
///
/// `content_hash_for(path)` resolves a document's repo-relative path to
/// its current content hash; `None` falls back to the deterministic
/// `pending|repo|path` artifact sentinel (see [`upsert_artifact`]).
#[must_use]
pub fn scip_to_patch(
    index: &ScipIndex,
    repo: &str,
    content_hash_for: &dyn Fn(&str) -> Option<String>,
) -> GraphPatch {
    let mut patch = GraphPatch::empty();
    let mut global_index: HashMap<String, DefinedSymbol> = HashMap::new();
    let mut defs_in_doc: HashMap<String, Vec<String>> = HashMap::new();

    // ---- Pass 1: definitions ----
    for doc in &index.documents {
        for occ in &doc.occurrences {
            if !is_definition(occ.roles) {
                continue;
            }
            let parsed = parse_symbol(&occ.symbol);
            let ParsedSymbol::Global { descriptors, .. } = &parsed else {
                // `local N` definitions are document-scoped and never
                // become graph nodes (design.md gotcha #4).
                continue;
            };
            let qualified_name = parsed.qualified_name();
            let kind = classify_descriptors(descriptors);
            let symbol_key = symbol_natural_key(repo, &doc.language, &qualified_name);

            upsert_symbol_node(&mut patch, &symbol_key, &qualified_name, &doc.relative_path);
            let artifact_key =
                upsert_artifact(&mut patch, repo, &doc.relative_path, content_hash_for);
            patch.edges.push(make_scip_edge(
                "DEFINES",
                "Artifact",
                &artifact_key,
                "Symbol",
                &symbol_key,
            ));

            defs_in_doc
                .entry(doc.relative_path.clone())
                .or_default()
                .push(qualified_name.clone());
            global_index.entry(qualified_name).or_insert(DefinedSymbol {
                natural_key: symbol_key,
                enclosing_range: occ.enclosing_range,
                kind,
            });
        }
    }

    // ---- Pass 2: references ----
    let empty_defs: Vec<String> = Vec::new();
    for doc in &index.documents {
        let doc_defs = defs_in_doc.get(&doc.relative_path).unwrap_or(&empty_defs);
        for occ in &doc.occurrences {
            if is_definition(occ.roles) {
                continue;
            }
            let parsed = parse_symbol(&occ.symbol);
            if matches!(parsed, ParsedSymbol::Local(_)) {
                continue;
            }

            let (from_label, from_key, from_is_fn) =
                match find_enclosing_definition(&occ.range, doc_defs, &global_index) {
                    Some(def) => (
                        "Symbol",
                        def.natural_key.clone(),
                        def.kind == DescriptorKind::Method,
                    ),
                    None => {
                        let artifact_key =
                            upsert_artifact(&mut patch, repo, &doc.relative_path, content_hash_for);
                        ("Artifact", artifact_key, false)
                    }
                };

            match resolve_reference_target(&parsed, &global_index) {
                ResolvedRefTarget::Internal(def) => {
                    let edge_type = if from_is_fn && def.kind == DescriptorKind::Method {
                        "CALLS"
                    } else {
                        "REFERENCES"
                    };
                    patch.edges.push(make_scip_edge(
                        edge_type,
                        from_label,
                        &from_key,
                        "Symbol",
                        &def.natural_key,
                    ));
                }
                ResolvedRefTarget::External {
                    natural_key,
                    qualified_name,
                    kind,
                } => {
                    upsert_node(&mut patch, "ScipExternal", &natural_key, |props| {
                        props.insert("name".into(), Value::String(qualified_name));
                    });
                    let edge_type = if from_is_fn && kind == DescriptorKind::Method {
                        "CALLS"
                    } else {
                        "REFERENCES"
                    };
                    patch.edges.push(make_scip_edge(
                        edge_type,
                        from_label,
                        &from_key,
                        "ScipExternal",
                        &natural_key,
                    ));
                }
            }
        }
    }

    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] =
        include_bytes!("../../tests/fixtures/scip/rust_analyzer_1_96_fixture.scip");

    fn parse_fixture() -> ScipIndex {
        parse_scip_bytes(FIXTURE).expect("fixture must parse")
    }

    fn node_key<'a>(patch: &'a GraphPatch, label: &str, natural_key: &str) -> Option<&'a NodeOp> {
        patch
            .nodes
            .iter()
            .find(|n| n.label == label && n.natural_key == natural_key)
    }

    // ---------- §2.1 parser ----------

    #[test]
    fn parses_real_fixture_metadata_and_documents() {
        let index = parse_fixture();
        assert_eq!(index.tool_name, "rust-analyzer");
        assert_eq!(index.tool_version, "1.96.0 (ac68faa2 2026-05-25)");
        assert_eq!(index.documents.len(), 2);

        let paths: Vec<&str> = index
            .documents
            .iter()
            .map(|d| d.relative_path.as_str())
            .collect();
        assert!(paths.contains(&"src/lib.rs"), "paths: {paths:?}");
        assert!(paths.contains(&"src/storage.rs"), "paths: {paths:?}");
        for doc in &index.documents {
            assert!(
                !doc.relative_path.contains('\\'),
                "relative_path must be forward-slash normalized: {}",
                doc.relative_path
            );
        }
    }

    #[test]
    fn parses_real_fixture_occurrences_and_ranges() {
        let index = parse_fixture();
        let mut saw_definition = false;
        let mut saw_reference = false;
        let mut occurrence_count = 0usize;

        for doc in &index.documents {
            for occ in &doc.occurrences {
                occurrence_count += 1;
                assert!(
                    occ.range.start_line <= occ.range.end_line,
                    "range must decode sanely: {:?}",
                    occ.range
                );
                if let Some(enclosing) = &occ.enclosing_range {
                    assert!(enclosing.start_line <= enclosing.end_line);
                }
                if is_definition(occ.roles) {
                    saw_definition = true;
                } else {
                    saw_reference = true;
                }
            }
        }

        assert!(occurrence_count > 0);
        assert!(saw_definition, "fixture must contain a definition role");
        assert!(saw_reference, "fixture must contain a plain reference");
    }

    // ---------- symbol grammar ----------

    #[test]
    fn parses_local_symbol() {
        assert_eq!(parse_symbol("local 0"), ParsedSymbol::Local(0));
    }

    #[test]
    fn parses_global_symbol_with_name_and_version_package() {
        let parsed = parse_symbol("rust-analyzer cargo scip-fixture 0.1.0 storage/Store#");
        assert_eq!(
            parsed,
            ParsedSymbol::Global {
                scheme: "rust-analyzer".into(),
                manager: "cargo".into(),
                package: "scip-fixture 0.1.0".into(),
                descriptors: "storage/Store#".into(),
            }
        );
        assert_eq!(parsed.qualified_name(), "storage::Store");
    }

    #[test]
    fn parses_global_symbol_with_cross_crate_url_package_and_method_descriptor() {
        let raw = "rust-analyzer cargo core https://github.com/rust-lang/rust/library/core \
                    ops/arith/impl#[usize][`Add<Self>`]add().";
        let parsed = parse_symbol(raw);
        assert_eq!(
            parsed,
            ParsedSymbol::Global {
                scheme: "rust-analyzer".into(),
                manager: "cargo".into(),
                package: "core https://github.com/rust-lang/rust/library/core".into(),
                descriptors: "ops/arith/impl#[usize][`Add<Self>`]add().".into(),
            }
        );
    }

    #[test]
    fn qualified_name_treats_impl_scope_as_owner_type() {
        let parsed = parse_symbol("rust-analyzer cargo scip-fixture 0.1.0 impl#[Worker]new().");
        assert_eq!(parsed.qualified_name(), "Worker::new");
    }

    // ---------- §2.2/§2.3 resolver ----------

    fn always_hashed(path: &str) -> Option<String> {
        Some(format!("hash-of-{path}"))
    }

    #[test]
    fn resolver_emits_symbol_nodes_for_fixture_definitions() {
        let index = parse_fixture();
        let patch = scip_to_patch(&index, "myrepo", &always_hashed);

        for expected in [
            "myrepo|rust|Worker",
            "myrepo|rust|Worker::new",
            "myrepo|rust|Worker::run",
            "myrepo|rust|storage::Store",
            "myrepo|rust|storage::Store::open",
            "myrepo|rust|storage::Store::count",
            "myrepo|rust|helper",
        ] {
            assert!(
                node_key(&patch, "Symbol", expected).is_some(),
                "expected Symbol node {expected}; nodes: {:?}",
                patch
                    .nodes
                    .iter()
                    .filter(|n| n.label == "Symbol")
                    .map(|n| n.natural_key.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn resolver_emits_cross_module_call_edges_to_store_methods() {
        let index = parse_fixture();
        let patch = scip_to_patch(&index, "myrepo", &always_hashed);

        for target in [
            "myrepo|rust|storage::Store::open",
            "myrepo|rust|storage::Store::count",
        ] {
            let hit = patch.edges.iter().any(|e| {
                (e.edge_type == "CALLS" || e.edge_type == "REFERENCES")
                    && e.to_label == "Symbol"
                    && e.to_key == target
                    && e.from_label == "Symbol"
            });
            assert!(hit, "expected an edge from a Symbol to {target}");
        }
    }

    #[test]
    fn resolver_stamps_extracted_confidence_weight_and_analyzer_on_every_edge() {
        let index = parse_fixture();
        let patch = scip_to_patch(&index, "myrepo", &always_hashed);

        assert!(!patch.edges.is_empty());
        for edge in &patch.edges {
            assert_eq!(
                edge.props.get("confidence"),
                Some(&Value::String("extracted".into())),
                "edge {edge:?} missing extracted confidence"
            );
            assert_eq!(
                edge.props.get("confidence_score").and_then(Value::as_f64),
                Some(1.0)
            );
            assert_eq!(edge.weight, Some(1.0));
            assert_eq!(
                edge.props.get("analyzer"),
                Some(&Value::String("scip".into()))
            );
        }
    }

    #[test]
    fn resolver_stubs_cross_crate_reference_as_scip_external_without_dangling() {
        let index = parse_fixture();
        let patch = scip_to_patch(&index, "myrepo", &always_hashed);

        let external_edge = patch
            .edges
            .iter()
            .find(|e| e.to_label == "ScipExternal")
            .unwrap_or_else(|| panic!("expected at least one ScipExternal edge"));

        assert!(
            node_key(&patch, "ScipExternal", &external_edge.to_key).is_some(),
            "ScipExternal target must have a matching NodeOp (no dangling edge)"
        );
        assert!(
            node_key(
                &patch,
                external_edge.from_label.as_str(),
                &external_edge.from_key
            )
            .is_some(),
            "edge source must have a matching NodeOp (no dangling edge)"
        );
    }

    #[test]
    fn resolver_emits_no_nodes_or_edges_for_locals() {
        let index = parse_fixture();
        let patch = scip_to_patch(&index, "myrepo", &always_hashed);

        for node in &patch.nodes {
            assert!(
                !node.natural_key.contains("local#") && !node.natural_key.contains("|local|"),
                "locals must never become graph nodes: {}",
                node.natural_key
            );
        }
        for edge in &patch.edges {
            assert!(!edge.from_key.contains("local#") && !edge.to_key.contains("local#"));
        }
    }
}
