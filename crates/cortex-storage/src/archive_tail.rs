//! Bounded incremental decoding of the archive's zstd-compressed NDJSON.
//!
//! A compressed offset is resumable only at a frame boundary. Live frames
//! instead retain their start plus the number of committed complete lines;
//! reopening replays that frame and suppresses those lines. Keep a reader
//! between polls. If persisting a batch fails, reopen at the last durable
//! checkpoint before polling again.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use zstd::stream::raw::{Decoder, Operation};

const BUFFER_BYTES: usize = 32 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Durable position immediately after committed complete NDJSON lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailCheckpoint {
    /// Compressed offset of a frame from which decoding can restart.
    pub frame_start: u64,
    /// Complete lines to replay from that frame without emitting them again.
    pub line_ordinal: u64,
}

/// Work caps for one poll. Zero work caps perform no work.
#[derive(Debug, Clone, Copy)]
pub struct TailLimits {
    /// Maximum new compressed bytes read from disk.
    pub max_compressed_bytes: usize,
    /// Maximum new bytes produced by the decoder.
    pub max_decoded_bytes: usize,
    /// Maximum complete lines handled, including restart replay.
    pub max_lines: usize,
    /// Hard ceiling remains one MiB, even if the caller requests more.
    pub max_line_bytes: usize,
    /// Cooperative elapsed-time budget between bounded operations.
    pub max_duration: Duration,
}

impl Default for TailLimits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: 4 * 1024 * 1024,
            max_decoded_bytes: 8 * 1024 * 1024,
            max_lines: 256,
            max_line_bytes: MAX_LINE_BYTES,
            max_duration: Duration::from_millis(100),
        }
    }
}

/// Fatal errors require reopening after repairing or replacing the source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TailError {
    /// Source read failed.
    #[error("archive read: {0}")]
    Io(String),
    /// Decoder rejected compressed bytes.
    #[error("corrupt zstd archive: {0}")]
    Corrupt(String),
    /// Complete or partial line exceeded the configured memory bound.
    #[error("archive line exceeds {limit} bytes")]
    LineTooLong {
        /// Effective configured byte limit.
        limit: usize,
    },
}

/// Complete lines and their proposed transactional checkpoint.
#[derive(Debug, Default)]
pub struct TailBatch {
    /// Newline removed; UTF-8 and envelope validation belong to the caller.
    pub lines: Vec<Vec<u8>>,
    /// Complete lines handled, including suppressed restart replay.
    pub lines_processed: usize,
    /// Commit atomically with every line in this batch.
    pub checkpoint: TailCheckpoint,
    /// Newly read compressed bytes, including any retained for the next poll.
    pub bytes_read: usize,
    /// Newly produced decoded bytes, including any retained for the next poll.
    pub bytes_decoded: usize,
    /// A read reached the current physical EOF. A later append can resume.
    pub eof: bool,
    /// EOF is inside a frame or incomplete line; this is not corruption.
    pub partial: bool,
    /// Restart replay has not yet reached the durable line ordinal.
    pub replaying: bool,
    /// Fatal source error; complete lines preceding it remain committable.
    pub error: Option<TailError>,
}

/// One live decoder; buffers and the zstd window have fixed upper bounds.
pub struct TailReader {
    file: File,
    decoder: Decoder<'static>,
    input: Vec<u8>,
    input_pos: usize,
    decoded: Vec<u8>,
    decoded_pos: usize,
    line: Vec<u8>,
    compressed_pos: u64,
    boundary: Option<u64>,
    frame_open: bool,
    ordinal: u64,
    replay_remaining: u64,
    checkpoint: TailCheckpoint,
    error: Option<TailError>,
}

impl TailReader {
    /// Open a regular file at a frame-safe checkpoint. Source identity and
    /// truncation/replacement detection belong to the manifest coordinator.
    pub fn open(path: &Path, checkpoint: TailCheckpoint) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || checkpoint.frame_start > metadata.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "archive checkpoint is outside a regular file",
            ));
        }
        file.seek(SeekFrom::Start(checkpoint.frame_start))?;
        let mut decoder = Decoder::new()?;
        // Reject oversized decoder windows instead of allocating from an
        // untrusted frame header. Eight MiB covers the production encoder.
        decoder.set_parameter(zstd::stream::raw::DParameter::WindowLogMax(23))?;
        Ok(Self {
            file,
            decoder,
            input: Vec::new(),
            input_pos: 0,
            decoded: Vec::new(),
            decoded_pos: 0,
            line: Vec::new(),
            compressed_pos: checkpoint.frame_start,
            boundary: None,
            frame_open: false,
            ordinal: 0,
            replay_remaining: checkpoint.line_ordinal,
            checkpoint,
            error: None,
        })
    }

    /// Decode a bounded batch. Complete lines in a still-open flushed frame
    /// may be committed: restart rebuilds decoder state by replaying them.
    /// The duration cap is cooperative between bounded decode/read calls;
    /// filesystem calls themselves are synchronous.
    pub fn poll(&mut self, limits: TailLimits) -> TailBatch {
        let started = Instant::now();
        let mut batch = TailBatch::default();
        let mut lines_processed = 0;
        let line_limit = limits.max_line_bytes.min(MAX_LINE_BYTES);
        if self.line.len() > line_limit {
            self.error = Some(TailError::LineTooLong { limit: line_limit });
        }
        while self.error.is_none()
            && limits.max_compressed_bytes > 0
            && limits.max_decoded_bytes > 0
            && lines_processed < limits.max_lines
            && started.elapsed() < limits.max_duration
        {
            if self.decoded_pos < self.decoded.len() {
                let byte = self.decoded[self.decoded_pos];
                if byte == b'\n' {
                    self.decoded_pos += 1;
                    self.ordinal += 1;
                    lines_processed += 1;
                    if self.replay_remaining > 0 {
                        self.replay_remaining -= 1;
                        self.line.clear();
                    } else {
                        batch.lines.push(std::mem::take(&mut self.line));
                        self.checkpoint.line_ordinal = self.ordinal;
                    }
                } else if self.line.len() >= line_limit {
                    self.error = Some(TailError::LineTooLong { limit: line_limit });
                } else {
                    self.line.push(byte);
                    self.decoded_pos += 1;
                }
                self.settle_boundary();
                continue;
            }
            self.settle_boundary();
            if batch.bytes_decoded >= limits.max_decoded_bytes {
                break;
            }
            // Drain the decoder before reading more input: zstd can retain
            // output after filling the previous bounded output buffer.
            let output_size = BUFFER_BYTES.min(limits.max_decoded_bytes - batch.bytes_decoded);
            let mut output = vec![0; output_size];
            let status = match self
                .decoder
                .run_on_buffers(&self.input[self.input_pos..], &mut output)
            {
                Ok(status) => status,
                Err(error) => {
                    self.error = Some(TailError::Corrupt(error.to_string()));
                    break;
                }
            };
            self.input_pos += status.bytes_read;
            self.compressed_pos += status.bytes_read as u64;
            if status.bytes_read > 0 {
                self.frame_open = true;
            }
            batch.bytes_decoded += status.bytes_written;
            output.truncate(status.bytes_written);
            self.decoded = output;
            self.decoded_pos = 0;
            if status.remaining == 0 {
                self.frame_open = false;
                self.boundary = Some(self.compressed_pos);
            }
            self.settle_boundary();
            if status.bytes_read > 0 || status.bytes_written > 0 {
                continue;
            }
            if self.input_pos < self.input.len() {
                self.error = Some(TailError::Corrupt("decoder made no progress".into()));
                break;
            }
            if batch.bytes_read >= limits.max_compressed_bytes {
                break;
            }
            self.input.resize(
                BUFFER_BYTES.min(limits.max_compressed_bytes - batch.bytes_read),
                0,
            );
            match self.file.read(&mut self.input) {
                Ok(0) => {
                    self.input.clear();
                    self.input_pos = 0;
                    batch.eof = true;
                    break;
                }
                Ok(count) => {
                    self.input.truncate(count);
                    self.input_pos = 0;
                    batch.bytes_read += count;
                }
                Err(error) => {
                    self.error = Some(TailError::Io(error.to_string()));
                    break;
                }
            }
        }
        batch.checkpoint = self.checkpoint;
        batch.lines_processed = lines_processed;
        batch.partial = self.frame_open || !self.line.is_empty() || self.replay_remaining > 0;
        batch.replaying = self.replay_remaining > 0;
        batch.error = self.error.clone();
        batch
    }

    fn settle_boundary(&mut self) {
        if self.decoded_pos == self.decoded.len() {
            if let Some(boundary) = self.boundary.take() {
                // A line can straddle concatenated frames. Keep its earlier
                // restart anchor until an entire line ends at a boundary.
                if self.line.is_empty() && self.replay_remaining == 0 {
                    self.checkpoint = TailCheckpoint {
                        frame_start: boundary,
                        line_ordinal: 0,
                    };
                    self.ordinal = 0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn closed(path: &Path, bytes: &[u8]) {
        std::fs::write(path, zstd::encode_all(bytes, 1).unwrap()).unwrap();
    }

    fn drain(reader: &mut TailReader) -> TailBatch {
        let mut all = TailBatch::default();
        for _ in 0..100 {
            let batch = reader.poll(TailLimits::default());
            all.lines.extend(batch.lines);
            all.checkpoint = batch.checkpoint;
            all.bytes_read += batch.bytes_read;
            all.bytes_decoded += batch.bytes_decoded;
            all.eof = batch.eof;
            all.partial = batch.partial;
            all.replaying = batch.replaying;
            all.error = batch.error;
            if all.eof || all.error.is_some() {
                return all;
            }
        }
        panic!("fixture failed to reach EOF in bounded polls");
    }

    #[test]
    fn live_flushed_frame_accepts_append_and_restart_replays_only_committed_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("live.parquet");
        let mut writer = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
        writer.write_all(b"first\n").unwrap();
        writer.flush().unwrap();
        let mut reader = TailReader::open(&path, TailCheckpoint::default()).unwrap();
        let first = drain(&mut reader);
        assert_eq!(first.lines, vec![b"first".to_vec()]);
        assert!(first.partial);
        assert_eq!(
            first.checkpoint,
            TailCheckpoint {
                frame_start: 0,
                line_ordinal: 1
            }
        );
        assert!(first.error.is_none());
        let unchanged = drain(&mut reader);
        assert_eq!((unchanged.bytes_read, unchanged.bytes_decoded), (0, 0));

        writer.write_all(b"second\n").unwrap();
        writer.flush().unwrap();
        let second = drain(&mut reader);
        assert_eq!(second.lines, vec![b"second".to_vec()]);
        let mut restarted = TailReader::open(&path, first.checkpoint).unwrap();
        let replayed = drain(&mut restarted);
        assert_eq!(replayed.lines, vec![b"second".to_vec()]);
        assert_eq!(replayed.checkpoint, second.checkpoint);
        writer.finish().unwrap();
        let finished = drain(&mut restarted);
        assert!(finished.lines.is_empty());
        assert!(!finished.partial);
        assert_eq!(
            finished.checkpoint.frame_start,
            std::fs::metadata(&path).unwrap().len()
        );
        let mut closed_restart = TailReader::open(&path, finished.checkpoint).unwrap();
        let empty = drain(&mut closed_restart);
        assert_eq!((empty.bytes_read, empty.bytes_decoded), (0, 0));
        assert!(empty.eof && !empty.partial);
    }

    #[test]
    fn concatenated_frames_and_split_lines_keep_safe_restart_anchor() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("joined.parquet");
        let mut bytes = zstd::encode_all(b"fir".as_slice(), 1).unwrap();
        let second_start = bytes.len();
        bytes.extend(zstd::encode_all(b"st\nsecond\n".as_slice(), 1).unwrap());
        std::fs::write(&path, &bytes).unwrap();
        let mut reader = TailReader::open(&path, TailCheckpoint::default()).unwrap();
        let limits = TailLimits {
            max_lines: 1,
            ..TailLimits::default()
        };
        let first = reader.poll(limits);
        assert_eq!(first.lines, vec![b"first".to_vec()]);
        assert_eq!(
            first.checkpoint,
            TailCheckpoint {
                frame_start: 0,
                line_ordinal: 1
            }
        );
        assert_ne!(first.checkpoint.frame_start, second_start as u64);
        let mut restarted = TailReader::open(&path, first.checkpoint).unwrap();
        let remaining = drain(&mut restarted);
        assert_eq!(remaining.lines, vec![b"second".to_vec()]);
        assert_eq!(remaining.checkpoint.frame_start, bytes.len() as u64);
        assert_eq!(remaining.checkpoint.line_ordinal, 0);
    }

    #[test]
    fn compressed_output_event_and_time_caps_preserve_every_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bounded.parquet");
        closed(&path, b"one\ntwo\nthree\n");
        let mut reader = TailReader::open(&path, TailCheckpoint::default()).unwrap();
        let idle = reader.poll(TailLimits {
            max_duration: Duration::ZERO,
            ..TailLimits::default()
        });
        assert_eq!((idle.bytes_read, idle.bytes_decoded), (0, 0));
        let limits = TailLimits {
            max_compressed_bytes: 3,
            max_decoded_bytes: 2,
            max_lines: 1,
            ..TailLimits::default()
        };
        let mut lines = Vec::new();
        for _ in 0..100 {
            let batch = reader.poll(limits);
            assert!(batch.bytes_read <= 3);
            assert!(batch.bytes_decoded <= 2);
            assert!(batch.lines.len() <= 1);
            assert!(batch.error.is_none());
            lines.extend(batch.lines);
            if batch.eof {
                break;
            }
        }
        assert_eq!(
            lines,
            vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
    }

    #[test]
    fn incomplete_line_never_advances_checkpoint_and_can_finish_after_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("partial.parquet");
        let mut writer = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
        writer.write_all(b"unfinished").unwrap();
        writer.flush().unwrap();
        let mut reader = TailReader::open(&path, TailCheckpoint::default()).unwrap();
        let partial = drain(&mut reader);
        assert!(partial.lines.is_empty());
        assert!(partial.partial && partial.error.is_none());
        assert_eq!(partial.checkpoint, TailCheckpoint::default());
        writer.write_all(b"\n").unwrap();
        writer.finish().unwrap();
        let mut restarted = TailReader::open(&path, partial.checkpoint).unwrap();
        assert_eq!(drain(&mut restarted).lines, vec![b"unfinished".to_vec()]);
    }

    #[test]
    fn corrupt_frames_and_oversized_lines_are_visible_without_advancing_past_them() {
        let tmp = tempfile::tempdir().unwrap();
        let corrupt = tmp.path().join("corrupt.parquet");
        std::fs::write(&corrupt, b"not a zstd frame, long enough for a header").unwrap();
        let mut reader = TailReader::open(&corrupt, TailCheckpoint::default()).unwrap();
        let batch = drain(&mut reader);
        assert!(matches!(batch.error, Some(TailError::Corrupt(_))));
        assert_eq!(batch.checkpoint, TailCheckpoint::default());

        let huge = tmp.path().join("huge.parquet");
        closed(&huge, b"ok\n123456789\n");
        let mut reader = TailReader::open(&huge, TailCheckpoint::default()).unwrap();
        let limits = TailLimits {
            max_line_bytes: 8,
            ..TailLimits::default()
        };
        let batch = reader.poll(limits);
        assert_eq!(batch.lines, vec![b"ok".to_vec()]);
        assert_eq!(
            batch.checkpoint,
            TailCheckpoint {
                frame_start: 0,
                line_ordinal: 1
            }
        );
        assert_eq!(batch.error, Some(TailError::LineTooLong { limit: 8 }));
        assert!(reader.line.len() <= 8);
        let repeat = reader.poll(limits);
        assert!(repeat.lines.is_empty());
        assert_eq!((repeat.bytes_read, repeat.bytes_decoded), (0, 0));
    }

    #[test]
    fn interrupted_frame_snapshot_replays_prefix_and_retains_incomplete_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("live.parquet");
        let snapshot = tmp.path().join("interrupted.parquet");
        let mut writer = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
        writer.write_all(b"committed\nunfinished").unwrap();
        writer.flush().unwrap();
        // Snapshot bytes visible at abrupt process termination, without
        // dropping the encoder (which would finish its frame).
        std::fs::write(&snapshot, std::fs::read(&path).unwrap()).unwrap();
        let mut reader = TailReader::open(&snapshot, TailCheckpoint::default()).unwrap();
        let prefix = drain(&mut reader);
        assert_eq!(prefix.lines, vec![b"committed".to_vec()]);
        assert!(prefix.eof && prefix.partial && prefix.error.is_none());
        let mut restarted = TailReader::open(&snapshot, prefix.checkpoint).unwrap();
        let replay = drain(&mut restarted);
        assert!(replay.lines.is_empty());
        assert_eq!(replay.checkpoint, prefix.checkpoint);
        assert!(replay.partial && replay.error.is_none());
        writer.finish().unwrap();
    }

    #[test]
    fn later_checksum_error_is_visible_without_retracting_committed_live_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("checksum.parquet");
        let mut writer = zstd::Encoder::new(File::create(&path).unwrap(), 1).unwrap();
        writer.include_checksum(true).unwrap();
        writer.write_all(b"committed\n").unwrap();
        writer.flush().unwrap();
        let mut reader = TailReader::open(&path, TailCheckpoint::default()).unwrap();
        let prefix = drain(&mut reader);
        assert_eq!(prefix.lines, vec![b"committed".to_vec()]);
        writer.finish().unwrap();
        let length = std::fs::metadata(&path).unwrap().len();
        let mut footer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        footer.seek(SeekFrom::Start(length - 1)).unwrap();
        let original = std::fs::read(&path).unwrap();
        footer
            .write_all(&[original[original.len() - 1] ^ 0xff])
            .unwrap();
        footer.flush().unwrap();
        let failed = drain(&mut reader);
        assert!(matches!(failed.error, Some(TailError::Corrupt(_))));
        assert!(failed.lines.is_empty());
        assert_eq!(failed.checkpoint, prefix.checkpoint);
    }
}
