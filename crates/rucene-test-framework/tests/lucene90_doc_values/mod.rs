//! The own test methods of
//! `lucene/core/src/test/org/apache/lucene/codecs/lucene90/TestLucene90DocValuesFormat.java`
//! and `TestLucene90DocValuesFormatVariableSkipInterval.java`, which
//! `base_doc_values_format_suites.rs` declares on the classes' bindings.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::analysis::StandardAnalyzer;
use rucene::codecs::lucene90::doc_values::{
    Lucene90DocValuesFormat, DATA_EXTENSION, DIRECT_MONOTONIC_BLOCK_SHIFT, NUMERIC_BLOCK_SIZE,
    TERMS_DICT_BLOCK_LZ4_SIZE,
};
use rucene::codecs::per_field::PerFieldDocValuesFormat;
use rucene::codecs::{Codec, DocValuesFormat, PostingsFormat};
use rucene::document::{
    BinaryDocValuesField, Document, Field, NumericDocValuesField, NumericValue,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{
    DocValuesSkipper, IndexReader, IndexableField, NumericDocValues, SeekStatus, Term, TermsEnum,
};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{
    ByteBuffersDataOutput, DataInput, DataOutput, Directory, IOContext, IndexInput, IndexOutput,
    Lock,
};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::{
    AssertingCodec, DocValuesFormatForField, PostingsFormatForField,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::{Nightly, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use randomizedtesting::jdk::reflect::Method;

use super::{CompressingDocValuesSuite, DocValuesSuite};

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// The `NullPointerException` of calling a method on `null`.
fn npe(what: &str) -> ! {
    Throwable::new("java.lang.NullPointerException", format!("{what} is null")).throw()
}

/// The `i`-th field of class `T` named `name` that the Java test keeps after
/// `doc.add` (`field1`, `field2`, ...), reached through the document.
fn nth_field_mut<'a, T: rucene::index::IndexableField + 'static>(
    doc: &'a mut Document,
    name: &str,
    i: usize,
) -> &'a T {
    doc.get_fields_by_name_mut(name)
        .into_iter()
        .filter_map(|f| f.as_any().and_then(|a| a.downcast_ref::<T>()))
        .nth(i)
        .expect("INVARIANT: the document holds the field the test keeps")
}

/// The field of class `T` named `name` that the Java test keeps after
/// `doc.add`, reached through the document.
fn field_mut<'a, T: rucene::index::IndexableField + 'static>(
    doc: &'a mut Document,
    name: &str,
) -> &'a T {
    nth_field_mut::<T>(doc, name, 0)
}

/// `new BytesRef(CharSequence)`.
fn bytes(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

// -----------------------------------------------------------------------------
// TestLucene90DocValuesFormat
// -----------------------------------------------------------------------------

/// The fully qualified name of `TestLucene90DocValuesFormat`, which its
/// binding declares.
pub const LUCENE90_CLASS: &str = "org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormat";

/// A test method `TestLucene90DocValuesFormat` declares itself, run on the
/// class's instance.
fn lucene90_test(name: &str, body: fn(&CompressingDocValuesSuite) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut FormatTestInstance| {
            let suite = inst.subclass::<CompressingDocValuesSuite>();
            body(&suite).unwrap_or_else(|e| rethrow(e));
        })
}

/// The own test methods of `TestLucene90DocValuesFormat`, with their
/// annotations, in declaration order.
pub fn lucene90_methods() -> Vec<Method> {
    vec![
        lucene90_test(
            "testSortedSetVariableLengthBigVsStoredFields",
            test_sorted_set_variable_length_big_vs_stored_fields,
        ),
        lucene90_test(
            "testSortedSetVariableLengthManyVsStoredFields",
            test_sorted_set_variable_length_many_vs_stored_fields,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSortedVariableLengthBigVsStoredFields",
            test_sorted_variable_length_big_vs_stored_fields,
        ),
        lucene90_test(
            "testSortedVariableLengthManyVsStoredFields",
            test_sorted_variable_length_many_vs_stored_fields,
        )
        .annotate(Nightly),
        lucene90_test("testTermsEnumFixedWidth", test_terms_enum_fixed_width).annotate(Nightly),
        lucene90_test("testTermsEnumVariableWidth", test_terms_enum_variable_width)
            .annotate(Nightly),
        lucene90_test("testTermsEnumRandomMany", test_terms_enum_random_many).annotate(Nightly),
        lucene90_test(
            "testTermsEnumLongSharedPrefixes",
            test_terms_enum_long_shared_prefixes,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSparseDocValuesVsStoredFields",
            test_sparse_doc_values_vs_stored_fields,
        ),
        lucene90_test(
            "testDenseNumericLongValuesBulkFetch",
            test_dense_numeric_long_values_bulk_fetch,
        ),
        lucene90_test(
            "testSortedSetAroundBlockSize",
            test_sorted_set_around_block_size,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSortedNumericAroundBlockSize",
            test_sorted_numeric_around_block_size,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSortedNumericBlocksOfVariousBitsPerValue",
            test_sorted_numeric_blocks_of_various_bits_per_value,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSparseSortedNumericBlocksOfVariousBitsPerValue",
            test_sparse_sorted_numeric_blocks_of_various_bits_per_value,
        )
        .annotate(Nightly),
        lucene90_test(
            "testNumericBlocksOfVariousBitsPerValue",
            test_numeric_blocks_of_various_bits_per_value,
        )
        .annotate(Nightly),
        lucene90_test(
            "testSparseNumericBlocksOfVariousBitsPerValue",
            test_sparse_numeric_blocks_of_various_bits_per_value,
        )
        .annotate(Nightly),
        lucene90_test("testNumericFieldJumpTables", test_numeric_field_jump_tables)
            .annotate(Nightly),
        lucene90_test(
            "testReseekAfterSkipDecompression",
            test_reseek_after_skip_decompression,
        ),
        lucene90_test("testLargeTermsCompression", test_large_terms_compression),
        lucene90_test(
            "testSortedTermsDictLookupOrd",
            test_sorted_terms_dict_lookup_ord,
        ),
        lucene90_test(
            "testSortedSetTermsDictLookupOrd",
            test_sorted_set_terms_dict_lookup_ord,
        ),
        lucene90_test("testTermsEnumDictionary", test_terms_enum_dictionary),
        lucene90_test("testTermsEnumConsistency", test_terms_enum_consistency),
        lucene90_test(
            "testSkipIndexStoredSeparately",
            test_skip_index_stored_separately,
        ),
    ]
}

// TODO: these big methods can easily blow up some of the other ram-hungry codecs...
// for now just keep them here, as we want to test this for this format.

/// Port of `TestLucene90DocValuesFormat.testSortedSetVariableLengthBigVsStoredFields`.
fn test_sorted_set_variable_length_big_vs_stored_fields(
    suite: &CompressingDocValuesSuite,
) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        let num_docs = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(100)
        } else {
            LuceneTestCase::at_least(10)
        };
        suite.do_test_sorted_set_vs_stored_fields(num_docs, 1, 32766, 16, 100)?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedSetVariableLengthManyVsStoredFields`.
fn test_sorted_set_variable_length_many_vs_stored_fields(
    suite: &CompressingDocValuesSuite,
) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        suite.do_test_sorted_set_vs_stored_fields(
            TestUtil::next_int(&*random(), 1024, 2049),
            1,
            500,
            16,
            100,
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedVariableLengthBigVsStoredFields`.
fn test_sorted_variable_length_big_vs_stored_fields(
    suite: &CompressingDocValuesSuite,
) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        suite.do_test_sorted_vs_stored_fields_lengths(
            LuceneTestCase::at_least(100),
            1.0,
            1,
            32766,
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedVariableLengthManyVsStoredFields`.
fn test_sorted_variable_length_many_vs_stored_fields(
    suite: &CompressingDocValuesSuite,
) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        suite.do_test_sorted_vs_stored_fields_lengths(
            TestUtil::next_int(&*random(), 1024, 2049),
            1.0,
            1,
            500,
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testTermsEnumFixedWidth`.
fn test_terms_enum_fixed_width(suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        do_test_terms_enum_random(
            suite,
            TestUtil::next_int(&*random(), 1025, 5121),
            &mut || TestUtil::random_simple_string_between(&*random(), 10, 10),
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testTermsEnumVariableWidth`.
fn test_terms_enum_variable_width(suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        do_test_terms_enum_random(
            suite,
            TestUtil::next_int(&*random(), 1025, 5121),
            &mut || TestUtil::random_simple_string_between(&*random(), 1, 500),
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testTermsEnumRandomMany`.
fn test_terms_enum_random_many(suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        do_test_terms_enum_random(
            suite,
            TestUtil::next_int(&*random(), 1025, 8121),
            &mut || TestUtil::random_simple_string_between(&*random(), 1, 500),
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testTermsEnumLongSharedPrefixes`.
fn test_terms_enum_long_shared_prefixes(suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        do_test_terms_enum_random(
            suite,
            TestUtil::next_int(&*random(), 1025, 5121),
            &mut || {
                let mut chars = vec![b'a'; random().next_int_bound(500) as usize];
                if !chars.is_empty() {
                    let at = random().next_int_bound(chars.len() as i32) as usize;
                    chars[at] = b'b';
                }
                String::from_utf8(chars).expect("ASCII")
            },
        )?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSparseDocValuesVsStoredFields`.
fn test_sparse_doc_values_vs_stored_fields(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        do_test_sparse_doc_values_vs_stored_fields()?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testDenseNumericLongValuesBulkFetch`.
fn test_dense_numeric_long_values_bulk_fetch(_suite: &CompressingDocValuesSuite) -> Result<()> {
    for bits_per_value in [8, 16, 24, 32, 40, 48, 56, 64] {
        do_test_dense_numeric_long_values_bulk_fetch(bits_per_value)?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.doTestDenseNumericLongValuesBulkFetch` (`private`).
fn do_test_dense_numeric_long_values_bulk_fetch(bits_per_value: i32) -> Result<()> {
    let num_docs = 512usize;
    let mut expected = vec![0i64; num_docs];
    let mask = if bits_per_value == 64 {
        -1i64
    } else {
        (1i64 << bits_per_value) - 1
    };
    for value in expected.iter_mut() {
        *value = if bits_per_value == 64 {
            random().next_long()
        } else {
            random().next_long() & mask
        };
    }

    let dir = LuceneTestCase::new_directory();
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    for value in &expected {
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("numeric", *value)));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;

    {
        let reader = directory_reader::open_with_writer(&*writer)?;
        let leaf = Arc::clone(&reader).leaves()?[0].leaf_reader();
        let mut values = leaf
            .get_numeric_doc_values("numeric")?
            .unwrap_or_else(|| npe("values"));
        let mut single_values = leaf
            .get_numeric_doc_values("numeric")?
            .unwrap_or_else(|| npe("singleValues"));
        assert_bulk_long_values(
            bits_per_value,
            &mut *values,
            &mut *single_values,
            &expected,
            0,
            128,
            1,
        )?;
        assert_bulk_long_values(
            bits_per_value,
            &mut *values,
            &mut *single_values,
            &expected,
            17,
            128,
            1,
        )?;
        assert_bulk_long_values(
            bits_per_value,
            &mut *values,
            &mut *single_values,
            &expected,
            0,
            128,
            2,
        )?;
        drop((values, single_values, leaf));
        reader.close()?;
    }

    writer.close()?;
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.assertBulkLongValues` (`private static`).
fn assert_bulk_long_values(
    bits_per_value: i32,
    values: &mut dyn NumericDocValues,
    single_values: &mut dyn NumericDocValues,
    expected: &[i64],
    start_doc: i32,
    size: i32,
    doc_step: i32,
) -> Result<()> {
    let mut docs = vec![0i32; size as usize];
    let mut actual = vec![0i64; size as usize];
    for (i, doc) in docs.iter_mut().enumerate() {
        *doc = start_doc + i as i32 * doc_step;
    }

    values.long_values_no_offsets(size, &docs, &mut actual, 0)?;
    if size != 0 {
        Assert::assert_equals(docs[size as usize - 1], values.doc_id());
    }
    for i in 0..size as usize {
        Assert::assert_true(single_values.advance_exact(docs[i])?);
        let single_value = single_values.long_value()?;
        Assert::assert_equals_msg(
            Some(&format!(
                "bitsPerValue={bits_per_value} doc={} startDoc={start_doc} docStep={doc_step} indexedValue={}",
                docs[i],
                expected[docs[i] as usize]
            )),
            single_value,
            actual[i],
        );
        Assert::assert_equals_msg(
            Some(&format!("bitsPerValue={bits_per_value} doc={}", docs[i])),
            expected[docs[i] as usize],
            single_value,
        );
    }

    // Verify offset-aware variant produces identical results
    if size > 2 {
        let docs_offset = 1usize;
        let slice_size = size as usize - 2;
        let mut offset_actual = vec![i64::MIN; slice_size + 4];
        let values_offset = 2usize;
        values.long_values(
            slice_size as i32,
            &docs,
            docs_offset as i32,
            &mut offset_actual,
            values_offset as i32,
            0,
        )?;
        for i in 0..slice_size {
            Assert::assert_equals_msg(
                Some(&format!(
                    "offset variant mismatch at i={i} bpv={bits_per_value}"
                )),
                actual[docs_offset + i],
                offset_actual[values_offset + i],
            );
        }
        // sentinel positions should be untouched
        Assert::assert_equals(i64::MIN, offset_actual[0]);
        Assert::assert_equals(i64::MIN, offset_actual[1]);
        Assert::assert_equals(i64::MIN, offset_actual[values_offset + slice_size]);
    }
    Ok(())
}

/// `field.numericValue().longValue()` of a stored field.
fn stored_long(field: &dyn rucene::index::IndexableField) -> Result<i64> {
    // `Number.longValue()`.
    Ok(
        match field
            .numeric_value()
            .unwrap_or_else(|| npe("numericValue()"))
        {
            NumericValue::Int(v) => i64::from(v),
            NumericValue::Long(v) => v,
            NumericValue::Float(v) => v as i64,
            NumericValue::Double(v) => v as i64,
        },
    )
}

/// Port of `TestLucene90DocValuesFormat.doTestSparseDocValuesVsStoredFields` (`private`).
fn do_test_sparse_doc_values_vs_stored_fields() -> Result<()> {
    let mut values = vec![0i64; TestUtil::next_int(&*random(), 1, 500) as usize];
    for value in values.iter_mut() {
        *value = random().next_long();
    }

    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

    // sparse compression is only enabled if less than 1% of docs have a value
    let avg_gap = 100;

    let num_docs = LuceneTestCase::at_least(200);
    let mut i = random().next_int_bound(avg_gap * 2);
    while i >= 0 {
        writer.add_document(&Document::new())?;
        i -= 1;
    }
    let max_num_values_per_doc = if random().next_boolean() {
        1
    } else {
        TestUtil::next_int(&*random(), 2, 5)
    };
    for _ in 0..num_docs {
        let mut doc = Document::new();

        // single-valued
        let mut doc_value = values[random().next_int_bound(values.len() as i32) as usize];
        doc.add(Box::new(NumericDocValuesField::new("numeric", doc_value)));
        doc.add(Box::new(SortedDocValuesField::new(
            "sorted",
            bytes(&doc_value.to_string()),
        )));
        doc.add(Box::new(BinaryDocValuesField::new(
            "binary",
            bytes(&doc_value.to_string()),
        )));
        doc.add(Box::new(StoredField::new_number(
            "value",
            NumericValue::Long(doc_value),
        )?));

        // multi-valued
        let num_values = TestUtil::next_int(&*random(), 1, max_num_values_per_doc);
        for _ in 0..num_values {
            doc_value = values[random().next_int_bound(values.len() as i32) as usize];
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "sorted_numeric",
                doc_value,
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "sorted_set",
                bytes(&doc_value.to_string()),
            )));
            doc.add(Box::new(StoredField::new_number(
                "values",
                NumericValue::Long(doc_value),
            )?));
        }

        writer.add_document(&doc)?;

        // add a gap
        let mut j = TestUtil::next_int(&*random(), 0, avg_gap * 2);
        while j >= 0 {
            writer.add_document(&Document::new())?;
            j -= 1;
        }
    }

    if random().next_boolean() {
        writer.force_merge(1)?;
    }

    let index_reader = writer.get_reader()?;
    writer.close()?;

    for context in Arc::clone(&index_reader).leaves()? {
        let reader = context.leaf_reader();
        let mut numeric = DocValues::get_numeric(&*reader, "numeric")?;

        let mut sorted = DocValues::get_sorted(&*reader, "sorted")?;

        let mut binary = DocValues::get_binary(&*reader, "binary")?;

        let mut sorted_numeric = DocValues::get_sorted_numeric(&*reader, "sorted_numeric")?;

        let mut sorted_set = DocValues::get_sorted_set(&*reader, "sorted_set")?;

        let stored_fields = reader.stored_fields()?;
        for i in 0..reader.max_doc() {
            let doc = stored_fields.document(i)?;
            let value = match doc.get_field("value") {
                None => None,
                Some(value_field) => Some(stored_long(value_field)?),
            };

            match value {
                None => {
                    Assert::assert_true_msg(
                        &format!("{} vs {i}", numeric.doc_id()),
                        numeric.doc_id() < i,
                    );
                }
                Some(value) => {
                    Assert::assert_equals(i, numeric.next_doc()?);
                    Assert::assert_equals(i, binary.next_doc()?);
                    Assert::assert_equals(i, sorted.next_doc()?);
                    Assert::assert_equals(value, numeric.long_value()?);
                    Assert::assert_true(sorted.ord_value()? >= 0);
                    Assert::assert_true(
                        bytes(&value.to_string()) == sorted.lookup_ord(sorted.ord_value()?)?,
                    );
                    Assert::assert_true(bytes(&value.to_string()) == binary.binary_value()?);
                }
            }

            let values_fields = doc.get_fields_by_name("values");
            if values_fields.is_empty() {
                Assert::assert_true_msg(
                    &format!("{} vs {i}", sorted_numeric.doc_id()),
                    sorted_numeric.doc_id() < i,
                );
            } else {
                let mut value_set: HashSet<i64> = HashSet::new();
                for sf in &values_fields {
                    value_set.insert(stored_long(*sf)?);
                }

                Assert::assert_equals(i, sorted_numeric.next_doc()?);
                Assert::assert_equals(
                    values_fields.len() as i32,
                    sorted_numeric.doc_value_count()?,
                );
                for _ in 0..sorted_numeric.doc_value_count()? {
                    Assert::assert_true(value_set.contains(&sorted_numeric.next_value()?));
                }
                Assert::assert_equals(i, sorted_set.next_doc()?);

                Assert::assert_equals(value_set.len() as i32, sorted_set.doc_value_count()?);
                for _ in 0..sorted_set.doc_value_count()? {
                    let ord = sorted_set.next_ord()?;
                    let term = sorted_set.lookup_ord(ord)?.utf8_to_string()?;
                    // `Long.parseLong` throws `NumberFormatException` on a malformed term.
                    let parsed: i64 = term.parse().unwrap_or_else(|_| {
                        Throwable::new(
                            "java.lang.NumberFormatException",
                            format!("For input string: \"{term}\""),
                        )
                        .throw()
                    });
                    Assert::assert_true(value_set.contains(&parsed));
                }
            }
        }
    }

    index_reader.close()?;
    dir.close()
}

/// `new AssertingCodec() { getPostingsFormatForField(field) { return pf; }
/// getDocValuesFormatForField(field) { return dv; } }`.
fn asserting_codec_with(
    pf: Arc<dyn PostingsFormat>,
    dv: Arc<dyn DocValuesFormat>,
) -> Arc<dyn Codec> {
    let postings: PostingsFormatForField = Arc::new(move |_field: &str| Arc::clone(&pf));
    let doc_values: DocValuesFormatForField = Arc::new(move |_field: &str| Arc::clone(&dv));
    Arc::new(AssertingCodec::with_formats_for_field(
        Some(postings),
        Some(doc_values),
        None,
    ))
}

// TODO: try to refactor this and some termsenum tests into the base class.
// to do this we need to fix the test class to get a DVF not a Codec so we can setup
// the postings format correctly.
/// Port of `TestLucene90DocValuesFormat.doTestTermsEnumRandom` (`private`).
fn do_test_terms_enum_random(
    suite: &CompressingDocValuesSuite,
    num_docs: i32,
    values_producer: &mut dyn FnMut() -> String,
) -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    // set to duel against a codec which has ordinals:
    let pf = TestUtil::get_postings_format_with_ords(&*random());
    let codec = suite.get_codec();
    let dv = codec
        .doc_values_format()
        .as_any()
        .and_then(|any| any.downcast_ref::<PerFieldDocValuesFormat>())
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "the doc values format is not a PerFieldDocValuesFormat",
            )
            .throw()
        })
        .get_doc_values_format_for_field("random_field_name");
    conf.set_codec(asserting_codec_with(pf, dv))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

    // index some docs
    for i in 0..num_docs {
        let mut doc = Document::new();
        let id_field = StringField::new("id", i.to_string(), Store::NO)?;
        doc.add(Box::new(id_field));
        let num_values = random().next_int_bound(17);
        // create a random list of strings
        let mut values: Vec<String> = Vec::new();
        for _ in 0..num_values {
            values.push(values_producer());
        }

        // add in any order to the indexed field
        let mut unordered = values.clone();
        shuffle(&mut unordered, &*random());
        for v in &values {
            doc.add(Box::new(LuceneTestCase::new_string_field(
                "indexed",
                v,
                Store::NO,
            )));
        }

        // add in any order to the dv field
        let mut unordered2 = values.clone();
        shuffle(&mut unordered2, &*random());
        for v in &unordered2 {
            doc.add(Box::new(SortedSetDocValuesField::new("dv", bytes(v))));
        }

        writer.add_document(&doc)?;
        if random().next_int_bound(31) == 0 {
            writer.commit()?;
        }
    }

    // delete some docs
    let num_deletions = random().next_int_bound(num_docs / 10);
    for _ in 0..num_deletions {
        let id = random().next_int_bound(num_docs);
        writer.delete_documents(Term::new("id", bytes(&id.to_string())))?;
    }

    // compare per-segment
    let ir = writer.get_reader()?;
    for context in Arc::clone(&ir).leaves()? {
        let r = context.leaf_reader();
        if let Some(terms) = r.terms("indexed")? {
            let ssdv = r
                .get_sorted_set_doc_values("dv")?
                .unwrap_or_else(|| npe("ssdv"));
            Assert::assert_equals(terms.size()?, ssdv.get_value_count()?);
            let mut expected = terms.iterator()?;
            let mut actual = r
                .get_sorted_set_doc_values("dv")?
                .unwrap_or_else(|| npe("getSortedSetDocValues(\"dv\")"))
                .terms_enum()?;
            assert_equals_terms_enum(terms.size()?, &mut *expected, &mut *actual)?;

            let leaf = Arc::clone(&r);
            suite.do_test_sorted_set_enum_advance_independently(&|| {
                Ok(leaf
                    .get_sorted_set_doc_values("dv")?
                    .unwrap_or_else(|| npe("ssdv")))
            })?;
        }
    }
    ir.close()?;

    writer.force_merge(1)?;

    // now compare again after the merge
    let ir = writer.get_reader()?;
    let ar = LuceneTestCase::get_only_leaf_reader(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    if let Some(terms) = ar.terms("indexed")? {
        Assert::assert_equals(
            terms.size()?,
            ar.get_sorted_set_doc_values("dv")?
                .unwrap_or_else(|| npe("getSortedSetDocValues(\"dv\")"))
                .get_value_count()?,
        );
        let mut expected = terms.iterator()?;
        let mut actual = ar
            .get_sorted_set_doc_values("dv")?
            .unwrap_or_else(|| npe("getSortedSetDocValues(\"dv\")"))
            .terms_enum()?;
        assert_equals_terms_enum(terms.size()?, &mut *expected, &mut *actual)?;
    }
    drop(ar);
    ir.close()?;

    writer.close()?;
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.assertEquals(long, TermsEnum, TermsEnum)`
/// (`private`).
fn assert_equals_terms_enum(
    num_ords: i64,
    expected: &mut dyn TermsEnum,
    actual: &mut dyn TermsEnum,
) -> Result<()> {
    // sequential next() through all terms
    while let Some(r#ref) = expected.next()? {
        Assert::assert_true(Some(r#ref) == actual.next()?);
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }
    Assert::assert_true(actual.next()?.is_none());

    // sequential seekExact(ord) through all terms
    for i in 0..num_ords {
        expected.seek_ord(i)?;
        actual.seek_ord(i)?;
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }

    // sequential seekExact(BytesRef) through all terms
    for i in 0..num_ords {
        expected.seek_ord(i)?;
        Assert::assert_true(
            actual.seek_exact(
                &expected
                    .term()?
                    .ok_or_else(rucene::error::LuceneError::null_pointer)?,
            )?,
        );
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }

    // sequential seekCeil(BytesRef) through all terms
    for i in 0..num_ords {
        expected.seek_ord(i)?;
        Assert::assert_true(
            SeekStatus::FOUND
                == actual.seek_ceil(
                    &expected
                        .term()?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?,
                )?,
        );
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }

    // random seekExact(ord)
    for _ in 0..num_ords {
        let random_ord = TestUtil::next_long(&*random(), 0, num_ords - 1);
        expected.seek_ord(random_ord)?;
        actual.seek_ord(random_ord)?;
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }

    // random seekExact(BytesRef)
    for _ in 0..num_ords {
        let random_ord = TestUtil::next_long(&*random(), 0, num_ords - 1);
        expected.seek_ord(random_ord)?;
        actual.seek_exact(
            &expected
                .term()?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?,
        )?;
        Assert::assert_equals(expected.ord()?, actual.ord()?);
        Assert::assert_equals_nullable(expected.term()?, actual.term()?);
    }

    // random seekCeil(BytesRef)
    for _ in 0..num_ords {
        let target = bytes(&TestUtil::random_unicode_string(&*random()));
        let expected_status = expected.seek_ceil(&target)?;
        Assert::assert_true(expected_status == actual.seek_ceil(&target)?);
        if expected_status != SeekStatus::END {
            Assert::assert_equals(expected.ord()?, actual.ord()?);
            Assert::assert_equals_nullable(expected.term()?, actual.term()?);
        }
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedSetAroundBlockSize`.
fn test_sorted_set_around_block_size(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let frontier = 1 << DIRECT_MONOTONIC_BLOCK_SHIFT;
    for max_doc in frontier - 1..=frontier + 1 {
        let dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut out = ByteBuffersDataOutput::new();
        let mut doc = Document::new();
        let field1 = SortedSetDocValuesField::new("sset", BytesRef::new(Vec::new()));
        doc.add(Box::new(field1));
        let field2 = SortedSetDocValuesField::new("sset", BytesRef::new(Vec::new()));
        doc.add(Box::new(field2));
        for _ in 0..max_doc {
            let s1 = bytes(&TestUtil::random_simple_string_max(&*random(), 2));
            let s2 = bytes(&TestUtil::random_simple_string_max(&*random(), 2));
            nth_field_mut::<SortedSetDocValuesField>(&mut doc, "sset", 0)
                .set_bytes_value(s1.clone())?;
            nth_field_mut::<SortedSetDocValuesField>(&mut doc, "sset", 1)
                .set_bytes_value(s2.clone())?;
            w.add_document(&doc)?;
            let set: BTreeSet<BytesRef> = BTreeSet::from([s1, s2]);
            out.write_v_int(set.len() as i32)?;
            for r#ref in &set {
                out.write_v_int(r#ref.length as i32)?;
                out.write_bytes(r#ref.slice(), 0, r#ref.length)?;
            }
        }

        w.force_merge(1)?;
        let r = directory_reader::open_with_writer(&*w)?;
        w.close()?;
        let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        Assert::assert_equals(max_doc, sr.max_doc());
        let values = sr.get_sorted_set_doc_values("sset")?;
        Assert::assert_true(values.is_some()); // assertNotNull(values)
        let mut values = values.expect("assertNotNull(values)");
        let mut input = out.to_data_input()?;
        for i in 0..max_doc {
            Assert::assert_equals(i, values.next_doc()?);
            let num_values = input.read_v_int()?;
            Assert::assert_equals(num_values, values.doc_value_count()?);

            for _ in 0..num_values {
                let length = input.read_v_int()? as usize;
                let mut b = vec![0u8; length];
                input.read_bytes(Some(&mut b), 0, length)?;
                let ord = values.next_ord()?;
                Assert::assert_true(BytesRef::new(b) == values.lookup_ord(ord)?);
            }
        }
        drop((values, sr));
        r.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedNumericAroundBlockSize`.
fn test_sorted_numeric_around_block_size(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let frontier = 1 << DIRECT_MONOTONIC_BLOCK_SHIFT;
    for max_doc in frontier - 1..=frontier + 1 {
        let dir = LuceneTestCase::new_directory();
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut buffer = ByteBuffersDataOutput::new();

        let mut doc = Document::new();
        let field1 = SortedNumericDocValuesField::new("snum", 0);
        doc.add(Box::new(field1));
        let field2 = SortedNumericDocValuesField::new("snum", 0);
        doc.add(Box::new(field2));
        for _ in 0..max_doc {
            let s1 = i64::from(random().next_int_bound(100));
            let s2 = i64::from(random().next_int_bound(100));
            nth_field_mut::<SortedNumericDocValuesField>(&mut doc, "snum", 0).set_long_value(s1);
            nth_field_mut::<SortedNumericDocValuesField>(&mut doc, "snum", 1).set_long_value(s2);
            w.add_document(&doc)?;
            buffer.write_v_long(s1.min(s2))?;
            buffer.write_v_long(s1.max(s2))?;
        }

        w.force_merge(1)?;
        let r = directory_reader::open_with_writer(&*w)?;
        w.close()?;
        let sr = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        Assert::assert_equals(max_doc, sr.max_doc());
        let values = sr.get_sorted_numeric_doc_values("snum")?;
        Assert::assert_true(values.is_some()); // assertNotNull(values)
        let mut values = values.expect("assertNotNull(values)");
        let mut data_input = buffer.to_data_input()?;
        for i in 0..max_doc {
            Assert::assert_equals(i, values.next_doc()?);
            Assert::assert_equals(2, values.doc_value_count()?);
            Assert::assert_equals(data_input.read_v_long()?, values.next_value()?);
            Assert::assert_equals(data_input.read_v_long()?, values.next_value()?);
        }
        drop((values, sr));
        r.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestLucene90DocValuesFormat.testSortedNumericBlocksOfVariousBitsPerValue`.
fn test_sorted_numeric_blocks_of_various_bits_per_value(
    _suite: &CompressingDocValuesSuite,
) -> Result<()> {
    do_test_sorted_numeric_blocks_of_various_bits_per_value(&mut || {
        i64::from(TestUtil::next_int(&*random(), 1, 3))
    })
}

/// Port of `TestLucene90DocValuesFormat.testSparseSortedNumericBlocksOfVariousBitsPerValue`.
fn test_sparse_sorted_numeric_blocks_of_various_bits_per_value(
    _suite: &CompressingDocValuesSuite,
) -> Result<()> {
    do_test_sorted_numeric_blocks_of_various_bits_per_value(&mut || {
        i64::from(TestUtil::next_int(&*random(), 0, 2))
    })
}

/// Port of `TestLucene90DocValuesFormat.testNumericBlocksOfVariousBitsPerValue`.
fn test_numeric_blocks_of_various_bits_per_value(suite: &CompressingDocValuesSuite) -> Result<()> {
    do_test_sparse_numeric_blocks_of_various_bits_per_value(suite, 1.0)
}

/// Port of `TestLucene90DocValuesFormat.testSparseNumericBlocksOfVariousBitsPerValue`.
fn test_sparse_numeric_blocks_of_various_bits_per_value(
    suite: &CompressingDocValuesSuite,
) -> Result<()> {
    do_test_sparse_numeric_blocks_of_various_bits_per_value(suite, random().next_double())
}

// The LUCENE-8585 jump-tables enables O(1) skipping of IndexedDISI blocks, DENSE block lookup
// and numeric multi blocks. This test focuses on testing these jumps.
/// Port of `TestLucene90DocValuesFormat.testNumericFieldJumpTables`.
fn test_numeric_field_jump_tables(suite: &CompressingDocValuesSuite) -> Result<()> {
    // IndexedDISI block skipping only activated if target >= current+2, so we need at least 5
    // blocks to
    // trigger consecutive block skips
    let max_doc = LuceneTestCase::at_least(5 * 65536);

    let dir = LuceneTestCase::new_directory();
    let iw = create_fast_index_writer(&dir.as_directory(), max_doc)?;

    let id_field = Arc::new(LuceneTestCase::new_string_field("id", "", Store::NO));
    let stored_field = Arc::new(LuceneTestCase::new_string_field("stored", "", Store::YES));
    let dv_field = Arc::new(NumericDocValuesField::new("dv", 0));

    for i in 0..max_doc {
        let mut doc = Document::new();
        id_field.set_string_value(format!("{i:b}"))?;
        doc.add_shared(Arc::clone(&id_field) as Arc<dyn IndexableField>);
        if random().next_int_bound(100) > 10 {
            // Skip 10% to make DENSE blocks
            let value = random().next_int_bound(100000);
            stored_field.set_string_value(value.to_string())?;
            doc.add_shared(Arc::clone(&stored_field) as Arc<dyn IndexableField>);
            dv_field.set_value(i64::from(value));
            doc.add_shared(Arc::clone(&dv_field) as Arc<dyn IndexableField>);
        }
        iw.add_document(&doc)?;
    }
    iw.flush()?;
    iw.force_merge_with(1, true)?; // Single segment to force large enough structures
    iw.commit()?;
    iw.close()?;

    let d = dir.as_directory();
    suite.assert_dv_iterate(&d)?;
    // 1 is heavy (~20 s), so we do it rarely. 7 is a lot faster (8 s)
    assert_dv_advance(&d, if LuceneTestCase::rarely() { 1 } else { 7 })?;

    drop(d);
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.createFastIndexWriter` (`private`).
fn create_fast_index_writer(
    dir: &Arc<dyn Directory>,
    max_buffered_docs: i32,
) -> Result<Arc<IndexWriter>> {
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_max_buffered_docs(max_buffered_docs)?;
    conf.set_ram_buffer_size_mb(-1.0)?;
    conf.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    IndexWriter::new(Arc::clone(dir), conf)
}

/// The anonymous `LongSupplier` of `blocksOfVariousBPV()`.
struct BlocksOfVariousBpv {
    mul: i64,
    min: i64,
    i: usize,
    max_delta: i32,
}

impl BlocksOfVariousBpv {
    /// `getAsLong()`.
    fn get_as_long(&mut self) -> i64 {
        if self.i == NUMERIC_BLOCK_SIZE {
            // change the range of the random generated values on block boundaries, so we exercise
            // different bits-per-value for each block, and encourage block compression
            self.max_delta = 1 << random().next_int_bound(5);
            self.i = 0;
        }
        self.i += 1;
        self.min.wrapping_add(
            self.mul
                .wrapping_mul(i64::from(random().next_int_bound(self.max_delta))),
        )
    }
}

/// Port of `TestLucene90DocValuesFormat.blocksOfVariousBPV` (`private static`).
fn blocks_of_various_bpv() -> BlocksOfVariousBpv {
    // this helps exercise GCD compression:
    let mul = i64::from(TestUtil::next_int(&*random(), 1, 100));
    let min = i64::from(random().next_int());
    BlocksOfVariousBpv {
        mul,
        min,
        i: NUMERIC_BLOCK_SIZE,
        max_delta: 0,
    }
}

/// Port of `TestLucene90DocValuesFormat.doTestSortedNumericBlocksOfVariousBitsPerValue`
/// (`private`).
fn do_test_sorted_numeric_blocks_of_various_bits_per_value(
    counts: &mut dyn FnMut() -> i64,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_max_buffered_docs(LuceneTestCase::at_least(NUMERIC_BLOCK_SIZE as i32))?;
    conf.set_ram_buffer_size_mb(-1.0)?;
    conf.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;

    let num_docs = LuceneTestCase::at_least(NUMERIC_BLOCK_SIZE as i32 * 3);
    let mut values = blocks_of_various_bpv();
    let mut write_doc_values: Vec<Vec<i64>> = Vec::new();
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let value_count = counts() as i32;
        let mut value_array = vec![0i64; value_count as usize];
        for slot in value_array.iter_mut() {
            let value = values.get_as_long();
            *slot = value;
            doc.add(Box::new(SortedNumericDocValuesField::new("dv", value)));
        }
        value_array.sort_unstable();
        for value in &value_array {
            doc.add(Box::new(StoredField::new_string(
                "stored",
                value.to_string(),
            )?));
        }
        write_doc_values.push(value_array);
        writer.add_document(&doc)?;
        if random().next_int_bound(31) == 0 {
            writer.commit()?;
        }
    }
    writer.force_merge(1)?;

    writer.close()?;

    // compare
    let ir = directory_reader::open(dir.as_directory())?;
    TestUtil::check_reader(&(Arc::clone(&ir) as Arc<dyn IndexReader>))?;
    for context in Arc::clone(&ir).leaves()? {
        let r = context.leaf_reader();
        let mut doc_values = DocValues::get_sorted_numeric(&*r, "dv")?;
        let stored_fields = r.stored_fields()?;
        for i in 0..r.max_doc() {
            if i > doc_values.doc_id() {
                doc_values.next_doc()?;
            }
            let expected_stored = stored_fields.document(i)?.get_values("stored");
            if i < doc_values.doc_id() {
                Assert::assert_equals(0, expected_stored.len());
            } else {
                let count = doc_values.doc_value_count()? as usize;
                let mut read_value_array = vec![0i64; count];
                let mut actual_doc_value = vec![String::new(); count];
                for j in 0..count {
                    let actual_dv = doc_values.next_value()?;
                    read_value_array[j] = actual_dv;
                    actual_doc_value[j] = read_value_array[j].to_string();
                }
                let write_value_array = &write_doc_values[i as usize];
                // compare write values and read values
                Assert::assert_true(read_value_array == *write_value_array);

                // compare dv and stored values
                Assert::assert_true(expected_stored == actual_doc_value);
            }
        }
    }
    ir.close()?;
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.doTestSparseNumericBlocksOfVariousBitsPerValue`
/// (`private`).
fn do_test_sparse_numeric_blocks_of_various_bits_per_value(
    suite: &CompressingDocValuesSuite,
    density: f64,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    conf.set_max_buffered_docs(LuceneTestCase::at_least(NUMERIC_BLOCK_SIZE as i32))?;
    conf.set_ram_buffer_size_mb(-1.0)?;
    conf.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(
        random().next_boolean(),
    )))?;
    let writer = IndexWriter::new(dir.as_directory(), conf)?;
    let mut doc = Document::new();
    let stored_field = LuceneTestCase::new_string_field("stored", "", Store::YES);
    let dv_field = NumericDocValuesField::new("dv", 0);
    doc.add(Box::new(stored_field));
    doc.add(Box::new(dv_field));

    let num_docs = LuceneTestCase::at_least(NUMERIC_BLOCK_SIZE as i32 * 3);
    let mut longs = blocks_of_various_bpv();
    for _ in 0..num_docs {
        if random().next_double() > density {
            writer.add_document(&Document::new())?;
            continue;
        }
        let value = longs.get_as_long();
        field_mut::<Field>(&mut doc, "stored").set_string_value(value.to_string())?;
        field_mut::<NumericDocValuesField>(&mut doc, "dv").set_value(value);
        writer.add_document(&doc)?;
    }

    writer.force_merge(1)?;

    writer.close()?;

    // compare
    let d = dir.as_directory();
    suite.assert_dv_iterate(&d)?;
    // Tests all jump-lengths from 1 to maxDoc (quite slow ~= 1 minute for 200K docs)
    assert_dv_advance(&d, 1)?;

    drop(d);
    dir.close()
}

// Tests that advanceExact does not change the outcome
/// Port of `TestLucene90DocValuesFormat.assertDVAdvance` (`private`).
fn assert_dv_advance(dir: &Arc<dyn Directory>, jump_step: i32) -> Result<()> {
    let ir = directory_reader::open(Arc::clone(dir))?;
    TestUtil::check_reader(&(Arc::clone(&ir) as Arc<dyn IndexReader>))?;
    for context in Arc::clone(&ir).leaves()? {
        let r = context.leaf_reader();
        let stored_fields = r.stored_fields()?;

        let mut jump = jump_step;
        while jump < r.max_doc() {
            // Create a new instance each time to ensure jumps from the beginning
            let mut doc_values = DocValues::get_numeric(&*r, "dv")?;
            let mut doc_id = 0;
            while doc_id < r.max_doc() {
                let base = format!(
                    "document #{doc_id}/{}, jumping {jump} from #{}",
                    r.max_doc(),
                    doc_id - jump
                );
                let stored_value = stored_fields.document(doc_id)?.get("stored");
                match stored_value {
                    None => Assert::assert_false_msg(
                        &format!("There should be no DocValue for {base}"),
                        doc_values.advance_exact(doc_id)?,
                    ),
                    Some(stored_value) => {
                        Assert::assert_true_msg(
                            &format!("There should be a DocValue for {base}"),
                            doc_values.advance_exact(doc_id)?,
                        );
                        let parsed: i64 = stored_value.parse().unwrap_or_else(|_| {
                            Throwable::new(
                                "java.lang.NumberFormatException",
                                format!("For input string: \"{stored_value}\""),
                            )
                            .throw()
                        });
                        Assert::assert_equals_msg(
                            Some(&format!("The doc value should be correct for {base}")),
                            parsed,
                            doc_values.long_value()?,
                        );
                    }
                }
                doc_id += jump;
            }
            jump += jump_step;
        }
    }
    ir.close()
}

/// Port of `TestLucene90DocValuesFormat.testReseekAfterSkipDecompression`.
fn test_reseek_after_skip_decompression(suite: &CompressingDocValuesSuite) -> Result<()> {
    let cardinality = (TERMS_DICT_BLOCK_LZ4_SIZE << 1) + 11;
    let mut value_set: HashSet<String> = HashSet::with_capacity(cardinality);
    // Java iterates the `HashSet` into the list and sorts it, so the set's
    // iteration order does not matter.
    for _ in 0..cardinality {
        value_set.insert(TestUtil::random_simple_string_max(&*random(), 64));
    }
    let mut values: Vec<String> = value_set.into_iter().collect();
    values.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    // Create one non-existent value just between block-1 and block-2.
    let nonexistent_value = format!(
        "{}{}",
        values[TERMS_DICT_BLOCK_LZ4_SIZE - 1],
        TestUtil::random_simple_string_between(&*random(), 64, 128)
    );
    let doc_values = values.len();

    let directory = LuceneTestCase::new_directory();
    {
        let analyzer = Arc::new(StandardAnalyzer::new());
        let config = IndexWriterConfig::with_analyzer(Some(analyzer));
        config.set_codec(suite.get_codec())?;
        config.set_use_compound_file(false)?;
        let writer = IndexWriter::new(directory.as_directory(), config)?;
        for i in 0..280 {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "id",
                format!("Doc{i}"),
                Store::NO,
            )?));
            doc.add(Box::new(SortedDocValuesField::new(
                "sdv",
                bytes(&values[i % doc_values]),
            )));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        writer.force_merge(1)?;
        let d_reader = directory_reader::open_with_writer(&*writer)?;
        writer.close()?;

        let reader =
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&d_reader) as Arc<dyn IndexReader>)?;
        // Check values count.
        let ssdv_multi = reader
            .get_sorted_doc_values("sdv")?
            .unwrap_or_else(|| npe("ssdvMulti"));
        Assert::assert_equals(doc_values as i32, ssdv_multi.get_value_count()?);

        // Seek to first block.
        let ord1 = ssdv_multi.lookup_term(&bytes(&values[0]))?;
        Assert::assert_true(ord1 >= 0);
        let ord2 = ssdv_multi.lookup_term(&bytes(&values[1]))?;
        Assert::assert_true(ord2 >= ord1);
        // Ensure re-seek logic is correct after skip-decompression.
        let nonexistent_ord2 = ssdv_multi.lookup_term(&bytes(&nonexistent_value))?;
        Assert::assert_true(nonexistent_ord2 < 0);
        drop((ssdv_multi, reader));
        d_reader.close()?;
    }
    directory.close()
}

/// Port of `TestLucene90DocValuesFormat.testLargeTermsCompression`.
fn test_large_terms_compression(suite: &CompressingDocValuesSuite) -> Result<()> {
    let cardinality = 64;
    // Java iterates a `HashSet` into the list; Rucene keeps Java's iteration
    // order through `JavaHashSet`.
    let mut values_set = rucene::util::java_collections::JavaHashSet::new();
    for _ in 0..cardinality {
        let length = TestUtil::next_int(&*random(), 512, 1024);
        values_set.insert(TestUtil::random_simple_string_max(&*random(), length));
    }
    let values_count = values_set.len();
    let values: Vec<String> = values_set.iter().cloned().collect();

    let directory = LuceneTestCase::new_directory();
    {
        let analyzer = Arc::new(StandardAnalyzer::new());
        let config = IndexWriterConfig::with_analyzer(Some(analyzer));
        config.set_codec(suite.get_codec())?;
        config.set_use_compound_file(false)?;
        let writer = IndexWriter::new(directory.as_directory(), config)?;
        for i in 0..256 {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "id",
                format!("Doc{i}"),
                Store::NO,
            )?));
            doc.add(Box::new(SortedDocValuesField::new(
                "sdv",
                bytes(&values[i % values_count]),
            )));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        writer.force_merge(1)?;
        let ireader = directory_reader::open_with_writer(&*writer)?;
        writer.close()?;

        let reader =
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&ireader) as Arc<dyn IndexReader>)?;
        // Check values count.
        let ssdv_multi = reader
            .get_sorted_doc_values("sdv")?
            .unwrap_or_else(|| npe("ssdvMulti"));
        Assert::assert_equals(values_count as i32, ssdv_multi.get_value_count()?);
        drop((ssdv_multi, reader));
        ireader.close()?;
    }
    directory.close()
}

/// Port of `TestLucene90DocValuesFormat.testSortedTermsDictLookupOrd`.
fn test_sorted_terms_dict_lookup_ord(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    let field = SortedDocValuesField::new("foo", BytesRef::new(Vec::new()));
    doc.add(Box::new(field));
    let num_docs = LuceneTestCase::at_least(TERMS_DICT_BLOCK_LZ4_SIZE as i32 + 1);
    for i in 0..num_docs {
        field_mut::<SortedDocValuesField>(&mut doc, "foo")
            .set_bytes_value(bytes(&i.to_string()))?;
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    let leaf_reader =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    do_test_terms_dict_lookup_ord(
        &mut *leaf_reader
            .get_sorted_doc_values("foo")?
            .unwrap_or_else(|| npe("getSortedDocValues(\"foo\")"))
            .terms_enum()?,
    )?;
    drop(leaf_reader);
    reader.close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.testSortedSetTermsDictLookupOrd`.
fn test_sorted_set_terms_dict_lookup_ord(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    let field = SortedSetDocValuesField::new("foo", BytesRef::new(Vec::new()));
    doc.add(Box::new(field));
    let num_docs = LuceneTestCase::at_least(2 * TERMS_DICT_BLOCK_LZ4_SIZE as i32 + 1);
    for i in 0..num_docs {
        field_mut::<SortedSetDocValuesField>(&mut doc, "foo")
            .set_bytes_value(bytes(&i.to_string()))?;
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*writer)?;
    let leaf_reader =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    do_test_terms_dict_lookup_ord(
        &mut *leaf_reader
            .get_sorted_set_doc_values("foo")?
            .unwrap_or_else(|| npe("getSortedSetDocValues(\"foo\")"))
            .terms_enum()?,
    )?;
    drop(leaf_reader);
    reader.close()?;
    writer.close()?;
    dir.close()
}

/// Port of `TestLucene90DocValuesFormat.doTestTermsDictLookupOrd` (`private`).
fn do_test_terms_dict_lookup_ord(te: &mut dyn TermsEnum) -> Result<()> {
    let mut terms: Vec<BytesRef> = Vec::new();
    while let Some(term) = te.next()? {
        terms.push(BytesRef::deep_copy_of(&term));
    }

    // iterate in order
    for (i, term) in terms.iter().enumerate() {
        te.seek_ord(i as i64)?;
        Assert::assert_equals_nullable(Some(term.clone()), te.term()?);
    }

    // iterate in reverse order
    for i in (0..terms.len()).rev() {
        te.seek_ord(i as i64)?;
        Assert::assert_equals_nullable(Some(terms[i].clone()), te.term()?);
    }

    // iterate in forward order with random gaps
    let mut i = random().next_int_bound(5) as usize;
    while i < terms.len() {
        te.seek_ord(i as i64)?;
        Assert::assert_equals_nullable(Some(terms[i].clone()), te.term()?);
        i += random().next_int_bound(5) as usize;
    }
    Ok(())
}

// Exercise the logic that leverages the first term of a block as a dictionary for suffixes of
// other terms
/// Port of `TestLucene90DocValuesFormat.testTermsEnumDictionary`.
fn test_terms_enum_dictionary(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config();
    let iwriter = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;
    let mut doc = Document::new();
    let field = SortedDocValuesField::new("field", bytes("abc0defghijkl"));
    doc.add(Box::new(field));
    iwriter.add_document(&doc)?;
    field_mut::<SortedDocValuesField>(&mut doc, "field").set_bytes_value(bytes("abc1defghijkl"))?;
    iwriter.add_document(&doc)?;
    field_mut::<SortedDocValuesField>(&mut doc, "field").set_bytes_value(bytes("abc2defghijkl"))?;
    iwriter.add_document(&doc)?;
    iwriter.force_merge(1)?;
    iwriter.close()?;

    let reader = directory_reader::open(directory.as_directory())?;
    let leaf_reader =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let values = leaf_reader
        .get_sorted_doc_values("field")?
        .unwrap_or_else(|| npe("values"));
    let mut terms_enum = values.terms_enum()?;
    Assert::assert_true(Some(bytes("abc0defghijkl")) == terms_enum.next()?);
    Assert::assert_true(Some(bytes("abc1defghijkl")) == terms_enum.next()?);
    Assert::assert_true(Some(bytes("abc2defghijkl")) == terms_enum.next()?);
    Assert::assert_true(terms_enum.next()?.is_none());

    drop((terms_enum, leaf_reader));
    reader.close()?;
    directory.close()
}

// Testing termsEnum seekCeil edge case, where inconsistent internal state led to
// IndexOutOfBoundsException
// see https://github.com/apache/lucene/pull/12555 for details
/// Port of `TestLucene90DocValuesFormat.testTermsEnumConsistency`.
fn test_terms_enum_consistency(_suite: &CompressingDocValuesSuite) -> Result<()> {
    let num_terms = TERMS_DICT_BLOCK_LZ4_SIZE as i32 + 10; // need more than one block of unique terms.
    let directory = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config();
    let iwriter = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), conf)?;

    // for simplicity, we will generate sorted list of terms which are a) unique b) all greater than
    // the term that we want to use for the test
    let term_a = b'A' as i32;
    let string_supplier = |n: i32| -> String {
        debug_assert!(n < 25 * 25);
        let chars = [
            char::from_u32((term_a + 1 + n / 25) as u32).expect("an ASCII letter"),
            char::from_u32((term_a + 1 + n % 25) as u32).expect("an ASCII letter"),
        ];
        chars.iter().collect()
    };
    let mut doc = Document::new();
    let field = SortedDocValuesField::new("field", bytes(&string_supplier(0)));
    doc.add(Box::new(field));
    iwriter.add_document(&doc)?;
    for i in 1..num_terms {
        field_mut::<SortedDocValuesField>(&mut doc, "field")
            .set_bytes_value(bytes(&string_supplier(i)))?;
        iwriter.add_document(&doc)?;
    }
    // merging to one segment to make sure we have more than one block (TERMS_DICT_BLOCK_LZ4_SIZE)
    // in a segment, to trigger next block decompression.
    iwriter.force_merge(1)?;
    iwriter.close()?;

    let reader = directory_reader::open(directory.as_directory())?;
    let leaf_reader =
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let values = leaf_reader
        .get_sorted_doc_values("field")?
        .unwrap_or_else(|| npe("values"));
    let mut terms_enum = values.terms_enum()?;

    // Position terms enum at 0
    terms_enum.seek_ord(0)?;
    Assert::assert_equals(0, terms_enum.ord()?);
    // seekCeil to a term which doesn't exist in the index
    Assert::assert_true(SeekStatus::NOT_FOUND == terms_enum.seek_ceil(&bytes("A"))?);
    // ... and before any other term in the index
    Assert::assert_equals(0, terms_enum.ord()?);

    Assert::assert_equals_nullable(Some(bytes(&string_supplier(0))), terms_enum.term()?);
    // read more than one block of terms to trigger next block decompression
    for i in 1..num_terms {
        Assert::assert_true(Some(bytes(&string_supplier(i))) == terms_enum.next()?);
    }
    Assert::assert_true(terms_enum.next()?.is_none());
    drop((terms_enum, leaf_reader));
    reader.close()?;
    directory.close()
}

/// Port of `TestLucene90DocValuesFormat.testSkipIndexStoredSeparately`.
fn test_skip_index_stored_separately(suite: &CompressingDocValuesSuite) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let conf = LuceneTestCase::new_index_writer_config();
        conf.set_codec(suite.get_codec())?;
        conf.set_use_compound_file(false)?;
        let writer = IndexWriter::new(dir.as_directory(), conf)?;
        for i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::indexed_field("dv", i)));
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        writer.close()?;
    }

    // Wrap the directory so that any attempt to slice() the .dvd file throws.
    // The producer constructor opens .dvd for header/checksum validation (sequential reads),
    // but getSkipper() should only slice the .dvs file, never the .dvd file.
    let no_data_slice_dir: Arc<dyn Directory> = Arc::new(NoDataSliceDirectory {
        r#in: dir.as_directory(),
    });

    {
        let reader = directory_reader::open(no_data_slice_dir)?;
        let leaf =
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let skipper = leaf.get_doc_values_skipper("dv")?;
        Assert::assert_true(skipper.is_some()); // assertNotNull(skipper)
        let mut skipper = skipper.expect("assertNotNull(skipper)");
        Assert::assert_equals(100, skipper.global_doc_count());
        skipper.advance(0)?;
        Assert::assert_equals(0, skipper.min_doc_id(0));
        Assert::assert_true(skipper.max_doc_id(0) >= 0);
        Assert::assert_true(skipper.global_min_value() <= skipper.global_max_value());
        drop((skipper, leaf));
        reader.close()?;
    }
    dir.close()
}

/// The anonymous `FilterDirectory` of `testSkipIndexStoredSeparately`: its
/// inputs over `.dvd` files refuse to slice.
struct NoDataSliceDirectory {
    r#in: Arc<dyn Directory>,
}

impl Directory for NoDataSliceDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }
    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }
    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_output(name, context)
    }
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.r#in.create_temp_output(prefix, suffix, context)
    }
    fn sync(&self, names: &[String]) -> Result<()> {
        self.r#in.sync(names)
    }
    fn sync_metadata(&self) -> Result<()> {
        self.r#in.sync_metadata()
    }
    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.r#in.rename(source, dest)
    }
    /// `openInput(name, context)`: the wrapped input, behind a
    /// `FilterIndexInput` that refuses to slice when it is a `.dvd` file.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let input = self.r#in.open_input(name, context)?;
        if name.ends_with(&format!(".{DATA_EXTENSION}")) {
            return Ok(Box::new(NoSliceIndexInput {
                resource_description: format!("no-slice-{name}"),
                r#in: input,
            }));
        }
        Ok(input)
    }
    /// `Directory.openChecksumInput`, inherited by `FilterDirectory`, over
    /// this directory's `openInput`.
    fn open_checksum_input(
        &self,
        name: &str,
    ) -> Result<Box<rucene::store::BufferedChecksumIndexInput>> {
        Ok(Box::new(rucene::store::BufferedChecksumIndexInput::new(
            self.open_input(name, &*rucene::store::READONCE_IO_CONTEXT)?,
        )))
    }
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }
    /// `FilterDirectory.close()` closes the wrapped directory; the test never
    /// closes this wrapper, only the wrapped directory, through its
    /// try-with-resources.
    fn close(&self) -> Result<()> {
        Ok(())
    }
    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }
    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(self.r#in.as_ref())
    }
}

/// The anonymous `FilterIndexInput` of `testSkipIndexStoredSeparately`:
/// `slice` throws `AssertionError`.
struct NoSliceIndexInput {
    resource_description: String,
    r#in: Box<dyn IndexInput>,
}

impl std::fmt::Debug for NoSliceIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoSliceIndexInput")
            .field("resource_description", &self.resource_description)
            .finish()
    }
}

impl DataInput for NoSliceIndexInput {
    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_byte(&mut self) -> Result<u8> {
        self.r#in.read_byte()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        self.r#in.read_bytes(b, offset, len)
    }

    /// `IndexInput.skipBytes(numBytes)`, which `FilterIndexInput` inherits.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(rucene::error::LuceneError::illegal_argument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }
}

impl IndexInput for NoSliceIndexInput {
    fn close(&mut self) -> Result<()> {
        self.r#in.close()
    }

    fn file_pointer(&self) -> i64 {
        self.r#in.file_pointer()
    }

    fn length(&self) -> i64 {
        self.r#in.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.r#in.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        _offset: i64,
        _length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Throwable::new(
            "java.lang.AssertionError",
            format!("should not slice .dvd file for skip index access: {slice_description}"),
        )
        .throw()
    }

    /// Java's anonymous class inherits `IndexInput.clone()` (`Object.clone`),
    /// a copy of the same class over the same `in`; the copy here is over a
    /// clone of `in`, refusing to slice as well.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(NoSliceIndexInput {
            resource_description: self.resource_description.clone(),
            r#in: self.r#in.clone_input()?,
        }))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn as_filter_index_input(&self) -> Option<(std::any::TypeId, &dyn IndexInput)> {
        Some((
            std::any::TypeId::of::<NoSliceIndexInput>(),
            self.r#in.as_ref(),
        ))
    }
}

// -----------------------------------------------------------------------------
// TestLucene90DocValuesFormatVariableSkipInterval
// -----------------------------------------------------------------------------

/// The fully qualified name of `TestLucene90DocValuesFormatVariableSkipInterval`,
/// which its binding declares.
pub const VARIABLE_SKIP_INTERVAL_CLASS: &str =
    "org.apache.lucene.codecs.lucene90.TestLucene90DocValuesFormatVariableSkipInterval";

/// A test method `TestLucene90DocValuesFormatVariableSkipInterval` declares
/// itself, run on the class's instance.
fn skip_interval_test(name: &str, body: fn(&DocValuesSuite) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut FormatTestInstance| {
            let suite = inst.subclass::<DocValuesSuite>();
            body(&suite).unwrap_or_else(|e| rethrow(e));
        })
}

/// The own test methods of `TestLucene90DocValuesFormatVariableSkipInterval`,
/// in declaration order.
pub fn variable_skip_interval_methods() -> Vec<Method> {
    vec![
        skip_interval_test("testSkipIndexIntervalSize", test_skip_index_interval_size),
        skip_interval_test("testSkipperAllEqualValue", test_skipper_all_equal_value),
        skip_interval_test("testSkipperFewValuesSorted", test_skipper_few_values_sorted),
        skip_interval_test(
            "testSkipperAllEqualValueWithGaps",
            test_skipper_all_equal_value_with_gaps,
        ),
        skip_interval_test(
            "testSkipperAllEqualValueWithMultiValues",
            test_skipper_all_equal_value_with_multi_values,
        ),
    ]
}

/// Port of `TestLucene90DocValuesFormatVariableSkipInterval.testSkipIndexIntervalSize`.
fn test_skip_index_interval_size(_suite: &DocValuesSuite) -> Result<()> {
    let ex = LuceneTestCase::expect_throws("java.lang.IllegalArgumentException", || {
        Lucene90DocValuesFormat::with_interval(random().next_int_range(i32::MIN, 2));
        Ok(())
    });
    Assert::assert_true(
        ex.message()
            .unwrap_or_else(|| npe("getMessage()"))
            .contains("skipIndexIntervalSize must be > 1"),
    );
    Ok(())
}

/// `new IndexWriterConfig().setCodec(getCodec())`.
fn skip_interval_config(suite: &DocValuesSuite) -> Result<IndexWriterConfig> {
    let config = IndexWriterConfig::new();
    config.set_codec(suite.get_codec())?;
    Ok(config)
}

/// `reader.leaves().get(0).reader().getDocValuesSkipper("dv")`, after
/// `assertEquals(1, reader.leaves().size())` and before `assertNotNull`.
fn only_skipper(reader: &Arc<dyn DirectoryReader>) -> Result<Box<dyn DocValuesSkipper>> {
    let leaves = Arc::clone(reader).leaves()?;
    Assert::assert_equals(1, leaves.len());
    let skipper = leaves[0].leaf_reader().get_doc_values_skipper("dv")?;
    Assert::assert_true(skipper.is_some()); // assertNotNull(skipper)
    Ok(skipper.expect("assertNotNull(skipper)"))
}

/// Port of `TestLucene90DocValuesFormatVariableSkipInterval.testSkipperAllEqualValue`.
fn test_skipper_all_equal_value(suite: &DocValuesSuite) -> Result<()> {
    let config = skip_interval_config(suite)?;
    let directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
        let num_docs = LuceneTestCase::at_least(100);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::indexed_field("dv", 0)));
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        {
            let reader = writer.get_reader()?;
            let mut skipper = only_skipper(&reader)?;
            skipper.advance(0)?;
            Assert::assert_equals(0i64, skipper.min_value(0));
            Assert::assert_equals(0i64, skipper.max_value(0));
            Assert::assert_equals(num_docs, skipper.doc_count(0));
            let next = skipper.max_doc_id(0) + 1;
            skipper.advance(next)?;
            Assert::assert_equals(NO_MORE_DOCS, skipper.min_doc_id(0));
            drop(skipper);
            reader.close()?;
        }
        writer.close()?;
    }
    directory.close()
}

// break on different value
/// Port of `TestLucene90DocValuesFormatVariableSkipInterval.testSkipperFewValuesSorted`.
fn test_skipper_few_values_sorted(suite: &DocValuesSuite) -> Result<()> {
    let config = skip_interval_config(suite)?;
    let reverse = random().next_boolean();
    config.set_index_sort(Sort::new_fields(vec![SortField::new_reverse(
        Some("dv".to_string()),
        SortFieldType::Long,
        reverse,
    )?])?)?;
    let directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
        let intervals = random().next_int_range(2, 10);
        let mut num_docs = vec![0i32; intervals as usize];
        for i in 0..intervals {
            num_docs[i as usize] = random().next_int_bound(10) + 16;
            for _ in 0..num_docs[i as usize] {
                let mut doc = Document::new();
                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "dv",
                    i64::from(i),
                )));
                writer.add_document(&doc)?;
            }
        }
        writer.force_merge(1)?;
        {
            let reader = writer.get_reader()?;
            let mut skipper = only_skipper(&reader)?;
            Assert::assert_equals(num_docs.iter().sum::<i32>(), skipper.global_doc_count());
            skipper.advance(0)?;
            let order: Vec<i32> = if reverse {
                (0..intervals).rev().collect()
            } else {
                (0..intervals).collect()
            };
            for i in order {
                Assert::assert_equals(i64::from(i), skipper.min_value(0));
                Assert::assert_equals(i64::from(i), skipper.max_value(0));
                Assert::assert_equals(num_docs[i as usize], skipper.doc_count(0));
                let next = skipper.max_doc_id(0) + 1;
                skipper.advance(next)?;
            }
            Assert::assert_equals(NO_MORE_DOCS, skipper.min_doc_id(0));
            drop(skipper);
            reader.close()?;
        }
        writer.close()?;
    }
    directory.close()
}

// break on empty doc values
/// Port of `TestLucene90DocValuesFormatVariableSkipInterval.testSkipperAllEqualValueWithGaps`.
fn test_skipper_all_equal_value_with_gaps(suite: &DocValuesSuite) -> Result<()> {
    let config = skip_interval_config(suite)?;
    config.set_index_sort(Sort::new_fields(vec![SortField::new_reverse(
        Some("sort".to_string()),
        SortFieldType::Long,
        false,
    )?])?)?;
    let directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
        let gaps = random().next_int_range(2, 10);
        let mut num_docs = vec![0i32; gaps as usize];
        let mut totaldocs = 0i64;
        #[allow(clippy::needless_range_loop)] // Java's indexed loop, kept as the test writes it
        for i in 0..gaps as usize {
            num_docs[i] = random().next_int_bound(10) + 16;
            for _ in 0..num_docs[i] {
                let mut doc = Document::new();
                doc.add(Box::new(NumericDocValuesField::new("sort", totaldocs)));
                totaldocs += 1;
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "dv", 0,
                )));
                writer.add_document(&doc)?;
            }
            // add doc with empty "dv"
            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new("sort", totaldocs)));
            totaldocs += 1;
            writer.add_document(&doc)?;
        }
        writer.force_merge(1)?;
        {
            let reader = writer.get_reader()?;
            let mut skipper = only_skipper(&reader)?;
            Assert::assert_equals(num_docs.iter().sum::<i32>(), skipper.global_doc_count());
            skipper.advance(0)?;
            for count in &num_docs {
                Assert::assert_equals(0i64, skipper.min_value(0));
                Assert::assert_equals(0i64, skipper.max_value(0));
                Assert::assert_equals(*count, skipper.doc_count(0));
                let next = skipper.max_doc_id(0) + 1;
                skipper.advance(next)?;
            }
            Assert::assert_equals(NO_MORE_DOCS, skipper.min_doc_id(0));
            drop(skipper);
            reader.close()?;
        }
        writer.close()?;
    }
    directory.close()
}

// break on multi-values
/// Port of `TestLucene90DocValuesFormatVariableSkipInterval.testSkipperAllEqualValueWithMultiValues`.
fn test_skipper_all_equal_value_with_multi_values(suite: &DocValuesSuite) -> Result<()> {
    let config = skip_interval_config(suite)?;
    config.set_index_sort(Sort::new_fields(vec![SortField::new_reverse(
        Some("sort".to_string()),
        SortFieldType::Long,
        false,
    )?])?)?;
    let directory = LuceneTestCase::new_directory();
    {
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
        let gaps = random().next_int_range(2, 10);
        let mut num_docs = vec![0i32; gaps as usize];
        let mut totaldocs = 0i64;
        for i in 0..gaps as usize {
            let docs = random().next_int_bound(10) + 16;
            num_docs[i] += docs;
            for _ in 0..docs {
                let mut doc = Document::new();
                doc.add(Box::new(NumericDocValuesField::new("sort", totaldocs)));
                totaldocs += 1;
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "dv", 0,
                )));
                writer.add_document(&doc)?;
            }
            if i != gaps as usize - 1 {
                // add doc with mutivalues
                let mut doc = Document::new();
                doc.add(Box::new(NumericDocValuesField::new("sort", totaldocs)));
                totaldocs += 1;
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "dv", 0,
                )));
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "dv", 0,
                )));
                writer.add_document(&doc)?;
                num_docs[i + 1] = 1;
            }
        }
        writer.force_merge(1)?;
        {
            let reader = writer.get_reader()?;
            let mut skipper = only_skipper(&reader)?;
            Assert::assert_equals(num_docs.iter().sum::<i32>(), skipper.global_doc_count());
            skipper.advance(0)?;
            for count in &num_docs {
                Assert::assert_equals(0i64, skipper.min_value(0));
                Assert::assert_equals(0i64, skipper.max_value(0));
                Assert::assert_equals(*count, skipper.doc_count(0));
                let next = skipper.max_doc_id(0) + 1;
                skipper.advance(next)?;
            }
            Assert::assert_equals(NO_MORE_DOCS, skipper.min_doc_id(0));
            drop(skipper);
            reader.close()?;
        }
        writer.close()?;
    }
    directory.close()
}
