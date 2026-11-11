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

//! Port of `org.apache.lucene.tests.index.BaseSegmentInfoFormatTestCase`:
//! abstract class to do basic tests for si format. NOTE: This test focuses
//! on the si impl, nothing else. The [stretch] goal is for this test to be so
//! thorough in testing a new si format that if this test passes, then all
//! Lucene tests should also pass. Ie, if there is some bug in a given si
//! Format that this test fails to catch then this test needs to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseSegmentInfoFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testAddsSelfToFiles`, `testDiagnostics`, `testAttributes`: Java expects
//!   `UnsupportedOperationException` from adding to `info2.files()`,
//!   `info2.getDiagnostics()` and `info2.getAttributes()`, which are
//!   unmodifiable views. The port's accessors return a copy
//!   (`files()`, `get_attributes()`) or a shared reference with no mutator
//!   (`get_diagnostics()`), so the write cannot reach the `SegmentInfo`; the
//!   tests check instead that the segment still answers what was written.
//! * Java's `null` index sort is the port's `Sort::default()`, which the
//!   segment info formats write as "no sort", as `SegmentInfo` holds it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, StoredField};
use rucene::error::Result;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_file_names;
use rucene::index::segment_info::SegmentInfo;
use rucene::search::{
    MissingValue, Sort, SortField, SortFieldType, SortedNumericSelectorType,
    SortedNumericSortField, SortedSetSelectorType, SortedSetSortField,
};
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::{StringHelper, ID_LENGTH};
use rucene::util::Version;

use super::base_field_info_format_test_case::{expect_fake_io_exception, CallStackFailure};
use super::base_index_file_format_test_case::{
    assert_array_equals, assert_equals_debug, format_test, rethrow, BaseIndexFileFormatTestCase,
};
use crate::store::mock_directory_wrapper::Failure;
use crate::store::MockDirectoryWrapper;
use crate::util::lucene_test_case::LuceneTestCase;
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseSegmentInfoFormatTestCase`
/// (`public abstract class BaseSegmentInfoFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseSegmentInfoFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(segment_infos_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseSegmentInfoFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testFiles", |c| c.test_files()))
            .method(t("testHasBlocks", |c| c.test_has_blocks()))
            .method(t("testAddsSelfToFiles", |c| c.test_adds_self_to_files()))
            .method(t("testDiagnostics", |c| c.test_diagnostics()))
            .method(t("testAttributes", |c| c.test_attributes()))
            .method(t("testUniqueID", |c| c.test_unique_id()))
            .method(t("testVersions", |c| c.test_versions()))
            .method(t("testSort", |c| c.test_sort()))
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

/// The instance's subclass as a `BaseSegmentInfoFormatTestCase`.
fn segment_infos_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseSegmentInfoFormatTestCase {
    c.as_segment_infos().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseSegmentInfoFormatTestCase",
        )
        .throw()
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `Long.toString(value, Character.MAX_RADIX)` of a non-negative `long`.
fn to_string_radix36(mut value: i64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII digits")
}

/// `info.setFiles(Collections.emptySet())`.
fn set_no_files(info: &SegmentInfo) {
    info.set_files(std::iter::empty::<String>())
        .unwrap_or_else(|e| rethrow(e));
}

/// Port of the members of `BaseSegmentInfoFormatTestCase`.
pub trait BaseSegmentInfoFormatTestCase: BaseIndexFileFormatTestCase {
    /// `getVersions()`: returns the versions this SI should test.
    fn get_versions(&self) -> Vec<Version>;

    /// `supportsMinVersion()`: whether this format records min versions.
    fn supports_min_version(&self) -> bool {
        true
    }

    /// `supportsIndexSort()`.
    fn supports_index_sort(&self) -> bool {
        true
    }

    /// `supportsHasBlocks()`.
    fn supports_has_blocks(&self) -> bool {
        true
    }

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn segment_infos_add_random_fields(&self, doc: &mut Document) {
        doc.add(Box::new(
            StoredField::new_string("foobar", TestUtil::random_simple_string(&*random()))
                .unwrap_or_else(|e| rethrow(e)),
        ));
    }

    /// `new SegmentInfo(dir, getVersions()[0], getVersions()[0], "_123", 1,
    /// false, hasBlocks, codec, diagnostics, id, attributes, indexSort)`, the
    /// segment most tests write.
    #[allow(clippy::too_many_arguments)]
    fn new_info(
        &self,
        dir: Arc<dyn Directory>,
        has_blocks: bool,
        diagnostics: HashMap<String, String>,
        id: [u8; ID_LENGTH],
        attributes: HashMap<String, String>,
        index_sort: Sort,
    ) -> SegmentInfo {
        let v = self.get_versions()[0];
        SegmentInfo::new(
            dir,
            v,
            Some(v),
            "_123".to_string(),
            1,
            false,
            has_blocks,
            self.get_codec(),
            diagnostics,
            id,
            attributes,
            index_sort,
        )
        .unwrap_or_else(|e| rethrow(e))
    }

    /// `testFiles()`: test files map.
    fn test_files(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let id = StringHelper::random_id();
            let info = self.new_info(
                dir.as_directory(),
                false,
                HashMap::new(),
                id,
                HashMap::new(),
                Sort::default(),
            );
            set_no_files(&info);
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            assert_equals_debug(&info.files()?, &info2.files()?);
        }
        dir.close()
    }

    /// `testHasBlocks()`.
    fn test_has_blocks(&self) -> Result<()> {
        LuceneTestCase::assume_true(
            "test requires a codec that can read/write hasBlocks",
            self.supports_has_blocks(),
        );

        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let id = StringHelper::random_id();
            let info = self.new_info(
                dir.as_directory(),
                random().next_boolean(),
                HashMap::new(),
                id,
                HashMap::new(),
                Sort::default(),
            );
            set_no_files(&info);
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            Assert::assert_equals(info.get_has_blocks(), info2.get_has_blocks());
        }
        dir.close()
    }

    /// `testAddsSelfToFiles()`: tests SI writer adds itself to files...
    fn test_adds_self_to_files(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let id = StringHelper::random_id();
            let info = self.new_info(
                dir.as_directory(),
                false,
                HashMap::new(),
                id,
                HashMap::new(),
                Sort::default(),
            );
            let original_files: HashSet<String> = ["_123.a".to_string()].into_iter().collect();
            info.set_files(original_files.iter().cloned())?;
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;

            let modified_files = info.files()?;
            Assert::assert_true(original_files.is_subset(&modified_files));
            Assert::assert_true_msg(
                "did you forget to add yourself to files()",
                modified_files.len() > original_files.len(),
            );

            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            assert_equals_debug(&info.files()?, &info2.files()?);

            // files set should be immutable: `files()` hands out a copy, so the
            // segment keeps its files.
            let mut copy = info2.files()?;
            copy.insert("bogus".to_string());
            Assert::assert_false(info2.files()?.contains("bogus"));
        }
        dir.close()
    }

    /// `testDiagnostics()`: test diagnostics map.
    fn test_diagnostics(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let id = StringHelper::random_id();
            let mut diagnostics = HashMap::new();
            diagnostics.insert("key1".to_string(), "value1".to_string());
            diagnostics.insert("key2".to_string(), "value2".to_string());
            let info = self.new_info(
                dir.as_directory(),
                false,
                diagnostics.clone(),
                id,
                HashMap::new(),
                Sort::default(),
            );
            set_no_files(&info);
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            assert_equals_debug(&diagnostics, &info2.get_diagnostics().to_std());

            // diagnostics map should be immutable: the reference has no mutator.
            Assert::assert_false(info2.get_diagnostics().get("bogus").is_some());
        }
        dir.close()
    }

    /// `testAttributes()`: test attributes map.
    fn test_attributes(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let codec = self.get_codec();
            let id = StringHelper::random_id();
            let mut attributes = HashMap::new();
            attributes.insert("key1".to_string(), "value1".to_string());
            attributes.insert("key2".to_string(), "value2".to_string());
            let info = self.new_info(
                dir.as_directory(),
                false,
                HashMap::new(),
                id,
                attributes.clone(),
                Sort::default(),
            );
            set_no_files(&info);
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            assert_equals_debug(&attributes, &info2.get_attributes());

            // attributes map should be immutable: `get_attributes()` hands out a
            // copy, so the segment keeps its attributes.
            let mut copy = info2.get_attributes();
            copy.insert("bogus".to_string(), "bogus".to_string());
            Assert::assert_false(info2.get_attributes().contains_key("bogus"));
        }
        dir.close()
    }

    /// `testUniqueID()`: test unique ID.
    fn test_unique_id(&self) -> Result<()> {
        let codec = self.get_codec();
        let dir = LuceneTestCase::new_directory();
        {
            let id = StringHelper::random_id();
            let info = self.new_info(
                dir.as_directory(),
                false,
                HashMap::new(),
                id,
                HashMap::new(),
                Sort::default(),
            );
            set_no_files(&info);
            codec
                .segment_info_format()
                .write(&*dir.as_directory(), &info, &*DEFAULT_IO_CONTEXT)?;
            let info2 = codec.segment_info_format().read(
                dir.as_directory(),
                "_123",
                &id,
                &*DEFAULT_IO_CONTEXT,
            )?;
            assert_array_equals(&id, &info2.id());
        }
        dir.close()
    }

    /// `testVersions()`: test versions.
    fn test_versions(&self) -> Result<()> {
        let codec = self.get_codec();
        for v in self.get_versions() {
            for min_v in [Some(v), None] {
                let dir = LuceneTestCase::new_directory();
                {
                    let id = StringHelper::random_id();
                    let info = SegmentInfo::new(
                        dir.as_directory(),
                        v,
                        min_v,
                        "_123".to_string(),
                        1,
                        false,
                        false,
                        Arc::clone(&codec),
                        HashMap::new(),
                        id,
                        HashMap::new(),
                        Sort::default(),
                    )?;
                    set_no_files(&info);
                    codec.segment_info_format().write(
                        &*dir.as_directory(),
                        &info,
                        &*DEFAULT_IO_CONTEXT,
                    )?;
                    let info2 = codec.segment_info_format().read(
                        dir.as_directory(),
                        "_123",
                        &id,
                        &*DEFAULT_IO_CONTEXT,
                    )?;
                    Assert::assert_equals(info2.version(), v);
                    if self.supports_min_version() {
                        assert_equals_debug(&info2.min_version(), &min_v);
                    } else {
                        assert_equals_debug(&info2.min_version(), &None);
                    }
                }
                dir.close()?;
            }
        }
        Ok(())
    }

    /// `randomIndexSortField()` (`private`).
    fn random_index_sort_field(&self) -> SortField {
        let r = random();
        let reversed = r.next_boolean();
        let ok = |res: Result<SortField>| res.unwrap_or_else(|e| rethrow(e));
        match r.next_int_bound(10) {
            0 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then(|| MissingValue::Int(r.next_int()));
                ok(SortField::new_with_missing(
                    Some(name),
                    SortFieldType::Int,
                    reversed,
                    missing,
                ))
            }
            1 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then(|| MissingValue::Int(r.next_int()));
                ok(SortedNumericSortField::with_missing_value(
                    &name,
                    SortFieldType::Int,
                    reversed,
                    SortedNumericSelectorType::MIN,
                    missing,
                )
                .map(SortField::from))
            }
            2 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then(|| MissingValue::Long(r.next_long()));
                ok(SortField::new_with_missing(
                    Some(name),
                    SortFieldType::Long,
                    reversed,
                    missing,
                ))
            }
            3 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then(|| MissingValue::Long(r.next_long()));
                ok(SortedNumericSortField::with_missing_value(
                    &name,
                    SortFieldType::Long,
                    reversed,
                    SortedNumericSelectorType::MIN,
                    missing,
                )
                .map(SortField::from))
            }
            4 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r
                    .next_boolean()
                    .then(|| MissingValue::Float(r.next_float()));
                ok(SortField::new_with_missing(
                    Some(name),
                    SortFieldType::Float,
                    reversed,
                    missing,
                ))
            }
            5 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r
                    .next_boolean()
                    .then(|| MissingValue::Float(r.next_float()));
                ok(SortedNumericSortField::with_missing_value(
                    &name,
                    SortFieldType::Float,
                    reversed,
                    SortedNumericSelectorType::MIN,
                    missing,
                )
                .map(SortField::from))
            }
            6 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r
                    .next_boolean()
                    .then(|| MissingValue::Double(r.next_double()));
                ok(SortField::new_with_missing(
                    Some(name),
                    SortFieldType::Double,
                    reversed,
                    missing,
                ))
            }
            7 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r
                    .next_boolean()
                    .then(|| MissingValue::Double(r.next_double()));
                ok(SortedNumericSortField::with_missing_value(
                    &name,
                    SortFieldType::Double,
                    reversed,
                    SortedNumericSelectorType::MIN,
                    missing,
                )
                .map(SortField::from))
            }
            8 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then_some(MissingValue::StringLast);
                ok(SortField::new_with_missing(
                    Some(name),
                    SortFieldType::String,
                    reversed,
                    missing,
                ))
            }
            9 => {
                let name = TestUtil::random_simple_string(&*r);
                let missing = r.next_boolean().then_some(MissingValue::StringLast);
                ok(SortedSetSortField::with_missing_value(
                    &name,
                    reversed,
                    SortedSetSelectorType::MIN,
                    missing,
                )
                .map(SortField::from))
            }
            _ => Assert::fail_without_message(),
        }
    }

    /// `testSort()`: test sort.
    fn test_sort(&self) -> Result<()> {
        LuceneTestCase::assume_true(
            "test requires a codec that can read/write index sort",
            self.supports_index_sort(),
        );

        let iters = LuceneTestCase::at_least(5);
        for i in 0..iters {
            let sort = if i == 0 {
                Sort::default()
            } else {
                let num_sort_fields = TestUtil::next_int(&*random(), 1, 3);
                let sort_fields: Vec<SortField> = (0..num_sort_fields)
                    .map(|_| self.random_index_sort_field())
                    .collect();
                Sort::new_fields(sort_fields)?
            };

            let dir = LuceneTestCase::new_directory();
            {
                let codec = self.get_codec();
                let id = StringHelper::random_id();
                let info = self.new_info(
                    dir.as_directory(),
                    false,
                    HashMap::new(),
                    id,
                    HashMap::new(),
                    sort.clone(),
                );
                set_no_files(&info);
                codec.segment_info_format().write(
                    &*dir.as_directory(),
                    &info,
                    &*DEFAULT_IO_CONTEXT,
                )?;
                let info2 = codec.segment_info_format().read(
                    dir.as_directory(),
                    "_123",
                    &id,
                    &*DEFAULT_IO_CONTEXT,
                )?;
                assert_equals_debug(&sort, info2.index_sort());
            }
            dir.close()?;
        }
        Ok(())
    }

    /// The exception tests' shared preamble: a mock directory failing on
    /// `method`, and the segment `_123` of the first version.
    fn exception_fixture(
        &self,
        method: &'static str,
    ) -> (
        MockDirectoryWrapper,
        Arc<CallStackFailure>,
        SegmentInfo,
        [u8; ID_LENGTH],
    ) {
        let fail = CallStackFailure::new(method);
        let dir = LuceneTestCase::new_mock_directory();
        dir.fail_on(Arc::clone(&fail) as Arc<dyn Failure>);
        let id = StringHelper::random_id();
        let info = self.new_info(
            dir.as_directory(),
            false,
            HashMap::new(),
            id,
            HashMap::new(),
            Sort::default(),
        );
        set_no_files(&info);
        (dir, fail, info, id)
    }

    /// `testExceptionOnCreateOutput()`: test segment infos write that hits
    /// exception immediately on open. make sure we get our exception back, no
    /// file handle leaks, etc.
    fn test_exception_on_create_output(&self) -> Result<()> {
        let (dir, fail, info, _id) = self.exception_fixture("create_output");
        {
            let codec = self.get_codec();

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec
                    .segment_info_format()
                    .write(&dir, &info, &*DEFAULT_IO_CONTEXT)
            });
            fail.clear_do_fail();
        }
        drop((fail, info, _id));
        dir.close()
    }

    /// `testExceptionOnCloseOutput()`: test segment infos write that hits
    /// exception on close. make sure we get our exception back, no file handle
    /// leaks, etc.
    fn test_exception_on_close_output(&self) -> Result<()> {
        let (dir, fail, info, _id) = self.exception_fixture("close");
        {
            let codec = self.get_codec();

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec
                    .segment_info_format()
                    .write(&dir, &info, &*DEFAULT_IO_CONTEXT)
            });
            fail.clear_do_fail();
        }
        drop((fail, info, _id));
        dir.close()
    }

    /// `testExceptionOnOpenInput()`: test segment infos read that hits
    /// exception immediately on open. make sure we get our exception back, no
    /// file handle leaks, etc.
    fn test_exception_on_open_input(&self) -> Result<()> {
        let (dir, fail, info, id) = self.exception_fixture("open_input");
        {
            let codec = self.get_codec();
            codec
                .segment_info_format()
                .write(&dir, &info, &*DEFAULT_IO_CONTEXT)?;

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec.segment_info_format().read(
                    dir.as_directory(),
                    "_123",
                    &id,
                    &*DEFAULT_IO_CONTEXT,
                )
            });
            fail.clear_do_fail();
        }
        drop((fail, info, id));
        dir.close()
    }

    /// `testExceptionOnCloseInput()`: test segment infos read that hits
    /// exception on close make sure we get our exception back, no file handle
    /// leaks, etc.
    fn test_exception_on_close_input(&self) -> Result<()> {
        let (dir, fail, info, id) = self.exception_fixture("close");
        {
            let codec = self.get_codec();
            codec
                .segment_info_format()
                .write(&dir, &info, &*DEFAULT_IO_CONTEXT)?;

            fail.set_do_fail();
            expect_fake_io_exception(|| {
                codec.segment_info_format().read(
                    dir.as_directory(),
                    "_123",
                    &id,
                    &*DEFAULT_IO_CONTEXT,
                )
            });
            fail.clear_do_fail();
        }
        drop((fail, info, id));
        dir.close()
    }

    /// `testRandom()`: sets some otherwise hard-to-test properties: random
    /// segment names, ID values, document count, etc and round-trips.
    fn test_random(&self) -> Result<()> {
        let codec = self.get_codec();
        let versions = self.get_versions();
        for _ in 0..10 {
            let dir = LuceneTestCase::new_directory();
            {
                // Java's `dir`: one handle, so `assertSame(expected.dir,
                // actual.dir)` compares the same reference.
                let directory = dir.as_directory();
                let version = versions[random().next_int_bound(versions.len() as i32) as usize];
                // `Math.abs(random().nextLong())`: `Long.MIN_VALUE` stays negative.
                let random_segment_index = random().next_long().wrapping_abs();
                let name = format!(
                    "_{}",
                    if random_segment_index != i64::MIN {
                        to_string_radix36(random_segment_index)
                    } else {
                        to_string_radix36(i64::from(random().next_int_bound(i32::MAX)))
                    }
                );
                let doc_count = TestUtil::next_int(&*random(), 1, MAX_DOCS);
                let is_compound_file = random().next_boolean();
                // `new HashSet<>()`, iterated in the JDK's order.
                let mut files = rucene::util::java_collections::JavaHashSet::new();
                let num_files = random().next_int_bound(10);
                for j in 0..num_files {
                    let file = index_file_names::segment_file_name(&name, "", &j.to_string());
                    files.insert(file.clone());
                    directory
                        .create_output(&file, &*DEFAULT_IO_CONTEXT)?
                        .close()?;
                }
                let mut diagnostics = HashMap::new();
                let num_diags = random().next_int_bound(10);
                for _ in 0..num_diags {
                    diagnostics.insert(
                        TestUtil::random_unicode_string(&*random()),
                        TestUtil::random_unicode_string(&*random()),
                    );
                }
                let mut id = [0u8; ID_LENGTH];
                random().next_bytes(&mut id);

                let mut attributes = HashMap::new();
                let num_attributes = random().next_int_bound(10);
                for _ in 0..num_attributes {
                    attributes.insert(
                        TestUtil::random_unicode_string(&*random()),
                        TestUtil::random_unicode_string(&*random()),
                    );
                }

                let info = SegmentInfo::new(
                    Arc::clone(&directory),
                    version,
                    None,
                    name.clone(),
                    doc_count,
                    is_compound_file,
                    false,
                    Arc::clone(&codec),
                    diagnostics,
                    id,
                    attributes,
                    Sort::default(),
                )?;
                info.set_files(files.iter().cloned())?;
                codec
                    .segment_info_format()
                    .write(&*directory, &info, &*DEFAULT_IO_CONTEXT)?;
                let info2 = codec.segment_info_format().read(
                    Arc::clone(&directory),
                    &name,
                    &id,
                    &*DEFAULT_IO_CONTEXT,
                )?;
                self.assert_segment_info_equal(&info, &info2)?;
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `assertEquals(SegmentInfo expected, SegmentInfo actual)` (`protected
    /// final`).
    fn assert_segment_info_equal(
        &self,
        expected: &SegmentInfo,
        actual: &SegmentInfo,
    ) -> Result<()> {
        // `assertSame(expected.dir, actual.dir)`
        Assert::assert_true(Arc::ptr_eq(&expected.directory, &actual.directory));
        Assert::assert_equals_str(None, Some(&expected.name), Some(&actual.name));
        assert_equals_debug(&expected.files()?, &actual.files()?);
        // we don't assert this, because SI format has nothing to do with it... set by SIS
        // assertSame(expected.getCodec(), actual.getCodec());
        assert_equals_debug(
            &expected.get_diagnostics().to_std(),
            &actual.get_diagnostics().to_std(),
        );
        Assert::assert_equals(i64::from(expected.max_doc()?), i64::from(actual.max_doc()?));
        assert_array_equals(&expected.id(), &actual.id());
        Assert::assert_equals(
            expected.get_use_compound_file(),
            actual.get_use_compound_file(),
        );
        Assert::assert_equals(expected.version(), actual.version());
        assert_equals_debug(&expected.get_attributes(), &actual.get_attributes());
        Ok(())
    }
}
