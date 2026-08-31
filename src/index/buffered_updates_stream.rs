//! `BufferedUpdatesStream` and `FieldTermIterator` ported from
//! `org.apache.lucene.index`.
//!
//! Carries frozen delete packets from the writer's delete queue to the segments
//! they apply to, and decides which segments each generation reaches.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use crate::error::{LuceneError, Result};
use crate::index::documents_writer::FrozenBufferedUpdates;
use crate::index::segment_info::SegmentCommitInfo;
use crate::util::BytesRef;

/// Iterates the terms of one field, remembering which delete generation they
/// came from.
///
/// Equivalent to `org.apache.lucene.index.FieldTermIterator`, which extends
/// `BytesRefIterator` with the field name and the generation.
pub trait FieldTermIterator {
    /// Returns the next term, or `None` at the end.
    ///
    /// Equivalent to `BytesRefIterator.next()`.
    fn next(&mut self) -> Result<Option<BytesRef>>;

    /// Returns the field the current term belongs to.
    ///
    /// Equivalent to `FieldTermIterator.field()`.
    fn field(&self) -> Option<&str>;

    /// Returns the delete generation the current term came from.
    ///
    /// Equivalent to `FieldTermIterator.delGen()`.
    fn del_gen(&self) -> i64;
}

/// What applying a batch of packets achieved.
///
/// Equivalent to `BufferedUpdatesStream.ApplyDeletesResult`.
#[derive(Debug, Default)]
pub struct ApplyDeletesResult {
    /// Whether any document was actually deleted.
    pub any_deletes: bool,
    /// Segments that lost every document and can be dropped outright.
    pub all_deleted: Vec<SegmentCommitInfo>,
}

/// Tracks which delete generations have finished being applied.
///
/// Equivalent to `BufferedUpdatesStream.FinishedSegments`.
#[derive(Debug, Default)]
struct FinishedSegments {
    /// The generation below which everything has been applied.
    completed_del_gen: AtomicI64,
    /// Generations that finished out of order and are waiting for the ones
    /// below them.
    finished_del_gens: Mutex<HashSet<i64>>,
}

impl FinishedSegments {
    fn clear(&self) {
        self.completed_del_gen.store(0, Ordering::Release);
        if let Ok(mut guard) = self.finished_del_gens.lock() {
            guard.clear();
        }
    }

    /// Returns whether `del_gen` is still being applied.
    fn still_running(&self, del_gen: i64) -> bool {
        if del_gen <= self.completed_del_gen.load(Ordering::Acquire) {
            return false;
        }
        match self.finished_del_gens.lock() {
            Ok(guard) => !guard.contains(&del_gen),
            Err(_) => true,
        }
    }

    /// Records that `del_gen` finished, advancing the completed watermark over
    /// every consecutive generation that has also finished.
    fn finished_segment(&self, del_gen: i64) {
        let Ok(mut guard) = self.finished_del_gens.lock() else {
            return;
        };
        guard.insert(del_gen);
        loop {
            let next = self.completed_del_gen.load(Ordering::Acquire) + 1;
            if guard.remove(&next) {
                self.completed_del_gen.store(next, Ordering::Release);
            } else {
                break;
            }
        }
    }
}

/// The queue of frozen delete packets waiting to reach their segments.
///
/// Equivalent to `org.apache.lucene.index.BufferedUpdatesStream`.
///
/// **Divergence from Lucene 10.5.0.** Java's `waitApply` opens a
/// `SegmentReader` per affected segment and resolves the deletes there, through
/// `ReadersAndUpdates` and the writer's reader pool. Applying a packet is
/// therefore part of the writer, not of the stream; this port keeps the stream's
/// own responsibilities — ordering the packets by generation, tracking which
/// generations are still running, and selecting the segments a packet reaches —
/// and leaves resolution to the caller that owns the readers.
#[derive(Debug, Default)]
pub struct BufferedUpdatesStream {
    updates: Mutex<Vec<FrozenBufferedUpdates>>,
    next_gen: AtomicI64,
    bytes_used: AtomicI64,
    finished_segments: FinishedSegments,
}

impl BufferedUpdatesStream {
    /// Creates an empty stream. The first generation handed out is `1`, as in
    /// Java.
    pub fn new() -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            next_gen: AtomicI64::new(1),
            bytes_used: AtomicI64::new(0),
            finished_segments: FinishedSegments::default(),
        }
    }

    /// Appends `packet` to the stream and returns the generation it was given.
    ///
    /// Equivalent to `BufferedUpdatesStream.push`.
    pub fn push(&self, packet: FrozenBufferedUpdates) -> Result<i64> {
        let del_gen = self.next_gen.fetch_add(1, Ordering::AcqRel);
        // `BufferedUpdates` does not track its own RAM yet, so the packet's
        // weight is counted from the operations it carries, which is what
        // `any()` and `ram_bytes_used()` are actually used for here.
        let bytes = (packet.updates().terms_size() + packet.updates().queries_size()) as i64;
        let mut guard = self.updates.lock().map_err(|_| {
            LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
        })?;
        guard.push(packet);
        self.bytes_used.fetch_add(bytes, Ordering::Relaxed);
        Ok(del_gen)
    }

    /// Returns how many packets are waiting.
    ///
    /// Equivalent to `BufferedUpdatesStream.getPendingUpdatesCount`.
    pub fn get_pending_updates_count(&self) -> usize {
        self.updates.lock().map(|guard| guard.len()).unwrap_or(0)
    }

    /// Drops every packet and resets the generation counter.
    ///
    /// Equivalent to `BufferedUpdatesStream.clear`, which only `IndexWriter`'s
    /// rollback calls.
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.updates.lock().map_err(|_| {
            LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
        })?;
        guard.clear();
        self.next_gen.store(1, Ordering::Release);
        self.bytes_used.store(0, Ordering::Release);
        self.finished_segments.clear();
        Ok(())
    }

    /// Returns whether any packet is buffered.
    ///
    /// Equivalent to `BufferedUpdatesStream.any`.
    pub fn any(&self) -> bool {
        self.bytes_used.load(Ordering::Relaxed) != 0
    }

    /// Returns the RAM the buffered packets occupy.
    ///
    /// Equivalent to `BufferedUpdatesStream.ramBytesUsed`.
    pub fn ram_bytes_used(&self) -> i64 {
        self.bytes_used.load(Ordering::Relaxed)
    }

    /// Returns the generation the next packet will be given.
    ///
    /// Equivalent to `BufferedUpdatesStream.getNextGen`.
    pub fn get_next_gen(&self) -> i64 {
        self.next_gen.fetch_add(1, Ordering::AcqRel)
    }

    /// Returns whether `del_gen` is still being applied.
    ///
    /// Equivalent to `BufferedUpdatesStream.stillRunning`.
    pub fn still_running(&self, del_gen: i64) -> bool {
        self.finished_segments.still_running(del_gen)
    }

    /// Records that everything up to `del_gen` has been applied.
    ///
    /// Equivalent to `BufferedUpdatesStream.finishedSegment`.
    pub fn finished_segment(&self, del_gen: i64) {
        self.finished_segments.finished_segment(del_gen);
    }

    /// Returns the segments a packet of generation `del_gen` applies to.
    ///
    /// A packet reaches every segment flushed **before** it, which is what the
    /// per-segment `buffered_deletes_gen` records. A segment-private packet
    /// reaches only the segment it was frozen for.
    ///
    /// Equivalent to the segment selection inside
    /// `BufferedUpdatesStream.openSegmentStates`.
    pub fn segments_for<'a>(
        &self,
        packet: &FrozenBufferedUpdates,
        del_gen: i64,
        segments: &'a [SegmentCommitInfo],
    ) -> Vec<&'a SegmentCommitInfo> {
        match packet.private_segment_name() {
            Some(name) => segments
                .iter()
                .filter(|info| info.info.name == name)
                .collect(),
            None => segments
                .iter()
                .filter(|info| info.get_buffered_deletes_gen() < del_gen)
                .collect(),
        }
    }

    /// Removes and returns every buffered packet, oldest first.
    ///
    /// The caller resolves them against its readers and then reports each
    /// generation through [`finished_segment`](Self::finished_segment).
    pub fn take_packets(&self) -> Result<Vec<FrozenBufferedUpdates>> {
        let mut guard = self.updates.lock().map_err(|_| {
            LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
        })?;
        let taken = std::mem::take(&mut *guard);
        self.bytes_used.store(0, Ordering::Release);
        Ok(taken)
    }
}
