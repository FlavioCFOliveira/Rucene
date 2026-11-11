//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSortingCodecReader.java`.
//!
//! `SortingCodecReader.wrap(CodecReader, Sort)` is
//! `SortingCodecReader::wrap_sort` in Rucene (Rust has no overloading).

use std::sync::Arc;

use randomizedtesting::generators::{RandomPicks, RandomStrings};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::hnsw::graph_provider::HnswGraphProvider;
use rucene::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader;
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, KnnFloatVectorField, LongPoint,
    NumericDocValuesField, SortedDocValuesField, SortedNumericDocValuesField,
    SortedSetDocValuesField, Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::sorting_codec_reader::SortingCodecReader;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::term_query::TermQuery;
use rucene::search::{
    IndexSearcher, SortedNumericSortField, SortedSetSelectorType, SortedSetSortField, NO_MORE_DOCS,
};
use rucene::util::hnsw::HnswGraph;
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_equals, assert_not_null, class, random, shuffle,
    test,
};

/// `TestSortingCodecReader`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSortingCodecReader",
        vec![
            test("testSortOnAddIndicesOrd", test_sort_on_add_indices_ord),
            test("testSortOnAddIndicesInt", test_sort_on_add_indices_int),
            test(
                "testSortOnAddIndicesRandom",
                test_sort_on_add_indices_random,
            ),
        ],
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `new BytesRef(s)`.
fn bytes(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// `SortingCodecReader.wrap(SlowCodecReaderWrapper.wrap(leaf), indexSort)`.
fn sorting_wrap(leaf: Arc<dyn LeafReader>, index_sort: &Sort) -> Result<Arc<dyn CodecReader>> {
    SortingCodecReader::wrap_sort(SlowCodecReaderWrapper::wrap(leaf)?, index_sort.clone())
}

/// Port of `TestSortingCodecReader.testSortOnAddIndicesOrd`.
fn test_sort_on_add_indices_ord() -> Result<()> {
    let tmp_dir = LuceneTestCase::new_directory();
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(tmp_dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(SortedSetDocValuesField::new("foo", bytes("b"))));
    w.add_document(&doc)?;

    doc.add(Box::new(SortedSetDocValuesField::new("foo", bytes("a"))));
    doc.add(Box::new(SortedSetDocValuesField::new("foo", bytes("b"))));
    doc.add(Box::new(SortedSetDocValuesField::new("foo", bytes("b"))));
    w.add_document(&doc)?;

    w.commit()?;

    let index_sort = Sort::new_fields(vec![SortedSetSortField::with_selector(
        "foo",
        false,
        SortedSetSelectorType::MIN,
    )?
    .into_sort_field()])?;
    {
        let reader = directory_reader::open(tmp_dir.as_directory())?;
        let result = (|| -> Result<()> {
            for ctx in Arc::clone(&reader).leaves()? {
                let wrap = sorting_wrap(ctx.leaf_reader(), &index_sort)?;
                Assert::assert_true_msg(
                    &wrap.to_string(),
                    wrap.to_string().starts_with("SortingCodecReader("),
                );
                let sorting_codec_reader = wrap
                    .leaf_as_any()
                    .and_then(|a| a.downcast_ref::<SortingCodecReader>())
                    .unwrap_or_else(|| {
                        Throwable::new("java.lang.ClassCastException", "not a SortingCodecReader")
                            .throw()
                    });
                let field_infos = ctx.leaf_reader().get_field_infos();
                let field_info = field_infos
                    .field_info("foo")
                    .unwrap_or_else(|| crate::support::npe("fieldInfo"));
                let mut sorted_set_doc_values = sorting_codec_reader
                    .get_doc_values_reader()?
                    .unwrap_or_else(|| crate::support::npe("getDocValuesReader"))
                    .get_sorted_set(field_info)?
                    .unwrap_or_else(|| crate::support::npe("getSortedSet"));
                sorted_set_doc_values.next_doc()?;
                assert_equals(&sorted_set_doc_values.doc_value_count()?, &2);
                sorted_set_doc_values.next_doc()?;
                assert_equals(&sorted_set_doc_values.doc_value_count()?, &1);
                assert_equals(&sorted_set_doc_values.next_doc()?, &NO_MORE_DOCS);
            }
            Ok(())
        })();
        let closed = reader.close();
        result.and(closed)?;
    }
    IOUtils::close([
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
        IOUtils::closeable(|| tmp_dir.close()),
    ])
}

/// Port of `TestSortingCodecReader.testSortOnAddIndicesInt`.
fn test_sort_on_add_indices_int() -> Result<()> {
    let tmp_dir = LuceneTestCase::new_directory();
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    let w = IndexWriter::new(tmp_dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("foo", 18)));
    w.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("foo", -1)));
    w.add_document(&doc)?;
    w.commit()?;

    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("foo", 7)));
    w.add_document(&doc)?;
    w.commit()?;
    w.close()?;
    let index_sort = Sort::new_fields(vec![SortField::new(
        Some("foo".to_string()),
        SortFieldType::Int,
    )?])?;

    let iwc = mock_iwc();
    iwc.set_index_sort(index_sort.clone())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    {
        let reader = directory_reader::open(tmp_dir.as_directory())?;
        let result = (|| -> Result<()> {
            let mut readers: Vec<Arc<dyn CodecReader>> = Vec::new();
            for ctx in Arc::clone(&reader).leaves()? {
                let wrap = sorting_wrap(ctx.leaf_reader(), &index_sort)?;
                Assert::assert_true_msg(
                    &wrap.to_string(),
                    wrap.to_string().starts_with("SortingCodecReader("),
                );
                readers.push(wrap);
            }
            w.add_indexes_codec_readers(&readers)?;
            Ok(())
        })();
        let closed = reader.close();
        result.and(closed)?;
    }
    let r = directory_reader::open_with_writer(&*w)?;
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    assert_equals(&3, &leaf.max_doc());
    let mut values = leaf
        .get_numeric_doc_values("foo")?
        .unwrap_or_else(|| crate::support::npe("getNumericDocValues"));
    assert_equals(&0, &values.next_doc()?);
    assert_equals(&-1i64, &values.long_value()?);
    assert_equals(&1, &values.next_doc()?);
    assert_equals(&7i64, &values.long_value()?);
    assert_equals(&2, &values.next_doc()?);
    assert_equals(&18i64, &values.long_value()?);
    assert_not_null(&leaf.get_meta_data().sort());
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
        IOUtils::closeable(|| tmp_dir.close()),
    ])
}

/// `vectorsReader instanceof HnswGraphProvider hnswGraphProvider ?
/// hnswGraphProvider.getGraph("vector") : null`. Rucene's only
/// `HnswGraphProvider` reader is `Lucene99HnswVectorsReader`.
fn graph_of(leaf: &Arc<dyn LeafReader>) -> Result<Option<Box<dyn HnswGraph>>> {
    let codec_reader = Arc::clone(leaf).as_codec_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
    });
    let vectors_reader = codec_reader.get_vector_reader()?;
    match vectors_reader
        .as_ref()
        .and_then(|r| r.as_any())
        .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
    {
        Some(provider) => Ok(Some(HnswGraphProvider::get_graph(provider, "vector")?)),
        None => Ok(None),
    }
}

/// Port of `TestSortingCodecReader.testSortOnAddIndicesRandom`.
fn test_sort_on_add_indices_random() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let result = sort_on_add_indices_random_in(&dir);
    let closed = dir.close();
    result.and(closed)
}

/// The body of `try (Directory dir = newDirectory()) { ... }` in
/// `testSortOnAddIndicesRandom`.
fn sort_on_add_indices_random_in(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<()> {
    let num_docs = LuceneTestCase::at_least(200);
    let actual_num_docs;
    let mut doc_ids: Vec<i32> = Vec::with_capacity(num_docs as usize);
    for i in 0..num_docs {
        doc_ids.push(i);
    }
    shuffle(&mut doc_ids, &*random());
    // If true, index a vector and points for every doc
    let dense = random().next_boolean();
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let result = (|| -> Result<i32> {
            for i in 0..num_docs as usize {
                let doc_id = doc_ids[i];
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new(
                    "string_id",
                    doc_id.to_string(),
                    Store::YES,
                )?));
                if dense || doc_id % 3 == 0 {
                    doc.add(Box::new(LongPoint::new("point_id", &[doc_id as i64])?));
                }
                let s = RandomStrings::random_realistic_unicode_of_length(&*random(), 25);
                doc.add(Box::new(TextField::new(
                    "text_field",
                    s.clone(),
                    Store::YES,
                )?));
                doc.add(Box::new(BinaryDocValuesField::new("text_field", bytes(&s))));
                doc.add(Box::new(TextField::new(
                    "another_text_field",
                    s.clone(),
                    Store::YES,
                )?));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "another_text_field",
                    bytes(&s),
                )));
                doc.add(Box::new(SortedNumericDocValuesField::new(
                    "sorted_numeric_dv",
                    doc_id as i64,
                )));
                doc.add(Box::new(SortedDocValuesField::new(
                    "binary_sorted_dv",
                    bytes(&doc_id.to_string()),
                )));
                doc.add(Box::new(BinaryDocValuesField::new(
                    "binary_dv",
                    bytes(&doc_id.to_string()),
                )));
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "sorted_set_dv",
                    bytes(&doc_id.to_string()),
                )));
                if dense || doc_id % 2 == 0 {
                    doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                        "vector",
                        &[doc_id as f32],
                    )?));
                }
                doc.add(Box::new(NumericDocValuesField::new(
                    "foo",
                    random().next_int_bound(20) as i64,
                )));

                let ft = FieldType::new_from(StringField::type_not_stored());
                ft.set_store_term_vectors(true)?;
                doc.add(Box::new(Field::new(
                    "term_vectors",
                    format!("test{doc_id}"),
                    ft,
                )?));
                if !LuceneTestCase::rarely() {
                    doc.add(Box::new(NumericDocValuesField::new("id", doc_id as i64)));
                    doc.add(Box::new(SortedSetDocValuesField::new(
                        "sorted_set_sort_field",
                        bytes(&format!("{doc_id:06}")),
                    )));
                    doc.add(Box::new(SortedDocValuesField::new(
                        "sorted_binary_sort_field",
                        bytes(&format!("{doc_id:06}")),
                    )));
                    doc.add(Box::new(SortedNumericDocValuesField::new(
                        "sorted_numeric_sort_field",
                        doc_id as i64,
                    )));
                } else {
                    doc.add(Box::new(NumericDocValuesField::new(
                        "alt_id",
                        doc_id as i64,
                    )));
                }
                iw.add_document(&doc)?;
                if i > 0 && random().next_int_bound(5) == 0 {
                    let id = *RandomPicks::random_from(&*random(), &doc_ids[0..i]);
                    iw.delete_documents(Term::from_text("string_id", &id.to_string()))?;
                }
            }
            iw.commit()?;
            Ok(iw.get_doc_stats()?.num_docs)
        })();
        let closed = iw.close();
        actual_num_docs = result.and_then(|n| closed.map(|()| n))?;
    }
    let alt_id = || SortField::new(Some("alt_id".to_string()), SortFieldType::Int);
    let sorts = vec![
        Sort::new_fields(vec![
            SortField::new(Some("id".to_string()), SortFieldType::Int)?,
            alt_id()?,
        ])?,
        Sort::new_fields(vec![
            SortedSetSortField::new("sorted_set_sort_field", false)?.into_sort_field(),
            alt_id()?,
        ])?,
        Sort::new_fields(vec![
            SortedNumericSortField::new("sorted_numeric_sort_field", SortFieldType::Int)?
                .into_sort_field(),
            alt_id()?,
        ])?,
        Sort::new_fields(vec![
            SortField::new_reverse(
                Some("sorted_binary_sort_field".to_string()),
                SortFieldType::String,
                false,
            )?,
            alt_id()?,
        ])?,
    ];
    let index_sort = RandomPicks::random_from(&*random(), &sorts).clone();
    let sort_dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_index_sort(index_sort.clone())?;
        let writer = IndexWriter::new(sort_dir.as_directory(), iwc)?;
        let result = (|| -> Result<()> {
            {
                let reader = directory_reader::open(dir.as_directory())?;
                let result = (|| -> Result<()> {
                    let mut readers: Vec<Arc<dyn CodecReader>> = Vec::new();
                    for ctx in Arc::clone(&reader).leaves()? {
                        let wrap = sorting_wrap(ctx.leaf_reader(), &index_sort)?;
                        readers.push(Arc::clone(&wrap));
                        let term_vectors_reader = wrap
                            .get_term_vectors_reader()?
                            .unwrap_or_else(|| crate::support::npe("getTermVectorsReader"));
                        let mut clone = term_vectors_reader.clone_reader()?;
                        Assert::assert_false(std::ptr::addr_eq(
                            &*term_vectors_reader as *const _,
                            &*clone as *const _,
                        ));
                        clone.close()?;
                    }
                    writer.add_indexes_codec_readers(&readers)?;
                    Ok(())
                })();
                let closed = reader.close();
                result.and(closed)?;
            }
            LuceneTestCase::assume_true("must have at least one doc", actual_num_docs > 0);
            let r = directory_reader::open_with_writer(&*writer)?;
            let result = check_sorted_index(&r, actual_num_docs, dense);
            let closed = r.close();
            result.and(closed)
        })();
        let closed = writer.close();
        result.and(closed)
    })();
    let closed = sort_dir.close();
    result.and(closed)
}

/// The checks of `testSortOnAddIndicesRandom` over the sorted index `r`.
fn check_sorted_index(
    r: &Arc<dyn rucene::index::DirectoryReader>,
    actual_num_docs: i32,
    dense: bool,
) -> Result<()> {
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(r) as Arc<dyn IndexReader>)?;
    assert_equals(&actual_num_docs, &leaf.max_doc());
    let npe = |what: &str| -> ! { crate::support::npe(what) };
    let mut binary_dv = leaf
        .get_binary_doc_values("binary_dv")?
        .unwrap_or_else(|| npe("binary_dv"));
    let mut sorted_numeric_dv = leaf
        .get_sorted_numeric_doc_values("sorted_numeric_dv")?
        .unwrap_or_else(|| npe("sorted_numeric_dv"));
    let mut sorted_set_dv = leaf
        .get_sorted_set_doc_values("sorted_set_dv")?
        .unwrap_or_else(|| npe("sorted_set_dv"));
    let mut binary_sorted_dv = leaf
        .get_sorted_doc_values("binary_sorted_dv")?
        .unwrap_or_else(|| npe("binary_sorted_dv"));
    let mut vector_values = leaf
        .get_float_vector_values("vector")?
        .unwrap_or_else(|| npe("vector"));
    let mut graph = graph_of(&leaf)?;
    let mut ids = leaf
        .get_numeric_doc_values("id")?
        .unwrap_or_else(|| npe("id"));
    let mut prev_value: i64 = -1;
    let mut using_alt_ids = false;
    let mut values_iterator = vector_values.iterator()?;
    for _i in 0..actual_num_docs {
        let mut id_next = ids.next_doc()?;
        if id_next == NO_MORE_DOCS {
            Assert::assert_false(using_alt_ids);
            using_alt_ids = true;
            ids = leaf
                .get_numeric_doc_values("alt_id")?
                .unwrap_or_else(|| npe("alt_id"));
            id_next = ids.next_doc()?;
            binary_dv = leaf
                .get_binary_doc_values("binary_dv")?
                .unwrap_or_else(|| npe("binary_dv"));
            sorted_numeric_dv = leaf
                .get_sorted_numeric_doc_values("sorted_numeric_dv")?
                .unwrap_or_else(|| npe("sorted_numeric_dv"));
            sorted_set_dv = leaf
                .get_sorted_set_doc_values("sorted_set_dv")?
                .unwrap_or_else(|| npe("sorted_set_dv"));
            binary_sorted_dv = leaf
                .get_sorted_doc_values("binary_sorted_dv")?
                .unwrap_or_else(|| npe("binary_sorted_dv"));
            vector_values = leaf
                .get_float_vector_values("vector")?
                .unwrap_or_else(|| npe("vector"));
            values_iterator = vector_values.iterator()?;
            prev_value = -1;
        }
        Assert::assert_true_msg(
            &format!("{prev_value} < {}", ids.long_value()?),
            prev_value < ids.long_value()?,
        );
        prev_value = ids.long_value()?;
        Assert::assert_true(binary_dv.advance_exact(id_next)?);
        Assert::assert_true(sorted_numeric_dv.advance_exact(id_next)?);
        Assert::assert_true(sorted_set_dv.advance_exact(id_next)?);
        Assert::assert_true(binary_sorted_dv.advance_exact(id_next)?);
        if dense || prev_value % 2 == 0 {
            assert_equals(&id_next, &values_iterator.advance(id_next)?);
            if let Some(graph) = graph.as_mut().filter(|g| g.size() > 0) {
                graph.seek(0, values_iterator.index())?;
                assert_not_equals(&NO_MORE_DOCS, &graph.next_neighbor()?);
            }
        }

        let id_bytes = bytes(&ids.long_value()?.to_string());
        assert_equals(&id_bytes, &binary_dv.binary_value()?);
        assert_equals(
            &id_bytes,
            &binary_sorted_dv.lookup_ord(binary_sorted_dv.ord_value()?)?,
        );
        let ord = sorted_set_dv.next_ord()?;
        assert_equals(&id_bytes, &sorted_set_dv.lookup_ord(ord)?);
        assert_equals(&1, &sorted_set_dv.doc_value_count()?);
        assert_equals(&1, &sorted_numeric_dv.doc_value_count()?);
        assert_equals(&ids.long_value()?, &sorted_numeric_dv.next_value()?);

        if dense || prev_value % 2 == 0 {
            let vector_value = vector_values.vector_value(values_iterator.index())?;
            assert_equals(&1, &vector_value.len());
            assert_equals_delta(
                ids.long_value()? as f32 as f64,
                vector_value[0] as f64,
                0.001,
            );
        }

        let term_vectors = leaf
            .term_vectors()?
            .get(id_next)?
            .unwrap_or_else(|| npe("termVectors"));
        Assert::assert_true(
            term_vectors
                .terms("term_vectors")?
                .unwrap_or_else(|| npe("terms"))
                .iterator()?
                .seek_exact(&bytes(&format!("test{}", ids.long_value()?)))?,
        );
        assert_equals(
            &Some(ids.long_value()?.to_string()),
            &leaf.stored_fields()?.document(id_next)?.get("string_id"),
        );
        let searcher = IndexSearcher::new(Arc::clone(r) as Arc<dyn IndexReader>)?;
        let result = searcher.search(
            LongPoint::new_exact_query("point_id", ids.long_value()?)?,
            10,
        )?;
        if dense || ids.long_value()? % 3 == 0 {
            assert_equals(&1i64, &result.total_hits.value());
            assert_equals(&id_next, &result.score_docs[0].doc);
        } else {
            assert_equals(&0i64, &result.total_hits.value());
        }

        let result = searcher.search(
            Arc::new(TermQuery::new(Term::from_text(
                "string_id",
                &ids.long_value()?.to_string(),
            ))),
            1,
        )?;
        assert_equals(&1i64, &result.total_hits.value());
        assert_equals(&id_next, &result.score_docs[0].doc);
    }
    assert_equals(&NO_MORE_DOCS, &ids.next_doc()?);
    Ok(())
}
