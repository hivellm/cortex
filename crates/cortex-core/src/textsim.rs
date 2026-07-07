//! Phase27c §3.1 — shared text-similarity primitives.
//!
//! Lifted from `cortex-cli/src/ops/memory_consolidate.rs` so the
//! graph dedup pass (`cortex-workers::graph::dedup`) and the memory
//! consolidation CLI share one implementation. (The phase27c
//! proposal called this "the MinHash util"; what actually existed —
//! verified before the lift — is this hashed-4-gram bag + cosine,
//! which plays the same blocking role: cheap approximate surface
//! similarity. True MinHash/LSH banding can replace the O(n²) cosine
//! blocking when candidate-set scale demands it.)
//!
//! Everything here is pure — no IO, no async — per cortex-core's
//! layer contract.

/// FNV-1a 64-bit hash over raw bytes. Stable across runs/platforms —
/// the n-gram binning below depends on that stability for
/// deterministic blocking.
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Deterministic hashed-4-gram bag vector: hashes 4-byte windows of
/// the lowercased input into a `dim`-dimensional bag, then
/// unit-normalises (so [`cosine`] == dot product). Two texts with
/// overlapping 4-grams produce overlapping vectors, so cosine tracks
/// surface n-gram overlap. Inputs shorter than 4 bytes collapse to a
/// single-bin unit vector.
#[must_use]
pub fn ngram_vector(text: &str, dim: usize) -> Vec<f32> {
    let dim = dim.max(8);
    let mut vec = vec![0.0f32; dim];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    if bytes.len() < 4 {
        vec[0] = 1.0;
        return vec;
    }
    for window in bytes.windows(4) {
        let h = fnv1a_64(window);
        let bin = (h as usize) % dim;
        vec[bin] += 1.0;
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

/// Cosine similarity between two unit-norm vectors. Falls back to
/// `0.0` on length mismatch so callers never panic on a bad input.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum::<f32>()
}

/// Shannon entropy of the byte distribution, in bits. Low-entropy
/// names ("aaa", "x") carry too little signal to dedup safely — the
/// phase27c §3.2 entropy gate skips them.
#[must_use]
pub fn shannon_entropy_bits(text: &str) -> f64 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for b in bytes {
        counts[*b as usize] += 1;
    }
    let n = bytes.len() as f64;
    counts
        .iter()
        .filter(|c| **c > 0)
        .map(|c| {
            let p = *c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ngram_vector_is_unit_norm() {
        let v = ngram_vector("the graph write pipeline", 256);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm = {norm}");
    }

    #[test]
    fn cosine_self_similarity_is_one() {
        let v = ngram_vector("nexus_client", 256);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_tracks_surface_overlap() {
        let a = ngram_vector("graph_worker_backpressure", 256);
        let b = ngram_vector("graph_worker_backpressur", 256);
        let c = ngram_vector("meilisearch_index_router", 256);
        assert!(cosine(&a, &b) > 0.8, "near-identical names score high");
        assert!(
            cosine(&a, &c) < cosine(&a, &b),
            "unrelated names score lower"
        );
    }

    #[test]
    fn cosine_length_mismatch_is_zero_not_panic() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
    }

    #[test]
    fn tiny_inputs_collapse_to_single_bin() {
        let v = ngram_vector("ab", 64);
        assert!((v[0] - 1.0).abs() < 1e-6);
        assert!(v[1..].iter().all(|x| *x == 0.0));
    }

    #[test]
    fn entropy_separates_informative_from_degenerate_names() {
        // "render_edge_merge" ≈ 2.55 bits (17 bytes over 7 symbols,
        // e-heavy); a repeated-letter degenerate name sits at 0.
        assert!(shannon_entropy_bits("render_edge_merge") > 2.0);
        assert!(shannon_entropy_bits("aaaa") < 0.5);
        assert_eq!(shannon_entropy_bits(""), 0.0);
    }

    #[test]
    fn fnv1a_is_stable() {
        // Pinned value — deterministic blocking depends on this hash
        // never changing across platforms or releases.
        assert_eq!(fnv1a_64(b"test"), 0xf9e6e6ef197c2b25);
    }
}
