# Synap — Overview

## Purpose

Synap is a unified, high-performance in-memory data infrastructure system combining key-value storage, message queuing, event streaming, and pub/sub messaging in a single cohesive platform. It targets use cases requiring real-time, low-latency operations with multi-protocol support (HTTP, TCP/MessagePack, Redis-compatible RESP3).

## Role in HiveLLM

Synap is a **core infrastructure service** used across HiveLLM as a high-performance alternative/complement to Redis for caching, messaging, and event streaming. It integrates with Cortex as a consumable data source and with other Hive services (Nexus, Vectorizer, Expert) via SDKs.

## Language & Stack

- **Language**: Rust Edition 2024 (nightly 1.85+)
- **Runtime**: Tokio (async I/O, 64-way sharding)
- **Web Framework**: Axum (HTTP/WebSocket)
- **Storage**: radix_trie (memory-efficient KV), in-memory data structures
- **Protocols**: HTTP/JSON, MessagePack over TCP (`synap://`), Redis RESP3 (`:6379`)
- **Serialization**: serde, MessagePack, YAML

## Maturity

**Version**: 0.12.0 (production-ready)  
**Status**: Phase 1–3 complete; Phase 4 (clustering, GUI) in progress  
**Test Coverage**: 636+ tests, 99.30% code coverage  
**Deployment**: Docker (hivehub/synap), released on Docker Hub
