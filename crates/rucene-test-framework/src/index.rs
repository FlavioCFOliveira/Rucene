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
//! [`PerThreadPKLookup`].
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
//! with its own [`IndexReaderCore`](rucene::index::index_reader::IndexReaderCore)
//! (reference count, closed state): a `FilterLeafReader` subclass registers
//! itself as a parent of the wrapped reader (`in.registerParentReader(this)`,
//! `FilterLeafReader.java:332-339`) and closes it when it closes; a
//! `FilterCodecReader` subclass calls the wrapped reader's `doClose()`.
//! Core's own [`FilterLeafReader`](rucene::index::filter_reader::FilterLeafReader)
//! shares the wrapped reader's core instead (an undeclared divergence of
//! core, out of this module's scope).

/// Forwards the named [`LeafReader`](rucene::index::LeafReader) members of a
/// subclass to the wrapped reader held in the named field (`self.$inner`), as
/// the members a Java subclass inherits forward to `in`; `core` answers the
/// subclass's own `core` field.
///
/// The mode is the Java base class the members are inherited from:
/// `filter` (`FilterLeafReader`), whose members call `ensureOpen()` on the
/// subclass first where Java's do (`FilterLeafReader.java:341-476`; not
/// `numDocs`, `maxDoc`, `getFieldInfos`, the points and vectors accessors),
/// and whose `doClose()` is `in.close()`; or `codec` (`FilterCodecReader`),
/// whose members forward without `ensureOpen()` and whose `doClose()` is
/// `in.doClose()` (`FilterCodecReader.java:88-131`).
macro_rules! forward_leaf_reader {
    ($mode:ident $inner:ident; $($m:ident),* $(,)?) => {
        $( forward_leaf_reader!(@ $mode $inner $m); )*
    };
    (@guard filter $self:ident) => {
        $self.core.ensure_open()?;
    };
    (@guard codec $self:ident) => {};
    (@guard_unchecked filter $self:ident) => {
        $crate::index::ensure_open_unchecked(&$self.core);
    };
    (@guard_unchecked codec $self:ident) => {};
    (@ $mode:ident $inner:ident core) => {
        fn core(&self) -> &rucene::index::index_reader::IndexReaderCore {
            &self.core
        }
    };
    (@ $mode:ident $inner:ident filter_leaf_reader_delegate) => {
        /// `this instanceof FilterLeafReader`: the delegate `getDelegate()`
        /// returns.
        fn filter_leaf_reader_delegate(
            &self,
        ) -> Option<&std::sync::Arc<dyn rucene::index::LeafReader>> {
            Some(&self.$inner)
        }
    };
    (@ filter $inner:ident do_close) => {
        /// `in.close()`.
        fn do_close(&self) -> rucene::error::Result<()> {
            std::sync::Arc::clone(&self.$inner).as_index_reader().close()
        }
    };
    (@ codec $inner:ident do_close) => {
        /// `in.doClose()`.
        fn do_close(&self) -> rucene::error::Result<()> {
            self.$inner.do_close()
        }
    };
    (@ $mode:ident $inner:ident term_vectors) => {
        fn term_vectors(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::leaf_reader::TermVectors>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.term_vectors()
        }
    };
    (@ $mode:ident $inner:ident num_docs) => {
        fn num_docs(&self) -> i32 {
            self.$inner.num_docs()
        }
    };
    (@ $mode:ident $inner:ident max_doc) => {
        fn max_doc(&self) -> i32 {
            self.$inner.max_doc()
        }
    };
    (@ $mode:ident $inner:ident stored_fields) => {
        fn stored_fields(
            &self,
        ) -> rucene::error::Result<Box<dyn rucene::index::index_reader::StoredFields>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.stored_fields()
        }
    };
    (@ $mode:ident $inner:ident get_reader_cache_helper) => {
        fn get_reader_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            self.$inner.get_reader_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident get_core_cache_helper) => {
        fn get_core_cache_helper(
            &self,
        ) -> Option<Box<dyn rucene::index::index_reader::CacheHelper>> {
            self.$inner.get_core_cache_helper()
        }
    };
    (@ $mode:ident $inner:ident terms) => {
        fn terms(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::Terms>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.terms(field)
        }
    };
    (@ $mode:ident $inner:ident get_numeric_doc_values) => {
        fn get_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_binary_doc_values) => {
        fn get_binary_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_binary_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_doc_values) => {
        fn get_sorted_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_sorted_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_numeric_doc_values) => {
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_sorted_numeric_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_sorted_set_doc_values) => {
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_sorted_set_doc_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_norm_values) => {
        fn get_norm_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_norm_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_doc_values_skipper) => {
        fn get_doc_values_skipper(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.get_doc_values_skipper(field)
        }
    };
    (@ $mode:ident $inner:ident get_float_vector_values) => {
        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
            self.$inner.get_float_vector_values(field)
        }
    };
    (@ $mode:ident $inner:ident get_byte_vector_values) => {
        fn get_byte_vector_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
            self.$inner.get_byte_vector_values(field)
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
            self.$inner.get_field_infos()
        }
    };
    (@ $mode:ident $inner:ident get_live_docs) => {
        fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
            forward_leaf_reader!(@guard_unchecked $mode self);
            self.$inner.get_live_docs()
        }
    };
    (@ $mode:ident $inner:ident get_point_values) => {
        fn get_point_values(
            &self,
            field: &str,
        ) -> rucene::error::Result<Option<Box<dyn rucene::index::PointValues>>> {
            self.$inner.get_point_values(field)
        }
    };
    (@ $mode:ident $inner:ident check_integrity) => {
        fn check_integrity(&self) -> rucene::error::Result<()> {
            forward_leaf_reader!(@guard $mode self);
            self.$inner.check_integrity()
        }
    };
    (@ $mode:ident $inner:ident get_meta_data) => {
        fn get_meta_data(&self) -> rucene::index::LeafMetaData {
            forward_leaf_reader!(@guard_unchecked $mode self);
            self.$inner.get_meta_data()
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
/// is thrown (a panic carrying the Java exception).
pub(crate) fn ensure_open_unchecked(core: &rucene::index::index_reader::IndexReaderCore) {
    if let Err(e) = core.ensure_open() {
        throw_unchecked(e)
    }
}

/// Throws `e`, an unchecked Java exception, from a signature that cannot
/// fail.
pub(crate) fn throw_unchecked(e: rucene::error::LuceneError) -> ! {
    match e {
        rucene::error::LuceneError::AlreadyClosed { message, .. } => {
            randomizedtesting::jdk::exception::throw(
                "org.apache.lucene.store.AlreadyClosedException",
                message,
            )
        }
        other => panic!("{other}"),
    }
}

/// The reader state of a new `FilterLeafReader` subclass over `in`: a core
/// of its own, registered as a parent of `in` (`in.registerParentReader(this)`,
/// `FilterLeafReader.java:332-339`), so that closing `in` makes this reader
/// fail `ensureOpen()`.
pub(crate) fn filter_leaf_core(
    r#in: &std::sync::Arc<dyn rucene::index::LeafReader>,
) -> rucene::index::index_reader::IndexReaderCore {
    let core = rucene::index::index_reader::IndexReaderCore::new();
    r#in.core().register_parent_core(&core);
    core
}

pub mod all_deleted_filter_reader;
pub mod asserting_directory_reader;
pub mod asserting_leaf_reader;
pub mod field_filter_leaf_reader;
pub mod merge_reader_wrapper;
pub mod merging_codec_reader;
pub mod merging_directory_reader_wrapper;
pub mod mismatched_codec_reader;
pub mod mismatched_directory_reader;
pub mod mismatched_leaf_reader;
pub mod own_cache_key_multi_reader;
pub mod per_thread_pk_lookup;
pub mod points_stack_tracker;
mod shared_producers;

pub use all_deleted_filter_reader::AllDeletedFilterReader;
pub use asserting_directory_reader::AssertingDirectoryReader;
pub use asserting_leaf_reader::AssertingLeafReader;
pub use field_filter_leaf_reader::FieldFilterLeafReader;
pub use merge_reader_wrapper::MergeReaderWrapper;
pub use merging_codec_reader::MergingCodecReader;
pub use merging_directory_reader_wrapper::MergingDirectoryReaderWrapper;
pub use mismatched_codec_reader::MismatchedCodecReader;
pub use mismatched_directory_reader::MismatchedDirectoryReader;
pub use mismatched_leaf_reader::MismatchedLeafReader;
pub use own_cache_key_multi_reader::OwnCacheKeyMultiReader;
pub use per_thread_pk_lookup::PerThreadPKLookup;
pub use points_stack_tracker::PointsStackTracker;
