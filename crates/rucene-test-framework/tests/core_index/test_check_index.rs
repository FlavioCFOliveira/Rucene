//! Port of `lucene/core/src/test/org/apache/lucene/index/TestCheckIndex.java`.
//!
//! `TestCheckIndex extends BaseTestCheckIndex`: `setUp()` opens `directory =
//! newDirectory()` after `super.setUp()`, `tearDown()` closes it before
//! `super.tearDown()`; `testDeletedDocs`, `testChecksumsOnly`,
//! `testChecksumsOnlyVerbose` and `testObtainsLock` call the inherited
//! methods of the same name on it. This is the class's only binding (it was
//! bound in `base_test_check_index_suites.rs` with the four wrappers alone).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::PayloadAttribute;
use rucene::analysis::TokenStream;
use rucene::document::{
    BinaryPoint, Document, Field, FieldType, KnnFloatVectorField, NumericDocValuesField,
    NumericValue, Store, StoredField, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::check_index::{self, CheckIndex, Level};
use rucene::index::index_commit::IndexCommit;
use rucene::index::index_deletion_policy::IndexDeletionPolicy;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{NoMergePolicy, SoftDeletesRetentionMergePolicy};
use rucene::index::terms::Term;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::Query;
use rucene::store::Directory;
use rucene::util::vector_util;
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::analysis::{CannedTokenStream, Token};
use rucene_test_framework::index::base_index_file_format_test_case::rethrow;
use rucene_test_framework::index::base_test_check_index;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, random, IAE};
use crate::support::set_check_index_on_close;

/// The instance of `TestCheckIndex`: its `directory` field.
#[derive(Default)]
struct CheckIndexInstance {
    directory: Option<TestDirectory>,
}

impl CheckIndexInstance {
    /// `directory`, as the inherited methods take it.
    fn directory(&self) -> Arc<dyn Directory> {
        self.directory
            .as_ref()
            .expect("setUp opened the directory")
            .as_directory()
    }
}

/// A wrapper test: `public void <name>() throws IOException { <name>(directory); }`.
fn wrapper(name: &str, body: fn(Arc<dyn Directory>) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut CheckIndexInstance| {
            body(inst.directory()).unwrap_or_else(|e| rethrow(e))
        })
}

/// A test of the class's own that does not read `directory`.
fn own(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |_inst: &mut CheckIndexInstance| body().unwrap_or_else(|e| rethrow(e)))
}

/// `setUp()`: `super.setUp(); directory = newDirectory();`.
fn set_up(inst: &mut CheckIndexInstance) {
    LuceneTestCase::super_set_up();
    inst.directory = Some(LuceneTestCase::new_directory());
}

/// `tearDown()`: `directory.close(); super.tearDown();`.
fn tear_down(inst: &mut CheckIndexInstance) {
    if let Some(mut directory) = inst.directory.take() {
        directory.close().unwrap_or_else(|e| rethrow(e));
    }
    LuceneTestCase::super_tear_down();
}

/// `TestCheckIndex`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestCheckIndex")
        .extends(&base_test_check_index::class())
        .constructor(Constructor::no_args(CheckIndexInstance::default))
        .method(
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body(set_up),
        )
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(tear_down),
        )
        .method(wrapper(
            "testDeletedDocs",
            base_test_check_index::test_deleted_docs,
        ))
        .method(wrapper(
            "testChecksumsOnly",
            base_test_check_index::test_checksums_only,
        ))
        .method(wrapper(
            "testChecksumsOnlyVerbose",
            base_test_check_index::test_checksums_only_verbose,
        ))
        .method(wrapper(
            "testObtainsLock",
            base_test_check_index::test_obtains_lock,
        ))
        .method(own("testCheckIndexAllValid", test_check_index_all_valid))
        .method(own(
            "testInvalidThreadCountArgument",
            test_invalid_thread_count_argument,
        ))
        .method(own(
            "testPriorBrokenCommitPoint",
            test_prior_broken_commit_point,
        ))
        .build()
}

/// `new Token(text, start, end) {{ setPayload(new BytesRef(payload)); }}`.
fn token_with_payload(text: &str, start: i32, end: i32, payload: &str) -> Token {
    let mut token = Token::with_offsets(text, start, end);
    token.set_payload(Some(BytesRef::from_char_sequence(payload)));
    token
}

/// Port of `TestCheckIndex.testCheckIndexAllValid`.
fn test_check_index_all_valid() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let live_doc_count = 1 + random().next_int_bound(10);
    let config = LuceneTestCase::new_index_writer_config();
    config.set_index_sort(Sort::new_fields(vec![SortField::new_reverse(
        Some("sort_field".to_string()),
        SortFieldType::Int,
        true,
    )?])?)?;
    config.set_soft_deletes_field(Some("soft_delete".to_string()));
    // preserves soft-deletes across merges
    let inner = config.merge_policy();
    config.set_merge_policy(Some(Arc::new(SoftDeletesRetentionMergePolicy::new(
        "soft_delete",
        Arc::new(|| Arc::new(MatchAllDocsQuery::instance()) as Arc<dyn Query>),
        inner,
    ))))?;
    {
        let w = IndexWriter::new(dir.as_directory(), config)?;
        for _ in 0..live_doc_count {
            let mut doc = Document::new();

            // stored field
            doc.add(Box::new(StringField::new(
                "id",
                random().next_int().to_string(),
                Store::YES,
            )?));
            doc.add(Box::new(StoredField::new_string(
                "field",
                format!("value{}", TestUtil::random_simple_string(&*random())),
            )?));

            // vector
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "v1",
                &random_vector(3)?,
            )?));
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "v2",
                &random_vector(3)?,
            )?));

            // doc value
            doc.add(Box::new(NumericDocValuesField::new(
                "dv",
                random().next_long(),
            )));
            // doc value with skip index
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "dv_skip",
                random().next_long(),
            )));

            // point value
            let mut point = vec![0u8; 4];
            NumericUtils::int_to_sortable_bytes(random().next_int(), &mut point, 0);
            doc.add(Box::new(BinaryPoint::new(
                "point",
                &[BytesRef::new(point)],
            )?));

            // term vector
            let token1 = token_with_payload("bar", 0, 3, "pay1");
            let token2 = token_with_payload("bar", 4, 8, "pay2");
            let ft = FieldType::new_from(TextField::type_not_stored());
            ft.set_store_term_vectors(true)?;
            ft.set_store_term_vector_positions(true)?;
            ft.set_store_term_vector_payloads(true)?;
            let stream: Arc<AtomicRefCell<dyn TokenStream>> =
                Arc::new(AtomicRefCell::new(CannedTokenStream::new(vec![
                    token1, token2,
                ])));
            doc.add(Box::new(Field::new_with_token_stream(
                "termvector",
                stream,
                ft,
            )?));

            w.add_document(&doc)?;
        }

        let mut tombstone = Document::new();
        tombstone.add(Box::new(NumericDocValuesField::new("soft_delete", 1)));
        w.soft_update_document(
            Term::from_text("id", "1"),
            &tombstone,
            &[Field::new_with_number(
                "soft_delete",
                NumericValue::Long(1),
                NumericDocValuesField::type_().clone(),
            )?],
        )?;
        w.force_merge(1)?;
        w.close()?;
    }

    let mut output: Vec<u8> = Vec::new();
    let status = TestUtil::check_index_with(
        dir.as_directory(),
        Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS,
        true,
        true,
        Some(&mut output),
    )?;
    let output = String::from_utf8_lossy(&output).to_string();

    assert_equals(&1, &status.segment_infos.len());

    let seg_status = &status.segment_infos[0];

    // confirm live docs testing status
    let live_doc_status = seg_status.live_doc_status.as_ref().expect("liveDocStatus");
    assert_equals(&0, &live_doc_status.num_deleted);
    Assert::assert_true(output.contains("test: check live docs"));
    Assert::assert_true(live_doc_status.error.is_none());

    // confirm field infos testing status
    let field_info_status = seg_status
        .field_info_status
        .as_ref()
        .expect("fieldInfoStatus");
    assert_equals(&9, &field_info_status.tot_fields);
    Assert::assert_true(output.contains("test: field infos"));
    Assert::assert_true(field_info_status.error.is_none());

    // confirm field norm (from term vector) testing status
    let field_norm_status = seg_status
        .field_norm_status
        .as_ref()
        .expect("fieldNormStatus");
    assert_equals(&1, &field_norm_status.tot_fields);
    Assert::assert_true(output.contains("test: field norms"));
    Assert::assert_true(field_norm_status.error.is_none());

    // confirm term index testing status
    let term_index_status = seg_status
        .term_index_status
        .as_ref()
        .expect("termIndexStatus");
    Assert::assert_true(term_index_status.term_count > 0);
    Assert::assert_true(term_index_status.tot_freq > 0);
    Assert::assert_true(term_index_status.tot_pos > 0);
    Assert::assert_true(output.contains("test: terms, freq, prox"));
    Assert::assert_true(term_index_status.error.is_none());

    // confirm stored field testing status
    // add storedField from tombstone doc
    let stored_field_status = seg_status
        .stored_field_status
        .as_ref()
        .expect("storedFieldStatus");
    assert_equals(&(live_doc_count + 1), &stored_field_status.doc_count);
    assert_equals(
        &(2 * live_doc_count as i64),
        &stored_field_status.tot_fields,
    );
    Assert::assert_true(output.contains("test: stored fields"));
    Assert::assert_true(stored_field_status.error.is_none());

    // confirm term vector testing status
    let term_vector_status = seg_status
        .term_vector_status
        .as_ref()
        .expect("termVectorStatus");
    assert_equals(&live_doc_count, &term_vector_status.doc_count);
    assert_equals(&(live_doc_count as i64), &term_vector_status.tot_vectors);
    Assert::assert_true(output.contains("test: term vectors"));
    Assert::assert_true(term_vector_status.error.is_none());

    // confirm doc values testing status
    let doc_values_status = seg_status
        .doc_values_status
        .as_ref()
        .expect("docValuesStatus");
    assert_equals(&3, &doc_values_status.total_numeric_fields);
    assert_equals(&1, &doc_values_status.total_skipping_index);
    Assert::assert_true(output.contains("test: docvalues"));
    Assert::assert_true(doc_values_status.error.is_none());

    // confirm point values testing status
    let points_status = seg_status.points_status.as_ref().expect("pointsStatus");
    assert_equals(&1, &points_status.total_value_fields);
    assert_equals(&(live_doc_count as i64), &points_status.total_value_points);
    Assert::assert_true(output.contains("test: points"));
    Assert::assert_true(points_status.error.is_none());

    // confirm vector testing status
    let vector_values_status = seg_status
        .vector_values_status
        .as_ref()
        .expect("vectorValuesStatus");
    assert_equals(
        &(2 * live_doc_count as i64),
        &vector_values_status.total_vector_values,
    );
    assert_equals(&2, &vector_values_status.total_knn_vector_fields);
    Assert::assert_true(output.contains("test: vectors"));
    Assert::assert_true(vector_values_status.error.is_none());

    // confirm index sort testing status
    Assert::assert_true(output.contains("test: index sort"));
    Assert::assert_true(
        seg_status
            .index_sort_status
            .as_ref()
            .expect("indexSortStatus")
            .error
            .is_none(),
    );

    // confirm soft deletes testing status
    Assert::assert_true(output.contains("test: check soft deletes"));
    Assert::assert_true(
        seg_status
            .soft_deletes_status
            .as_ref()
            .expect("softDeletesStatus")
            .error
            .is_none(),
    );
    dir.close()
}

/// Port of `TestCheckIndex.testInvalidThreadCountArgument`.
fn test_invalid_thread_count_argument() -> Result<()> {
    let args = ["-threadCount", "0"];
    expect_throws(IAE, || check_index::parse_options(&args));
    Ok(())
}

/// `private float[] randomVector(int dim)`.
fn random_vector(dim: usize) -> Result<Vec<f32>> {
    let mut v = vec![0f32; dim];
    for i in 0..dim {
        v[i] = random().next_float();
    }
    vector_util::l2normalize(&mut v, true)?;
    Ok(v)
}

/// `private static class DeleteNothingIndexDeletionPolicy extends
/// IndexDeletionPolicy`: never deletes any commit points! Do not use in
/// production!!
#[derive(Debug)]
struct DeleteNothingIndexDeletionPolicy;

impl IndexDeletionPolicy for DeleteNothingIndexDeletionPolicy {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestCheckIndex$DeleteNothingIndexDeletionPolicy"
    }

    fn on_init(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }

    fn on_commit(&self, _commits: &[Arc<dyn IndexCommit>]) -> Result<()> {
        Ok(())
    }
}

/// Port of `TestCheckIndex.testPriorBrokenCommitPoint`:
/// https://github.com/apache/lucene/issues/7820 -- when the most recent
/// commit point in the index is OK, but older commit points are broken,
/// CheckIndex fails to detect and correct that, while opening an IndexWriter
/// on the index will fail since IndexWriter loads all commit points on init.
fn test_prior_broken_commit_point() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    // disable this normally useful test infra feature since this test intentionally leaves broken
    // indices:
    set_check_index_on_close(&dir, false);
    let iwc = IndexWriterConfig::new();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?
        .set_index_deletion_policy(Some(Arc::new(DeleteNothingIndexDeletionPolicy)))?;
    {
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        // create first segment, and commit point referencing only segment 0
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", "a".into(), Store::NO)?));
        iw.add_document(&doc)?;
        iw.commit()?;

        // NOTE: we are (illegally) relying on precise file naming here -- if Codec or IW's
        // behaviour changes, this may need fixing:
        Assert::assert_true(LuceneTestCase::slow_file_exists(
            &*dir.as_directory(),
            "_0.si",
        )?);

        // create second segment, and another commit point referencing only segment 1
        doc.add(Box::new(StringField::new("id", "a".into(), Store::NO)?));
        iw.update_document(Term::from_text("id", "a"), &doc)?;
        iw.commit()?;

        // NOTE: we are (illegally) relying on precise file naming here -- if Codec or IW's
        // behaviour changes, this may need fixing:
        Assert::assert_true(LuceneTestCase::slow_file_exists(
            &*dir.as_directory(),
            "_0.si",
        )?);
        Assert::assert_true(LuceneTestCase::slow_file_exists(
            &*dir.as_directory(),
            "_1.si",
        )?);
        iw.close()?;
    }

    {
        let mut checkers = CheckIndex::new(dir.as_directory())?;
        let check_index_status = checkers.check_index()?;
        Assert::assert_true(check_index_status.clean);
        checkers.close()?;
    }

    // now corrupt segment 0, which is referenced by only the first commit point, by removing its
    // .si file (_0.si)
    dir.as_directory().delete_file("_0.si")?;

    {
        let mut checkers = CheckIndex::new(dir.as_directory())?;
        let check_index_status = checkers.check_index()?;
        Assert::assert_false(check_index_status.clean);
        checkers.close()?;
    }
    dir.close()
}
