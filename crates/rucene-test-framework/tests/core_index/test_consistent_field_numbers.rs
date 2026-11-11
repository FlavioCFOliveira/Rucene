//! Port of `lucene/core/src/test/org/apache/lucene/index/TestConsistentFieldNumbers.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Field, FieldType, Store, StoredField, TextField};
use rucene::error::Result;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{LogMergePolicy, NoMergePolicy};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::IndexableField;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::{FailOnNonBulkMergesInfoStream, LuceneTestCase};

use crate::core_support::{assert_equals, assert_null, class, random, test};

/// `TestConsistentFieldNumbers`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestConsistentFieldNumbers",
        vec![
            test(
                "testSameFieldNumbersAcrossSegments",
                test_same_field_numbers_across_segments,
            ),
            test("testAddIndexes", test_add_indexes),
            test("testFieldNumberGaps", test_field_number_gaps),
            test("testManyFields", test_many_fields),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_iwc() -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    Ok(iwc)
}

/// `new TextField(name, value, store)`.
fn text(name: &str, value: &str, store: Store) -> Result<Box<TextField>> {
    Ok(Box::new(TextField::new(name, value.to_string(), store)?))
}

/// `fis.fieldInfo(number).name` (Java dereferences a `null` field info).
fn name_of(fis: &FieldInfos, number: i32) -> String {
    fis.field_info_by_number(number)
        .unwrap_or_else(|| crate::support::npe("fieldInfo(number)"))
        .name
        .clone()
}

/// `IndexWriter.readFieldInfos(sis.info(i))`.
fn field_infos(sis: &SegmentInfos, i: usize) -> Result<FieldInfos> {
    IndexWriter::read_field_infos(sis.info(i))
}

/// Port of `TestConsistentFieldNumbers.testSameFieldNumbersAcrossSegments`.
fn test_same_field_numbers_across_segments() -> Result<()> {
    for i in 0..2 {
        let dir = LuceneTestCase::new_directory();
        let mut writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;

        let mut d1 = Document::new();
        d1.add(text("f1", "first field", Store::YES)?);
        d1.add(text("f2", "second field", Store::YES)?);
        writer.add_document(&d1)?;

        if i == 1 {
            writer.close()?;
            writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
        } else {
            writer.commit()?;
        }

        let mut d2 = Document::new();
        d2.add(text("f2", "second field", Store::NO)?);
        d2.add(text("f1", "first field", Store::YES)?);
        d2.add(text("f3", "third field", Store::NO)?);
        d2.add(text("f4", "fourth field", Store::NO)?);
        writer.add_document(&d2)?;

        writer.close()?;

        let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
        assert_equals(&2, &sis.size());

        let fis1 = field_infos(&sis, 0)?;
        let fis2 = field_infos(&sis, 1)?;

        assert_equals("f1", &name_of(&fis1, 0));
        assert_equals("f2", &name_of(&fis1, 1));
        assert_equals("f1", &name_of(&fis2, 0));
        assert_equals("f2", &name_of(&fis2, 1));
        assert_equals("f3", &name_of(&fis2, 2));
        assert_equals("f4", &name_of(&fis2, 3));

        let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
        writer.force_merge(1)?;
        writer.close()?;

        let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
        assert_equals(&1, &sis.size());

        let fis3 = field_infos(&sis, 0)?;

        assert_equals("f1", &name_of(&fis3, 0));
        assert_equals("f2", &name_of(&fis3, 1));
        assert_equals("f3", &name_of(&fis3, 2));
        assert_equals("f4", &name_of(&fis3, 3));

        dir.close()?;
    }
    Ok(())
}

/// Port of `TestConsistentFieldNumbers.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    let dir1 = LuceneTestCase::new_directory();
    let dir2 = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir1.as_directory(), no_merge_iwc()?)?;

    let mut d1 = Document::new();
    d1.add(text("f1", "first field", Store::YES)?);
    d1.add(text("f2", "second field", Store::YES)?);
    writer.add_document(&d1)?;

    writer.close()?;
    let writer = IndexWriter::new(dir2.as_directory(), no_merge_iwc()?)?;

    let mut d2 = Document::new();
    d2.add(text("f2", "second field", Store::YES)?);
    d2.add(text("f1", "first field", Store::YES)?);
    d2.add(text("f3", "third field", Store::YES)?);
    d2.add(text("f4", "fourth field", Store::YES)?);
    writer.add_document(&d2)?;

    writer.close()?;

    let writer = IndexWriter::new(dir1.as_directory(), no_merge_iwc()?)?;
    writer.add_indexes_dirs(&[dir2.as_directory()])?;
    writer.close()?;

    let sis = SegmentInfos::read_latest_commit(dir1.as_directory())?;
    assert_equals(&2, &sis.size());

    let fis1 = field_infos(&sis, 0)?;
    let fis2 = field_infos(&sis, 1)?;

    assert_equals("f1", &name_of(&fis1, 0));
    assert_equals("f2", &name_of(&fis1, 1));
    // make sure the ordering of the "external" segment is preserved
    assert_equals("f2", &name_of(&fis2, 0));
    assert_equals("f1", &name_of(&fis2, 1));
    assert_equals("f3", &name_of(&fis2, 2));
    assert_equals("f4", &name_of(&fis2, 3));

    dir1.close()?;
    dir2.close()
}

/// Port of `TestConsistentFieldNumbers.testFieldNumberGaps`.
fn test_field_number_gaps() -> Result<()> {
    let num_iters = LuceneTestCase::at_least(13);
    for _ in 0..num_iters {
        let dir = LuceneTestCase::new_directory();
        {
            let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
            let mut d = Document::new();
            d.add(text("f1", "d1 first field", Store::YES)?);
            d.add(text("f2", "d1 second field", Store::YES)?);
            writer.add_document(&d)?;
            writer.close()?;
            let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
            assert_equals(&1, &sis.size());
            let fis1 = field_infos(&sis, 0)?;
            assert_equals("f1", &name_of(&fis1, 0));
            assert_equals("f2", &name_of(&fis1, 1));
        }

        {
            let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
            let mut d = Document::new();
            d.add(text("f1", "d2 first field", Store::YES)?);
            d.add(Box::new(StoredField::new_bytes(
                "f3",
                BytesRef::new(vec![1, 2, 3]),
            )?));
            writer.add_document(&d)?;
            writer.close()?;
            let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
            assert_equals(&2, &sis.size());
            let fis1 = field_infos(&sis, 0)?;
            let fis2 = field_infos(&sis, 1)?;
            assert_equals("f1", &name_of(&fis1, 0));
            assert_equals("f2", &name_of(&fis1, 1));
            assert_equals("f1", &name_of(&fis2, 0));
            assert_null(&fis2.field_info_by_number(1).map(|f| f.name.clone()));
            assert_equals("f3", &name_of(&fis2, 2));
        }

        {
            let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
            let mut d = Document::new();
            d.add(text("f1", "d3 first field", Store::YES)?);
            d.add(text("f2", "d3 second field", Store::YES)?);
            d.add(Box::new(StoredField::new_bytes(
                "f3",
                BytesRef::new(vec![1, 2, 3, 4, 5]),
            )?));
            writer.add_document(&d)?;
            writer.close()?;
            let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
            assert_equals(&3, &sis.size());
            let fis1 = field_infos(&sis, 0)?;
            let fis2 = field_infos(&sis, 1)?;
            let fis3 = field_infos(&sis, 2)?;
            assert_equals("f1", &name_of(&fis1, 0));
            assert_equals("f2", &name_of(&fis1, 1));
            assert_equals("f1", &name_of(&fis2, 0));
            assert_null(&fis2.field_info_by_number(1).map(|f| f.name.clone()));
            assert_equals("f3", &name_of(&fis2, 2));
            assert_equals("f1", &name_of(&fis3, 0));
            assert_equals("f2", &name_of(&fis3, 1));
            assert_equals("f3", &name_of(&fis3, 2));
        }

        {
            let writer = IndexWriter::new(dir.as_directory(), no_merge_iwc()?)?;
            writer.delete_documents_by_terms(vec![Term::from_text("f1", "d1")])?;
            // nuke the first segment entirely so that the segment with gaps is
            // loaded first!
            writer.force_merge_deletes()?;
            writer.close()?;
        }

        let iwc = mock_iwc();
        iwc.set_merge_policy(Some(Arc::new(LogMergePolicy::by_byte_size())))?;
        iwc.set_info_stream(Arc::new(FailOnNonBulkMergesInfoStream))?;
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        writer.force_merge(1)?;
        writer.close()?;

        let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
        assert_equals(&1, &sis.size());
        let fis1 = field_infos(&sis, 0)?;
        assert_equals("f1", &name_of(&fis1, 0));
        assert_equals("f2", &name_of(&fis1, 1));
        assert_equals("f3", &name_of(&fis1, 2));
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestConsistentFieldNumbers.testManyFields`.
fn test_many_fields() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(200);
    let max_fields = LuceneTestCase::at_least(50);

    let mut docs = vec![[0i32; 4]; num_docs as usize];
    for doc in docs.iter_mut() {
        for v in doc.iter_mut() {
            *v = random().next_int_bound(max_fields);
        }
    }

    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;

    for doc in &docs {
        let mut d = Document::new();
        for &number in doc {
            d.add(get_field(number)?);
        }

        writer.add_document(&d)?;
    }

    writer.force_merge(1)?;
    writer.close()?;

    let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
    for si in sis.iter() {
        let fis = IndexWriter::read_field_infos(si)?;

        for fi in fis.iter() {
            let number: i32 = fi.name.parse().unwrap_or_else(|_| {
                randomizedtesting::jdk::exception::Throwable::new(
                    "java.lang.NumberFormatException",
                    format!("For input string: \"{}\"", fi.name),
                )
                .throw()
            });
            let expected = get_field(number)?;
            assert_equals(
                &expected.field_type().index_options(),
                &fi.get_index_options(),
            );
            assert_equals(
                &expected.field_type().store_term_vectors(),
                &fi.has_term_vectors(),
            );
        }
    }

    dir.close()
}

/// `private Field getField(int number)`.
fn get_field(number: i32) -> Result<Box<dyn IndexableField>> {
    let mode = number % 16;
    let field_name = number.to_string();
    let custom_type = FieldType::new_from(TextField::type_stored());

    let custom_type2 = FieldType::new_from(TextField::type_stored());
    custom_type2.set_tokenized(false)?;

    let custom_type3 = FieldType::new_from(TextField::type_not_stored());
    custom_type3.set_tokenized(false)?;

    let custom_type4 = FieldType::new_from(TextField::type_not_stored());
    custom_type4.set_tokenized(false)?;
    custom_type4.set_store_term_vectors(true)?;
    custom_type4.set_store_term_vector_offsets(true)?;

    let custom_type5 = FieldType::new_from(TextField::type_not_stored());
    custom_type5.set_store_term_vectors(true)?;
    custom_type5.set_store_term_vector_offsets(true)?;

    let custom_type6 = FieldType::new_from(TextField::type_stored());
    custom_type6.set_tokenized(false)?;
    custom_type6.set_store_term_vectors(true)?;
    custom_type6.set_store_term_vector_offsets(true)?;

    let custom_type7 = FieldType::new_from(TextField::type_not_stored());
    custom_type7.set_tokenized(false)?;
    custom_type7.set_store_term_vectors(true)?;
    custom_type7.set_store_term_vector_offsets(true)?;

    let custom_type8 = FieldType::new_from(TextField::type_stored());
    custom_type8.set_tokenized(false)?;
    custom_type8.set_store_term_vectors(true)?;
    custom_type8.set_store_term_vector_positions(true)?;

    let custom_type9 = FieldType::new_from(TextField::type_not_stored());
    custom_type9.set_store_term_vectors(true)?;
    custom_type9.set_store_term_vector_positions(true)?;

    let custom_type10 = FieldType::new_from(TextField::type_stored());
    custom_type10.set_tokenized(false)?;
    custom_type10.set_store_term_vectors(true)?;
    custom_type10.set_store_term_vector_positions(true)?;

    let custom_type11 = FieldType::new_from(TextField::type_not_stored());
    custom_type11.set_tokenized(false)?;
    custom_type11.set_store_term_vectors(true)?;
    custom_type11.set_store_term_vector_positions(true)?;

    let custom_type12 = FieldType::new_from(TextField::type_stored());
    custom_type12.set_store_term_vectors(true)?;
    custom_type12.set_store_term_vector_offsets(true)?;
    custom_type12.set_store_term_vector_positions(true)?;

    let custom_type13 = FieldType::new_from(TextField::type_not_stored());
    custom_type13.set_store_term_vectors(true)?;
    custom_type13.set_store_term_vector_offsets(true)?;
    custom_type13.set_store_term_vector_positions(true)?;

    let custom_type14 = FieldType::new_from(TextField::type_stored());
    custom_type14.set_tokenized(false)?;
    custom_type14.set_store_term_vectors(true)?;
    custom_type14.set_store_term_vector_offsets(true)?;
    custom_type14.set_store_term_vector_positions(true)?;

    let custom_type15 = FieldType::new_from(TextField::type_not_stored());
    custom_type15.set_tokenized(false)?;
    custom_type15.set_store_term_vectors(true)?;
    custom_type15.set_store_term_vector_offsets(true)?;
    custom_type15.set_store_term_vector_positions(true)?;

    let field = |t: FieldType| -> Result<Box<dyn IndexableField>> {
        Ok(Box::new(Field::new(
            &field_name,
            "some text".to_string(),
            t,
        )?))
    };
    match mode {
        0 => field(custom_type),
        1 => Ok(Box::new(TextField::new(
            &field_name,
            "some text".to_string(),
            Store::NO,
        )?)),
        2 => field(custom_type2),
        3 => field(custom_type3),
        4 => field(custom_type4),
        5 => field(custom_type5),
        6 => field(custom_type6),
        7 => field(custom_type7),
        8 => field(custom_type8),
        9 => field(custom_type9),
        10 => field(custom_type10),
        11 => field(custom_type11),
        12 => field(custom_type12),
        13 => field(custom_type13),
        14 => field(custom_type14),
        15 => field(custom_type15),
        // `return null`, which `d.add` dereferences.
        _ => crate::support::npe("getField(number)"),
    }
}
