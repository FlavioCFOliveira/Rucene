//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTransactionRollback.java`.
//!
//! Test class to illustrate using IndexDeletionPolicy to provide multi-level rollback capability.
//! This test case creates an index of records 1 to 100, introducing a commit point every 10 records.
//!
//! A "keep all" deletion policy is used to ensure we keep all commit points for testing purposes

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::IndexDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_bits::MultiBits;
use rucene::index::IndexReader;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, random};
use crate::support::npe;

/// `private static final String FIELD_RECORD_ID = "record_id"`.
const FIELD_RECORD_ID: &str = "record_id";

/// `TestTransactionRollback`: its field `dir`, set by `setUp()`.
#[derive(Default)]
struct TestTransactionRollbackInstance {
    dir: Option<TestDirectory>,
}

impl TestTransactionRollbackInstance {
    fn dir(&self) -> &TestDirectory {
        self.dir.as_ref().unwrap_or_else(|| npe("dir"))
    }
}

/// A method of the class over its instance.
fn method(name: &str, body: fn(&mut TestTransactionRollbackInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestTransactionRollbackInstance| {
            body(inst).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// `TestTransactionRollback`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestTransactionRollback")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(
            TestTransactionRollbackInstance::default,
        ))
        .method(method("setUp", set_up))
        .method(method("tearDown", tear_down))
        .method(method("testRepeatedRollBacks", test_repeated_roll_backs))
        .method(method(
            "testRollbackDeletionPolicy",
            test_rollback_deletion_policy,
        ))
        .build()
}

/// `w.setLiveCommitData(data.entrySet())` of a one-entry `HashMap`.
fn set_index_data(w: &IndexWriter, value: String) -> Result<()> {
    w.set_live_commit_data(vec![("index".to_string(), value)])
}

/// Rolls back index to a chosen ID
fn roll_back_last(inst: &TestTransactionRollbackInstance, id: i32) -> Result<()> {
    // System.out.println("Attempting to rollback to "+id);
    let ids = format!("-{id}");
    let mut last: Option<Arc<dyn IndexCommit>> = None;
    let commits = directory_reader::list_commits(inst.dir().as_directory())?;
    for commit in commits {
        let ud = commit.get_user_data()?;
        if !ud.is_empty()
            && ud
                .get("index")
                .unwrap_or_else(|| npe("ud.get(\"index\")"))
                .ends_with(&ids)
        {
            last = Some(commit);
        }
    }

    let Some(last) = last else {
        return Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            format!("Couldn't find commit point {id}"),
        ));
    };

    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_index_deletion_policy(Some(Arc::new(RollbackDeletionPolicy {
        rollback_point: id,
    })))?;
    iwc.set_index_commit(Some(last));
    let w = IndexWriter::new(inst.dir().as_directory(), iwc)?;
    set_index_data(&w, format!("Rolled back to 1-{id}"))?;
    w.close()
}

/// Port of `TestTransactionRollback.testRepeatedRollBacks`.
fn test_repeated_roll_backs(inst: &mut TestTransactionRollbackInstance) -> Result<()> {
    let mut expected_last_record_id = 100;
    while expected_last_record_id > 10 {
        expected_last_record_id -= 10;
        roll_back_last(inst, expected_last_record_id)?;

        // `new BitSet(100)`, then `set(1, expectedLastRecordId + 1, true)`.
        let mut expecteds = vec![false; 101];
        for bit in expecteds
            .iter_mut()
            .take(expected_last_record_id as usize + 1)
            .skip(1)
        {
            *bit = true;
        }
        check_expecteds(inst, expecteds)?;
    }
    Ok(())
}

/// `private void checkExpecteds(BitSet expecteds)`.
fn check_expecteds(inst: &TestTransactionRollbackInstance, mut expecteds: Vec<bool>) -> Result<()> {
    let r: Arc<dyn IndexReader> = directory_reader::open(inst.dir().as_directory())?;

    // Perhaps not the most efficient approach but meets our
    // needs here.
    let live_docs = MultiBits::get_live_docs(&r)?;
    let stored_fields = r.stored_fields()?;
    for i in 0..r.max_doc() {
        if live_docs.as_ref().is_none_or(|l| l.get(i as usize)) {
            if let Some(sval) = stored_fields.document(i)?.get(FIELD_RECORD_ID) {
                let val: usize = sval.parse().expect("Integer.parseInt");
                // `BitSet.get` of a bit past the end is `false`.
                Assert::assert_true_msg(
                    &format!("Did not expect document #{val}"),
                    expecteds.get(val).copied().unwrap_or(false),
                );
                if val < expecteds.len() {
                    expecteds[val] = false;
                }
            }
        }
    }
    r.close()?;
    assert_equals_msg(
        Some("Should have 0 docs remaining "),
        &0,
        &expecteds.iter().filter(|b| **b).count(),
    );
    Ok(())
}

/*
private void showAvailableCommitPoints() throws Exception {
  Collection commits = DirectoryReader.listCommits(dir);
  for (Iterator iterator = commits.iterator(); iterator.hasNext();) {
    IndexCommit comm = (IndexCommit) iterator.next();
    System.out.print("\t Available commit point:["+comm.getUserData()+"] files=");
    Collection files = comm.getFileNames();
    for (Iterator iterator2 = files.iterator(); iterator2.hasNext();) {
      String filename = (String) iterator2.next();
      System.out.print(filename+", ");
    }
    System.out.println();
  }
}
*/

/// `setUp()`.
fn set_up(inst: &mut TestTransactionRollbackInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir = LuceneTestCase::new_directory();

    // Build index, of records 1 to 100, committing after each batch of 10
    let sdp: Arc<dyn IndexDeletionPolicy> = Arc::new(KeepAllDeletionPolicy);
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_index_deletion_policy(Some(sdp))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    for current_record_id in 1..=100 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            FIELD_RECORD_ID,
            &current_record_id.to_string(),
            Store::YES,
        )));
        w.add_document(&doc)?;

        if current_record_id % 10 == 0 {
            set_index_data(&w, format!("records 1-{current_record_id}"))?;
            w.commit()?;
        }
    }

    w.close()?;
    inst.dir = Some(dir);
    Ok(())
}

/// `tearDown()`.
fn tear_down(inst: &mut TestTransactionRollbackInstance) -> Result<()> {
    inst.dir.take().unwrap_or_else(|| npe("dir")).close()?;
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Rolls back to previous commit point
#[derive(Debug)]
struct RollbackDeletionPolicy {
    rollback_point: i32,
}

impl IndexDeletionPolicy for RollbackDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestTransactionRollback$RollbackDeletionPolicy"
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        for commit in commits {
            let user_data = commit.get_user_data()?;
            if !user_data.is_empty() {
                // Label for a commit point is "Records 1-30"
                // This code reads the last id ("30" in this example) and deletes it
                // if it is after the desired rollback point
                let x = user_data
                    .get("index")
                    .unwrap_or_else(|| npe("userData.get(\"index\")"));
                let last_val = &x[x.rfind('-').map_or(0, |i| i + 1)..];
                let last: i32 = last_val.parse().expect("Integer.parseInt");
                if last > self.rollback_point {
                    /*
                    System.out.print("\tRolling back commit point:" +
                                     " UserData="+commit.getUserData() +")  ("+(commits.size()-1)+" commit points left) files=");
                    Collection files = commit.getFileNames();
                    for (Iterator iterator2 = files.iterator(); iterator2.hasNext();) {
                      System.out.print(" "+iterator2.next());
                    }
                    System.out.println();
                    */

                    commit.delete()?;
                }
            }
        }
        Ok(())
    }
}

/// `static class DeleteLastCommitPolicy extends IndexDeletionPolicy`.
#[derive(Debug)]
struct DeleteLastCommitPolicy;

impl IndexDeletionPolicy for DeleteLastCommitPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestTransactionRollback$DeleteLastCommitPolicy"
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_init(&self, commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        commits[commits.len() - 1].delete()
    }
}

/// Port of `TestTransactionRollback.testRollbackDeletionPolicy`.
fn test_rollback_deletion_policy(inst: &mut TestTransactionRollbackInstance) -> Result<()> {
    for _ in 0..2 {
        // Unless you specify a prior commit point, rollback
        // should not work:
        let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(&*random()),
        )));
        iwc.set_index_deletion_policy(Some(Arc::new(DeleteLastCommitPolicy)))?;
        IndexWriter::new(inst.dir().as_directory(), iwc)?.close()?;
        let r: Arc<dyn IndexReader> = directory_reader::open(inst.dir().as_directory())?;
        assert_equals_msg(None, &100, &r.num_docs());
        r.close()?;
    }
    Ok(())
}

/// Keeps all commit points (used to build index)
#[derive(Debug)]
struct KeepAllDeletionPolicy;

impl IndexDeletionPolicy for KeepAllDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestTransactionRollback$KeepAllDeletionPolicy"
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_init(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }
}
