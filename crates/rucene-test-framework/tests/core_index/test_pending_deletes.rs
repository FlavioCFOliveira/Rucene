//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPendingDeletes.java`.
//!
//! `TestPendingSoftDeletes` extends this class and overrides
//! `newPendingDeletes`; the inherited tests are the `pub` functions here,
//! over [`AnyPendingDeletes`], the `PendingDeletes` the factory returns. In
//! this port `PendingDeletes` and `PendingSoftDeletes` are two types and
//! their methods take the segment they track (`commitInfo`), which the test
//! holds.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs;
use rucene::error::{LuceneError, Result};
use rucene::index::field_infos::FieldInfos;
use rucene::index::{PendingDeletes, PendingSoftDeletes, SegmentCommitInfo, SegmentInfo};
use rucene::search::Sort;
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{Bits, Version};
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::npe;

/// `TestPendingDeletes`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPendingDeletes",
        vec![
            test("testDeleteDoc", || test_delete_doc(&new_pending_deletes)),
            test("testWriteLiveDocs", || {
                test_write_live_docs(&new_pending_deletes)
            }),
            test("testIsFullyDeleted", || {
                test_is_fully_deleted(&new_pending_deletes)
            }),
        ],
    )
}

/// The `PendingDeletes` a `newPendingDeletes` returns: the class itself or
/// its subclass `PendingSoftDeletes`.
pub enum AnyPendingDeletes {
    /// `new PendingDeletes(commitInfo)`.
    Hard(PendingDeletes),
    /// `new PendingSoftDeletes(field, commitInfo)`.
    Soft(PendingSoftDeletes),
}

impl AnyPendingDeletes {
    /// `delete(docID)`.
    pub fn delete(&mut self, doc_id: i32, info: &SegmentCommitInfo) -> Result<bool> {
        match self {
            Self::Hard(d) => d.delete(doc_id, info),
            Self::Soft(d) => d.delete(doc_id, info),
        }
    }

    /// `getLiveDocs()`.
    pub fn get_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        match self {
            Self::Hard(d) => d.get_live_docs(),
            Self::Soft(d) => d.get_live_docs(),
        }
    }

    /// `numPendingDeletes()`.
    pub fn num_pending_deletes(&self) -> i32 {
        match self {
            Self::Hard(d) => d.num_pending_deletes(),
            Self::Soft(d) => d.num_pending_deletes(),
        }
    }

    /// `writeLiveDocs(dir)`.
    pub fn write_live_docs(
        &mut self,
        dir: &dyn Directory,
        info: &mut SegmentCommitInfo,
    ) -> Result<bool> {
        match self {
            Self::Hard(d) => d.write_live_docs(dir, info),
            Self::Soft(d) => d.write_live_docs(dir, info),
        }
    }

    /// `isFullyDeleted(() -> null)`: the supplier returns `null`, which only
    /// `PendingSoftDeletes` would dereference, and only when the segment has
    /// doc values for the soft-deletes field.
    pub fn is_fully_deleted_null_supplier(&mut self, info: &SegmentCommitInfo) -> Result<bool> {
        match self {
            Self::Hard(d) => Ok(d.is_fully_deleted(info)),
            Self::Soft(d) => {
                d.is_fully_deleted_with(info, &mut |_, _| Err(LuceneError::null_pointer()))
            }
        }
    }
}

/// `protected PendingDeletes newPendingDeletes(SegmentCommitInfo commitInfo)`.
fn new_pending_deletes(commit_info: &SegmentCommitInfo) -> AnyPendingDeletes {
    AnyPendingDeletes::Hard(PendingDeletes::new(commit_info))
}

/// The factory a subclass overrides.
pub type NewPendingDeletes = dyn Fn(&SegmentCommitInfo) -> AnyPendingDeletes;

/// `new SegmentInfo(dir, Version.LATEST, Version.LATEST, "test", max_doc, false, false,
/// Codec.getDefault(), Collections.emptyMap(), StringHelper.randomId(), new HashMap<>(), null)`.
pub fn test_segment_info(dir: &Arc<dyn Directory>, max_doc: i32) -> Result<SegmentInfo> {
    SegmentInfo::new(
        Arc::clone(dir),
        Version::LATEST,
        Some(Version::LATEST),
        "test".to_string(),
        max_doc,
        false,
        false,
        codecs::get_default().unwrap_or_else(|| npe("Codec.getDefault()")),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )
}

/// `new SegmentCommitInfo(si, 0, 0, -1, -1, -1, StringHelper.randomId())`.
pub fn test_commit_info(si: SegmentInfo) -> Result<SegmentCommitInfo> {
    SegmentCommitInfo::new(si, 0, 0, -1, -1, -1, StringHelper::random_id())
}

/// `liveDocs.get(doc)` of non-null live docs.
fn live(live_docs: &Option<Box<dyn Bits>>, doc: usize) -> bool {
    live_docs
        .as_ref()
        .unwrap_or_else(|| npe("liveDocs"))
        .get(doc)
}

/// Port of `TestPendingDeletes.testDeleteDoc`.
pub fn test_delete_doc(new_pending_deletes: &NewPendingDeletes) -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let si = test_segment_info(&dir, 10)?;
    let commit_info = test_commit_info(si)?;
    let mut deletes = new_pending_deletes(&commit_info);
    Assert::assert_true(deletes.get_live_docs().is_none());
    let doc_to_delete = TestUtil::next_int(&*random(), 0, 7);
    Assert::assert_true(deletes.delete(doc_to_delete, &commit_info)?);
    Assert::assert_true(deletes.get_live_docs().is_some());
    assert_equals(&1, &deletes.num_pending_deletes());

    let live_docs = deletes.get_live_docs();
    Assert::assert_false(live(&live_docs, doc_to_delete as usize));
    Assert::assert_false(deletes.delete(doc_to_delete, &commit_info)?); // delete again

    Assert::assert_true(live(&live_docs, 8));
    Assert::assert_true(deletes.delete(8, &commit_info)?);
    Assert::assert_true(live(&live_docs, 8)); // we have a snapshot
    assert_equals(&2, &deletes.num_pending_deletes());

    Assert::assert_true(live(&live_docs, 9));
    Assert::assert_true(deletes.delete(9, &commit_info)?);
    Assert::assert_true(live(&live_docs, 9));

    // now make sure new live docs see the deletions
    let live_docs = deletes.get_live_docs();
    Assert::assert_false(live(&live_docs, 9));
    Assert::assert_false(live(&live_docs, 8));
    Assert::assert_false(live(&live_docs, doc_to_delete as usize));
    assert_equals(&3, &deletes.num_pending_deletes());
    drop((deletes, commit_info));
    dir.close()
}

/// `Codec.getDefault().liveDocsFormat().readLiveDocs(dir, commitInfo, IOContext.DEFAULT)`.
fn read_live_docs(dir: &dyn Directory, commit_info: &SegmentCommitInfo) -> Result<Box<dyn Bits>> {
    codecs::get_default()
        .unwrap_or_else(|| npe("Codec.getDefault()"))
        .live_docs_format()
        .read_live_docs(dir, commit_info, &*DEFAULT_IO_CONTEXT)
}

/// Port of `TestPendingDeletes.testWriteLiveDocs`.
pub fn test_write_live_docs(new_pending_deletes: &NewPendingDeletes) -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let si = test_segment_info(&dir, 6)?;
    let mut commit_info = test_commit_info(si)?;
    let mut deletes = new_pending_deletes(&commit_info);
    Assert::assert_false(deletes.write_live_docs(&*dir, &mut commit_info)?);
    assert_equals(&0, &dir.list_all()?.len());
    let second_doc_deletes = random().next_boolean();
    deletes.delete(5, &commit_info)?;
    if second_doc_deletes {
        deletes.get_live_docs();
        deletes.delete(2, &commit_info)?;
    }
    assert_equals(&-1, &commit_info.get_del_gen());
    assert_equals(&0, &commit_info.get_del_count());

    assert_equals(
        &(if second_doc_deletes { 2 } else { 1 }),
        &deletes.num_pending_deletes(),
    );
    Assert::assert_true(deletes.write_live_docs(&*dir, &mut commit_info)?);
    assert_equals(&1, &dir.list_all()?.len());
    let live_docs = read_live_docs(&*dir, &commit_info)?;
    Assert::assert_false(live_docs.get(5));
    if second_doc_deletes {
        Assert::assert_false(live_docs.get(2));
    } else {
        Assert::assert_true(live_docs.get(2));
    }
    Assert::assert_true(live_docs.get(0));
    Assert::assert_true(live_docs.get(1));
    Assert::assert_true(live_docs.get(3));
    Assert::assert_true(live_docs.get(4));

    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(
        &(if second_doc_deletes { 2 } else { 1 }),
        &commit_info.get_del_count(),
    );
    assert_equals(&1, &commit_info.get_del_gen());

    deletes.delete(0, &commit_info)?;
    Assert::assert_true(deletes.write_live_docs(&*dir, &mut commit_info)?);
    assert_equals(&2, &dir.list_all()?.len());
    let live_docs = read_live_docs(&*dir, &commit_info)?;
    Assert::assert_false(live_docs.get(5));
    if second_doc_deletes {
        Assert::assert_false(live_docs.get(2));
    } else {
        Assert::assert_true(live_docs.get(2));
    }
    Assert::assert_false(live_docs.get(0));
    Assert::assert_true(live_docs.get(1));
    Assert::assert_true(live_docs.get(3));
    Assert::assert_true(live_docs.get(4));

    assert_equals(&0, &deletes.num_pending_deletes());
    assert_equals(
        &(if second_doc_deletes { 3 } else { 2 }),
        &commit_info.get_del_count(),
    );
    assert_equals(&2, &commit_info.get_del_gen());
    drop((deletes, commit_info, live_docs));
    dir.close()
}

/// Port of `TestPendingDeletes.testIsFullyDeleted`.
pub fn test_is_fully_deleted(new_pending_deletes: &NewPendingDeletes) -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let si = test_segment_info(&dir, 3)?;
    let mut commit_info = test_commit_info(si)?;
    let field_infos = FieldInfos::empty();
    commit_info
        .info
        .codec()
        .unwrap_or_else(|| npe("si.getCodec()"))
        .field_infos_format()
        .write(
            &*dir,
            &commit_info.info,
            "",
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        )?;
    let mut deletes = new_pending_deletes(&commit_info);
    for i in 0..3 {
        Assert::assert_true(deletes.delete(i, &commit_info)?);
        if random().next_boolean() {
            Assert::assert_true(deletes.write_live_docs(&*dir, &mut commit_info)?);
        }
        assert_equals(
            &(i == 2),
            &deletes.is_fully_deleted_null_supplier(&commit_info)?,
        );
    }
    Ok(())
}
