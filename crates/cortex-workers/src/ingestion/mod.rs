//! Cortex ingestion service — HTTP router + archive + Synap publisher.
//!
//! The service accepts envelope-compliant events at `/v1/events` (and
//! `/v1/events/batch`), runs a defense-in-depth redaction pass, writes a
//! durable archive record, and publishes the event on the appropriate
//! Synap stream (`cortex.events.raw` live or `cortex.events.bootstrap`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod archive;
pub mod config;
pub mod metrics;
pub mod publisher;
pub mod router;

pub use archive::{ArchiveError, ArchiveWriter, NdJsonZstdArchive};
pub use config::{IngestionConfig, Source};
pub use metrics::Metrics;
pub use publisher::{MemoryPublisher, Publisher, PublisherError, SynapPublisher};
pub use router::{build_router, AppState};
