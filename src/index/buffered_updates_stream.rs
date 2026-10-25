//! `BufferedUpdatesStream` and `FieldTermIterator` ported from
//! `org.apache.lucene.index`.
//!
//! Carries frozen delete packets from the writer's delete queue to the segments
//! they apply to, and decides which segments each generation reaches.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{LuceneError, Result};
use crate::index::documents_writer::FrozenBufferedUpdates;
use crate::index::index_writer::IndexWriter;
use crate::index::segment_info::SegmentCommitInfo;
use crate::util::{BytesRef, InfoStream, NoOutputInfoStream};

/// The info-stream component name Lucene logs buffered deletes under.
///
/// Equivalent to the `"BD"` literal `BufferedUpdatesStream` passes to the info
/// stream.
const BD: &str = "BD";

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

/// One segment held open while a packet of deletes is resolved against it.
///
/// Equivalent to `BufferedUpdatesStream.SegmentState`. It pins the segment's
/// pooled reader for the duration, and remembers the deletion count the segment
/// started with so the caller can tell how many this packet added.
pub struct SegmentState {
    /// The buffered-deletes generation of the segment.
    pub del_gen: i64,
    /// The pooled reader and updates of the segment.
    pub rld: std::sync::Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
    /// The reader opened over the segment, released when this state closes.
    pub reader: std::sync::Arc<crate::index::segment_reader::SegmentReader>,
    /// The deletion count before this packet was applied.
    pub start_del_count: i32,
}

impl std::fmt::Debug for SegmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SegmentState({})", self.rld.segment_name())
    }
}

impl SegmentState {
    /// Opens the segment's reader and records where its deletions stood.
    ///
    /// Equivalent to `SegmentState(ReadersAndUpdates, IOConsumer, SegmentCommitInfo)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the segment's reader raises.
    pub fn new(
        rld: std::sync::Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
        info: &SegmentCommitInfo,
    ) -> Result<Self> {
        let reader = rld.get_reader(&*crate::store::DEFAULT_IO_CONTEXT)?;
        let start_del_count = rld.get_del_count();
        Ok(Self {
            del_gen: info.get_buffered_deletes_gen(),
            rld,
            reader,
            start_del_count,
        })
    }

    /// Releases the reader this state pinned.
    ///
    /// Equivalent to `SegmentState.close()`, whose `IOConsumer` half —
    /// returning the `ReadersAndUpdates` to the writer's pool — is the caller's
    /// to run, because only the writer owns the pool.
    ///
    /// # Errors
    ///
    /// Propagates whatever releasing the reader raises.
    pub fn close(&self) -> Result<()> {
        self.rld.release(&self.reader)
    }
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

    /// Returns the generation below which every packet has finished applying.
    ///
    /// Equivalent to `BufferedUpdatesStream.FinishedSegments.getCompletedDelGen()`.
    fn get_completed_del_gen(&self) -> i64 {
        self.completed_del_gen.load(Ordering::Acquire)
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
pub struct BufferedUpdatesStream {
    /// The packets still waiting to be applied.
    ///
    /// Equivalent to `BufferedUpdatesStream.updates`, a `Set<FrozenBufferedUpdates>`.
    /// The port holds shared handles so that a snapshot can be taken for
    /// `waitApply` without removing the packets from the stream, which is what
    /// Java's `new HashSet<>(updates)` does with references.
    updates: Mutex<Vec<Arc<FrozenBufferedUpdates>>>,
    next_gen: AtomicI64,
    bytes_used: AtomicI64,
    finished_segments: FinishedSegments,
    /// Where the stream reports what it applied.
    ///
    /// Equivalent to `BufferedUpdatesStream.infoStream`.
    info_stream: Arc<dyn InfoStream>,
}

impl std::fmt::Debug for BufferedUpdatesStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferedUpdatesStream")
            .field("packets", &self.get_pending_updates_count())
            .field("bytes_used", &self.bytes_used.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for BufferedUpdatesStream {
    fn default() -> Self {
        Self::new()
    }
}

impl BufferedUpdatesStream {
    /// Creates an empty stream that reports nothing. The first generation
    /// handed out is `1`, as in Java.
    pub fn new() -> Self {
        Self::with_info_stream(Arc::new(NoOutputInfoStream))
    }

    /// Creates an empty stream reporting to `info_stream`.
    ///
    /// Equivalent to `BufferedUpdatesStream(InfoStream)`.
    pub fn with_info_stream(info_stream: Arc<dyn InfoStream>) -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            next_gen: AtomicI64::new(1),
            bytes_used: AtomicI64::new(0),
            finished_segments: FinishedSegments::default(),
            info_stream,
        }
    }

    /// Returns the RAM a single packet accounts for.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads `packet.bytesUsed`, which
    /// `FrozenBufferedUpdates` computes when it freezes the packet's
    /// `PrefixCodedTerms` and query array. `BufferedUpdates` in this port does
    /// not track its own RAM yet, so the weight is counted from the operations
    /// the packet carries — which is what `any()` and `ram_bytes_used()` are
    /// actually used for here. Both `push` and
    /// [`check_delete_stats`](Self::check_delete_stats) go through this one
    /// function, so the running total and the check always agree.
    fn packet_bytes_used(packet: &FrozenBufferedUpdates) -> i64 {
        (packet.updates().terms_size() + packet.updates().queries_size()) as i64
    }

    /// Appends `packet` to the stream and returns the generation it was given.
    ///
    /// Equivalent to `BufferedUpdatesStream.push`.
    pub fn push(&self, packet: Arc<FrozenBufferedUpdates>) -> Result<i64> {
        let mut guard = self.updates.lock().map_err(|_| {
            LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
        })?;
        // The insert operation must be atomic: letting threads increment the
        // generation and push afterwards risks packets landing out of order,
        // which would apply deletes to the wrong segments.
        let del_gen = self.next_gen.fetch_add(1, Ordering::AcqRel);
        // Stamp the generation onto the packet, exactly where Java does it:
        // `FrozenBufferedUpdates.apply` refuses to run without it.
        packet.set_del_gen(del_gen);
        debug_assert!(!packet.is_empty());
        debug_assert!(self.check_delete_stats(&guard));

        let bytes = Self::packet_bytes_used(&packet);
        guard.push(packet);
        self.bytes_used.fetch_add(bytes, Ordering::Relaxed);
        if self.info_stream.is_enabled(BD) {
            self.info_stream.message(
                BD,
                &format!(
                    "push new packet, packetCount={}, bytesUsed={:.3} MB",
                    guard.len(),
                    self.bytes_used.load(Ordering::Relaxed) as f64 / 1024. / 1024.
                ),
            );
        }
        debug_assert!(self.check_delete_stats(&guard));

        Ok(del_gen)
    }

    /// Checks that the running RAM total agrees with the buffered packets.
    ///
    /// Equivalent to the assert-only `org.apache.lucene.index.BufferedUpdatesStream#checkDeleteStats`.
    ///
    /// Java takes no argument and reads the `updates` set under the method's own
    /// `synchronized`; a Rust `Mutex` is not reentrant, so the already-locked
    /// packets are passed in by the caller that holds them.
    ///
    /// # Panics
    ///
    /// Panics with Java's message when the totals disagree.
    fn check_delete_stats(&self, updates: &[Arc<FrozenBufferedUpdates>]) -> bool {
        let mut bytes_used2 = 0i64;
        for packet in updates {
            bytes_used2 += Self::packet_bytes_used(packet);
        }
        let bytes_used = self.bytes_used.load(Ordering::Relaxed);
        assert_eq!(
            bytes_used2, bytes_used,
            "bytesUsed2={bytes_used2} vs {bytes_used}"
        );
        true
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
    pub fn take_packets(&self) -> Result<Vec<Arc<FrozenBufferedUpdates>>> {
        let mut guard = self.updates.lock().map_err(|_| {
            LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
        })?;
        let taken = std::mem::take(&mut *guard);
        self.bytes_used.store(0, Ordering::Release);
        Ok(taken)
    }

    /// Records that `packet` has finished being resolved, and drops it.
    ///
    /// Equivalent to `BufferedUpdatesStream.finished(FrozenBufferedUpdates)`,
    /// which indexing threads call once they are fully done resolving a packet.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the stream's lock is
    /// poisoned.
    pub fn finished(&self, packet: &Arc<FrozenBufferedUpdates>) -> Result<()> {
        {
            let mut guard = self.updates.lock().map_err(|_| {
                LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
            })?;
            guard.retain(|held| !Arc::ptr_eq(held, packet));
            self.bytes_used
                .fetch_sub(Self::packet_bytes_used(packet), Ordering::Relaxed);
        }
        self.finished_segment(packet.del_gen());
        Ok(())
    }

    /// Returns the generation up to and including which every packet is known
    /// to have finished applying.
    ///
    /// Equivalent to `BufferedUpdatesStream.getCompletedDelGen()`.
    pub fn get_completed_del_gen(&self) -> i64 {
        self.finished_segments.get_completed_del_gen()
    }

    /// Waits for every in-flight packet to finish being resolved.
    ///
    /// Equivalent to `BufferedUpdatesStream.waitApplyAll(IndexWriter)`, which
    /// a refresh or a commit calls. Java asserts that the caller does **not**
    /// hold the writer's monitor; in this port that is structural, because
    /// `writer` is borrowed shared and the writer's own state guard is taken
    /// inside `try_apply`.
    ///
    /// # Errors
    ///
    /// Propagates whatever resolving a packet raises.
    pub fn wait_apply_all(&self, writer: &IndexWriter) -> Result<()> {
        let wait_for = {
            let guard = self.updates.lock().map_err(|_| {
                LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
            })?;
            guard.clone()
        };
        self.wait_apply(wait_for, writer)
    }

    /// Waits only for the in-flight packets that apply to the segments a merge
    /// is about to consume.
    ///
    /// Equivalent to `BufferedUpdatesStream.waitApplyForMerge(List<SegmentCommitInfo>,
    /// IndexWriter)`: a packet whose generation is at most the largest
    /// buffered-deletes generation among `merge_infos` still has deletes for a
    /// subset of those segments, so the merge must not finish before it does.
    ///
    /// # Errors
    ///
    /// Propagates whatever resolving a packet raises.
    pub fn wait_apply_for_merge(
        &self,
        merge_infos: &[SegmentCommitInfo],
        writer: &IndexWriter,
    ) -> Result<()> {
        let mut max_del_gen = i64::MIN;
        for info in merge_infos {
            max_del_gen = max_del_gen.max(info.get_buffered_deletes_gen());
        }

        let wait_for: Vec<Arc<FrozenBufferedUpdates>> = {
            let guard = self.updates.lock().map_err(|_| {
                LuceneError::IllegalState("buffered updates stream lock poisoned".to_string())
            })?;
            guard
                .iter()
                .filter(|packet| packet.del_gen() <= max_del_gen)
                .cloned()
                .collect()
        };

        if self.info_stream.is_enabled(BD) {
            self.info_stream.message(
                BD,
                &format!(
                    "waitApplyForMerge: {} packets, {} merging segments",
                    wait_for.len(),
                    merge_infos.len()
                ),
            );
        }

        self.wait_apply(wait_for, writer)
    }

    /// Resolves every packet in `wait_for`, blocking on the ones another thread
    /// is already resolving.
    ///
    /// Equivalent to the private `org.apache.lucene.index.BufferedUpdatesStream#waitApply`.
    /// Frozen packets are normally resolved by the indexing threads that create
    /// them; a packet that is still unresolved when this runs is resolved here
    /// instead, and one that somebody else is currently applying is set aside
    /// and then waited for, so that all of them are due before the caller
    /// continues.
    ///
    /// # Errors
    ///
    /// Propagates whatever resolving a packet raises.
    fn wait_apply(
        &self,
        wait_for: Vec<Arc<FrozenBufferedUpdates>>,
        writer: &IndexWriter,
    ) -> Result<()> {
        let start_ns = std::time::Instant::now();
        let packet_count = wait_for.len();

        if wait_for.is_empty() {
            if self.info_stream.is_enabled(BD) {
                self.info_stream
                    .message(BD, "waitApply: no deletes to apply");
            }
            return Ok(());
        }

        if self.info_stream.is_enabled(BD) {
            self.info_stream
                .message(BD, &format!("waitApply: {packet_count} packets"));
        }

        let mut pending_packets = Vec::new();
        let mut total_del_count = 0i64;
        for packet in &wait_for {
            // If somebody else is currently applying it, move on to the next one
            // and force-apply below.
            if !writer.try_apply(packet)? {
                pending_packets.push(Arc::clone(packet));
            }
            total_del_count += packet.total_del_count();
        }
        for packet in &pending_packets {
            // Now block on all the packets that were concurrently applied, to
            // ensure they are due before we continue.
            writer.force_apply(packet)?;
        }

        if self.info_stream.is_enabled(BD) {
            self.info_stream.message(
                BD,
                &format!(
                    "waitApply: done {packet_count} packets; totalDelCount={total_del_count}; totBytesUsed={}; took {:.2} msec",
                    self.bytes_used.load(Ordering::Relaxed),
                    start_ns.elapsed().as_secs_f64() * 1000.0
                ),
            );
        }
        Ok(())
    }
}
