//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNoDeletionPolicy.java`.
//!
//! * `idp.onInit(null)` and `idp.onCommit(null)`: a Rust slice cannot be
//!   `null`; the calls pass no commits, which `NoDeletionPolicy` never reads
//!   either.
//! * `testFinalSingleton` and `testMethodsOverridden` reflect on the class;
//!   Rucene has no run-time reflection, so they compare declared lists: the
//!   constructors of `NoDeletionPolicy` (a struct cannot be subclassed, so it
//!   is final; its private field makes `instance()` the only way to get one,
//!   Java's private constructor) and the `IndexDeletionPolicy` methods its
//!   `impl` defines.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::Store;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_deletion_policy::NoDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, random, test};

/// `TestNoDeletionPolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNoDeletionPolicy",
        vec![
            test("testNoDeletionPolicy", test_no_deletion_policy),
            test("testFinalSingleton", test_final_singleton),
            test("testMethodsOverridden", test_methods_overridden),
            test("testAllCommitsRemain", test_all_commits_remain),
        ],
    )
}

/// `NoDeletionPolicy.class.getModifiers()` has `final`: a Rust struct.
const NO_DELETION_POLICY_IS_FINAL: bool = true;

/// `NoDeletionPolicy.class.getDeclaredConstructors()`: name and whether it
/// is private. The struct's private `_private` field makes its literal
/// constructor the only one, reachable inside its module alone.
const NO_DELETION_POLICY_CONSTRUCTORS: &[(&str, bool)] = &[("NoDeletionPolicy()", true)];

/// The methods of `IndexDeletionPolicy` (`IndexDeletionPolicy.java:60-93`).
const INDEX_DELETION_POLICY_METHODS: &[&str] = &["onInit", "onCommit"];

/// The `IndexDeletionPolicy` methods `impl IndexDeletionPolicy for
/// NoDeletionPolicy` defines (`src/index/index_deletion_policy.rs`).
const NO_DELETION_POLICY_METHODS: &[&str] = &["onInit", "onCommit"];

/// Port of `TestNoDeletionPolicy.testNoDeletionPolicy`.
fn test_no_deletion_policy() -> Result<()> {
    let idp = NoDeletionPolicy::instance();
    idp.on_init(&[])?;
    idp.on_commit(&[])?;
    Ok(())
}

/// Port of `TestNoDeletionPolicy.testFinalSingleton`.
fn test_final_singleton() -> Result<()> {
    Assert::assert_true(NO_DELETION_POLICY_IS_FINAL);
    let ctors = NO_DELETION_POLICY_CONSTRUCTORS;
    let names: Vec<&str> = ctors.iter().map(|(n, _)| *n).collect();
    assert_equals_msg(
        Some(&format!(
            "expected 1 private ctor only: [{}]",
            names.join(", ")
        )),
        &1,
        &ctors.len(),
    );
    Assert::assert_true_msg(
        &format!("that 1 should be private: {}", ctors[0].0),
        ctors[0].1,
    );
    Ok(())
}

/// Ensures that all methods of IndexDeletionPolicy are
/// overridden/implemented. That's important to ensure that NoDeletionPolicy
/// overrides everything, so that no unexpected behavior/error occurs.
/// NOTE: even though IndexDeletionPolicy is an interface today, and so all
/// methods must be implemented by NoDeletionPolicy, this test is important
/// in case one day IDP becomes an abstract class.
///
/// Port of `TestNoDeletionPolicy.testMethodsOverridden`.
fn test_methods_overridden() -> Result<()> {
    for m in INDEX_DELETION_POLICY_METHODS {
        Assert::assert_true_msg(
            &format!("{m} is not overridden !"),
            NO_DELETION_POLICY_METHODS.contains(m),
        );
    }
    Ok(())
}

/// Port of `TestNoDeletionPolicy.testAllCommitsRemain`.
fn test_all_commits_remain() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_index_deletion_policy(Some(NoDeletionPolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    for i in 0..10 {
        let mut doc = rucene::document::Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "c",
            &format!("a{i}"),
            Store::YES,
        )));
        writer.add_document(&doc)?;
        writer.commit()?;
        assert_equals_msg(
            Some("wrong number of commits !"),
            &(i + 1),
            &directory_reader::list_commits(dir.as_directory())?.len(),
        );
    }
    writer.close()?;
    drop(writer);
    dir.close()
}
