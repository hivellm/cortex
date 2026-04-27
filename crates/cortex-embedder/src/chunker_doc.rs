//! Markdown / document chunker.
//!
//! Splits on H1/H2/H3 boundaries and merges tiny sections (<256 chars of
//! trimmed content) into the next one. Long fenced code blocks (> 40 lines)
//! are extracted as separate chunks with `source = Code`, while the section
//! chunk retains a `[[code-block:N]]` marker so boundaries stay stable
//! across re-runs.

use anyhow::Result;

use crate::chunker::{Chunk, ChunkMetadata, ChunkSource, Chunker};
use crate::chunker_fallback::{event_text, sha256_hex};
use crate::embedder::EnrichedEvent;
use crate::identity::dedup_key;
use crate::routing::collection_for;

/// Code fences longer than this (in lines, not counting fences themselves)
/// are extracted as sibling code chunks.
pub const LONG_FENCE_LINES: usize = 40;

/// Section content shorter than this (trimmed char count) is merged into the
/// next section.
pub const MIN_SECTION_CHARS: usize = 256;

/// Section-aware document chunker.
#[derive(Debug, Default, Clone, Copy)]
pub struct DocChunker;

impl DocChunker {
    /// Create a new doc chunker.
    pub fn new() -> Self {
        Self
    }
}

impl Chunker for DocChunker {
    fn chunk(&self, event: &EnrichedEvent) -> Result<Vec<Chunk>> {
        let text = event_text(event);
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let collection = collection_for(&event.kind, "cortex", event.context_repo.as_deref());
        Ok(chunk_markdown(event, &text, &collection))
    }
}

/// A heading recognised by the chunker.
#[derive(Debug, Clone)]
struct Heading {
    level: u8,
    title: String,
}

/// A raw section between two heading boundaries.
#[derive(Debug, Clone)]
struct RawSection {
    /// Stack of ancestor headings that own this section (inclusive of the
    /// opening heading itself, unless the section starts before any heading
    /// in which case the stack is empty).
    path: Vec<Heading>,
    /// Byte offset in the full text where the heading line (or the document
    /// body, for the leading section) begins.
    byte_start: usize,
    /// Byte offset one past the last byte of this section's body.
    byte_end: usize,
    /// Raw text for this section *including* the heading line.
    text: String,
}

/// An extracted long fenced code block.
#[derive(Debug, Clone)]
struct ExtractedFence {
    /// Language info string (after the triple backticks / tildes), if any.
    language: Option<String>,
    /// Byte offset inside the full document where the block opens.
    byte_start: usize,
    /// Byte offset one past the closing fence line.
    byte_end: usize,
    /// Block body (lines between the fences, without the fence lines).
    body: String,
    /// Index into the owning raw section list.
    owning_section: usize,
}

fn chunk_markdown(event: &EnrichedEvent, text: &str, collection: &str) -> Vec<Chunk> {
    let sections = split_sections(text);
    if sections.is_empty() {
        return Vec::new();
    }

    // Extract long fences from each section, rewriting their bodies with
    // `[[code-block:N]]` markers. Numbering is local to the owning section
    // so a re-run of the same markdown produces identical sentinel strings.
    let mut rewritten_sections: Vec<RawSection> = Vec::with_capacity(sections.len());
    let mut fences: Vec<ExtractedFence> = Vec::new();

    for (idx, section) in sections.iter().enumerate() {
        let (rewritten_body, section_fences) =
            extract_long_fences(&section.text, section.byte_start, idx);
        fences.extend(section_fences);
        let mut rewritten = section.clone();
        rewritten.text = rewritten_body;
        rewritten_sections.push(rewritten);
    }

    // Merge tiny sections forward. "Tiny" is measured AFTER stripping the
    // heading line, to avoid the heading itself making the section pass.
    let merged = merge_tiny_sections(rewritten_sections);

    // Emit chunks: sections in document order, then extracted fences in
    // document order. Deterministic ordering keeps `dedup_key` ordinals stable.
    let mut out: Vec<Chunk> = Vec::new();
    let mut ordinal: u32 = 0;

    for section in &merged {
        let trimmed = section.text.trim();
        if trimmed.is_empty() {
            continue;
        }
        let symbol = section_symbol(&section.path);
        let byte_range = (
            (section.byte_start.min(u32::MAX as usize)) as u32,
            (section.byte_end.min(u32::MAX as usize)) as u32,
        );
        let chunk_hash = sha256_hex(&section.text);
        let key = dedup_key(&event.event_id, ordinal, &chunk_hash);
        out.push(Chunk {
            dedup_key: key,
            parent_event_id: event.event_id.clone(),
            parent_content_hash: event.content_hash.clone(),
            chunk_content_hash: chunk_hash,
            collection: collection.to_string(),
            text: section.text.clone(),
            metadata: ChunkMetadata {
                kind: event.kind,
                topics: event.classifier.topics.clone(),
                severity: event.classifier.severity,
                repo: event.context_repo.clone(),
                path: event.context_path.clone(),
                symbol,
                byte_range: Some(byte_range),
                language: None,
                source: ChunkSource::Doc,
                prompt_version: None,
            },
        });
        ordinal = ordinal.saturating_add(1);
    }

    // Sibling code chunks for every long fence. Each inherits the parent's
    // section path as `symbol`.
    for fence in &fences {
        let parent_path = sections
            .get(fence.owning_section)
            .map(|s| s.path.clone())
            .unwrap_or_default();
        let symbol = section_symbol(&parent_path);
        let byte_range = (
            (fence.byte_start.min(u32::MAX as usize)) as u32,
            (fence.byte_end.min(u32::MAX as usize)) as u32,
        );
        let chunk_hash = sha256_hex(&fence.body);
        let key = dedup_key(&event.event_id, ordinal, &chunk_hash);
        out.push(Chunk {
            dedup_key: key,
            parent_event_id: event.event_id.clone(),
            parent_content_hash: event.content_hash.clone(),
            chunk_content_hash: chunk_hash,
            collection: collection.to_string(),
            text: fence.body.clone(),
            metadata: ChunkMetadata {
                kind: event.kind,
                topics: event.classifier.topics.clone(),
                severity: event.classifier.severity,
                repo: event.context_repo.clone(),
                path: event.context_path.clone(),
                symbol,
                byte_range: Some(byte_range),
                language: fence.language.clone(),
                source: ChunkSource::Code,
                prompt_version: None,
            },
        });
        ordinal = ordinal.saturating_add(1);
    }

    out
}

/// Walk the document and break it into raw sections on H1/H2/H3 boundaries.
///
/// Headings inside fenced code blocks are ignored — a `#` on a line inside
/// ``` ... ``` is not a section marker.
fn split_sections(text: &str) -> Vec<RawSection> {
    let mut sections: Vec<RawSection> = Vec::new();
    let mut stack: Vec<Heading> = Vec::new();

    let mut cur_start: usize = 0;
    let mut cur_path: Vec<Heading> = Vec::new();
    let mut cur_text = String::new();

    let mut in_fence = false;
    let mut fence_marker: Option<String> = None;

    let mut offset: usize = 0;
    let bytes = text.as_bytes();
    while offset < bytes.len() {
        let line_end = memchr_newline(bytes, offset).unwrap_or(bytes.len());
        let line = &text[offset..line_end];
        let next_offset = if line_end < bytes.len() {
            line_end + 1
        } else {
            line_end
        };

        // Fence tracking — only triple backtick / triple tilde (after optional
        // leading whitespace) counts.
        let trimmed = line.trim_start();
        let is_backtick_fence = trimmed.starts_with("```");
        let is_tilde_fence = trimmed.starts_with("~~~");
        if !in_fence && (is_backtick_fence || is_tilde_fence) {
            in_fence = true;
            fence_marker = Some(if is_backtick_fence { "```" } else { "~~~" }.into());
        } else if in_fence {
            if let Some(marker) = &fence_marker {
                if trimmed.starts_with(marker.as_str()) {
                    in_fence = false;
                    fence_marker = None;
                }
            }
        }

        let heading = if !in_fence {
            parse_heading(line)
        } else {
            None
        };

        if let Some(h) = heading {
            if !cur_text.is_empty() || !cur_path.is_empty() {
                sections.push(RawSection {
                    path: cur_path.clone(),
                    byte_start: cur_start,
                    byte_end: offset,
                    text: cur_text.clone(),
                });
            }

            while let Some(top) = stack.last() {
                if top.level >= h.level {
                    stack.pop();
                } else {
                    break;
                }
            }
            stack.push(h.clone());

            cur_start = offset;
            cur_path = stack.clone();
            cur_text = String::new();
            cur_text.push_str(line);
            if next_offset > line_end {
                cur_text.push('\n');
            }
        } else {
            cur_text.push_str(line);
            if next_offset > line_end {
                cur_text.push('\n');
            }
        }

        offset = next_offset;
    }

    if !cur_text.is_empty() || !cur_path.is_empty() {
        sections.push(RawSection {
            path: cur_path,
            byte_start: cur_start,
            byte_end: text.len(),
            text: cur_text,
        });
    }

    sections
}

fn memchr_newline(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..].iter().position(|b| *b == b'\n').map(|p| p + from)
}

/// Recognise `#`, `##`, `###` at the start of a line (after optional leading
/// whitespace) followed by a space. H4+ are intentionally ignored — they live
/// inside the containing H3's body.
fn parse_heading(line: &str) -> Option<Heading> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=3).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    let title = rest.strip_prefix(' ')?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    Some(Heading {
        level: hashes as u8,
        title,
    })
}

/// Extract long fenced code blocks from a section body, replacing them with
/// `[[code-block:N]]` markers. Short fences (≤ `LONG_FENCE_LINES`) are
/// left untouched.
fn extract_long_fences(
    body: &str,
    section_byte_start: usize,
    owning_section: usize,
) -> (String, Vec<ExtractedFence>) {
    let mut out = String::with_capacity(body.len());
    let mut fences: Vec<ExtractedFence> = Vec::new();
    let mut local_ordinal: u32 = 0;

    let mut lines_iter = body.split_inclusive('\n');
    let mut offset = 0usize;

    while let Some(line) = lines_iter.next() {
        let stripped = strip_trailing_newline(line);
        let trimmed = stripped.trim_start();
        let is_fence_open = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if !is_fence_open {
            out.push_str(line);
            offset += line.len();
            continue;
        }

        let marker = if trimmed.starts_with("```") {
            "```"
        } else {
            "~~~"
        };
        let info_string = trimmed
            .trim_start_matches(marker)
            .split_whitespace()
            .next()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        let open_line = line;
        let open_offset = offset;
        let mut inner_lines: Vec<&str> = Vec::new();
        let mut close_line: Option<&str> = None;
        let mut probe_offset = offset + open_line.len();

        for next in lines_iter.by_ref() {
            let s = strip_trailing_newline(next);
            if s.trim_start().starts_with(marker) {
                close_line = Some(next);
                probe_offset += next.len();
                break;
            }
            inner_lines.push(next);
            probe_offset += next.len();
        }

        let body_line_count = inner_lines.len();
        if body_line_count > LONG_FENCE_LINES {
            let body_text: String = inner_lines.concat();
            let fence_byte_start = section_byte_start + open_offset;
            let fence_byte_end = section_byte_start + probe_offset;
            fences.push(ExtractedFence {
                language: info_string,
                byte_start: fence_byte_start,
                byte_end: fence_byte_end,
                body: body_text,
                owning_section,
            });
            out.push_str(&format!("[[code-block:{local_ordinal}]]\n"));
            local_ordinal = local_ordinal.saturating_add(1);
        } else {
            out.push_str(open_line);
            for inner in &inner_lines {
                out.push_str(inner);
            }
            if let Some(close) = close_line {
                out.push_str(close);
            }
        }

        offset = probe_offset;
    }

    (out, fences)
}

fn strip_trailing_newline(s: &str) -> &str {
    s.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(s)
}

/// Merge sections whose trimmed body (excluding heading) is < 256 chars
/// forward into the next section.
fn merge_tiny_sections(sections: Vec<RawSection>) -> Vec<RawSection> {
    if sections.is_empty() {
        return sections;
    }
    let mut out: Vec<RawSection> = Vec::with_capacity(sections.len());
    let mut carry: Option<RawSection> = None;

    for section in sections {
        let content = strip_leading_heading(&section.text);
        if content.trim().chars().count() < MIN_SECTION_CHARS {
            carry = Some(match carry.take() {
                Some(mut c) => {
                    c.byte_end = section.byte_end;
                    c.text.push_str(&section.text);
                    c
                }
                None => section,
            });
        } else if let Some(mut c) = carry.take() {
            c.byte_end = section.byte_end;
            c.text.push_str(&section.text);
            out.push(c);
        } else {
            out.push(section);
        }
    }

    if let Some(c) = carry {
        if let Some(last) = out.last_mut() {
            last.byte_end = c.byte_end;
            last.text.push_str(&c.text);
        } else {
            out.push(c);
        }
    }

    out
}

fn strip_leading_heading(text: &str) -> &str {
    if let Some(first_newline) = text.find('\n') {
        if parse_heading(&text[..first_newline]).is_some() {
            return &text[first_newline + 1..];
        }
    } else if parse_heading(text).is_some() {
        return "";
    }
    text
}

fn section_symbol(path: &[Heading]) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::with_capacity(path.len());
    for h in path {
        let prefix = "#".repeat(h.level as usize);
        parts.push(format!("{prefix} {}", h.title));
    }
    Some(parts.join(" > "))
}

