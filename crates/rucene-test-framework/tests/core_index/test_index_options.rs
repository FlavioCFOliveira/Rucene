//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexOptions.java`.
//!
//! * `w.addDocument(Collections.singleton(field))` adds a document holding
//!   that one field.
//! * Two distinct Java `FieldType` instances with the same settings are two
//!   clones of one Rust `FieldType`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexOptions, IndexReader, LeafReader};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test, IAE};
use crate::support::npe;

/// `IndexOptions.values()`.
const VALUES: [IndexOptions; 6] = [
    IndexOptions::NONE,
    IndexOptions::DOCS,
    IndexOptions::DOCS_AND_FREQS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
    IndexOptions::DOCS_AND_CUSTOM_FREQS,
];

/// `TestIndexOptions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexOptions",
        vec![
            test(
                "testChangeIndexOptionsViaAddDocument",
                test_change_index_options_via_add_document,
            ),
            test(
                "testChangeIndexOptionsViaAddIndexesCodecReader",
                test_change_index_options_via_add_indexes_codec_reader,
            ),
            test(
                "testChangeIndexOptionsViaAddIndexesDirectory",
                test_change_index_options_via_add_indexes_directory,
            ),
            test(
                "testChangeIndexOptionsViaAddDocumentFrozenFieldType",
                test_change_index_options_via_add_document_frozen_field_type,
            ),
            test(
                "testSameFrozenFieldTypeAcrossManyDocuments",
                test_same_frozen_field_type_across_many_documents,
            ),
            test(
                "testMultiValuedFieldSameFrozenFieldType",
                test_multi_valued_field_same_frozen_field_type,
            ),
            test(
                "testMultiValuedFieldMixedFrozenAndUnfrozenCompatibleTypes",
                test_multi_valued_field_mixed_frozen_and_unfrozen_compatible_types,
            ),
            test(
                "testMultiValuedFieldMixedFrozenIncompatibleTypes",
                test_multi_valued_field_mixed_frozen_incompatible_types,
            ),
            test(
                "testFrozenFieldTypeTransitionAcrossDocuments",
                test_frozen_field_type_transition_across_documents,
            ),
            test(
                "testFrozenFieldTypeCacheRestabilizes",
                test_frozen_field_type_cache_restabilizes,
            ),
            test(
                "testFrozenFieldTypeAcrossSegments",
                test_frozen_field_type_across_segments,
            ),
            test(
                "testMultiValueForcesDeoptimization",
                test_multi_value_forces_deoptimization,
            ),
            test(
                "testFrozenFieldTypeInDocumentBlock",
                test_frozen_field_type_in_document_block,
            ),
        ],
    )
}

/// `new FieldType(TextField.TYPE_STORED)`.
fn stored() -> FieldType {
    FieldType::new_from(TextField::type_stored())
}

/// `new FieldType(TextField.TYPE_STORED)` with `setIndexOptions(options)`.
fn stored_with(options: IndexOptions) -> Result<FieldType> {
    let mut ft = stored();
    ft.set_index_options(options)?;
    Ok(ft)
}

/// `Collections.singleton(new Field(name, value, ft))`.
fn singleton(name: &str, value: &str, ft: &FieldType) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(Field::new(name, value.to_string(), ft.clone())?));
    Ok(doc)
}

/// `new IndexWriter(dir, newIndexWriterConfig())`.
fn writer(dir: &TestDirectory) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )
}

/// `getOnlyLeafReader(DirectoryReader.open(w))`.
fn only_leaf(w: &IndexWriter) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(
        directory_reader::open_with_writer(w)? as Arc<dyn IndexReader>
    )
}

/// `IOUtils.close(...)` of a leaf reader in a try-with-resources.
fn close_leaf(r: Arc<dyn LeafReader>) -> Result<()> {
    r.as_index_reader().close()
}

/// `r.getFieldInfos().fieldInfo("foo").getIndexOptions()`.
fn foo_index_options(r: &dyn LeafReader) -> IndexOptions {
    r.get_field_infos()
        .field_info("foo")
        .unwrap_or_else(|| npe("fieldInfo"))
        .get_index_options()
}

/// Port of `TestIndexOptions.testChangeIndexOptionsViaAddDocument`.
fn test_change_index_options_via_add_document() -> Result<()> {
    for from in VALUES {
        for to in VALUES {
            do_test_change_index_options_via_add_document(from, to, false)?;
        }
    }
    Ok(())
}

/// `doTestChangeIndexOptionsViaAddDocument(from, to, freeze)`.
fn do_test_change_index_options_via_add_document(
    from: IndexOptions,
    to: IndexOptions,
    freeze: bool,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut ft1 = stored_with(from)?;
    if freeze {
        ft1.freeze();
    }
    w.add_document(&singleton("foo", "bar", &ft1)?)?;
    let mut ft2 = stored_with(to)?;
    if freeze {
        ft2.freeze();
    }
    if from == to {
        w.add_document(&singleton("foo", "bar", &ft2)?)?; // no exception
    } else {
        let e = expect_throws(IAE, || w.add_document(&singleton("foo", "bar", &ft2)?));
        assert_equals(
            &Some(format!(
                "Inconsistency of field data structures across documents for field [foo] of doc [1]. index options: expected '{from:?}', but it has '{to:?}'."
            )),
            &e.message().map(str::to_string),
        );
    }
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testChangeIndexOptionsViaAddIndexesCodecReader`.
fn test_change_index_options_via_add_indexes_codec_reader() -> Result<()> {
    for from in VALUES {
        for to in VALUES {
            do_test_change_index_options_add_indexes_codec_reader(from, to)?;
        }
    }
    Ok(())
}

/// `(CodecReader) leaf` (a `ClassCastException` when it is not one).
fn as_codec_reader(leaf: Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    leaf.as_codec_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
    })
}

/// `doTestChangeIndexOptionsAddIndexesCodecReader(from, to)`.
fn do_test_change_index_options_add_indexes_codec_reader(
    from: IndexOptions,
    to: IndexOptions,
) -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let w1 = writer(&dir1)?;
    let ft1 = stored_with(from)?;
    w1.add_document(&singleton("foo", "bar", &ft1)?)?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = writer(&dir2)?;
    let ft2 = stored_with(to)?;
    w2.add_document(&singleton("foo", "bar", &ft2)?)?;

    {
        let leaf = only_leaf(&w2)?;
        let cr = as_codec_reader(Arc::clone(&leaf));
        let result = (|| -> Result<()> {
            if from == to {
                w1.add_indexes_codec_readers(&[Arc::clone(&cr)])?; // no exception
                w1.force_merge(1)?;
                let r = only_leaf(&w1)?;
                let expected = if from == IndexOptions::NONE { to } else { from };
                assert_equals(&expected, &foo_index_options(&*r));
                close_leaf(r)?;
            } else {
                let e = expect_throws(IAE, || w1.add_indexes_codec_readers(&[Arc::clone(&cr)]));
                assert_equals(
                    &Some(format!(
                        "cannot change field \"foo\" from index options={from:?} to inconsistent index options={to:?}"
                    )),
                    &e.message().map(str::to_string),
                );
            }
            Ok(())
        })();
        drop(cr);
        let closed = close_leaf(leaf);
        result?;
        closed?;
    }

    close_all(vec![
        Box::new(|| w1.close()),
        Box::new(|| w2.close()),
        Box::new(|| dir1.close()),
        Box::new(|| dir2.close()),
    ])
}

/// `IOUtils.close(...)`: runs every close, then rethrows the first failure.
fn close_all<'a>(closes: Vec<Box<dyn FnOnce() -> Result<()> + 'a>>) -> Result<()> {
    let mut first: Option<LuceneError> = None;
    for close in closes {
        if let Err(e) = close() {
            first.get_or_insert(e);
        }
    }
    first.map_or(Ok(()), Err)
}

/// Port of `TestIndexOptions.testChangeIndexOptionsViaAddIndexesDirectory`.
fn test_change_index_options_via_add_indexes_directory() -> Result<()> {
    for from in VALUES {
        for to in VALUES {
            do_test_change_index_options_add_indexes_directory(from, to)?;
        }
    }
    Ok(())
}

/// `doTestChangeIndexOptionsAddIndexesDirectory(from, to)`.
fn do_test_change_index_options_add_indexes_directory(
    from: IndexOptions,
    to: IndexOptions,
) -> Result<()> {
    let mut dir1 = LuceneTestCase::new_directory();
    let w1 = writer(&dir1)?;
    let ft1 = stored_with(from)?;
    w1.add_document(&singleton("foo", "bar", &ft1)?)?;

    let mut dir2 = LuceneTestCase::new_directory();
    let w2 = writer(&dir2)?;
    let ft2 = stored_with(to)?;
    w2.add_document(&singleton("foo", "bar", &ft2)?)?;
    w2.close()?;

    if from == to {
        w1.add_indexes_dirs(&[dir2.as_directory()])?; // no exception
        w1.force_merge(1)?;
        let r = only_leaf(&w1)?;
        let expected = if from == IndexOptions::NONE { to } else { from };
        assert_equals(&expected, &foo_index_options(&*r));
        close_leaf(r)?;
    } else {
        let e = expect_throws(IAE, || w1.add_indexes_dirs(&[dir2.as_directory()]));
        assert_equals(
            &Some(format!(
                "cannot change field \"foo\" from index options={from:?} to inconsistent index options={to:?}"
            )),
            &e.message().map(str::to_string),
        );
    }

    close_all(vec![
        Box::new(|| w1.close()),
        Box::new(|| dir1.close()),
        Box::new(|| dir2.close()),
    ])
}

/// Port of `TestIndexOptions.testChangeIndexOptionsViaAddDocumentFrozenFieldType`.
fn test_change_index_options_via_add_document_frozen_field_type() -> Result<()> {
    for from in VALUES {
        for to in VALUES {
            do_test_change_index_options_via_add_document(from, to, true)?;
        }
    }
    Ok(())
}

/// Port of `TestIndexOptions.testSameFrozenFieldTypeAcrossManyDocuments`.
fn test_same_frozen_field_type_across_many_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut ft = stored_with(IndexOptions::DOCS_AND_FREQS)?;
    ft.freeze();
    let num_docs = LuceneTestCase::at_least(100);
    for i in 0..num_docs {
        w.add_document(&singleton("foo", &format!("bar{i}"), &ft)?)?;
    }
    w.force_merge(1)?;
    {
        let r = only_leaf(&w)?;
        assert_equals(&num_docs, &r.max_doc());
        assert_equals(&IndexOptions::DOCS_AND_FREQS, &foo_index_options(&*r));
        close_leaf(r)?;
    }
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testMultiValuedFieldSameFrozenFieldType`.
fn test_multi_valued_field_same_frozen_field_type() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut ft = stored();
    ft.freeze();
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("foo", "bar1".to_string(), ft.clone())?));
    doc.add(Box::new(Field::new("foo", "bar2".to_string(), ft.clone())?));
    doc.add(Box::new(Field::new("foo", "bar3".to_string(), ft.clone())?));
    w.add_document(&doc)?;
    {
        let r = only_leaf(&w)?;
        assert_equals(&1, &r.max_doc());
        assert_equals(&3, &r.stored_fields()?.document(0)?.get_values("foo").len());
        close_leaf(r)?;
    }
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testMultiValuedFieldMixedFrozenAndUnfrozenCompatibleTypes`.
fn test_multi_valued_field_mixed_frozen_and_unfrozen_compatible_types() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut frozen_ft = stored();
    frozen_ft.freeze();
    // Unfrozen copy with same settings
    let unfrozen_ft = stored();
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("foo", "bar1".to_string(), frozen_ft)?));
    doc.add(Box::new(Field::new(
        "foo",
        "bar2".to_string(),
        unfrozen_ft,
    )?));
    w.add_document(&doc)?;
    {
        let r = only_leaf(&w)?;
        assert_equals(&1, &r.max_doc());
        assert_equals(&2, &r.stored_fields()?.document(0)?.get_values("foo").len());
        close_leaf(r)?;
    }
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testMultiValuedFieldMixedFrozenIncompatibleTypes`.
fn test_multi_valued_field_mixed_frozen_incompatible_types() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut frozen_ft = stored_with(IndexOptions::DOCS)?;
    frozen_ft.freeze();
    let mut different_ft = stored_with(IndexOptions::DOCS_AND_FREQS)?;
    different_ft.freeze();
    let mut doc = Document::new();
    doc.add(Box::new(Field::new("foo", "bar1".to_string(), frozen_ft)?));
    doc.add(Box::new(Field::new(
        "foo",
        "bar2".to_string(),
        different_ft,
    )?));
    // throw exception because of intra-document inconsistency
    let e = expect_throws(IAE, || w.add_document(&doc));
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(&message, message.contains("index options"));
    w.close()?;
    dir.close()
}

/// The message of the `IllegalArgumentException` adding `doc` raises.
fn iae_message(w: &IndexWriter, doc: &Document) -> Option<String> {
    expect_throws(IAE, || w.add_document(doc))
        .message()
        .map(str::to_string)
}

/// Port of `TestIndexOptions.testFrozenFieldTypeTransitionAcrossDocuments`.
fn test_frozen_field_type_transition_across_documents() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    // First doc uses one frozen type
    let mut ft1 = stored_with(IndexOptions::DOCS_AND_FREQS)?;
    ft1.freeze();
    w.add_document(&singleton("foo", "bar", &ft1)?)?;
    // Second doc uses a different frozen type with different options — must fail
    let mut ft2 = stored_with(IndexOptions::DOCS)?;
    ft2.freeze();
    let message = iae_message(&w, &singleton("foo", "bar", &ft2)?);
    assert_equals(
        &Some(format!(
            "Inconsistency of field data structures across documents for field [foo] of doc [1]. index options: expected '{:?}', but it has '{:?}'.",
            IndexOptions::DOCS_AND_FREQS,
            IndexOptions::DOCS
        )),
        &message,
    );
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testFrozenFieldTypeCacheRestabilizes`.
fn test_frozen_field_type_cache_restabilizes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    // Two distinct frozen FieldType instances with identical settings
    let mut ft_a = stored();
    ft_a.freeze();
    let mut ft_b = stored();
    ft_b.freeze();
    // First batch with ftA
    for i in 0..10 {
        w.add_document(&singleton("foo", &format!("val{i}"), &ft_a)?)?;
    }
    // Switch to ftB (same schema, different instance) — triggers cache invalidation + restabilize
    for i in 10..20 {
        w.add_document(&singleton("foo", &format!("val{i}"), &ft_b)?)?;
    }
    // Switch back to ftA
    for i in 20..30 {
        w.add_document(&singleton("foo", &format!("val{i}"), &ft_a)?)?;
    }
    w.force_merge(1)?; // just in case we had a random flush
    {
        let r = only_leaf(&w)?;
        assert_equals(&30, &r.max_doc());
        close_leaf(r)?;
    }
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testFrozenFieldTypeAcrossSegments`.
fn test_frozen_field_type_across_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut ft = stored_with(IndexOptions::DOCS_AND_FREQS)?;
    ft.freeze();
    // Index several docs in the first segment using the frozen fast-path
    for i in 0..10 {
        w.add_document(&singleton("foo", &format!("val{i}"), &ft)?)?;
    }
    w.commit()?;
    // Same frozen type in a new segment — should succeed
    for i in 10..20 {
        w.add_document(&singleton("foo", &format!("val{i}"), &ft)?)?;
    }
    w.commit()?;
    {
        let r = directory_reader::open_with_writer(&*w)?;
        assert_equals(&20, &r.num_docs());
        r.close()?;
    }
    // Different frozen type in a third segment — must fail against global field infos
    let mut ft2 = stored_with(IndexOptions::DOCS)?;
    ft2.freeze();
    let message = iae_message(&w, &singleton("foo", "bar", &ft2)?).unwrap_or_default();
    Assert::assert_true_msg(&message, message.contains("index options"));
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testMultiValueForcesDeoptimization`.
fn test_multi_value_forces_deoptimization() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = writer(&dir)?;
    let mut ft_a = stored();
    ft_a.freeze();
    let mut ft_b = stored(); // same schema, different instance
    ft_b.freeze();
    // Doc 0
    w.add_document(&singleton("foo", "val0", &ft_a)?)?;
    // Doc 1
    let mut doc = Document::new();
    doc.add(Box::new(Field::new(
        "foo",
        "val1a".to_string(),
        ft_a.clone(),
    )?));
    doc.add(Box::new(Field::new(
        "foo",
        "val1b".to_string(),
        ft_b.clone(),
    )?));
    w.add_document(&doc)?;
    // Doc 2
    w.add_document(&singleton("foo", "val2", &ft_b)?)?;
    // Doc 3
    w.add_document(&singleton("foo", "val3", &ft_b)?)?;
    {
        let r = only_leaf(&w)?;
        assert_equals(&4, &r.max_doc());
        close_leaf(r)?;
    }
    // Doc 4: multi-valued with incompatible frozen type — must fail
    let mut ft_c = stored();
    ft_c.set_store_term_vectors(true)?;
    ft_c.freeze();
    let mut doc4 = Document::new();
    doc4.add(Box::new(Field::new(
        "foo",
        "val4a".to_string(),
        ft_b.clone(),
    )?));
    doc4.add(Box::new(Field::new("foo", "val4b".to_string(), ft_c)?));
    let message = iae_message(&w, &doc4).unwrap_or_default();
    Assert::assert_true_msg(&message, message.contains("store term vector"));
    w.close()?;
    dir.close()
}

/// Port of `TestIndexOptions.testFrozenFieldTypeInDocumentBlock`.
fn test_frozen_field_type_in_document_block() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_parent_field(Some("__parent".to_string()));
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut ft = stored();
    ft.freeze();
    let mut block = Vec::new();
    for i in 0..5 {
        let mut child = Document::new();
        child.add(Box::new(Field::new(
            "foo",
            format!("child{i}"),
            ft.clone(),
        )?));
        block.push(child);
    }
    let mut parent = Document::new();
    parent.add(Box::new(Field::new(
        "foo",
        "parent".to_string(),
        ft.clone(),
    )?));
    block.push(parent);
    w.add_documents(&block)?;
    {
        let r = only_leaf(&w)?;
        assert_equals(&6, &r.max_doc());
        close_leaf(r)?;
    }
    w.close()?;
    dir.close()
}
