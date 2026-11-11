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

//! Port of `org.apache.lucene.tests.index.BaseFieldInfoFormatTestCase`:
//! abstract class to do basic tests for fis format. NOTE: This test focuses
//! on the fis impl, nothing else. The [stretch] goal is for this test to be
//! so thorough in testing a new fis format that if this test passes, then
//! all Lucene tests should also pass. Ie, if there is some bug in a given fis
//! Format that this test fails to catch then this test needs to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseFieldInfoFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testImmutableAttributes`: Java expects `UnsupportedOperationException`
//!   from `attributes.put("bogus", "bogus")` on the map `FieldInfo.attributes()`
//!   returns. The port's `FieldInfo::attributes` returns a
//!   `JavaStringMap`, which has no mutator at all, so the write cannot even
//!   be expressed; the test checks instead that the attributes read back are
//!   the ones written, which is what the unmodifiable map protects.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::codecs::Codec;
use rucene::document::{Document, FieldType, StoredField, TextField};
use rucene::error::Result;
use rucene::index::field_infos::{FieldInfo, FieldInfos};
use rucene::index::point_values::{MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES};
use rucene::index::segment_info::SegmentInfo;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexableFieldType, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::internal::tests_hooks::{FieldInfosBuilder, TestSecrets};
use rucene::search::Sort;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::java_collections::JavaStringMap;
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;

use super::base_index_file_format_test_case::{
    assert_not_null, expect_throws_lucene, format_test, is_instance_of_java_class, rethrow,
    BaseIndexFileFormatTestCase,
};
use crate::store::mock_directory_wrapper::{fake_io_exception, Failure, FAKE_IO_EXCEPTION};
use crate::store::MockDirectoryWrapper;
use crate::util::lucene_test_case::LuceneTestCase;
use crate::util::TestUtil;

/// `IndexOptions.values()`.
pub const INDEX_OPTIONS_VALUES: [IndexOptions; 6] = [
    IndexOptions::NONE,
    IndexOptions::DOCS,
    IndexOptions::DOCS_AND_FREQS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
    IndexOptions::DOCS_AND_CUSTOM_FREQS,
];

/// `DocValuesType.values()`.
pub const DOC_VALUES_TYPE_VALUES: [DocValuesType; 6] = [
    DocValuesType::NONE,
    DocValuesType::NUMERIC,
    DocValuesType::BINARY,
    DocValuesType::SORTED,
    DocValuesType::SORTED_NUMERIC,
    DocValuesType::SORTED_SET,
];

/// `VectorSimilarityFunction.values()`.
pub const VECTOR_SIMILARITY_FUNCTION_VALUES: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// `VectorEncoding.values()`.
pub const VECTOR_ENCODING_VALUES: [VectorEncoding; 2] =
    [VectorEncoding::BYTE, VectorEncoding::FLOAT32];

/// The declaration of `org.apache.lucene.tests.index.BaseFieldInfoFormatTestCase`
/// (`public abstract class BaseFieldInfoFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseFieldInfoFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(field_infos_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseFieldInfoFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testOneField", |c| c.test_one_field()))
            .method(t("testImmutableAttributes", |c| {
                c.test_immutable_attributes()
            }))
            .method(t("testExceptionOnCreateOutput", |c| {
                c.test_exception_on_create_output()
            }))
            .method(t("testExceptionOnCloseOutput", |c| {
                c.test_exception_on_close_output()
            }))
            .method(t("testExceptionOnOpenInput", |c| {
                c.test_exception_on_open_input()
            }))
            .method(t("testExceptionOnCloseInput", |c| {
                c.test_exception_on_close_input()
            }))
            .method(t("testRandom", |c| c.test_random()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseFieldInfoFormatTestCase`.
fn field_infos_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseFieldInfoFormatTestCase {
    c.as_field_infos().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseFieldInfoFormatTestCase",
        )
        .throw()
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// The anonymous `Failure` of the exception tests: throws a
/// `FakeIOException` when `doFail` is set and a method named `method` is on
/// the call stack (`callStackContainsAnyOf(method)`). The port's frames are
/// Rust method names: `createOutput` is `create_output`, `openInput` is
/// `open_input`, `close` is `close`.
pub struct CallStackFailure {
    method: &'static str,
    do_fail: AtomicBool,
}

impl CallStackFailure {
    /// The failure on `method`.
    pub fn new(method: &'static str) -> Arc<Self> {
        Arc::new(Self {
            method,
            do_fail: AtomicBool::new(false),
        })
    }
}

impl Failure for CallStackFailure {
    fn eval(&self, _dir: &MockDirectoryWrapper) -> Result<()> {
        if self.do_fail() && LuceneTestCase::call_stack_contains_any_of(&[self.method]) {
            return Err(fake_io_exception());
        }
        Ok(())
    }

    fn do_fail_flag(&self) -> &AtomicBool {
        &self.do_fail
    }
}

/// `expectThrows(FakeIOException.class, runnable)`.
pub fn expect_fake_io_exception<T>(runnable: impl FnOnce() -> Result<T>) {
    expect_throws_lucene(
        FAKE_IO_EXCEPTION,
        |e| is_instance_of_java_class(e, FAKE_IO_EXCEPTION),
        runnable,
    );
}

/// `INDEX_PACKAGE_ACCESS.newFieldInfosBuilder(softDeletesField, parentField)`,
/// `INDEX_PACKAGE_ACCESS = TestSecrets.getIndexPackageAccess()`.
fn new_field_infos_builder(
    soft_deletes_field: Option<String>,
    parent_field: Option<String>,
) -> Box<dyn FieldInfosBuilder> {
    TestSecrets::get_index_package_access()
        .and_then(|access| {
            access.new_field_infos_builder(soft_deletes_field.as_deref(), parent_field.as_deref())
        })
        .unwrap_or_else(|e| rethrow(e))
}

/// `newSegmentInfo(dir, name)` (`protected static`): returns a new fake
/// segment.
pub fn new_segment_info(dir: Arc<dyn Directory>, name: &str) -> SegmentInfo {
    let min_version = if random().next_boolean() {
        None
    } else {
        Some(Version::LATEST)
    };
    SegmentInfo::new(
        dir,
        Version::LATEST,
        min_version,
        name.to_string(),
        10000,
        false,
        false,
        rucene::codecs::get_default().expect("Codec.getDefault()"),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )
    .unwrap_or_else(|e| rethrow(e))
}

/// `createFieldInfo()` (`private`).
fn create_field_info() -> FieldInfo {
    FieldInfo::new_full(
        "field",
        -1,
        false,
        false,
        false,
        TextField::type_stored().index_options(),
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        JavaStringMap::default(),
        0,
        0,
        0,
        0,
        VectorEncoding::FLOAT32,
        VectorSimilarityFunction::EUCLIDEAN,
        false,
        false,
    )
    .unwrap_or_else(|e| rethrow(e))
}

/// `getVectorsMaxDimensions(fieldName)` (`private`).
fn get_vectors_max_dimensions(field_name: &str) -> i32 {
    rucene::codecs::get_default()
        .expect("Codec.getDefault()")
        .knn_vectors_format()
        .get_max_dimensions(field_name)
}

/// What the exception tests' shared preamble builds.
type ExceptionFixture = (
    MockDirectoryWrapper,
    Arc<CallStackFailure>,
    Arc<dyn Codec>,
    SegmentInfo,
    FieldInfos,
);

/// Port of the members of `BaseFieldInfoFormatTestCase`.
pub trait BaseFieldInfoFormatTestCase: BaseIndexFileFormatTestCase {
    /// `supportDocValuesSkipIndex()`: override and return `false` if the
    /// format does not support setting doc values skip index.
    fn support_doc_values_skip_index(&self) -> bool {
        true
    }

    /// `supportCustomTermFreqs()`: override and return `false` if the format
    /// does not support `IndexOption.DOCS_AND_CUSTOM_TERM_FREQS`.
    fn support_custom_term_freqs(&self) -> bool {
        true
    }

    /// `addAttributes(fi)`: hook to add any codec attributes to fieldinfo
    /// instances added in this test.
    fn add_attributes(&self, _fi: &FieldInfo) {}

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn field_infos_add_random_fields(&self, doc: &mut Document) {
        doc.add(Box::new(
            StoredField::new_string("foobar", TestUtil::random_simple_string(&*random()))
                .unwrap_or_else(|e| rethrow(e)),
        ));
    }

    /// The exception tests' shared preamble: a mock directory failing on
    /// `method`, the codec, a segment and one field's infos.
    fn exception_fixture(&self, method: &'static str) -> ExceptionFixture {
        let fail = CallStackFailure::new(method);
        let dir = LuceneTestCase::new_mock_directory();
        dir.fail_on(Arc::clone(&fail) as Arc<dyn Failure>);
        let codec = self.get_codec();
        let segment_info = new_segment_info(dir.as_directory(), "_123");
        let fi = create_field_info();
        self.add_attributes(&fi);
        let mut builder = new_field_infos_builder(None, None);
        builder.add(&fi).unwrap_or_else(|e| rethrow(e));
        let infos = builder.finish().unwrap_or_else(|e| rethrow(e));
        (dir, fail, codec, segment_info, infos)
    }

    /// `testOneField()`: test field infos read/write with a single field.
    fn test_one_field(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let segment_info = new_segment_info(dir.as_directory(), "_123");
            let fi = create_field_info();
            self.add_attributes(&fi);

            let mut builder = new_field_infos_builder(None, None);
            builder.add(&fi)?;
            let infos = builder.finish()?;

            codec.field_infos_format().write(
                &*dir.as_directory(),
                &segment_info,
                "",
                &infos,
                &*DEFAULT_IO_CONTEXT,
            )?;

            let infos2 = codec.field_infos_format().read(
                &*dir.as_directory(),
                &segment_info,
                "",
                &*DEFAULT_IO_CONTEXT,
            )?;
            Assert::assert_equals(1i64, infos2.size() as i64);
            let field = infos2.field_info("field");
            assert_not_null(field.is_some());
            let field = field.expect("checked above");
            Assert::assert_true(field.get_index_options() != IndexOptions::NONE);
            Assert::assert_false(field.get_doc_values_type() != DocValuesType::NONE);
            Assert::assert_false(field.omits_norms());
            Assert::assert_false(field.has_payloads());
            Assert::assert_false(field.has_term_vectors());
            Assert::assert_equals(0i64, i64::from(field.get_point_dimension_count()));
            Assert::assert_equals(0i64, i64::from(field.get_vector_dimension()));
            Assert::assert_false(field.is_soft_deletes_field());
        }
        dir.close()
    }

    /// `testImmutableAttributes()`: test field infos attributes coming back
    /// are not mutable (see the module docs for the divergence).
    fn test_immutable_attributes(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let segment_info = new_segment_info(dir.as_directory(), "_123");
            let fi = create_field_info();
            self.add_attributes(&fi);
            fi.put_attribute("foo", "bar");
            fi.put_attribute("bar", "baz");

            let mut builder = new_field_infos_builder(None, None);
            builder.add(&fi)?;
            let infos = builder.finish()?;

            codec.field_infos_format().write(
                &*dir.as_directory(),
                &segment_info,
                "",
                &infos,
                &*DEFAULT_IO_CONTEXT,
            )?;

            let infos2 = codec.field_infos_format().read(
                &*dir.as_directory(),
                &segment_info,
                "",
                &*DEFAULT_IO_CONTEXT,
            )?;
            Assert::assert_equals(1i64, infos2.size() as i64);
            let field = infos2.field_info("field");
            assert_not_null(field.is_some());
            let attributes = field.expect("checked above").attributes();
            // shouldn't be able to modify attributes: the map has no mutator, so
            // what was written is what reads back.
            Assert::assert_equals_str(None, Some("bar"), attributes.get("foo").map(String::as_str));
            Assert::assert_equals_str(None, Some("baz"), attributes.get("bar").map(String::as_str));
            Assert::assert_false(attributes.get("bogus").is_some());
        }
        dir.close()
    }

    /// `testExceptionOnCreateOutput()`: test field infos write that hits
    /// exception immediately on open. make sure we get our exception back, no
    /// file handle leaks, etc.
    fn test_exception_on_create_output(&self) -> Result<()> {
        let (dir, fail, codec, segment_info, infos) = self.exception_fixture("create_output");
        {
            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec.field_infos_format().write(
                    &dir,
                    &segment_info,
                    "",
                    &infos,
                    &*DEFAULT_IO_CONTEXT,
                )
            });
            fail.clear_do_fail();
        }
        drop((fail, codec, segment_info, infos));
        dir.close()
    }

    /// `testExceptionOnCloseOutput()`: test field infos write that hits
    /// exception on close. make sure we get our exception back, no file handle
    /// leaks, etc.
    fn test_exception_on_close_output(&self) -> Result<()> {
        let (dir, fail, codec, segment_info, infos) = self.exception_fixture("close");
        {
            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec.field_infos_format().write(
                    &dir,
                    &segment_info,
                    "",
                    &infos,
                    &*DEFAULT_IO_CONTEXT,
                )
            });
            fail.clear_do_fail();
        }
        drop((fail, codec, segment_info, infos));
        dir.close()
    }

    /// `testExceptionOnOpenInput()`: test field infos read that hits
    /// exception immediately on open. make sure we get our exception back, no
    /// file handle leaks, etc.
    fn test_exception_on_open_input(&self) -> Result<()> {
        let (dir, fail, codec, segment_info, infos) = self.exception_fixture("open_input");
        {
            codec.field_infos_format().write(
                &dir,
                &segment_info,
                "",
                &infos,
                &*DEFAULT_IO_CONTEXT,
            )?;

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec
                    .field_infos_format()
                    .read(&dir, &segment_info, "", &*DEFAULT_IO_CONTEXT)
            });
            fail.clear_do_fail();
        }
        drop((fail, codec, segment_info, infos));
        dir.close()
    }

    /// `testExceptionOnCloseInput()`: test field infos read that hits
    /// exception on close. make sure we get our exception back, no file handle
    /// leaks, etc.
    fn test_exception_on_close_input(&self) -> Result<()> {
        let (dir, fail, codec, segment_info, infos) = self.exception_fixture("close");
        {
            codec.field_infos_format().write(
                &dir,
                &segment_info,
                "",
                &infos,
                &*DEFAULT_IO_CONTEXT,
            )?;

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec
                    .field_infos_format()
                    .read(&dir, &segment_info, "", &*DEFAULT_IO_CONTEXT)
            });
            fail.clear_do_fail();
        }
        drop((fail, codec, segment_info, infos));
        dir.close()
    }

    // TODO: more tests

    /// `testRandom()`: test field infos read/write with random fields, with
    /// different values.
    fn test_random(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let segment_info = new_segment_info(dir.as_directory(), "_123");

            // generate a bunch of fields
            let num_fields = LuceneTestCase::at_least(2000);
            // `new HashSet<>()`, iterated in the JDK's order.
            let mut field_names = rucene::util::java_collections::JavaHashSet::new();
            for _ in 0..num_fields {
                field_names.insert(TestUtil::random_unicode_string(&*random()));
            }

            let soft_deletes_field = if random().next_boolean() {
                Some(TestUtil::random_unicode_string(&*random()))
            } else {
                None
            };

            let mut parent_field = if random().next_boolean() {
                Some(TestUtil::random_unicode_string(&*random()))
            } else {
                None
            };

            if soft_deletes_field.is_some() && soft_deletes_field == parent_field {
                parent_field = None;
            }
            let mut builder =
                new_field_infos_builder(soft_deletes_field.clone(), parent_field.clone());

            for field in field_names.iter() {
                let field_type = self.random_field_type(&*random(), field);
                let mut store_term_vectors = false;
                let mut store_payloads = false;
                let mut omit_norms = false;
                if field_type.index_options() != IndexOptions::NONE {
                    store_term_vectors = field_type.store_term_vectors();
                    omit_norms = field_type.omit_norms();
                    if field_type
                        .index_options()
                        .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
                    {
                        store_payloads = random().next_boolean();
                    }
                }
                let mut doc_values_skip_index_type = DocValuesSkipIndexType::NONE;
                if matches!(
                    field_type.doc_values_type(),
                    DocValuesType::NUMERIC
                        | DocValuesType::SORTED
                        | DocValuesType::SORTED_NUMERIC
                        | DocValuesType::SORTED_SET
                ) {
                    doc_values_skip_index_type = field_type.doc_values_skip_index_type();
                }
                let fi = FieldInfo::new_full(
                    field.clone(),
                    -1,
                    store_term_vectors,
                    omit_norms,
                    store_payloads,
                    field_type.index_options(),
                    field_type.doc_values_type(),
                    doc_values_skip_index_type,
                    -1,
                    JavaStringMap::default(),
                    field_type.point_dimension_count(),
                    field_type.point_index_dimension_count(),
                    field_type.point_num_bytes(),
                    field_type.vector_dimension(),
                    field_type.vector_encoding(),
                    field_type.vector_similarity_function(),
                    soft_deletes_field.as_deref() == Some(field.as_str()),
                    parent_field.as_deref() == Some(field.as_str()),
                )?;
                self.add_attributes(&fi);
                builder.add(&fi)?;
            }
            let infos = builder.finish()?;
            codec.field_infos_format().write(
                &*dir.as_directory(),
                &segment_info,
                "",
                &infos,
                &*DEFAULT_IO_CONTEXT,
            )?;
            let infos2 = codec.field_infos_format().read(
                &*dir.as_directory(),
                &segment_info,
                "",
                &*DEFAULT_IO_CONTEXT,
            )?;
            self.assert_field_infos_equal(&infos, &infos2);
        }
        dir.close()
    }

    /// `randomFieldType(r, fieldName)` (`private`).
    fn random_field_type(&self, r: &dyn Random, field_name: &str) -> FieldType {
        let ty = FieldType::new();
        let ok = |res: Result<()>| res.unwrap_or_else(|e| rethrow(e));

        if r.next_boolean() {
            let values = INDEX_OPTIONS_VALUES;
            let mut index_options = values[r.next_int_bound(values.len() as i32) as usize];
            if index_options == IndexOptions::DOCS_AND_CUSTOM_FREQS
                && !self.support_custom_term_freqs()
            {
                index_options = IndexOptions::DOCS_AND_FREQS;
            }
            ok(ty.set_index_options(index_options));
            ok(ty.set_omit_norms(r.next_boolean()));

            if r.next_boolean() {
                ok(ty.set_store_term_vectors(true));
                if ty
                    .index_options()
                    .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
                {
                    ok(ty.set_store_term_vector_positions(r.next_boolean()));
                    ok(ty.set_store_term_vector_offsets(r.next_boolean()));
                    if ty.store_term_vector_positions() {
                        ok(ty.set_store_term_vector_payloads(r.next_boolean()));
                    }
                }
            }
        }

        if r.next_boolean() {
            let values = DOC_VALUES_TYPE_VALUES;
            let current = values[r.next_int_bound(values.len() as i32) as usize];
            ok(ty.set_doc_values_type(values[r.next_int_bound(values.len() as i32) as usize]));
            if matches!(
                current,
                DocValuesType::NUMERIC
                    | DocValuesType::SORTED_NUMERIC
                    | DocValuesType::SORTED
                    | DocValuesType::SORTED_SET
            ) {
                ok(
                    ty.set_doc_values_skip_index_type(if self.support_doc_values_skip_index() {
                        DocValuesSkipIndexType::RANGE
                    } else {
                        DocValuesSkipIndexType::NONE
                    }),
                );
            }
        }

        if r.next_boolean() {
            let dimension = 1 + r.next_int_bound(MAX_DIMENSIONS);
            let index_dimension = 1 + r.next_int_bound(dimension.min(MAX_INDEX_DIMENSIONS));
            let dimension_num_bytes = 1 + r.next_int_bound(MAX_NUM_BYTES);
            ok(ty.set_dimensions_with_index_count(dimension, index_dimension, dimension_num_bytes));
        }

        if r.next_boolean() && get_vectors_max_dimensions(field_name) > 0 {
            let dimension = 1 + r.next_int_bound(get_vectors_max_dimensions(field_name));
            let similarity_function =
                *RandomPicks::random_from(r, &VECTOR_SIMILARITY_FUNCTION_VALUES);
            let encoding = *RandomPicks::random_from(r, &VECTOR_ENCODING_VALUES);
            ok(ty.set_vector_attributes(dimension, encoding, similarity_function));
        }

        ty
    }

    /// `assertEquals(FieldInfos expected, FieldInfos actual)`: equality for
    /// entirety of fieldinfos.
    fn assert_field_infos_equal(&self, expected: &FieldInfos, actual: &FieldInfos) {
        Assert::assert_equals(expected.size() as i64, actual.size() as i64);
        for expected_field in expected.iter() {
            let actual_field = actual.field_info_by_number(expected_field.number);
            assert_not_null(actual_field.is_some());
            self.assert_field_info_equal(expected_field, actual_field.expect("checked above"));
        }
    }

    /// `assertEquals(FieldInfo expected, FieldInfo actual)`: equality for two
    /// individual fieldinfo objects.
    fn assert_field_info_equal(&self, expected: &FieldInfo, actual: &FieldInfo) {
        Assert::assert_equals(i64::from(expected.number), i64::from(actual.number));
        Assert::assert_equals(expected.name.as_str(), actual.name.as_str());
        Assert::assert_equals(
            format!("{:?}", expected.get_doc_values_type()),
            format!("{:?}", actual.get_doc_values_type()),
        );
        Assert::assert_equals(
            format!("{:?}", expected.doc_values_skip_index_type()),
            format!("{:?}", actual.doc_values_skip_index_type()),
        );
        Assert::assert_equals(
            format!("{:?}", expected.get_index_options()),
            format!("{:?}", actual.get_index_options()),
        );
        Assert::assert_equals(expected.has_norms(), actual.has_norms());
        Assert::assert_equals(expected.has_payloads(), actual.has_payloads());
        Assert::assert_equals(expected.has_term_vectors(), actual.has_term_vectors());
        Assert::assert_equals(expected.omits_norms(), actual.omits_norms());
        Assert::assert_equals(expected.get_doc_values_gen(), actual.get_doc_values_gen());
    }
}
