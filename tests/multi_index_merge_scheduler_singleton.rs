//! The process-wide `CombinedMergeScheduler` singleton that
//! `MultiIndexMergeScheduler(Directory)` shares, against Apache Lucene Core
//! 10.5.0 (`MultiIndexMergeScheduler.java:42-47, 73-79` and
//! `CombinedMergeScheduler.acquireSingleton`/`releaseSingleton`).
//!
//! The singleton is process state, so this case runs in a test binary of its
//! own: no other test can hold a reference while it counts them.

use std::sync::Arc;

use rucene::index::merge_scheduler::{
    CombinedMergeScheduler, MergeScheduler, MultiIndexMergeScheduler,
};
use rucene::store::{Directory, RamDirectory};

/// The singleton is created on first use and closed and dropped with its last
/// user.
#[test]
fn the_combined_singleton_lives_while_a_scheduler_uses_it() {
    assert!(CombinedMergeScheduler::peek_singleton().is_none());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let first = MultiIndexMergeScheduler::new(Arc::clone(&dir));
    let second = MultiIndexMergeScheduler::new(Arc::clone(&dir));
    assert!(Arc::ptr_eq(
        first.get_combined_merge_scheduler(),
        second.get_combined_merge_scheduler()
    ));
    assert!(CombinedMergeScheduler::peek_singleton().is_some());
    MergeScheduler::close(&first).unwrap();
    assert!(CombinedMergeScheduler::peek_singleton().is_some());
    MergeScheduler::close(&second).unwrap();
    assert!(CombinedMergeScheduler::peek_singleton().is_none());
    // One release too many is Java's `IllegalStateException`.
    assert!(MergeScheduler::close(&first).is_err());
}
