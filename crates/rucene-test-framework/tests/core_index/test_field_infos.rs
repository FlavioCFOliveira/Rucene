//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFieldInfos.java`.
//!
//! * `assertSame(FieldInfos.EMPTY, actual)` and `assertSame(expected,
//!   actual)` compare the infos for equality: `FieldInfos.getMergedFieldInfos`
//!   returns an owned `FieldInfos` (guide section 6, `assertSame`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::field_infos::{FieldInfo, FieldInfos, FieldNumbers};
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexReader, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::util::java_collections::{JavaHashMap, JavaStringMap};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, assert_null, class, random, test};
use crate::support::npe;

/// `TestFieldInfos`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFieldInfos",
        vec![
            test("testFieldInfos", test_field_infos),
            test("testFieldAttributes", test_field_attributes),
            test(
                "testFieldAttributesSingleSegment",
                test_field_attributes_single_segment,
            ),
            test("testMergedFieldInfos_empty", test_merged_field_infos_empty),
            test(
                "testMergedFieldInfos_singleLeaf",
                test_merged_field_infos_single_leaf,
            ),
            test(
                "testFieldNumbersAutoIncrement",
                test_field_numbers_auto_increment,
            ),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random())).setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    IndexWriter::new(dir.as_directory(), iwc)
}

/// `new IndexWriter(dir, newIndexWriterConfig(new MockAnalyzer(random())))`.
fn writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random()))),
    )
}

/// `fis.fieldInfo(n).name`.
fn name_by_number(fis: &FieldInfos, number: i32) -> String {
    fis.field_info_by_number(number)
        .unwrap_or_else(|| npe("fieldInfo"))
        .name
        .clone()
}

/// `fis.fieldInfo(name).name`.
fn name_by_name(fis: &FieldInfos, name: &str) -> String {
    fis.field_info(name)
        .unwrap_or_else(|| npe("fieldInfo"))
        .name
        .clone()
}

/// Port of `TestFieldInfos.testFieldInfos`.
fn test_field_infos() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = no_merge_writer(&dir)?;

    let mut d1 = Document::new();
    for i in 0..15 {
        d1.add(Box::new(StringField::new(
            &format!("f{i}"),
            format!("v{i}"),
            Store::YES,
        )?));
    }
    writer.add_document(&d1)?;
    writer.commit()?;

    let mut d2 = Document::new();
    d2.add(Box::new(StringField::new(
        "f0",
        "v0".to_string(),
        Store::YES,
    )?));
    d2.add(Box::new(StringField::new(
        "f15",
        "v15".to_string(),
        Store::YES,
    )?));
    d2.add(Box::new(StringField::new(
        "f16",
        "v16".to_string(),
        Store::YES,
    )?));
    writer.add_document(&d2)?;
    writer.commit()?;

    let d3 = Document::new();
    writer.add_document(&d3)?;
    writer.close()?;

    let sis = SegmentInfos::read_latest_commit(dir.as_directory())?;
    assert_equals(&3, &sis.size());

    let fis1 = IndexWriter::read_field_infos(sis.info(0))?;
    let fis2 = IndexWriter::read_field_infos(sis.info(1))?;
    let fis3 = IndexWriter::read_field_infos(sis.info(2))?;

    // testing dense FieldInfos
    let mut i = 0;
    for fi in fis1.iter() {
        assert_equals(&i, &fi.number);
        assert_equals(&format!("f{i}"), &fi.name);
        assert_equals(&format!("f{i}"), &name_by_number(&fis1, i)); // lookup by number
        assert_equals(&format!("f{i}"), &name_by_name(&fis1, &format!("f{i}"))); // lookup by name
        i += 1;
    }

    // testing sparse FieldInfos
    assert_equals("f0", name_by_number(&fis2, 0).as_str()); // lookup by number
    assert_equals("f0", name_by_name(&fis2, "f0").as_str()); // lookup by name
    assert_null(&fis2.field_info_by_number(1));
    assert_null(&fis2.field_info("f1"));
    assert_equals("f15", name_by_number(&fis2, 15).as_str());
    assert_equals("f15", name_by_name(&fis2, "f15").as_str());
    assert_equals("f16", name_by_number(&fis2, 16).as_str());
    assert_equals("f16", name_by_name(&fis2, "f16").as_str());

    // testing empty FieldInfos
    assert_null(&fis3.field_info_by_number(0)); // lookup by number
    assert_null(&fis3.field_info("f0")); // lookup by name
    assert_equals(&0, &fis3.size());
    let mut it3 = fis3.iter();
    Assert::assert_false(it3.next().is_some());
    dir.close()
}

/// `new FieldType()` with `setStored(true)`.
fn stored_type() -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_stored(true)?;
    Ok(t)
}

/// `fi.getAttribute(key)`.
fn attribute(fi: &FieldInfo, key: &str) -> Option<String> {
    fi.get_attribute(key)
}

/// Port of `TestFieldInfos.testFieldAttributes`.
fn test_field_attributes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = no_merge_writer(&dir)?;

    let mut type1 = stored_type()?;
    type1.put_attribute("testKey1".to_string(), "testValue1".to_string())?;

    let mut d1 = Document::new();
    d1.add(Box::new(Field::new("f1", "v1".to_string(), type1.clone())?));
    let mut type2 = FieldType::new_from(&type1);
    // changing the value after copying shouldn't impact the original type1
    type2.put_attribute("testKey1".to_string(), "testValue2".to_string())?;
    writer.add_document(&d1)?;
    writer.commit()?;

    let mut d2 = Document::new();
    type1.put_attribute("testKey1".to_string(), "testValueX".to_string())?;
    type1.put_attribute("testKey2".to_string(), "testValue2".to_string())?;
    d2.add(Box::new(Field::new("f1", "v2".to_string(), type1.clone())?));
    d2.add(Box::new(Field::new("f2", "v2".to_string(), type2.clone())?));
    writer.add_document(&d2)?;
    writer.commit()?;
    writer.force_merge(1)?;

    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let fis = FieldInfos::get_merged_field_infos(Arc::clone(&reader))?;
    assert_equals(&fis.size(), &2);
    for fi in fis.iter() {
        match fi.name.as_str() {
            "f1" => {
                // testKey1 can point to either testValue1 or testValueX based on the order
                // of merge, but we see textValueX winning here since segment_2 is merged on segment_1.
                assert_equals(&Some("testValueX".to_string()), &attribute(fi, "testKey1"));
                assert_equals(&Some("testValue2".to_string()), &attribute(fi, "testKey2"));
            }
            "f2" => {
                assert_equals(&Some("testValue2".to_string()), &attribute(fi, "testKey1"));
            }
            _ => Assert::fail("Unknown field"),
        }
    }
    reader.close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestFieldInfos.testFieldAttributesSingleSegment`.
fn test_field_attributes_single_segment() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = no_merge_writer(&dir)?;

    let mut d1 = Document::new();
    let mut type1 = stored_type()?;
    type1.put_attribute("att1".to_string(), "attdoc1".to_string())?;
    d1.add(Box::new(Field::new("f1", "v1".to_string(), type1.clone())?));
    // add field with the same name and an extra attribute
    type1.put_attribute("att2".to_string(), "attdoc1".to_string())?;
    d1.add(Box::new(Field::new("f1", "v1".to_string(), type1.clone())?));
    writer.add_document(&d1)?;

    let mut d2 = Document::new();
    type1.put_attribute("att1".to_string(), "attdoc2".to_string())?;
    type1.put_attribute("att2".to_string(), "attdoc2".to_string())?;
    type1.put_attribute("att3".to_string(), "attdoc2".to_string())?;
    let mut type2 = stored_type()?;
    type2.put_attribute("att4".to_string(), "attdoc2".to_string())?;
    d2.add(Box::new(Field::new("f1", "v2".to_string(), type1.clone())?));
    d2.add(Box::new(Field::new("f2", "v2".to_string(), type2.clone())?));
    writer.add_document(&d2)?;
    writer.commit()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let fis = FieldInfos::get_merged_field_infos(Arc::clone(&reader))?;

    // test that attributes for f1 are introduced by d1,
    // and not modified by d2
    let fi1 = fis.field_info("f1").unwrap_or_else(|| npe("fi1"));
    assert_equals(&Some("attdoc1".to_string()), &attribute(fi1, "att1"));
    assert_equals(&Some("attdoc1".to_string()), &attribute(fi1, "att2"));
    assert_null(&attribute(fi1, "att3"));

    // test that attributes for f2 are introduced by d2
    let fi2 = fis.field_info("f2").unwrap_or_else(|| npe("fi2"));
    assert_equals(&Some("attdoc2".to_string()), &attribute(fi2, "att4"));

    reader.close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestFieldInfos.testMergedFieldInfos_empty`.
fn test_merged_field_infos_empty() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let actual = FieldInfos::get_merged_field_infos(Arc::clone(&reader))?;

    assert_equals(&FieldInfos::empty(), &actual);

    reader.close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestFieldInfos.testMergedFieldInfos_singleLeaf`.
fn test_merged_field_infos_single_leaf() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir)?;

    let mut d1 = Document::new();
    d1.add(Box::new(StringField::new(
        "f1",
        "v1".to_string(),
        Store::YES,
    )?));
    writer.add_document(&d1)?;
    writer.commit()?;

    let mut d2 = Document::new();
    d2.add(Box::new(StringField::new(
        "f2",
        "v2".to_string(),
        Store::YES,
    )?));
    writer.add_document(&d2)?;
    writer.commit()?;

    writer.force_merge(1)?;

    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let actual = FieldInfos::get_merged_field_infos(Arc::clone(&reader))?;
    let expected = Arc::clone(&reader).leaves()?[0]
        .leaf_reader()
        .get_field_infos();

    assert_equals(&1, &Arc::clone(&reader).leaves()?.len());
    assert_equals(&expected, &actual);

    reader.close()?;
    writer.close()?;
    dir.close()
}

/// `new FieldInfo(name, -1, false, false, false, IndexOptions.NONE,
/// DocValuesType.NONE, DocValuesSkipIndexType.NONE, -1, new HashMap<>(), 0, 0,
/// 0, 0, VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, false,
/// false)`.
fn plain_field_info(name: &str) -> Result<FieldInfo> {
    FieldInfo::new_full(
        name,
        -1,
        false,
        false,
        false,
        IndexOptions::NONE,
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        JavaStringMap::Hash(JavaHashMap::new()),
        0,
        0,
        0,
        0,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
        false,
        false,
    )
}

/// Port of `TestFieldInfos.testFieldNumbersAutoIncrement`.
fn test_field_numbers_auto_increment() -> Result<()> {
    let field_numbers = FieldNumbers::new(
        Some("softDeletes".to_string()),
        Some("parentDoc".to_string()),
    )?;
    for i in 0..10 {
        field_numbers.add_or_get(&plain_field_info(&format!("field{i}"))?)?;
    }
    let mut idx = field_numbers.add_or_get(&plain_field_info("EleventhField")?)?;
    assert_equals_msg(Some("Field numbers 0 through 9 were allocated"), &10, &idx);

    field_numbers.clear();
    idx = field_numbers.add_or_get(&plain_field_info("PostClearField")?)?;
    assert_equals_msg(Some("Field numbers should reset after clear()"), &0, &idx);
    Ok(())
}
