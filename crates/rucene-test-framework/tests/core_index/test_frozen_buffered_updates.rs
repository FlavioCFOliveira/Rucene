//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFrozenBufferedUpdates.java`.
//!
//! * `TermDocsIterator.nextTerm` returns whether the term is present, and the
//!   `DocIdSetIterator` Java returns is read back through `docs()`
//!   (`src/index/documents_writer.rs`).

use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::documents_writer::TermDocsIterator;
use rucene::index::filter_reader::{FilterLeafReader, FilterTerms, FilterTermsEnum};
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{
    ImpactsEnum, IndexReader, LeafReader, PostingsEnum, SeekStatus, TermState, Terms, TermsEnum,
};
use rucene::search::NO_MORE_DOCS;
use rucene::util::attribute::AttributeSource;
use rucene::util::bytes_ref_array::{BytesRefArray, BytesRefIterator, SortableBytesRefArray};
use rucene::util::concurrent::Counter;
use rucene::util::sorter::StringSorterComparator;
use rucene::util::{BytesRef, FixedBitSet, IOBooleanSupplier};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test, JavaHashSet};

/// `TestFrozenBufferedUpdates`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFrozenBufferedUpdates",
        vec![test("testTermDocsIterator", test_term_docs_iterator)],
    )
}

/// `private static class PreferSeekExactLeafReader extends FilterLeafReader`.
struct PreferSeekExactLeafReader {
    inner: FilterLeafReader,
}

impl fmt::Debug for PreferSeekExactLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreferSeekExactLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for PreferSeekExactLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for PreferSeekExactLeafReader {
    crate::forward_leaf_reader!(filter inner;
        core, filter_leaf_reader_delegate, do_close, term_vectors, num_docs, max_doc,
        stored_fields, get_numeric_doc_values, get_binary_doc_values, get_sorted_doc_values,
        get_sorted_numeric_doc_values, get_sorted_set_doc_values, get_norm_values,
        get_doc_values_skipper, get_float_vector_values, get_byte_vector_values,
        search_nearest_vectors, search_nearest_vectors_byte, get_field_infos, get_live_docs,
        get_point_values, check_integrity, get_meta_data);

    /// `new FilterTerms(in.terms(field)) { … }`.
    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let terms = self.inner.get_delegate().terms(field)?;
        Ok(terms.map(|terms| {
            Box::new(PreferSeekExactTerms {
                inner: FilterTerms::new(terms),
            }) as Box<dyn Terms>
        }))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.inner.get_delegate().get_reader_cache_helper()
    }
}

/// The anonymous `FilterTerms` of `PreferSeekExactLeafReader.terms`.
struct PreferSeekExactTerms {
    inner: FilterTerms,
}

impl Terms for PreferSeekExactTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(PreferSeekExactTermsEnum {
            inner: FilterTermsEnum::new(self.inner.get_delegate().iterator()?),
        }))
    }
    fn size(&self) -> Result<i64> {
        self.inner.size()
    }
    fn sum_total_term_freq(&self) -> Result<i64> {
        self.inner.sum_total_term_freq()
    }
    fn sum_doc_freq(&self) -> Result<i64> {
        self.inner.sum_doc_freq()
    }
    fn doc_count(&self) -> Result<i32> {
        self.inner.doc_count()
    }
    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }
    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }
    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }
    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }
    fn stats(&self) -> Result<String> {
        self.inner.stats()
    }
}

/// The anonymous `FilterTermsEnum` whose `preferSeekExact()` is `true`.
struct PreferSeekExactTermsEnum {
    inner: FilterTermsEnum,
}

impl fmt::Display for PreferSeekExactTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl TermsEnum for PreferSeekExactTermsEnum {
    fn prefer_seek_exact(&self) -> bool {
        true
    }
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }
    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }
    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }
    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }
    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
    }
    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }
    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }
    fn doc_freq(&mut self) -> Result<i32> {
        self.inner.doc_freq()
    }
    fn total_term_freq(&mut self) -> Result<i64> {
        self.inner.total_term_freq()
    }
    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.inner.postings(reuse, flags)
    }
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.inner.impacts(flags)
    }
    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }
    fn prepare_seek_exact<'a>(
        &'a mut self,
        text: &BytesRef,
    ) -> Result<Option<Box<dyn IOBooleanSupplier + 'a>>> {
        self.inner.prepare_seek_exact(text)
    }
    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }
}

/// `new BytesRef(TestUtil.randomRealisticUnicodeString(random()))`.
fn random_id() -> BytesRef {
    BytesRef::from_char_sequence(&TestUtil::random_realistic_unicode_string(&*random()))
}

/// Port of `TestFrozenBufferedUpdates.testTermDocsIterator`.
fn test_term_docs_iterator() -> Result<()> {
    for _ in 0..5 {
        let mut dir = LuceneTestCase::new_directory();
        let writer = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let result = (|| -> Result<()> {
            let duplicates = random().next_boolean();
            let non_matches = random().next_boolean();
            let mut array = BytesRefArray::new(Arc::new(Counter::new_counter()));
            let num_docs = 10 + random().next_int_bound(1000);
            let mut random_ids: JavaHashSet<BytesRef> = JavaHashSet::new();
            for _ in 0..num_docs {
                loop {
                    let id = random_id();
                    if random_ids.add(id) {
                        break;
                    }
                }
            }
            let as_list = random_ids.to_vec();
            for r#ref in random_ids.iter() {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new_with_bytes(
                    "field",
                    r#ref.clone(),
                    Store::NO,
                )?));
                array.append(r#ref)?;
                if duplicates && LuceneTestCase::rarely() {
                    array.append(RandomPicks::random_from(&*random(), &as_list))?;
                }
                if non_matches && LuceneTestCase::rarely() {
                    let id = loop {
                        let id = random_id();
                        if !random_ids.contains(&id) {
                            break id;
                        }
                    };
                    array.append(&id)?;
                }
                writer.add_document(&doc)?;
            }
            writer.force_merge(1)?;
            writer.commit()?;
            let reader = directory_reader::open(dir.as_directory())?;
            let r = (|| -> Result<()> {
                let sorted = random().next_boolean();
                let prefer_seek_exact = random().next_boolean();
                let natural_order = |a: &BytesRef, b: &BytesRef| a.cmp(b) as i32;
                let mut values = if sorted {
                    array.iterator_with_sort_state(Some(
                        array.sort(StringSorterComparator::Generic(&natural_order), false),
                    ))
                } else {
                    array.iterator()
                };
                let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
                assert_equals(&1, &leaves.len());
                let mut leaf_reader: Arc<dyn LeafReader> = leaves[0].leaf_reader();
                if prefer_seek_exact {
                    leaf_reader = Arc::new(PreferSeekExactLeafReader {
                        inner: FilterLeafReader::new(leaf_reader)?,
                    });
                }
                let mut iterator = TermDocsIterator::new(&*leaf_reader, sorted);
                let mut bit_set = FixedBitSet::new(reader.max_doc() as usize);
                while let Some(r#ref) = values.next()? {
                    let present = iterator.next_term("field", &r#ref)?;
                    if !non_matches {
                        assert_not_null(&present.then_some(()));
                    }
                    if present {
                        let doc_id_set_iterator = iterator.docs()?;
                        loop {
                            let doc = doc_id_set_iterator.next_doc()?;
                            if doc == NO_MORE_DOCS {
                                break;
                            }
                            if !duplicates {
                                Assert::assert_false(bit_set.get(doc as usize));
                            }
                            bit_set.set(doc as usize);
                        }
                    }
                }
                assert_equals(&(reader.max_doc() as usize), &bit_set.cardinality());
                Ok(())
            })();
            let closed = reader.close();
            r?;
            closed
        })();
        let closed = writer.close();
        result?;
        closed?;
        dir.close()?;
    }
    Ok(())
}
