//! Integration tests for `cortex_workers::embedder::chunker_code`.

use cortex_core::events::Kind;
use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_workers::embedder::chunker_code::OVERSIZE_THRESHOLD_BYTES;
use cortex_workers::embedder::{ChunkSource, Chunker, CodeChunker, EnrichedEvent};
use serde_json::json;

fn make_event(path: &str, content: &str) -> EnrichedEvent {
    EnrichedEvent {
        event_id: "evt_code".into(),
        kind: Kind::ToolCall,
        content_hash: "hash_parent".into(),
        redacted_payload: json!({ "content": content }),
        classifier: ClassifierOutput {
            event_id: "evt_code".into(),
            kind_refinement: None,
            topics: vec![],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: vec![],
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: None,
        context_path: Some(path.into()),
        parent_event_id: None,
        session_id: None,
    }
}

#[test]
fn rust_file_with_ten_top_level_fns_produces_ten_chunks() {
    let mut source = String::new();
    let mut expected_symbols: Vec<String> = Vec::new();
    for i in 0..10 {
        let name = format!("handler_{i:02}");
        expected_symbols.push(name.clone());
        let pad = "    let _x = 1; let _y = 2; let _z = 3;\n".repeat(100);
        source.push_str(&format!("fn {name}() {{\n{pad}}}\n\n"));
    }

    let event = make_event("sample.rs", &source);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert_eq!(chunks.len(), 10, "one chunk per top-level fn");

    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.metadata.source, ChunkSource::Code);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        assert_eq!(
            chunk.metadata.symbol.as_deref(),
            Some(expected_symbols[i].as_str())
        );
        let (start, end) = chunk.metadata.byte_range.expect("byte range present");
        assert!(end > start, "byte range non-empty");
        assert!((end as usize) <= source.len());
        assert_eq!(
            chunk.text,
            &source[start as usize..end as usize],
            "chunk text should equal source slice"
        );
    }
}

#[test]
fn oversize_rust_fn_is_windowed() {
    let big_body = "let _n: usize = 123456789;\n".repeat(400);
    let src = format!("fn huge() {{\n{body}}}\n", body = big_body);
    assert!(src.len() > OVERSIZE_THRESHOLD_BYTES);

    let event = make_event("huge.rs", &src);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(
        chunks.len() > 1,
        "oversize fn must produce multiple fallback windows, got {}",
        chunks.len()
    );
    for chunk in &chunks {
        assert_eq!(chunk.metadata.source, ChunkSource::FallbackWindow);
        assert_eq!(chunk.metadata.language.as_deref(), Some("rust"));
        let (start, end) = chunk.metadata.byte_range.expect("byte range present");
        assert!(end > start);
        assert!((end as usize) <= src.len());
    }
}

#[test]
fn unknown_extension_returns_empty() {
    let event = make_event("program.ex", "defmodule Foo do\n  def bar, do: :ok\nend\n");
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(chunks.is_empty());
}

#[test]
fn chunks_are_deterministic_across_runs() {
    let src = "fn alpha() {}\nfn beta() {}\nstruct Gamma;\n";
    let event = make_event("d.rs", src);
    let a = CodeChunker::new().chunk(&event).unwrap();
    let b = CodeChunker::new().chunk(&event).unwrap();
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), 3);
    for (ca, cb) in a.iter().zip(b.iter()) {
        assert_eq!(ca.dedup_key, cb.dedup_key);
        assert_eq!(ca.metadata.symbol, cb.metadata.symbol);
        assert_eq!(ca.metadata.byte_range, cb.metadata.byte_range);
    }
}

// ---- Per-language grammar coverage ----
//
// Each test exercises a different tree-sitter grammar so the symbol
// extraction path for that language gets walked. The minimum body
// that survives `--name-only` filtering is a single top-level
// declaration. Tests assert at least one chunk lands and the
// language label is stamped — they intentionally don't check the
// extracted symbol name (those rules vary across grammar versions).

fn assert_one_chunk_with_language(path: &str, src: &str, lang_label: &str) {
    let event = make_event(path, src);
    let chunks = CodeChunker::new().chunk(&event).unwrap();
    assert!(
        !chunks.is_empty(),
        "{path}: expected at least one chunk for {lang_label}"
    );
    assert_eq!(
        chunks[0].metadata.language.as_deref(),
        Some(lang_label),
        "language label should be {lang_label}"
    );
}

#[test]
fn typescript_function_declaration() {
    assert_one_chunk_with_language(
        "lib.ts",
        "export function greet(name: string): string {\n  return `Hello ${name}`;\n}\n",
        "typescript",
    );
}

#[test]
fn tsx_component_function() {
    assert_one_chunk_with_language(
        "comp.tsx",
        "export function Comp(): JSX.Element {\n  return null;\n}\n",
        "tsx",
    );
}

#[test]
fn javascript_function_and_class() {
    assert_one_chunk_with_language(
        "lib.js",
        "function greet(name) { return 'Hello ' + name; }\nclass Greeter { greet(){} }\n",
        "javascript",
    );
}

#[test]
fn python_def_and_class() {
    assert_one_chunk_with_language(
        "lib.py",
        "def greet(name):\n    return f'Hello {name}'\n\nclass Greeter:\n    pass\n",
        "python",
    );
}

#[test]
fn go_func_declaration() {
    assert_one_chunk_with_language(
        "lib.go",
        "package main\n\nfunc Greet(name string) string {\n  return name\n}\n",
        "go",
    );
}

#[test]
fn java_class_declaration() {
    assert_one_chunk_with_language(
        "Lib.java",
        "public class Greeter {\n  public String greet(String name) { return name; }\n}\n",
        "java",
    );
}

#[test]
fn c_function_definition() {
    assert_one_chunk_with_language(
        "lib.c",
        "#include <stdio.h>\nint greet(const char* n) { printf(\"%s\\n\", n); return 0; }\n",
        "c",
    );
}

#[test]
fn cpp_function_definition() {
    assert_one_chunk_with_language(
        "lib.cpp",
        "namespace ns { int greet(const char* n) { return 0; } }\n",
        "cpp",
    );
}

#[test]
fn json_top_level_object_chunks() {
    // JSON grammars treat the document root as a "value"; at minimum
    // the chunker should not error out.
    let event = make_event("data.json", "{\"k\": \"v\", \"n\": 42}\n");
    let _ = CodeChunker::new().chunk(&event).unwrap();
}

#[test]
fn yaml_document_chunks() {
    let event = make_event("data.yaml", "key: value\nlist:\n  - a\n  - b\n");
    let _ = CodeChunker::new().chunk(&event).unwrap();
}

#[test]
fn toml_document_chunks() {
    let event = make_event(
        "Cargo.toml",
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    );
    let _ = CodeChunker::new().chunk(&event).unwrap();
}

#[test]
fn header_file_uses_c_grammar() {
    let event = make_event(
        "lib.h",
        "#ifndef LIB_H\n#define LIB_H\nint greet(const char* n);\n#endif\n",
    );
    let _ = CodeChunker::new().chunk(&event).unwrap();
}

#[test]
fn empty_text_returns_empty() {
    let event = make_event("empty.rs", "");
    assert!(CodeChunker::new().chunk(&event).unwrap().is_empty());
}

#[test]
fn missing_path_returns_empty() {
    let mut event = make_event("any.rs", "fn x() {}\n");
    event.context_path = None;
    assert!(CodeChunker::new().chunk(&event).unwrap().is_empty());
}
