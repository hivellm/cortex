//! Cortex OpenCode adapter — `EnvelopeProducer` over HTTP hook frames.
//!
//! The adapter receives hook frames from the OpenCode TS plugin
//! (`@hivellm/cortex-opencode-plugin`) via an HTTP `POST /hook`
//! listener, converts them to canonical `Envelope`s, and forwards
//! them through the `Publisher` trait to `cortex-ingestion`.
//!
//! # Architecture
//!
//! ```text
//! OpenCode plugin  →  POST /hook  →  server::start_server  →  MPSC
//!                                                                 ↓
//!                         OpenCodeProducer::produce()  ←  drain MPSC
//!                                     ↓
//!                               Publisher::publish
//! ```
//!
//! The daemon binary (a later task) wires `start_server` + the
//! `OpenCodeProducer` together and registers the producer with the
//! supervisor.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;
pub mod producer;
pub mod server;

pub use config::Config;
pub use producer::OpenCodeProducer;
pub use server::start_server;
