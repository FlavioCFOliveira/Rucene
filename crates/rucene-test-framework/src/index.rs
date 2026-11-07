/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.index`.
//!
//! The readers and reader wrappers of the package (rmp #344):
//! [`AssertingLeafReader`], [`AssertingDirectoryReader`],
//! [`AllDeletedFilterReader`], [`FieldFilterLeafReader`],
//! [`MergeReaderWrapper`], [`MergingCodecReader`],
//! [`MergingDirectoryReaderWrapper`], [`MismatchedCodecReader`],
//! [`MismatchedDirectoryReader`], [`MismatchedLeafReader`],
//! [`OwnCacheKeyMultiReader`], [`PointsStackTracker`] and
//! [`PerThreadPKLookup`]. The writer side the analysis tests' indexing branch
//! needs: [`RandomIndexWriter`] and the merge policies and listener
//! `LuceneTestCase.newIndexWriterConfig` draws from, [`MockRandomMergePolicy`],
//! [`AlcoholicMergePolicy`] and [`MockIndexWriterEventListener`].
//!
//! **Subclassing.** Java's `FilterLeafReader` and `FilterCodecReader` are
//! abstract classes their subclasses extend, overriding some members and
//! inheriting the forwarding of the rest. Rust has no implementation
//! inheritance, so each subclass here is a type of its own that holds the
//! wrapped reader, implements the members its Java class overrides, and
//! forwards the others with `forward_leaf_reader!` (a `FilterLeafReader`
//! subclass: to the wrapped reader, as Java's inherited members do) or
//! answers them through `rucene::index::codec_reader::final_members` (a
//! `FilterCodecReader` subclass: through its own producers, as the `final`
//! members of Java's `CodecReader` do).
//!
//! **Reader state.** As in Java, each subclass is an `IndexReader` of its own,
//! and that state is its base class's: a `FilterLeafReader` subclass holds a
//! core [`FilterLeafReader`](rucene::index::filter_reader::FilterLeafReader),
//! which registers itself as a parent of the wrapped reader
//! (`FilterLeafReader.java:332-339`) and closes it when it closes; a
//! `FilterCodecReader` subclass holds a core
//! [`FilterCodecReader`](rucene::index::filter_reader::FilterCodecReader),
//! whose `doClose()` is the wrapped reader's `doClose()`. The subclass's
//! `core` is its base's.

/// Forwards the named [`LeafReader`](rucene::index::LeafReader) members of a
/// subclass to its base, the core reader held in the named field
/// (`self.$inner`), as the members a Java subclass inherits from its base
/// class do: the base is a core
/// [`FilterLeafReader`](rucene::index::filter_reader::FilterLeafReader)
/// (mode `filter`) or
/// [`FilterCodecReader`](rucene::index::filter_reader::FilterCodecReader)
/// (mode `codec`), which owns the subclass's reader state and calls
/// `ensureOpen()` exactly where Java's base class does.
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
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::Terms>>> {
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

/// Answers the named [`LeafReader`](rucene::index::LeafReader) members of a
/// `FilterCodecReader` subclass through
/// [`final_members`](rucene::index::codec_reader::final_members), that is
/// through the subclass's own producers, as the members Java's `CodecReader`
/// declares `final` do.
macro_rules! codec_leaf_reader {
    ($($m:ident),* $(,)?) => { $( codec_leaf_reader!(@ $m); )* };
    (@ term_vectors) => {
        fn term_vectors(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::leaf_reader::TermVectors>> {
            rucene::index::codec_reader::final_members::term_vectors(self)
        }
    };
    (@ stored_fields) => {
        fn stored_fields(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::index_reader::StoredFields>> {
            rucene::index::codec_reader::final_members::stored_fields(self)
        }
    };
    (@ terms) => {
        fn terms(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::Terms>>> {
            rucene::index::codec_reader::final_members::terms(self, field)
        }
    };
    (@ get_numeric_doc_values) => {
        fn get_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            rucene::index::codec_reader::final_members::numeric_doc_values(self, field)
        }
    };
    (@ get_binary_doc_values) => {
        fn get_binary_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
            rucene::index::codec_reader::final_members::binary_doc_values(self, field)
        }
    };
    (@ get_sorted_doc_values) => {
        fn get_sorted_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
            rucene::index::codec_reader::final_members::sorted_doc_values(self, field)
        }
    };
    (@ get_sorted_numeric_doc_values) => {
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
            rucene::index::codec_reader::final_members::sorted_numeric_doc_values(self, field)
        }
    };
    (@ get_sorted_set_doc_values) => {
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
            rucene::index::codec_reader::final_members::sorted_set_doc_values(self, field)
        }
    };
    (@ get_norm_values) => {
        fn get_norm_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            rucene::index::codec_reader::final_members::norm_values(self, field)
        }
    };
    (@ get_doc_values_skipper) => {
        fn get_doc_values_skipper(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
            rucene::index::codec_reader::final_members::doc_values_skipper(self, field)
        }
    };
    (@ get_float_vector_values) => {
        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
            rucene::index::codec_reader::final_members::float_vector_values(self, field)
        }
    };
    (@ get_byte_vector_values) => {
        fn get_byte_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
            rucene::index::codec_reader::final_members::byte_vector_values(self, field)
        }
    };
    (@ search_nearest_vectors) => {
        fn search_nearest_vectors(
            &self,
            field: &str,
            target: &[f32],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            rucene::index::codec_reader::final_members::search_nearest_vectors(
                self,
                field,
                target,
                collector,
                accept_docs,
            )
        }
    };
    (@ search_nearest_vectors_byte) => {
        fn search_nearest_vectors_byte(
            &self,
            field: &str,
            target: &[u8],
            collector: &mut dyn rucene::search::knn::KnnCollector,
            accept_docs: &mut dyn rucene::search::AcceptDocs,
        ) -> rucene::error::Result<()> {
            rucene::index::codec_reader::final_members::search_nearest_vectors_byte(
                self,
                field,
                target,
                collector,
                accept_docs,
            )
        }
    };
    (@ get_point_values) => {
        fn get_point_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::PointValues>>> {
            rucene::index::codec_reader::final_members::point_values(self, field)
        }
    };
    (@ check_integrity) => {
        fn check_integrity(&self) -> rucene::error::Result<()> {
            rucene::index::codec_reader::final_members::check_integrity(self)
        }
    };
}

/// `ensureOpen()` where the Rust signature cannot fail (`getLiveDocs()`,
/// `getMetaData()`, ...): the `AlreadyClosedException`, unchecked in Java,
/// is thrown (a panic carrying the Java exception), as core's
/// [`IndexReaderCore::ensure_open_unchecked`](rucene::index::index_reader::IndexReaderCore::ensure_open_unchecked)
/// does.
pub(crate) fn ensure_open_unchecked(core: &rucene::index::index_reader::IndexReaderCore) {
    core.ensure_open_unchecked();
}

pub mod alcoholic_merge_policy;
pub mod all_deleted_filter_reader;
pub mod asserting_directory_reader;
pub mod asserting_leaf_reader;
pub mod base_index_file_format_test_case;
pub mod base_postings_format_test_case;
pub mod base_stored_fields_format_test_case;
pub mod doc_helper;
pub mod field_filter_leaf_reader;
pub mod force_merge_policy;
pub mod merge_reader_wrapper;
pub mod merging_codec_reader;
pub mod merging_directory_reader_wrapper;
pub mod mismatched_codec_reader;
pub mod mismatched_directory_reader;
pub mod mismatched_leaf_reader;
pub mod mock_index_writer_event_listener;
pub mod mock_random_merge_policy;
pub mod own_cache_key_multi_reader;
pub mod per_thread_pk_lookup;
pub mod points_stack_tracker;
pub mod random_codec;
pub mod random_index_writer;
pub mod random_postings_tester;
mod shared_producers;
pub mod suppressing_concurrent_merge_scheduler;

pub use alcoholic_merge_policy::AlcoholicMergePolicy;
pub use all_deleted_filter_reader::AllDeletedFilterReader;
pub use asserting_directory_reader::AssertingDirectoryReader;
pub use asserting_leaf_reader::AssertingLeafReader;
pub use doc_helper::DocHelper;
pub use field_filter_leaf_reader::FieldFilterLeafReader;
pub use force_merge_policy::ForceMergePolicy;
pub use merge_reader_wrapper::MergeReaderWrapper;
pub use merging_codec_reader::MergingCodecReader;
pub use merging_directory_reader_wrapper::MergingDirectoryReaderWrapper;
pub use mismatched_codec_reader::MismatchedCodecReader;
pub use mismatched_directory_reader::MismatchedDirectoryReader;
pub use mismatched_leaf_reader::MismatchedLeafReader;
pub use mock_index_writer_event_listener::MockIndexWriterEventListener;
pub use mock_random_merge_policy::MockRandomMergePolicy;
pub use own_cache_key_multi_reader::OwnCacheKeyMultiReader;
pub use per_thread_pk_lookup::PerThreadPKLookup;
pub use points_stack_tracker::PointsStackTracker;
pub use random_codec::RandomCodec;
pub use random_index_writer::RandomIndexWriter;
pub use random_postings_tester::RandomPostingsTester;
pub use suppressing_concurrent_merge_scheduler::SuppressingConcurrentMergeScheduler;
