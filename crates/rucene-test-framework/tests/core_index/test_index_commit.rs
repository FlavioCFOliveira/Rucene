//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexCommit.java`.
//!
//! * `assertEquals(ic1, ic2)` is `IndexCommit.equals`, which Rucene names
//!   `commit_equals`; `hashCode()` is `commit_hash`.
//! * `getFileNames()` and `getUserData()` return `null` in Java; Rucene's
//!   return a value, so these return empty collections (neither is called).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene::index::index_commit::IndexCommit;
use rucene::store::Directory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, test};

/// `TestIndexCommit`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexCommit",
        vec![test("testEqualsHashCode", test_equals_hash_code).annotate(Test::new())],
    )
}

/// The anonymous `IndexCommit`s of `testEqualsHashCode`.
struct AnonymousCommit {
    segments_file_name: &'static str,
    dir: Arc<dyn Directory>,
}

impl fmt::Debug for AnonymousCommit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnonymousCommit")
            .field("segments_file_name", &self.segments_file_name)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for AnonymousCommit {
    /// `Object.toString()`: `IndexCommit` overrides none.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class_name = if self.segments_file_name == "a" {
            "org.apache.lucene.index.TestIndexCommit$1"
        } else {
            "org.apache.lucene.index.TestIndexCommit$2"
        };
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            class_name, self,
        ))
    }
}

impl IndexCommit for AnonymousCommit {
    fn get_segments_file_name(&self) -> String {
        self.segments_file_name.to_string()
    }
    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.dir)
    }
    fn get_file_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
    fn delete(&self) -> Result<()> {
        Ok(())
    }
    fn get_generation(&self) -> i64 {
        0
    }
    fn get_user_data(&self) -> Result<HashMap<String, String>> {
        Ok(HashMap::new())
    }
    fn is_deleted(&self) -> bool {
        false
    }
    fn get_segment_count(&self) -> i32 {
        2
    }
}

/// Port of `TestIndexCommit.testEqualsHashCode`.
fn test_equals_hash_code() -> Result<()> {
    // LUCENE-2417: equals and hashCode() impl was inconsistent
    let mut dir = LuceneTestCase::new_directory();
    // Both commits answer the same `dir` object, as Java's anonymous classes
    // capture the one local.
    let directory = dir.as_directory();

    let ic1 = AnonymousCommit {
        segments_file_name: "a",
        dir: Arc::clone(&directory),
    };

    let ic2 = AnonymousCommit {
        segments_file_name: "b",
        dir: Arc::clone(&directory),
    };

    Assert::assert_true_msg(
        &format!("expected:<{ic1}> but was:<{ic2}>"),
        ic1.commit_equals(&ic2),
    );
    assert_equals_msg(
        Some("hash codes are not equals"),
        &ic1.commit_hash(),
        &ic2.commit_hash(),
    );
    drop((ic1, ic2, directory));
    dir.close()
}
