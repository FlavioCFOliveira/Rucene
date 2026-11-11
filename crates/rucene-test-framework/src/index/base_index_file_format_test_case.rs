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

//! Port of `org.apache.lucene.tests.index.BaseIndexFileFormatTestCase`:
//! common tests to all index formats.
//!
//! # How a suite runs
//!
//! Java's abstract class declares the inherited `test*` methods and calls the
//! abstract `getCodec()`/`addRandomFields(Document)` of the subclass by
//! virtual dispatch. The port splits the two halves of that class:
//!
//! * the trait [`BaseIndexFileFormatTestCase`] holds the members (the
//!   overridable hooks with Java's default bodies and the tests); a subclass
//!   implements it, overriding what Java's subclass overrides;
//! * [`class()`] is the reflective declaration the runner sees
//!   (`public abstract class BaseIndexFileFormatTestCase extends
//!   LuceneTestCase`), whose methods run on the test instance a subclass's
//!   constructor builds, a [`FormatTestInstance`] holding the subclass as a
//!   trait object.
//!
//! The suites that extend it (`BasePostingsFormatTestCase`,
//! `BaseStoredFieldsFormatTestCase`) declare their classes the same way and
//! reach their own trait through [`BaseIndexFileFormatTestCase::as_postings`]
//! and [`BaseIndexFileFormatTestCase::as_stored_fields`], Rust's stand-in for
//! the cast a Java subclass needs none of.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `ReadBytesDirectoryWrapper.openChecksumInput`: the port's
//!   `Directory::open_checksum_input` returns the concrete
//!   `BufferedChecksumIndexInput`, so the wrapper cannot return Java's
//!   anonymous `ChecksumIndexInput`; it wraps the tracked input of
//!   [`ReadBytesDirectoryWrapper`]'s `open_input` (the same file, `READONCE`,
//!   which is what the delegate's `openChecksumInput` opens) in a
//!   `BufferedChecksumIndexInput`, whose reads go through the tracking input
//!   one for one, so the same bytes are marked.
//! * `Accumulator`: Java walks the class and its superclasses for
//!   `EXCLUDED_CLASSES` and skips `Thread` references; the port's
//!   `RamObject` names only the object's own class, so the check is on that
//!   name, and no `RamObject` is a `Thread`. The class is unused in Lucene
//!   10.5.0 (it served the removed `testRamBytesUsed`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::doc_values::DocValuesProducer;
use rucene::codecs::norms::NormsProducer;
use rucene::codecs::Codec;
use rucene::document::{Document, Field, FieldType, NumericDocValuesField, StoredValue, TextField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::doc_values::{DocValuesIterator, NumericDocValues};
use rucene::index::documents_writer::BufferedUpdates;
use rucene::index::field_infos::{FieldInfo, FieldInfos};
use rucene::index::index_file_names;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::MergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::segment_info::SegmentInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::{Fields, Terms};
use rucene::index::{IndexableField, LeafReader};
use rucene::search::{DocIdSetIterator, Sort, NO_MORE_DOCS};
use rucene::store::{
    flush_io_context, BufferedChecksumIndexInput, DataInput, Directory, FlushInfo, IOContext,
    IndexInput, IndexOutput, Lock, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use rucene::util::bit_set::BitSet;
use rucene::util::string_helper::StringHelper;
use rucene::util::{BytesRef, FixedBitSet, NoOutputInfoStream, RamUsageEstimator, Version};

use crate::analysis::MockAnalyzer;
use crate::index::MergingDirectoryReaderWrapper;
use crate::store::Throttling;
use crate::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use crate::util::ram_usage_tester::{self, JavaObject, Queue, RamObject};
use crate::util::TestUtil;

/// Throws a Rucene error out of a test method, as Java's `throws Exception`:
/// the throwable has the Java class and message the error stands for.
pub fn rethrow(e: LuceneError) -> ! {
    let (class, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(&e);
    Throwable::new(class, message).throw()
}

/// `INDEX_WRITER_ACCESS.isDeleterClosed(iw)`.
fn is_deleter_closed(iw: &IndexWriter) -> bool {
    iw.is_deleter_closed().unwrap_or_else(|e| rethrow(e))
}

// -----------------------------------------------------------------------------
// EXCLUDED_CLASSES and Accumulator
// -----------------------------------------------------------------------------

/// `EXCLUDED_CLASSES`: "metadata or Directory-level objects", by Java class
/// name.
pub static EXCLUDED_CLASSES: &[&str] = &[
    // Directory objects, don't take into account, e.g. the NIO buffers
    "org.apache.lucene.store.Directory",
    "org.apache.lucene.store.IndexInput",
    // used for thread management, not by the index
    "org.apache.lucene.util.CloseableThreadLocal",
    "java.lang.ThreadLocal",
    // don't follow references to the top-level reader
    "org.apache.lucene.index.IndexReader",
    "org.apache.lucene.index.IndexReaderContext",
    // usually small but can bump memory usage for
    // memory-efficient things like stored fields
    "org.apache.lucene.index.FieldInfos",
    "org.apache.lucene.index.SegmentInfo",
    "org.apache.lucene.index.SegmentCommitInfo",
    "org.apache.lucene.index.FieldInfo",
    // constant overhead is typically due to strings
    // TODO: can we remove this and still pass the test consistently
    "java.lang.String",
];

/// Port of `BaseIndexFileFormatTestCase.Accumulator`: a
/// `RamUsageTester.Accumulator` that skips [`EXCLUDED_CLASSES`] (but the
/// root) and counts JDK collections and maps by their references only.
pub struct Accumulator {
    /// The address of `root`, the object being measured.
    root: *const (),
}

// SAFETY: `root` is only compared by address, never dereferenced.
unsafe impl Send for Accumulator {}

impl Accumulator {
    /// `Accumulator(Object root)`.
    pub fn new(root: &dyn RamObject) -> Self {
        Accumulator {
            root: root as *const dyn RamObject as *const (),
        }
    }
}

impl ram_usage_tester::Accumulator for Accumulator {
    fn accumulate_object<'a>(
        &mut self,
        o: &'a dyn RamObject,
        shallow_size: i64,
        field_values: &[(&'static str, Option<&'a dyn RamObject>)],
        queue: &mut Queue<'a>,
    ) -> i64 {
        if EXCLUDED_CLASSES.contains(&o.java_class_name())
            && !std::ptr::eq(o as *const dyn RamObject as *const (), self.root)
        {
            return 0;
        }
        // we have no way to estimate the size of these things in codecs although
        // something like a Collections.newSetFromMap(new HashMap<>()) uses quite
        // some memory... So for now the test ignores the overhead of such
        // collections but can we do better?
        match o.java_object() {
            JavaObject::JavaIterable { elements, .. } => {
                let v = elements.len() as i64 * RamUsageEstimator::NUM_BYTES_OBJECT_REF;
                queue.extend(elements);
                v
            }
            JavaObject::JavaMap { entries, .. } => {
                let v = 2 * entries.len() as i64 * RamUsageEstimator::NUM_BYTES_OBJECT_REF;
                queue.extend(entries.iter().map(|(k, _)| *k));
                queue.extend(entries.iter().map(|(_, v)| *v));
                v
            }
            _ => {
                let mut references: Queue<'a> = Vec::new();
                let v = shallow_size;
                references.extend(field_values.iter().map(|(_, value)| *value));
                // AssertingCodec adds Thread references to make sure objects are consumed in
                // the right thread (no `RamObject` of the port is a `Thread`)
                queue.extend(references);
                v
            }
        }
    }

    fn accumulate_array<'a>(
        &mut self,
        _array: &'a dyn RamObject,
        shallow_size: i64,
        values: &[Option<&'a dyn RamObject>],
        queue: &mut Queue<'a>,
    ) -> i64 {
        queue.extend_from_slice(values);
        shallow_size
    }
}

// -----------------------------------------------------------------------------
// The class and its instance
// -----------------------------------------------------------------------------

/// The test instance of every suite extending `BaseIndexFileFormatTestCase`:
/// the subclass, which a subclass's reflective constructor builds (running
/// Java's field initialisers, `random()` included), and the codec
/// `setUp()` saves.
pub struct FormatTestInstance {
    /// The subclass, as the trait its members are reached through.
    pub case: Arc<dyn BaseIndexFileFormatTestCase>,
    /// The subclass, for the methods it declares itself (`this` of its own
    /// overrides and hooks).
    subclass: Arc<dyn std::any::Any + Send + Sync>,
    /// `savedCodec`.
    saved_codec: Option<Arc<dyn Codec>>,
}

impl FormatTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseIndexFileFormatTestCase>(case: Arc<T>) -> Self {
        FormatTestInstance {
            case: Arc::clone(&case) as Arc<dyn BaseIndexFileFormatTestCase>,
            subclass: case,
            saved_codec: None,
        }
    }

    /// The subclass as its own type, for a method the subclass declares.
    ///
    /// # Panics
    ///
    /// `ClassCastException` when the instance is of another subclass.
    pub fn subclass<T: BaseIndexFileFormatTestCase>(&self) -> Arc<T> {
        Arc::clone(&self.subclass)
            .downcast::<T>()
            .unwrap_or_else(|_| {
                Throwable::new(
                    "java.lang.ClassCastException",
                    "the test instance is of another class",
                )
                .throw()
            })
    }
}

/// A test method of the abstract classes: `public void <name>() throws
/// Exception`, run on the instance's subclass.
pub fn format_test(
    name: &str,
    body: impl Fn(&dyn BaseIndexFileFormatTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut FormatTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow(e))
        })
}

/// The declaration of `org.apache.lucene.tests.index.BaseIndexFileFormatTestCase`
/// (`public abstract class BaseIndexFileFormatTestCase extends
/// LuceneTestCase`), with its overrides of `setUp()`/`tearDown()` and its
/// four tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.index.BaseIndexFileFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("setUp")
                    .throws(&["java.lang.Exception"])
                    .body(set_up),
            )
            .method(
                Method::new("tearDown")
                    .throws(&["java.lang.Exception"])
                    .body(tear_down),
            )
            .method(format_test("testMergeStability", |c| {
                c.test_merge_stability()
            }))
            .method(format_test("testMultiClose", |c| c.test_multi_close()))
            .method(format_test("testRandomExceptions", |c| {
                c.test_random_exceptions()
            }))
            .method(format_test("testCheckIntegrityReadsAllBytes", |c| {
                c.test_check_integrity_reads_all_bytes()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// `BaseIndexFileFormatTestCase.setUp()`, which a subclass overriding
/// `setUp()` calls as `super.setUp()`: `LuceneTestCase.setUp()`, then saves
/// the default codec and sets the subclass's.
pub fn set_up(inst: &mut FormatTestInstance) {
    LuceneTestCase::super_set_up();
    // set the default codec, so adding test cases to this isn't fragile
    inst.saved_codec = rucene::codecs::get_default();
    rucene::codecs::set_default(inst.case.get_codec());
}

/// `BaseIndexFileFormatTestCase.tearDown()`, which a subclass overriding
/// `tearDown()` calls as `super.tearDown()`: restores the default codec
/// `setUp()` saved, then `LuceneTestCase.tearDown()`.
pub fn tear_down(inst: &mut FormatTestInstance) {
    if let Some(saved) = inst.saved_codec.take() {
        rucene::codecs::set_default(saved); // restore
    }
    LuceneTestCase::super_tear_down();
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseIndexFileFormatTestCase`: the hooks a
/// subclass overrides (with Java's default bodies) and the tests.
pub trait BaseIndexFileFormatTestCase: Send + Sync + 'static {
    /// `getCodec()`: returns the codec to run tests against.
    fn get_codec(&self) -> Arc<dyn Codec>;

    /// `getCreatedVersionMajor()`: returns the major version that this codec
    /// is compatible with.
    fn get_created_version_major(&self) -> i32 {
        i32::from(Version::LATEST.major)
    }

    /// `addRandomFields(Document)`: add random fields to the provided
    /// document.
    fn add_random_fields(&self, doc: &mut Document);

    /// `excludedExtensionsFromByteCounts()`: return the list of extensions
    /// that should be excluded from byte counts when comparing indices that
    /// store the same content.
    fn excluded_extensions_from_byte_counts(&self) -> HashSet<String> {
        [
            // segment infos store various pieces of information that don't solely depend
            // on the content of the index in the diagnostics (such as a timestamp) so we
            // exclude this file from the bytes counts
            "si",
            // lock files are 0 bytes (one directory in the test could be RAMDir, the other FSDir)
            "lock",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// `mergeIsStable()`.
    fn merge_is_stable(&self) -> bool {
        true
    }

    /// `shouldTestMergeInstance()`: returns `false` if only the regular
    /// fields reader should be tested, and `true` if only the merge instance
    /// should be tested.
    fn should_test_merge_instance(&self) -> bool {
        false
    }

    /// The subclass as a `BasePostingsFormatTestCase`, when it is one.
    fn as_postings(
        &self,
    ) -> Option<&dyn crate::index::base_postings_format_test_case::BasePostingsFormatTestCase> {
        None
    }

    /// The subclass as a `BaseStoredFieldsFormatTestCase`, when it is one.
    fn as_stored_fields(
        &self,
    ) -> Option<
        &dyn crate::index::base_stored_fields_format_test_case::BaseStoredFieldsFormatTestCase,
    > {
        None
    }

    /// The subclass as a `BaseCompoundFormatTestCase`, when it is one.
    fn as_compound(
        &self,
    ) -> Option<&dyn crate::index::base_compound_format_test_case::BaseCompoundFormatTestCase> {
        None
    }

    /// The subclass as a `BaseNormsFormatTestCase`, when it is one.
    fn as_norms(
        &self,
    ) -> Option<&dyn crate::index::base_norms_format_test_case::BaseNormsFormatTestCase> {
        None
    }

    /// The subclass as a `BaseTermVectorsFormatTestCase`, when it is one.
    fn as_term_vectors(
        &self,
    ) -> Option<&dyn crate::index::base_term_vectors_format_test_case::BaseTermVectorsFormatTestCase>
    {
        None
    }

    /// The subclass as a `LegacyBaseDocValuesFormatTestCase` (and so a
    /// `BaseDocValuesFormatTestCase`), when it is one.
    fn as_doc_values(
        &self,
    ) -> Option<
        &dyn crate::index::legacy_base_doc_values_format_test_case::LegacyBaseDocValuesFormatTestCase,
    >{
        None
    }

    /// The subclass as a `BasePointsFormatTestCase`, when it is one.
    fn as_points(
        &self,
    ) -> Option<&dyn crate::index::base_points_format_test_case::BasePointsFormatTestCase> {
        None
    }

    /// The subclass as a `BaseKnnVectorsFormatTestCase`, when it is one.
    fn as_knn_vectors(
        &self,
    ) -> Option<&dyn crate::index::base_knn_vectors_format_test_case::BaseKnnVectorsFormatTestCase>
    {
        None
    }

    /// The subclass as a `BaseFieldInfoFormatTestCase`, when it is one.
    fn as_field_infos(
        &self,
    ) -> Option<&dyn crate::index::base_field_info_format_test_case::BaseFieldInfoFormatTestCase>
    {
        None
    }

    /// The subclass as a `BaseSegmentInfoFormatTestCase`, when it is one.
    fn as_segment_infos(
        &self,
    ) -> Option<&dyn crate::index::base_segment_info_format_test_case::BaseSegmentInfoFormatTestCase>
    {
        None
    }

    /// `applyCreatedVersionMajor(d)` (`final`): set the created version of
    /// the given directory (Java returns it).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the directory already has segments,
    /// and the errors of committing the empty `SegmentInfos`.
    fn apply_created_version_major(&self, d: &dyn Directory) -> Result<()> {
        if SegmentInfos::get_last_commit_generation_dir(d)? != -1 {
            return Err(LuceneError::illegal_argument(
                "Cannot set the created version on a Directory that already has segments"
                    .to_string(),
            ));
        }
        if self.get_created_version_major() != i32::from(Version::LATEST.major)
            || LuceneTestCase::random().next_boolean()
        {
            SegmentInfos::new(self.get_created_version_major())?.commit(d)?;
        }
        Ok(())
    }

    /// `bytesUsedByExtension(d)` (`private`).
    fn bytes_used_by_extension(&self, d: &dyn Directory) -> Result<HashMap<String, i64>> {
        let mut bytes_used_by_extension: HashMap<String, i64> = HashMap::new();
        for file in d.list_all()? {
            if index_file_names::is_codec_file(&file) {
                let ext = index_file_names::get_extension(&file)
                    .unwrap_or_default()
                    .to_string();
                let previous_length = bytes_used_by_extension.get(&ext).copied().unwrap_or(0);
                bytes_used_by_extension.insert(ext, previous_length + d.file_length(&file)?);
            }
        }
        for ext in self.excluded_extensions_from_byte_counts() {
            bytes_used_by_extension.remove(&ext);
        }
        Ok(bytes_used_by_extension)
    }

    /// `testMergeStability()`: the purpose of this test is to make sure that
    /// bulk merge doesn't accumulate useless data over runs.
    fn test_merge_stability(&self) -> Result<()> {
        LuceneTestCase::assume_true("merge is not stable", self.merge_is_stable());
        let dir = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*dir.as_directory())?;

        // do not use newMergePolicy that might return a MockMergePolicy that ignores the no-CFS
        // ratio do not use RIW which will change things up!
        let mp = LuceneTestCase::new_tiered_merge_policy();
        mp.set_no_cfs_ratio(0.0)?;
        let cfg = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        ))));
        cfg.set_use_compound_file(false)?;
        cfg.set_merge_policy(Some(Arc::new(mp)))?;
        if *VERBOSE {
            cfg.set_info_stream(Arc::new(rucene::util::PrintStreamInfoStream::new(
                Box::new(std::io::stdout()),
            )))?;
        }
        let w = IndexWriter::new(dir.as_directory(), cfg)?;
        let num_docs = LuceneTestCase::at_least(500);
        for _ in 0..num_docs {
            let mut d = Document::new();
            self.add_random_fields(&mut d);
            w.add_document(&d)?;
        }
        w.force_merge(1)?;
        w.commit()?;
        w.close()?;
        drop(w);
        let reader = directory_reader::open(dir.as_directory())?;

        let dir2 = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*dir2.as_directory())?;
        let mp = LuceneTestCase::new_tiered_merge_policy();
        mp.set_no_cfs_ratio(0.0)?;
        let cfg = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        ))));
        cfg.set_use_compound_file(false)?;
        cfg.set_merge_policy(Some(Arc::new(mp)))?;
        let w = IndexWriter::new(dir2.as_directory(), cfg)?;
        TestUtil::add_indexes_slowly(&w, std::slice::from_ref(&reader))?;

        w.commit()?;
        w.close()?;
        drop(w);

        Assert::assert_equals(
            JavaHashMap(self.bytes_used_by_extension(&*dir.as_directory())?),
            JavaHashMap(self.bytes_used_by_extension(&*dir2.as_directory())?),
        );

        reader.close()?;
        drop(reader);
        dir.close()?;
        dir2.close()?;
        Ok(())
    }

    /// `testMultiClose()`: calls close multiple times on closeable codec
    /// apis.
    fn test_multi_close(&self) -> Result<()> {
        // first make a one doc index
        let one_doc_index = LuceneTestCase::new_directory();
        self.apply_created_version_major(&*one_doc_index.as_directory())?;
        let iw = IndexWriter::new(
            one_doc_index.as_directory(),
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
                &*LuceneTestCase::random(),
            )))),
        )?;
        let mut one_doc = Document::new();
        let custom_type = FieldType::new_from(TextField::type_stored());
        custom_type.set_store_term_vectors(true)?;
        let custom_field = Field::new("field", "contents".to_string(), custom_type)?;
        let stored_value = custom_field.stored_value()?;
        one_doc.add(Box::new(custom_field));
        one_doc.add(Box::new(NumericDocValuesField::new("field", 5)));
        iw.add_document(&one_doc)?;
        let one_doc_dir_reader = directory_reader::open_with_writer(&*iw)?;
        let one_doc_reader =
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&one_doc_dir_reader) as _)?;
        iw.close()?;
        drop(iw);

        // now feed to codec apis manually
        // we use FSDir, things like ramdir are not guaranteed to cause fails if you write to them
        // after close(), etc
        let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "justSoYouGetSomeChannelErrors",
        ));
        let directory = dir.as_directory();
        let codec = self.get_codec();

        let segment_info = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            Sort::default(),
        )?;
        let field_infos_of_reader = one_doc_reader.get_field_infos();
        let proto = field_infos_of_reader
            .field_info("field")
            .ok_or_else(LuceneError::null_pointer)?;
        let field = FieldInfo::new_full(
            proto.name.clone(),
            proto.number,
            proto.has_term_vectors(),
            proto.omits_norms(),
            proto.has_payloads(),
            proto.get_index_options(),
            proto.get_doc_values_type(),
            proto.doc_values_skip_index_type(),
            proto.get_doc_values_gen(),
            Default::default(),
            proto.get_point_dimension_count(),
            proto.get_point_index_dimension_count(),
            proto.get_point_num_bytes(),
            proto.get_vector_dimension(),
            proto.get_vector_encoding(),
            proto.get_vector_similarity_function(),
            proto.is_soft_deletes_field(),
            proto.is_parent_field(),
        )?;

        let field_infos = FieldInfos::new(vec![field.clone()])?;

        let info_stream = NoOutputInfoStream;
        let seg_updates = BufferedUpdates::new("_0");
        let flush_context = flush_io_context(FlushInfo::new(1, 20));
        let write_state = rucene::codecs::state::SegmentWriteState::new(
            &info_stream,
            Arc::clone(&directory),
            &segment_info,
            &field_infos,
            &seg_updates,
            &*flush_context,
        );

        let read_state = rucene::codecs::state::SegmentReadState::new(
            &*directory,
            &segment_info,
            &field_infos,
            &*DEFAULT_IO_CONTEXT,
        );

        // PostingsFormat
        let fake_norms = OneDocNorms(Arc::clone(&one_doc_reader));
        {
            let mut consumer = codec.postings_format().fields_consumer(&write_state)?;
            let indexed_fields: BTreeSet<String> =
                FieldInfos::get_indexed_fields(Arc::clone(&one_doc_reader).as_index_reader())?
                    .into_iter()
                    .collect();
            let fields = OneDocFields {
                indexed_fields,
                reader: Arc::clone(&one_doc_reader),
            };
            consumer.write(&fields, Some(&fake_norms))?;
            consumer.close()?;
            consumer.close()?;
            consumer.close()?;
        }
        {
            let producer = codec.postings_format().fields_producer(&read_state)?;
            producer.close()?;
            producer.close()?;
            producer.close()?;
        }

        // DocValuesFormat
        {
            let mut consumer = codec.doc_values_format().fields_consumer(&write_state)?;
            consumer.add_numeric_field(&field, &OneValueProducer)?;
            consumer.close()?;
            consumer.close()?;
            consumer.close()?;
        }
        {
            let producer = codec.doc_values_format().fields_producer(&read_state)?;
            producer.close()?;
            producer.close()?;
            producer.close()?;
        }

        // NormsFormat
        {
            let mut consumer = codec.norms_format().norms_consumer(&write_state)?;
            consumer.add_norms_field(&field, &OneValueNorms)?;
            consumer.close()?;
            consumer.close()?;
            consumer.close()?;
        }
        {
            let producer = codec.norms_format().norms_producer(&read_state)?;
            producer.close()?;
            producer.close()?;
            producer.close()?;
        }

        // TermVectorsFormat
        {
            let mut consumer = codec.term_vectors_format().vectors_writer(
                &directory,
                &segment_info,
                write_state.context,
            )?;
            consumer.start_document(1)?;
            consumer.start_field(&field, 1, false, false, false)?;
            consumer.start_term(&BytesRef::from_char_sequence("testing"), 2)?;
            consumer.finish_term()?;
            consumer.finish_field()?;
            consumer.finish_document()?;
            consumer.finish(1)?;
            consumer.close()?;
            consumer.close()?;
            consumer.close()?;
        }
        {
            let mut producer = codec.term_vectors_format().vectors_reader(
                &*directory,
                &segment_info,
                &field_infos,
                read_state.context,
            )?;
            producer.close()?;
            producer.close()?;
            producer.close()?;
        }

        // StoredFieldsFormat
        {
            let mut consumer = codec.stored_fields_format().fields_writer(
                &directory,
                &segment_info,
                write_state.context,
            )?;
            consumer.start_document()?;
            match stored_value {
                Some(StoredValue::Integer(v)) => consumer.write_field_i32(&field, v)?,
                Some(StoredValue::Long(v)) => consumer.write_field_i64(&field, v)?,
                Some(StoredValue::Float(v)) => consumer.write_field_f32(&field, v)?,
                Some(StoredValue::Double(v)) => consumer.write_field_f64(&field, v)?,
                Some(StoredValue::Binary(v)) => {
                    consumer.write_field_bytes(&field, &v.bytes[v.offset..v.offset + v.length])?
                }
                Some(StoredValue::String(v)) => consumer.write_field_string(&field, &v)?,
                _ => randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.AssertionError",
                ),
            }
            consumer.finish_document()?;
            consumer.finish(1)?;
            consumer.close()?;
            consumer.close()?;
            consumer.close()?;
        }
        {
            let mut producer = codec.stored_fields_format().fields_reader(
                &*directory,
                &segment_info,
                &field_infos,
                read_state.context,
            )?;
            producer.close()?;
            producer.close()?;
            producer.close()?;
        }

        drop(write_state);
        drop(read_state);
        drop(segment_info);
        drop(directory);
        // `IOUtils.close(oneDocReader, oneDocIndex, dir)`
        drop(fake_norms);
        Arc::clone(&one_doc_reader).as_index_reader().close()?;
        drop(one_doc_reader);
        drop(one_doc_dir_reader);
        one_doc_index.close()?;
        dir.close()?;
        Ok(())
    }

    /// `testRandomExceptions()`: tests exception handling on write and
    /// openInput/createOutput.
    // TODO: this is really not ideal. each BaseXXXTestCase should have unit tests doing this.
    // but we use this shotgun approach to prevent bugs in the meantime: it just ensures the
    // codec does not corrupt the index or leak file handles.
    fn test_random_exceptions(&self) -> Result<()> {
        // disable slow things: we don't rely upon sleeps here.
        let dir = LuceneTestCase::new_mock_directory();
        self.apply_created_version_major(&*dir.as_directory())?;
        dir.set_throttling(Throttling::Never);
        dir.set_use_slow_open_closers(false);
        dir.set_random_io_exception_rate(0.001); // more rare

        // log all exceptions we hit, in case we fail (for debugging)
        let mut exception_log = String::new();

        let analyzer: Arc<dyn rucene::analysis::Analyzer> =
            Arc::new(MockAnalyzer::new(&*LuceneTestCase::random()));

        let new_conf =
            |analyzer: &Arc<dyn rucene::analysis::Analyzer>| -> Result<IndexWriterConfig> {
                let conf =
                    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(analyzer)));
                // just for now, try to keep this test reproducible
                conf.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
                conf.set_codec(self.get_codec())?;
                Ok(conf)
            };
        let conf = new_conf(&analyzer)?;

        let num_docs = LuceneTestCase::at_least(500);

        let mut iw = IndexWriter::new(dir.as_directory(), conf)?;
        let outcome = (|| -> Result<()> {
            let mut allow_already_closed = false;
            for i in 0..num_docs {
                dir.set_random_io_exception_rate_on_open(0.02); // turn on exceptions for openInput/createOutput

                let mut doc = Document::new();
                doc.add(Box::new(LuceneTestCase::new_string_field(
                    "id",
                    &i.to_string(),
                    rucene::document::Store::NO,
                )));
                self.add_random_fields(&mut doc);

                // single doc
                let added = iw.add_document(&doc).and_then(|_| {
                    // we made it, sometimes delete our doc
                    iw.delete_documents_by_terms(vec![rucene::index::terms::Term::new(
                        "id",
                        BytesRef::from_char_sequence(&i.to_string()),
                    )])
                    .map(|_| ())
                });
                match added {
                    Ok(()) => {}
                    Err(LuceneError {
                        kind: LuceneErrorKind::AlreadyClosed { .. },
                        ..
                    }) => {
                        // OK: writer was closed by abort; we just reopen now:
                        dir.set_random_io_exception_rate_on_open(0.0); // disable exceptions on openInput until next iteration
                        Assert::assert_true(is_deleter_closed(&iw));
                        Assert::assert_true(allow_already_closed);
                        allow_already_closed = false;
                        let conf = new_conf(&analyzer)?;
                        iw = IndexWriter::new(dir.as_directory(), conf)?;
                    }
                    Err(e) if e.is_io_exception() => {
                        handle_fake_io_exception(e, &mut exception_log);
                        allow_already_closed = true;
                    }
                    Err(e) => return Err(e),
                }

                if LuceneTestCase::random().next_int_bound(10) == 0 {
                    // trigger flush:
                    let flushed = (|| -> Result<()> {
                        if LuceneTestCase::random().next_boolean() {
                            let ir = directory_reader::open_with_writer_flags(
                                &*iw,
                                LuceneTestCase::random().next_boolean(),
                                false,
                            )?;
                            dir.set_random_io_exception_rate_on_open(0.0); // disable exceptions on openInput until next iteration
                            let checked = TestUtil::check_reader(&(Arc::clone(&ir) as _));
                            // `IOUtils.closeWhileHandlingException(ir)`
                            let _ = ir.close();
                            checked?;
                        } else {
                            dir.set_random_io_exception_rate_on_open(0.0); // disable exceptions on openInput until next iteration:
                                                                           // or we make slowExists angry and trip a scarier assert!
                            iw.commit()?;
                        }
                        if directory_reader::index_exists(&*dir.as_directory())? {
                            TestUtil::check_index(dir.as_directory())?;
                        }
                        Ok(())
                    })();
                    match flushed {
                        Ok(()) => {}
                        Err(LuceneError {
                            kind: LuceneErrorKind::AlreadyClosed { .. },
                            ..
                        }) => {
                            // OK: writer was closed by abort; we just reopen now:
                            dir.set_random_io_exception_rate_on_open(0.0); // disable exceptions on openInput until next iteration
                            Assert::assert_true(is_deleter_closed(&iw));
                            Assert::assert_true(allow_already_closed);
                            allow_already_closed = false;
                            let conf = new_conf(&analyzer)?;
                            iw = IndexWriter::new(dir.as_directory(), conf)?;
                        }
                        Err(e) if e.is_io_exception() => {
                            handle_fake_io_exception(e, &mut exception_log);
                            allow_already_closed = true;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            dir.set_random_io_exception_rate_on_open(0.0); // disable exceptions on openInput until next iteration:
                                                           // or we make slowExists angry and trip a scarier assert!
            if let Err(e) = iw.close() {
                if !e.is_io_exception() {
                    return Err(e);
                }
                handle_fake_io_exception(e, &mut exception_log);
                let _ = iw.rollback();
            }
            Ok(())
        })();
        let outcome = outcome.and_then(|()| {
            drop(iw);
            dir.close()
        });
        if let Err(t) = outcome {
            println!("Unexpected exception: dumping fake-exception-log:...");
            println!("{exception_log}");
            return Err(t);
        }

        if *VERBOSE {
            println!("TEST PASSED: dumping fake-exception-log:...");
            println!("{exception_log}");
        }
        Ok(())
    }

    /// `maybeWrapWithMergingReader(r)` (`final`).
    ///
    /// # Errors
    ///
    /// The errors of wrapping the reader.
    fn maybe_wrap_with_merging_reader(
        &self,
        r: Arc<dyn DirectoryReader>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        if self.should_test_merge_instance() {
            return Ok(Arc::new(MergingDirectoryReaderWrapper::new(r)?));
        }
        Ok(r)
    }

    /// `testCheckIntegrityReadsAllBytes()`: this test is the best effort at
    /// verifying that checkIntegrity doesn't miss any files. It tests that
    /// the combination of opening a reader and calling checkIntegrity on it
    /// reads all bytes of all files.
    fn test_check_integrity_reads_all_bytes(&self) -> Result<()> {
        LuceneTestCase::assume_false(
            "SimpleText doesn't store checksums of its files",
            self.get_codec().name() == "SimpleText",
        );
        let inner = LuceneTestCase::new_directory();
        let dir = Arc::new(FileTrackingDirectoryWrapper::new(inner.as_directory()));
        self.apply_created_version_major(&*dir)?;

        let cfg = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
            &*LuceneTestCase::random(),
        ))));
        let w = IndexWriter::new(Arc::clone(&dir) as Arc<dyn Directory>, cfg)?;
        let num_docs = LuceneTestCase::at_least(100);
        for _ in 0..num_docs {
            let mut d = Document::new();
            self.add_random_fields(&mut d);
            w.add_document(&d)?;
        }
        w.force_merge(1)?;
        w.commit()?;
        w.close()?;
        drop(w);

        let read_bytes_wrapper_dir = Arc::new(ReadBytesDirectoryWrapper::new(
            Arc::clone(&dir) as Arc<dyn Directory>
        ));
        let reader = directory_reader::open(Arc::clone(&read_bytes_wrapper_dir) as _)?;
        let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
        leaf_reader.check_integrity()?;

        let read_bytes_map = read_bytes_wrapper_dir.get_read_bytes();

        let mut unread_files: HashSet<String> = dir.get_files();
        println!("{}", java_array_to_string(&dir.list_all()?));
        for name in read_bytes_map.keys() {
            unread_files.remove(name);
        }
        unread_files.remove(rucene::index::index_file_deleter::WRITE_LOCK_NAME);
        Assert::assert_true_msg(
            &format!(
                "Some files have not been open: {}",
                java_set_to_string(&unread_files)
            ),
            unread_files.is_empty(),
        );

        let mut messages: Vec<String> = Vec::new();
        for (name, bits) in &read_bytes_map {
            let mut unread_bytes = bits.clone();
            let len = unread_bytes.length();
            unread_bytes.flip_range(0, len);
            let unread = unread_bytes.next_set_bit(0);
            if unread != i32::MAX {
                messages.push(format!(
                    "Offset {unread} of file {name} ({} bytes) was not read.",
                    unread_bytes.length()
                ));
            }
        }
        Assert::assert_true_msg(&messages.join("\n"), messages.is_empty());
        drop(leaf_reader);
        reader.close()?;
        drop(reader);
        drop(read_bytes_wrapper_dir);
        drop(dir);
        inner.close()?;
        Ok(())
    }
}

/// `handleFakeIOException(e, exceptionStream)` (`private`): logs an
/// `IOException` whose message (or a cause's) starts with `a random
/// IOException`, and rethrows any other.
fn handle_fake_io_exception(e: LuceneError, exception_stream: &mut String) {
    let mut ex: Option<&(dyn std::error::Error + 'static)> = Some(&e);
    while let Some(t) = ex {
        let message = match t.downcast_ref::<LuceneError>() {
            Some(l) => {
                Some(crate::analysis::base_token_stream_test_case::java_class_and_message(l).1)
            }
            None => t
                .downcast_ref::<std::io::Error>()
                .and_then(rucene::store::java_nio_file::java_message_of),
        };
        if let Some(message) = message {
            if message.starts_with("a random IOException") {
                exception_stream.push_str(&format!("\nTEST: got expected fake exc:{message}\n"));
                exception_stream.push_str(&format!("{e:?}\n"));
                return;
            }
        }
        ex = t.source();
    }

    rethrow(e)
}

/// `expectThrows(expectedType, runnable)` over a Rucene result: the error
/// the runnable returns is the thrown exception, an instance of
/// `expected_type` when `is_instance` says so. Returns it; panics with
/// JUnit's `AssertionFailedError` when nothing is thrown or the error is of
/// another class, as `LuceneTestCase.expectThrows` does.
///
/// Java's `expectThrows` catches every `Throwable` the runnable throws,
/// the `AssertionError` of a Java `assert` included
/// (`LuceneTestCase._expectThrows`, `LuceneTestCase.java:3080-3096`); the port's `assert`s panic, so a panic
/// of the runnable is caught too and checked the same way (an instance of
/// `expected_type` is returned as an unchecked error of its class).
pub fn expect_throws_lucene<T>(
    expected_type: &str,
    is_instance: impl Fn(&LuceneError) -> bool,
    runnable: impl FnOnce() -> Result<T>,
) -> LuceneError {
    let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(runnable)) {
        Ok(outcome) => outcome,
        Err(payload) => {
            let t = Throwable::from_panic(payload);
            if t.is_instance(expected_type) {
                return LuceneError::unchecked(
                    interned_class_name(t.class_name()),
                    t.message().unwrap_or_default().to_string(),
                );
            }
            LuceneTestCase::expect_throws(expected_type, || Err(t));
            unreachable!("expectThrows throws for an exception of another class")
        }
    };
    match outcome {
        Err(e) if is_instance(&e) => e,
        Err(e) => {
            let (class, message) =
                crate::analysis::base_token_stream_test_case::java_class_and_message(&e);
            LuceneTestCase::expect_throws(expected_type, || Err(Throwable::new(class, message)));
            unreachable!("expectThrows throws for an exception of another class")
        }
        Ok(_) => {
            LuceneTestCase::expect_throws(expected_type, || Ok(()));
            unreachable!("expectThrows throws when nothing is thrown")
        }
    }
}

/// The `'static` name [`LuceneError::unchecked`] needs for a Java class
/// name: each distinct name is leaked once.
fn interned_class_name(name: &str) -> &'static str {
    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(n) = names.iter().find(|n| **n == name) {
        return n;
    }
    let n: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.push(n);
    n
}

/// `assertArrayEquals(expecteds, actuals)` over two arrays of primitives
/// (JUnit 4.13's `ComparisonCriteria.arrayEquals`): an
/// `ArrayComparisonFailure` naming the first differing index, after `array
/// lengths differed, …` when the lengths differ.
pub fn assert_array_equals<T: PartialEq + fmt::Display>(expecteds: &[T], actuals: &[T]) {
    assert_array_equals_msg(None, expecteds, actuals)
}

/// `assertArrayEquals(message, expecteds, actuals)` over two arrays of
/// primitives; see [`assert_array_equals`].
pub fn assert_array_equals_msg<T: PartialEq + fmt::Display>(
    message: Option<&str>,
    expecteds: &[T],
    actuals: &[T],
) {
    if expecteds == actuals {
        return;
    }
    let mut header = message.map_or(String::new(), |m| format!("{m}: "));
    if expecteds.len() != actuals.len() {
        header.push_str(&format!(
            "array lengths differed, expected.length={} actual.length={}",
            expecteds.len(),
            actuals.len()
        ));
    }
    let len = expecteds.len().max(actuals.len());
    for i in 0..len {
        let (e, a) = (expecteds.get(i), actuals.get(i));
        if e != a {
            let render = |v: Option<&T>| v.map_or("end of array".to_string(), |v| v.to_string());
            let separator = if header.is_empty() || header.ends_with(' ') {
                ""
            } else {
                "; "
            };
            Throwable::new(
                "org.junit.internal.ArrayComparisonFailure",
                format!(
                    "{header}{separator}arrays first differed at element [{i}]; expected:<{}> but was:<{}>",
                    render(e),
                    render(a)
                ),
            )
            .throw()
        }
    }
}

/// Whether Java's `catch (UnsupportedOperationException e)` catches the error.
pub fn is_unsupported_operation(e: &LuceneError) -> bool {
    match e {
        LuceneError {
            kind: LuceneErrorKind::UnsupportedOperation { .. },
            ..
        } => true,
        LuceneError {
            kind: LuceneErrorKind::Unchecked { class, .. },
            ..
        } => *class == "java.lang.UnsupportedOperationException",
        _ => false,
    }
}

/// Whether the error stands for an instance of the Java class `class`
/// (`class.isInstance(e)`), by the Java class
/// `java_class_and_message` gives it.
pub fn is_instance_of_java_class(e: &LuceneError, class: &str) -> bool {
    let (actual, _) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    actual == class
        || class.rsplit('.').next() == Some(actual.as_str())
        || Throwable::new(actual, "").is_instance(class)
}

/// Whether Java's `catch (CorruptIndexException e)` catches the error.
pub fn is_corrupt_index(e: &LuceneError) -> bool {
    matches!(
        e,
        LuceneError {
            kind: LuceneErrorKind::CorruptIndex(_),
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexFormatTooOld { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexFormatTooNew { .. },
            ..
        }
    )
}

/// The message of an exception, as `getMessage()` returns it.
pub fn exception_message(e: &LuceneError) -> String {
    crate::analysis::base_token_stream_test_case::java_class_and_message(e).1
}

/// A `Set<String>` as JUnit's `assertEquals` compares and prints it
/// (`AbstractCollection.toString()` of a `HashSet`, in the JDK's iteration
/// order).
#[derive(PartialEq)]
pub struct JavaStringSet(pub HashSet<String>);

impl fmt::Display for JavaStringSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_set_to_string(&self.0))
    }
}

/// `Arrays.toString(String[])`.
fn java_array_to_string(values: &[String]) -> String {
    format!("[{}]", values.join(", "))
}

/// `Set<String>.toString()` of a `HashSet`, in the JDK's iteration order.
fn java_set_to_string(values: &HashSet<String>) -> String {
    let keys: Vec<(String, i32)> = values
        .iter()
        .map(|k| (k.clone(), randomizedtesting::jdk::lang::string_hash_code(k)))
        .collect();
    let order = randomizedtesting::jdk::util::hash_iteration_order(&keys, |a, b| a == b);
    let items: Vec<&str> = order.iter().map(|&i| keys[i].0.as_str()).collect();
    format!("[{}]", items.join(", "))
}

/// A `HashMap<String, Long>` as JUnit's `assertEquals` compares and prints
/// it (`AbstractMap.toString()`, in the JDK's iteration order).
#[derive(PartialEq)]
struct JavaHashMap(HashMap<String, i64>);

impl fmt::Display for JavaHashMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let keys: Vec<(String, i32)> = self
            .0
            .keys()
            .map(|k| (k.clone(), randomizedtesting::jdk::lang::string_hash_code(k)))
            .collect();
        let order = randomizedtesting::jdk::util::hash_iteration_order(&keys, |a, b| a == b);
        let items: Vec<String> = order
            .iter()
            .map(|&i| format!("{}={}", keys[i].0, self.0[&keys[i].0]))
            .collect();
        write!(f, "{{{}}}", items.join(", "))
    }
}

// -----------------------------------------------------------------------------
// testMultiClose's anonymous classes
// -----------------------------------------------------------------------------

/// The `fakeNorms` of `testMultiClose`: the norms of the one-document
/// reader, for a field that has them.
#[derive(Debug)]
struct OneDocNorms(Arc<dyn LeafReader>);

impl NormsProducer for OneDocNorms {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        if !field.has_norms() {
            return Ok(None);
        }
        self.0.get_norm_values(&field.name)
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(OneDocNorms(Arc::clone(&self.0))))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// The anonymous `Fields` of `testMultiClose`: the indexed fields of the
/// one-document reader, sorted.
struct OneDocFields {
    indexed_fields: BTreeSet<String>,
    reader: Arc<dyn LeafReader>,
}

impl Fields for OneDocFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.indexed_fields.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.reader.terms(field)
    }

    fn size(&self) -> i32 {
        self.indexed_fields.len() as i32
    }
}

/// The anonymous `NumericDocValues` of `testMultiClose`: document 0 has the
/// value 5.
struct OneValue {
    doc_id: i32,
}

impl fmt::Display for OneValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NumericDocValues")
    }
}

impl DocIdSetIterator for OneValue {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == 1 {
            self.doc_id = NO_MORE_DOCS;
        }
        Ok(self.doc_id)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.doc_id <= 0 && target == 0 {
            self.doc_id = 0;
        } else {
            self.doc_id = NO_MORE_DOCS;
        }
        Ok(self.doc_id)
    }

    fn cost(&self) -> i64 {
        1
    }
}

impl DocValuesIterator for OneValue {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc_id = target;
        Ok(target == 0)
    }
}

impl NumericDocValues for OneValue {
    fn long_value(&self) -> Result<i64> {
        Ok(5)
    }
}

/// The anonymous `EmptyDocValuesProducer` of `testMultiClose`, whose
/// `getNumeric` serves [`OneValue`].
#[derive(Debug)]
struct OneValueProducer;

impl DocValuesProducer for OneValueProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(Some(Box::new(OneValue { doc_id: -1 })))
    }

    fn get_binary(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn rucene::index::doc_values::BinaryDocValues>>> {
        rucene::index::doc_values::EmptyDocValuesProducer.get_binary(field)
    }

    fn get_sorted(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn rucene::index::doc_values::SortedDocValues>>> {
        rucene::index::doc_values::EmptyDocValuesProducer.get_sorted(field)
    }

    fn get_sorted_numeric(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn rucene::index::doc_values::SortedNumericDocValues>>> {
        rucene::index::doc_values::EmptyDocValuesProducer.get_sorted_numeric(field)
    }

    fn get_sorted_set(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn rucene::index::doc_values::SortedSetDocValues>>> {
        rucene::index::doc_values::EmptyDocValuesProducer.get_sorted_set(field)
    }

    fn get_skipper(
        &self,
        field: &FieldInfo,
    ) -> Result<Option<Box<dyn rucene::index::doc_values::DocValuesSkipper>>> {
        rucene::index::doc_values::EmptyDocValuesProducer.get_skipper(field)
    }

    fn check_integrity(&self) -> Result<()> {
        rucene::index::doc_values::EmptyDocValuesProducer.check_integrity()
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(OneValueProducer))
    }

    fn close(&self) -> Result<()> {
        rucene::index::doc_values::EmptyDocValuesProducer.close()
    }
}

/// The anonymous `NormsProducer` of `testMultiClose` the norms consumer
/// receives: [`OneValue`] for every field.
#[derive(Debug)]
struct OneValueNorms;

impl NormsProducer for OneValueNorms {
    fn get_norms(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(Some(Box::new(OneValue { doc_id: -1 })))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(OneValueNorms))
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// FileTrackingDirectoryWrapper
// -----------------------------------------------------------------------------

/// Port of `BaseIndexFileFormatTestCase.FileTrackingDirectoryWrapper`
/// (`extends FilterDirectory`): a directory that tracks created files that
/// haven't been deleted.
pub struct FileTrackingDirectoryWrapper {
    r#in: Arc<dyn Directory>,
    files: Mutex<HashSet<String>>,
}

impl FileTrackingDirectoryWrapper {
    /// Sole constructor.
    pub fn new(r#in: Arc<dyn Directory>) -> Self {
        FileTrackingDirectoryWrapper {
            r#in,
            files: Mutex::new(HashSet::new()),
        }
    }

    /// `getFiles()`: get the set of created files.
    pub fn get_files(&self) -> HashSet<String> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Directory for FileTrackingDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
        self.r#in.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_string());
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
        {
            let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
            files.remove(source);
            files.insert(dest.to_string());
        }
        self.r#in.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.r#in.open_input(name, context)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.r#in.open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }

    /// `FilterDirectory.close()` closes the delegate; the delegate here is
    /// the shared test directory, which its owner closes.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(&*self.r#in)
    }
}

impl fmt::Display for FileTrackingDirectoryWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FileTrackingDirectoryWrapper({})",
            self.r#in.directory_type_name()
        )
    }
}

// -----------------------------------------------------------------------------
// ReadBytesIndexInputWrapper and ReadBytesDirectoryWrapper
// -----------------------------------------------------------------------------

/// The `IntConsumer readByte` of a [`ReadBytesIndexInputWrapper`]: marks a
/// byte of the file as read.
type ReadByte = Arc<dyn Fn(i32) + Send + Sync>;

/// `Math.toIntExact(v)`.
fn to_int_exact(v: i64) -> Result<i32> {
    i32::try_from(v)
        .map_err(|_| LuceneError::from(LuceneErrorKind::Arithmetic("integer overflow".to_string())))
}

/// Port of `BaseIndexFileFormatTestCase.ReadBytesIndexInputWrapper`
/// (`extends FilterIndexInput`): reports every byte read to `readByte`.
struct ReadBytesIndexInputWrapper {
    resource_description: String,
    r#in: Box<dyn IndexInput>,
    read_byte: ReadByte,
}

impl ReadBytesIndexInputWrapper {
    fn new(r#in: Box<dyn IndexInput>, read_byte: ReadByte) -> Self {
        ReadBytesIndexInputWrapper {
            resource_description: r#in.to_java_string(),
            r#in,
            read_byte,
        }
    }
}

impl DataInput for ReadBytesIndexInputWrapper {
    fn read_byte(&mut self) -> Result<u8> {
        (self.read_byte)(to_int_exact(self.file_pointer())?);
        self.r#in.read_byte()
    }

    fn to_java_string(&self) -> String {
        self.resource_description.clone()
    }

    fn read_bytes(&mut self, b: Option<&mut [u8]>, offset: usize, len: usize) -> Result<()> {
        let fp = to_int_exact(self.file_pointer())?;
        for i in 0..len as i32 {
            (self.read_byte)(fp.checked_add(i).ok_or_else(|| {
                LuceneError::from(LuceneErrorKind::Arithmetic("integer overflow".to_string()))
            })?);
        }
        self.r#in.read_bytes(b, offset, len)
    }

    /// `IndexInput.skipBytes`: `seek(getFilePointer() + numBytes)`.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "numBytes must be >= 0, got {num_bytes}"
            )));
        }
        let skip_to = self.file_pointer() + num_bytes;
        self.seek(skip_to)
    }
}

impl IndexInput for ReadBytesIndexInputWrapper {
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
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        let slice = self.r#in.slice(slice_description, offset, length)?;
        let read_byte = Arc::clone(&self.read_byte);
        Ok(Box::new(ReadBytesIndexInputWrapper::new(
            slice,
            Arc::new(move |o| {
                // `Math.toIntExact(offset + o)`
                let at = i32::try_from(offset + i64::from(o)).unwrap_or_else(|_| {
                    Throwable::new("java.lang.ArithmeticException", "integer overflow").throw()
                });
                read_byte(at)
            }),
        )))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(ReadBytesIndexInputWrapper::new(
            self.r#in.clone_input()?,
            Arc::clone(&self.read_byte),
        )))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

/// Port of `BaseIndexFileFormatTestCase.ReadBytesDirectoryWrapper`
/// (`extends FilterDirectory`): a directory that tracks read bytes.
pub struct ReadBytesDirectoryWrapper {
    r#in: Arc<dyn Directory>,
    read_bytes: Mutex<HashMap<String, Arc<Mutex<FixedBitSet>>>>,
}

impl ReadBytesDirectoryWrapper {
    /// Sole constructor.
    pub fn new(r#in: Arc<dyn Directory>) -> Self {
        ReadBytesDirectoryWrapper {
            r#in,
            read_bytes: Mutex::new(HashMap::new()),
        }
    }

    /// `getReadBytes()`: get information about which bytes have been read.
    pub fn get_read_bytes(&self) -> HashMap<String, FixedBitSet> {
        self.read_bytes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.lock().unwrap_or_else(PoisonError::into_inner).clone(),
                )
            })
            .collect()
    }

    /// The bit set of `name`, created for a file of `length` bytes
    /// (`readBytes.computeIfAbsent(name, n -> new FixedBitSet(toIntExact(in.length())))`),
    /// checked against `length`.
    fn bits_of(&self, name: &str, length: i64) -> Result<Arc<Mutex<FixedBitSet>>> {
        let set = {
            let mut map = self
                .read_bytes
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match map.get(name) {
                Some(set) => Arc::clone(set),
                None => {
                    let set =
                        Arc::new(Mutex::new(FixedBitSet::new(to_int_exact(length)? as usize)));
                    map.insert(name.to_string(), Arc::clone(&set));
                    set
                }
            }
        };
        if set.lock().unwrap_or_else(PoisonError::into_inner).length() as i64 != length {
            return Err(LuceneError::illegal_state(String::new()));
        }
        Ok(set)
    }
}

impl Directory for ReadBytesDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.r#in.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.r#in.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.r#in.file_length(name)
    }

    fn create_output(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn create_temp_output(
        &self,
        _prefix: &str,
        _suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::unsupported_operation(String::new()))
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

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let r#in = self.r#in.open_input(name, context)?;
        let set = self.bits_of(name, r#in.length())?;
        Ok(Box::new(ReadBytesIndexInputWrapper::new(
            r#in,
            Arc::new(move |i| {
                set.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set(i as usize)
            }),
        )))
    }

    /// See the module docs: the checksum input reads through the tracked
    /// input of [`Self::open_input`].
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*READONCE_IO_CONTEXT)?,
        )))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.r#in.obtain_lock(name)
    }

    /// `FilterDirectory.close()` closes the delegate; the delegate here is
    /// shared, and its owner closes it.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.r#in.get_pending_deletions()
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        Some(&*self.r#in)
    }
}

/// JUnit's `assertNotNull(object)`: an `AssertionError` without a message
/// when the object is `null` (`present` is false).
pub fn assert_not_null(present: bool) {
    if !present {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.AssertionError")
    }
}

/// JUnit's `assertNotNull(message, object)`.
pub fn assert_not_null_msg(message: &str, present: bool) {
    if !present {
        Throwable::new("java.lang.AssertionError", message).throw()
    }
}

/// JUnit's `assertNull(object)`: `expected null, but was:<object>` when the
/// object (rendered by its `toString()`) is not `null`.
pub fn assert_null(actual: Option<String>) {
    if let Some(actual) = actual {
        Throwable::new(
            "java.lang.AssertionError",
            format!("expected null, but was:<{actual}>"),
        )
        .throw()
    }
}

/// JUnit's `assertEquals(expected, actual)` over two objects whose
/// `toString()` the port renders with `Debug`: `expected:<…> but was:<…>`.
pub fn assert_equals_debug<T: PartialEq + fmt::Debug>(expected: &T, actual: &T) {
    if expected != actual {
        Throwable::new(
            "java.lang.AssertionError",
            format!("expected:<{expected:?}> but was:<{actual:?}>"),
        )
        .throw()
    }
}
