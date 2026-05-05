# VectorizerSync Overview

## Purpose

VectorizerSync is a cross-platform desktop application that enables users to manage and synchronize project directories with Vectorizer workspaces (local or remote HiveHub Cloud). It bridges the gap between local development projects and Vectorizer's vector database and search engine.

## Core Role

- **Local Project Management**: Select and organize multiple project directories
- **Workspace Configuration**: Export `workspace.yml` files in Vectorizer format for local use
- **Cloud Synchronization**: Automatic file sync to HiveHub Cloud (optional)
- **Real-Time Monitoring**: File system watcher for change detection and synchronization

## Technology Stack

| Component | Technology |
|-----------|-----------|
| **Runtime** | Electron (cross-platform desktop) |
| **Language** | TypeScript 5.x |
| **UI Framework** | React |
| **Database** | SQLite (local, user home directory) |
| **File Monitoring** | Chokidar (file system watcher) |
| **Config Format** | YAML (`workspace.yml`) |

## Platform Support

- Windows 10+
- macOS 10.15+
- Linux (Ubuntu 20.04+, Fedora 33+)

## Key Features

1. **Project Directory Selection**: Browse and manage multiple projects
2. **Workspace Export**: Auto-generate Vectorizer-compatible `workspace.yml`
3. **Smart File Filtering**: Auto-exclude node_modules, builds, large files (>100KB)
4. **Real-Time Sync**: Monitor and sync file changes automatically
5. **Quota Management**: Real-time HiveHub plan limit enforcement
6. **Notifications**: Internal alerts + HiveHub notifications
7. **Cross-Platform**: Native Windows/macOS/Linux support with platform-specific handling

## Status

**Version**: 0.1.0 (Pre-release)  
**Phase**: Documentation Phase (planning complete, implementation pending)

## Architecture Pattern

**Three-tier Electron architecture**:
- Main process: Database manager, file system watcher, sync engine, API client
- Renderer process: React UI components, state management
- IPC bridge: Inter-process communication for sync operations

Data flow: Local filesystem → File watcher → Sync queue → Database → HiveHub API (optional)
