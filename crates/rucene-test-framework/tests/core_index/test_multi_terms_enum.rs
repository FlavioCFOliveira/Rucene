//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMultiTermsEnum.java`.
//!
//! * `MigratingCodecReader extends FilterCodecReader` forwards to a
//!   `FilterCodecReader` and overrides `getPostingsReader`,
//!   `getCoreCacheHelper` and `getReaderCacheHelper`.
//! * **Divergence.** `BaseMigratingFieldsProducer.close()` calls
//!   `delegate.close()` (`TestMultiTermsEnum.java:233-236`); the delegate
//!   `super.getPostingsReader()` returns is the reader's own producer,
//!   shared through an `Arc`, which can only be closed while no other
//!   handle exists: `close` closes it then, and leaves it to its reader
//!   otherwise. The merge instance a producer creates is owned and always
//!   closed.
//! * `Directory.close()` takes `&mut self`: the bare `ByteBuffersDirectory`
//!   is closed through its `Arc` once the writer and the reader released
//!   their handles.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::Analyzer;
use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::points::PointsReader;
use rucene::codecs::postings::FieldsProducer;
use rucene::codecs::stored_fields::StoredFieldsReader;
use rucene::codecs::term_vectors::TermVectorsReader;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_reader::FilterCodecReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::{AcceptStatus, Fields, FilteredTermsEnum, FilteredTermsEnumFilter};
use rucene::index::{FieldInfos, LeafReader, Terms, TermsEnum};
use rucene::store::ByteBuffersDirectory;
use rucene::util::{Bits, BytesRef};
use rucene_test_framework::analysis::MockAnalyzer;

use crate::core_support::{class, random, test};

/// `TestMultiTermsEnum`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMultiTermsEnum",
        vec![test("testNoTermsInField", test_no_terms_in_field)],
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))`.
fn plain_iwc() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(
        Arc::new(MockAnalyzer::new(&*random())) as Arc<dyn Analyzer>
    ))
}

/// LUCENE-6826
///
/// Port of `TestMultiTermsEnum.testNoTermsInField`.
fn test_no_terms_in_field() -> Result<()> {
    let mut directory = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&directory) as _, plain_iwc())?;
    let mut document = Document::new();
    document.add(Box::new(StringField::new(
        "deleted",
        "0".into(),
        Store::YES,
    )?));
    writer.add_document(&document)?;

    let reader = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;
    drop(writer);

    let directory2 = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&directory2) as _, plain_iwc())?;

    let leaves = Arc::clone(&reader).leaves()?;
    let mut codec_readers: Vec<Arc<dyn CodecReader>> = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        let codec_reader = leaf.leaf_reader().as_codec_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
        });
        codec_readers.push(Arc::new(MigratingCodecReader::new(codec_reader)));
    }
    writer.add_indexes_codec_readers(&codec_readers)?; // <- bang
    drop(codec_readers);
    drop(leaves);

    // `IOUtils.close(writer, reader, directory)`: the first failure wins.
    let closed_writer = writer.close();
    drop(writer);
    let closed_reader = reader.close();
    drop(reader);
    // `Directory.close` takes the directory mutably: it is closed once the
    // writer and the reader have released their handles.
    let closed_directory = match Arc::get_mut(&mut directory) {
        Some(d) => rucene::store::Directory::close(d),
        None => Ok(()),
    };
    closed_writer.and(closed_reader).and(closed_directory)
}

/// `private static class MigratingCodecReader extends FilterCodecReader`.
struct MigratingCodecReader {
    base: FilterCodecReader,
    r#in: Arc<dyn CodecReader>,
}

impl MigratingCodecReader {
    fn new(r#in: Arc<dyn CodecReader>) -> Self {
        MigratingCodecReader {
            base: FilterCodecReader::new(Arc::clone(&r#in)),
            r#in,
        }
    }
}

impl std::fmt::Display for MigratingCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.base, f)
    }
}

impl std::fmt::Debug for MigratingCodecReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigratingCodecReader")
            .finish_non_exhaustive()
    }
}

impl LeafReader for MigratingCodecReader {
    crate::forward_leaf_reader!(filter base;
        core,
        max_doc,
        num_docs,
        do_close,
        check_integrity,
        get_meta_data,
        get_field_infos,
        term_vectors,
        stored_fields,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_point_values,
    );

    fn as_codec_reader(self: Arc<Self>) -> Option<Arc<dyn CodecReader>> {
        Some(self)
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        self.base.get_live_docs()
    }
}

impl CodecReader for MigratingCodecReader {
    fn get_fields_reader(&self) -> Result<Option<Box<dyn StoredFieldsReader>>> {
        self.base.get_fields_reader()
    }

    fn get_term_vectors_reader(&self) -> Result<Option<Box<dyn TermVectorsReader>>> {
        self.base.get_term_vectors_reader()
    }

    fn get_norms_reader(&self) -> Result<Option<Arc<dyn NormsProducer>>> {
        self.base.get_norms_reader()
    }

    fn get_doc_values_reader(&self) -> Result<Option<Arc<dyn DocValuesProducer>>> {
        self.base.get_doc_values_reader()
    }

    fn get_postings_reader(&self) -> Result<Option<Arc<dyn FieldsProducer>>> {
        let delegate = self.base.get_postings_reader()?;
        Ok(delegate.map(|delegate| {
            Arc::new(MigratingFieldsProducer::new(
                Delegate::Shared(delegate),
                Arc::new(LeafReader::get_field_infos(self)),
            )) as Arc<dyn FieldsProducer>
        }))
    }

    fn get_points_reader(&self) -> Result<Option<Arc<dyn PointsReader>>> {
        self.base.get_points_reader()
    }

    fn get_vector_reader(&self) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
        self.base.get_vector_reader()
    }

    /// `this instanceof FilterCodecReader`: the delegate.
    fn filter_codec_reader_delegate(&self) -> Option<&Arc<dyn CodecReader>> {
        Some(&self.r#in)
    }
}

/// The `delegate` of a `BaseMigratingFieldsProducer`: the reader's own
/// producer, or a merge instance this producer owns.
enum Delegate {
    Shared(Arc<dyn FieldsProducer>),
    Owned(Box<dyn FieldsProducer>),
}

impl Delegate {
    fn get(&self) -> &dyn FieldsProducer {
        match self {
            Delegate::Shared(d) => d.as_ref(),
            Delegate::Owned(d) => d.as_ref(),
        }
    }
}

/// `private static class MigratingFieldsProducer extends BaseMigratingFieldsProducer`
/// (and `BaseMigratingFieldsProducer`, whose members it inherits).
struct MigratingFieldsProducer {
    delegate: Delegate,
    new_field_info: Arc<FieldInfos>,
}

impl MigratingFieldsProducer {
    fn new(delegate: Delegate, new_field_info: Arc<FieldInfos>) -> Self {
        MigratingFieldsProducer {
            delegate,
            new_field_info,
        }
    }

    /// `BaseMigratingFieldsProducer.terms(field)`: `delegate.terms(field)`.
    fn super_terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.delegate.get().terms(field)
    }

    /// `create(delegate, newFieldInfo)`.
    fn create(delegate: Box<dyn FieldsProducer>, new_field_info: Arc<FieldInfos>) -> Self {
        MigratingFieldsProducer::new(Delegate::Owned(delegate), new_field_info)
    }
}

impl Fields for MigratingFieldsProducer {
    /// `BaseMigratingFieldsProducer.iterator()`: the names of `newFieldInfo`.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.new_field_info.iter().map(|fi| fi.name.to_string()))
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        if "deleted" == field {
            let deleted_terms = self.super_terms("deleted")?;
            if let Some(deleted_terms) = deleted_terms {
                return Ok(Some(Box::new(ValueFilteredTerms::new(
                    deleted_terms,
                    BytesRef::from_char_sequence("1"),
                ))));
            }
            Ok(None)
        } else {
            self.super_terms(field)
        }
    }

    /// `BaseMigratingFieldsProducer.size()`: `newFieldInfo.size()`.
    fn size(&self) -> i32 {
        self.new_field_info.size() as i32
    }
}

impl FieldsProducer for MigratingFieldsProducer {
    fn check_integrity(&self) -> Result<()> {
        self.delegate.get().check_integrity()
    }

    /// `getMergeInstance()`: `create(delegate.getMergeInstance(), newFieldInfo)`.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(MigratingFieldsProducer::create(
            self.delegate.get().get_merge_instance()?,
            Arc::clone(&self.new_field_info),
        )))
    }

    fn close(&mut self) -> Result<()> {
        match &mut self.delegate {
            Delegate::Owned(d) => d.close(),
            Delegate::Shared(d) => match Arc::get_mut(d) {
                Some(d) => d.close(),
                None => Ok(()),
            },
        }
    }
}

/// `private static class ValueFilteredTerms extends Terms`.
struct ValueFilteredTerms {
    delegate: Box<dyn Terms>,
    value: BytesRef,
}

impl ValueFilteredTerms {
    fn new(delegate: Box<dyn Terms>, value: BytesRef) -> Self {
        ValueFilteredTerms { delegate, value }
    }
}

/// The anonymous `FilteredTermsEnum` of `ValueFilteredTerms.iterator()`.
struct ValueFilter {
    value: BytesRef,
}

impl FilteredTermsEnumFilter for ValueFilter {
    fn accept(&mut self, term: &BytesRef) -> Result<AcceptStatus> {
        let comparison = term.cmp(&self.value);
        Ok(if comparison.is_lt() {
            // I don't think it will actually get here because they are supposed to call
            // nextSeekTerm
            // to get the initial term to seek to.
            AcceptStatus::NoAndSeek
        } else if comparison.is_gt() {
            AcceptStatus::End
        } else {
            // comparison == 0
            AcceptStatus::Yes
        })
    }

    fn next_seek_term(&mut self, current_term: Option<&BytesRef>) -> Result<Option<BytesRef>> {
        if current_term.is_none_or(|t| t.cmp(&self.value).is_lt()) {
            return Ok(Some(self.value.clone()));
        }
        Ok(None)
    }
}

/// `throw new UnsupportedOperationException()`.
fn unsupported<T>() -> Result<T> {
    Err(rucene::error::LuceneError::UnsupportedOperation(
        String::new(),
    ))
}

impl Terms for ValueFilteredTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        // `new FilteredTermsEnum(tenum)`: `startWithSeek` is `true`.
        Ok(Box::new(FilteredTermsEnum::with_start_with_seek(
            self.delegate.iterator()?,
            Box::new(ValueFilter {
                value: self.value.clone(),
            }),
            true,
        )))
    }

    fn size(&self) -> Result<i64> {
        unsupported()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        unsupported()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        unsupported()
    }

    fn doc_count(&self) -> Result<i32> {
        unsupported()
    }

    fn has_freqs(&self) -> bool {
        self.delegate.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.delegate.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.delegate.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.delegate.has_payloads()
    }
}
