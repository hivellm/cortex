//! Phase9j support helpers — shared between every retention canary
//! test. The `synth_corpus` module is the deterministic fixture
//! generator the spec promises; everything else (archive setup,
//! backend seeding) lives next to it so the test files themselves
//! stay assertion-focused.

#![allow(dead_code)]

pub mod synth_corpus;
