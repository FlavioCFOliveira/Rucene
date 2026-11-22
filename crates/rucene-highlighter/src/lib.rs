//! This crate ports Apache Lucene 10.5.0 `lucene/highlighter` to Rust: the
//! original highlighter, the fast vector highlighter, the unified highlighter
//! and the match highlighter.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.search`: `org.apache.lucene.search.uhighlight` is the
//! module [`uhighlight`]. Each Java source file is one module, named in
//! snake_case (`UnifiedHighlighter.java` is
//! [`uhighlight::unified_highlighter`]). A nested Java type is declared in the
//! module of its source file under its own name; the name of its enclosing
//! type is prefixed only when its own name is a name of the Rust prelude or
//! the name of another item of that module.
//!
//! Java strings and character sequences are indexed by UTF-16 code units; the
//! offsets of tokens, passages and fragments count UTF-16 units here as in
//! Java, and the text they index is held as UTF-16 where they slice it.

// Private Java members that only a subclass or a test reaches are kept even
// when nothing in the crate calls them.
#![allow(dead_code)]

/// Forwards the named `LeafReader` methods to the field `$inner`, as a Java
/// `FilterLeafReader` subclass inherits them.
macro_rules! forward_leaf_reader {
    ($mode:ident $inner:ident; $($m:ident),* $(,)?) => {
        $( forward_leaf_reader!(@ $mode $inner $m); )*
    };
    (@ $mode:ident $inner:ident core) => {
        fn core(&self) -> &rucene::index::index_reader::IndexReaderCore {
            rucene::index::LeafReader::core(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident filter_leaf_reader_delegate) => {
        /// `this instanceof FilterLeafReader`: the delegate `getDelegate()`
        /// returns.
        fn filter_leaf_reader_delegate(
            &self,
        ) -> Option<&std::sync::Arc<dyn rucene::index::LeafReader>> {
            rucene::index::LeafReader::filter_leaf_reader_delegate(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident do_close) => {
        /// The base class's `doClose()`: `in.close()` for a
        /// `FilterLeafReader`, `in.doClose()` for a `FilterCodecReader`.
        fn do_close(&self) -> rucene::error::Result<()> {
            rucene::index::LeafReader::do_close(&self.$inner)
        }
    };
    (@ $mode:ident $inner:ident term_vectors) => {
        fn term_vectors(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::leaf_reader::TermVectors>> {
            (&self.$inner as &dyn rucene::index::LeafReader).term_vectors()
        }
    };
    (@ $mode:ident $inner:ident num_docs) => {
        fn num_docs(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).num_docs()
        }
    };
    (@ $mode:ident $inner:ident max_doc) => {
        fn max_doc(&self) -> i32 {
            (&self.$inner as &dyn rucene::index::LeafReader).max_doc()
        }
    };
    (@ $mode:ident $inner:ident stored_fields) => {
        fn stored_fields(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::index_reader::StoredFields>> {
            (&self.$inner as &dyn rucene::index::LeafReader).stored_fields()
        }
    };
    (@ $mode:ident $inner:ident get_reader_cache_helper) => {
        fn get_reader_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_reader_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident get_core_cache_helper) => {
        fn get_core_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_core_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident terms) => {
        fn terms(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<std::sync::Arc<dyn rucene::index::Terms>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).terms(field)
        }
    };
    (@ $mode:ident $inner:ident get_numeric_doc_values) => {
        fn get_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_binary_doc_values) => {
        fn get_binary_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_binary_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_doc_values) => {
        fn get_sorted_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_numeric_doc_values) => {
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_set_doc_values) => {
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_sorted_set_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_norm_values) => {
        fn get_norm_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_norm_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_doc_values_skipper) => {
        fn get_doc_values_skipper(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_doc_values_skipper(field)
        }
    };
    (@ $mode:ident $inner:ident get_float_vector_values) => {
        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_float_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_byte_vector_values) => {
        fn get_byte_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_byte_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors) => {
        fn search_nearest_vectors(
            &self,
            field: &str,
            target: &[f32],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident search_nearest_vectors_byte) => {
        fn search_nearest_vectors_byte(
            &self,
            field: &str,
            target: &[u8],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            self.$inner
                .search_nearest_vectors_byte(field, target, collector, accept_docs)
        }
    };
    (@ $mode:ident $inner:ident get_field_infos) => {
        fn get_field_infos(&self) -> rucene::index::FieldInfos {
            (&self.$inner as &dyn rucene::index::LeafReader).get_field_infos()
        }
    };
    (@ $mode:ident $inner:ident get_live_docs) => {
        fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_live_docs()
        }
    };
    (@ $mode:ident $inner:ident get_point_values) => {
        fn get_point_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::PointValues>>> {
            (&self.$inner as &dyn rucene::index::LeafReader).get_point_values(field)
        }
    };
    (@ $mode:ident $inner:ident check_integrity) => {
        fn check_integrity(&self) -> rucene::error::Result<()> {
            (&self.$inner as &dyn rucene::index::LeafReader).check_integrity()
        }
    };
    (@ $mode:ident $inner:ident get_meta_data) => {
        fn get_meta_data(&self) -> rucene::index::LeafMetaData {
            (&self.$inner as &dyn rucene::index::LeafReader).get_meta_data()
        }
    };
}


pub(crate) use forward_leaf_reader;

pub(crate) mod java;
pub(crate) mod wrappers;

pub mod highlight;
pub mod matchhighlight;
pub mod uhighlight;
pub mod vectorhighlight;
