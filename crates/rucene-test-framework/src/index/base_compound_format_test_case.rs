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

//! Port of `org.apache.lucene.tests.index.BaseCompoundFormatTestCase`:
//! abstract class to do basic tests for a compound format. NOTE: This test
//! focuses on the compound impl, nothing else. The [stretch] goal is for this
//! test to be so thorough in testing a new `CompoundFormat` that if this test
//! passes, then all Lucene tests should also pass. Ie, if there is some bug in
//! a given `CompoundFormat` that this test fails to catch then this test needs
//! to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseCompoundFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testPassIOContext`, `testLargeCFS`: Java wraps the test directory in an
//!   anonymous `FilterDirectory` / an `NRTCachingDirectory` that own it. The
//!   port's wrappers own a `Box<dyn Directory>`, while the test directory is
//!   shared (`TestDirectory`); [`OwningTestDirectory`] delegates every call to
//!   it and closes it when closed, which is what Java's `FilterDirectory`
//!   does.
//! * `assertSame(myContext, context)` (`testPassIOContext`): Java compares
//!   object identity; the port compares the addresses of the two
//!   `IOContext`s.
//! * `assertNotNull` of a stream or an array (`assertSameStreams`,
//!   `assertEqualArrays`): a Rust reference is never null, so the checks
//!   always pass and are not made.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util::{self, FOOTER_MAGIC};
use rucene::codecs::compound::CompoundDirectory;
use rucene::document::{Document, Store, StoredField};
use rucene::error::{LuceneError, Result};
use rucene::index::segment_info::SegmentInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::search::{Sort, NO_MORE_DOCS};
use rucene::store::{
    flush_io_context, BufferedChecksumIndexInput, Directory, FlushInfo, IOContext, IndexInput,
    IndexOutput, Lock, NRTCachingDirectory, DEFAULT_IO_CONTEXT,
};
use rucene::util::bit_set::BitSet;
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;

use super::base_index_file_format_test_case::{
    exception_message, expect_throws_lucene, format_test, is_corrupt_index,
    is_unsupported_operation, BaseIndexFileFormatTestCase, FileTrackingDirectoryWrapper,
    JavaStringSet, ReadBytesDirectoryWrapper,
};
use crate::index::random_index_writer::RandomIndexWriter;
use crate::util::lucene_test_case::{LuceneTestCase, TestDirectory};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BaseCompoundFormatTestCase`
/// (`public abstract class BaseCompoundFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests and its override of
/// `testMergeStability()`.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseCompoundFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(compound_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseCompoundFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testEmpty", |c| c.test_empty()))
            .method(t("testSingleFile", |c| c.test_single_file()))
            .method(t("testTwoFiles", |c| c.test_two_files()))
            .method(t("testDoubleClose", |c| c.test_double_close()))
            .method(t("testPassIOContext", |c| c.test_pass_io_context()))
            .method(t("testLargeCFS", |c| c.test_large_cfs()))
            .method(t("testListAll", |c| c.test_list_all()))
            .method(t("testCreateOutputDisabled", |c| {
                c.test_create_output_disabled()
            }))
            .method(t("testDeleteFileDisabled", |c| {
                c.test_delete_file_disabled()
            }))
            .method(t("testRenameFileDisabled", |c| {
                c.test_rename_file_disabled()
            }))
            .method(t("testSyncDisabled", |c| c.test_sync_disabled()))
            .method(t("testMakeLockDisabled", |c| c.test_make_lock_disabled()))
            .method(t("testRandomFiles", |c| c.test_random_files()))
            .method(t("testManySubFiles", |c| c.test_many_sub_files()))
            .method(t("testClonedStreamsClosing", |c| {
                c.test_cloned_streams_closing()
            }))
            .method(t("testRandomAccess", |c| c.test_random_access()))
            .method(t("testRandomAccessClones", |c| {
                c.test_random_access_clones()
            }))
            .method(t("testFileNotFound", |c| c.test_file_not_found()))
            .method(t("testReadPastEOF", |c| c.test_read_past_eof()))
            .method(t("testMergeStability", |c| {
                c.compound_test_merge_stability()
            }))
            .method(t("testResourceNameInsideCompoundFile", |c| {
                c.test_resource_name_inside_compound_file()
            }))
            .method(t("testMissingCodecHeadersAreCaught", |c| {
                c.test_missing_codec_headers_are_caught()
            }))
            .method(t("testCorruptFilesAreCaught", |c| {
                c.test_corrupt_files_are_caught()
            }))
            .method(t("testCheckIntegrity", |c| c.test_check_integrity()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseCompoundFormatTestCase`.
fn compound_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseCompoundFormatTestCase {
    c.as_compound().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseCompoundFormatTestCase",
        )
        .throw()
    })
}

// -----------------------------------------------------------------------------
// The static helpers
// -----------------------------------------------------------------------------

/// `si.getCodec()`, which `newSegmentInfo` sets.
fn codec_of(si: &SegmentInfo) -> Arc<dyn rucene::codecs::Codec> {
    si.codec().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `si.getCodec().compoundFormat().write(dir, si, context)`.
fn write_compound(dir: &dyn Directory, si: &SegmentInfo, context: &dyn IOContext) -> Result<()> {
    codec_of(si).compound_format().write(dir, si, context)
}

/// `si.getCodec().compoundFormat().getCompoundReader(dir, si)`.
fn compound_reader(dir: &dyn Directory, si: &SegmentInfo) -> Result<Box<dyn CompoundDirectory>> {
    codec_of(si).compound_format().get_compound_reader(dir, si)
}

/// `newIOContext(random())`.
fn random_io_context() -> Box<dyn IOContext> {
    LuceneTestCase::new_io_context(&*LuceneTestCase::random())
}

/// `newSegmentInfo(dir, name)` (`protected static`): returns a new fake
/// segment.
pub fn new_segment_info(dir: &Arc<dyn Directory>, name: &str) -> Result<SegmentInfo> {
    let min_version = if LuceneTestCase::random().next_boolean() {
        None
    } else {
        Some(Version::LATEST)
    };
    let codec = rucene::codecs::get_default().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    SegmentInfo::new(
        Arc::clone(dir),
        Version::LATEST,
        min_version,
        name.to_string(),
        10000,
        false,
        false,
        codec,
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )
}

/// `createRandomFile(dir, name, size, segId)` (`protected static`): creates
/// a file of the specified size with random data.
pub fn create_random_file(dir: &dyn Directory, name: &str, size: i32, seg_id: &[u8]) -> Result<()> {
    let rnd = LuceneTestCase::random();
    let mut os = dir.create_output(name, &*random_io_context())?;
    let written = (|| -> Result<()> {
        codec_util::write_index_header(os.as_mut(), "Foo", 0, seg_id, "suffix")?;
        for _ in 0..size {
            let b = rnd.next_int_bound(256) as u8;
            os.write_byte(b)?;
        }
        codec_util::write_footer(os.as_mut())
    })();
    close_output(os.as_mut(), written)
}

/// The try-with-resources of an `IndexOutput`: closed on every exit, the
/// body's failure first.
fn close_output(os: &mut dyn IndexOutput, body: Result<()>) -> Result<()> {
    let closed = os.close();
    body?;
    closed
}

/// `createSequenceFile(dir, name, start, size, segID, segSuffix)`
/// (`protected static`): creates a file of the specified size with
/// sequential data. The first byte is written as the start byte provided.
/// All subsequent bytes are computed as start + offset where offset is the
/// number of the byte.
pub fn create_sequence_file(
    dir: &dyn Directory,
    name: &str,
    start: u8,
    size: i32,
    seg_id: &[u8],
    seg_suffix: &str,
) -> Result<()> {
    let mut os = dir.create_output(name, &*random_io_context())?;
    let written = (|| -> Result<()> {
        codec_util::write_index_header(os.as_mut(), "Foo", 0, seg_id, seg_suffix)?;
        let mut start = start;
        for _ in 0..size {
            os.write_byte(start)?;
            start = start.wrapping_add(1);
        }
        codec_util::write_footer(os.as_mut())
    })();
    close_output(os.as_mut(), written)
}

/// `assertSameStreams(msg, expected, test)` (`protected static`).
pub fn assert_same_streams(
    msg: &str,
    expected: &mut dyn IndexInput,
    test: &mut dyn IndexInput,
) -> Result<()> {
    Assert::assert_equals_msg(
        Some(&format!("{msg} length")),
        expected.length(),
        test.length(),
    );
    Assert::assert_equals_msg(
        Some(&format!("{msg} position")),
        expected.file_pointer(),
        test.file_pointer(),
    );

    let mut expected_buffer = vec![0u8; 512];
    let mut test_buffer = vec![0u8; expected_buffer.len()];

    let mut remainder = expected.length() - expected.file_pointer();
    while remainder > 0 {
        let read_len = remainder.min(expected_buffer.len() as i64) as usize;
        expected.read_bytes(&mut expected_buffer, 0, read_len)?;
        test.read_bytes(&mut test_buffer, 0, read_len)?;
        assert_equal_arrays(
            &format!("{msg}, remainder {remainder}"),
            &expected_buffer,
            &test_buffer,
            0,
            read_len,
        );
        remainder -= read_len as i64;
    }
    Ok(())
}

/// `assertSameStreams(msg, expected, actual, seekTo)` (`protected static`).
pub fn assert_same_streams_at(
    msg: &str,
    expected: &mut dyn IndexInput,
    actual: &mut dyn IndexInput,
    seek_to: i64,
) -> Result<()> {
    if seek_to >= 0 && seek_to < expected.length() {
        expected.seek(seek_to)?;
        actual.seek(seek_to)?;
        assert_same_streams(&format!("{msg}, seek(mid)"), expected, actual)?;
    }
    Ok(())
}

/// `assertSameSeekBehavior(msg, expected, actual)` (`protected static`).
pub fn assert_same_seek_behavior(
    msg: &str,
    expected: &mut dyn IndexInput,
    actual: &mut dyn IndexInput,
) -> Result<()> {
    // seek to 0
    let mut point = 0;
    assert_same_streams_at(&format!("{msg}, seek(0)"), expected, actual, point)?;

    // seek to middle
    point = expected.length() / 2;
    assert_same_streams_at(&format!("{msg}, seek(mid)"), expected, actual, point)?;

    // seek to end - 2
    point = expected.length() - 2;
    assert_same_streams_at(&format!("{msg}, seek(end-2)"), expected, actual, point)?;

    // seek to end - 1
    point = expected.length() - 1;
    assert_same_streams_at(&format!("{msg}, seek(end-1)"), expected, actual, point)?;

    // seek to the end
    point = expected.length();
    assert_same_streams_at(&format!("{msg}, seek(end)"), expected, actual, point)?;

    // seek past end
    point = expected.length() + 1;
    assert_same_streams_at(&format!("{msg}, seek(end+1)"), expected, actual, point)
}

/// `assertEqualArrays(msg, expected, test, start, len)` (`protected static`).
pub fn assert_equal_arrays(msg: &str, expected: &[u8], test: &[u8], start: usize, len: usize) {
    for i in start..len {
        Assert::assert_equals_msg(
            Some(&format!("{msg} {i}")),
            i64::from(expected[i] as i8),
            i64::from(test[i] as i8),
        );
    }
}

/// `createLargeCFS(dir)` (`protected static`): setup a large compound file
/// with a number of components, each of which is a sequential file (so that
/// we can easily tell that we are reading in the right byte). The methods
/// sets up 20 files - `_123.0` to `_123.19`, the size of each file is 1000
/// bytes.
pub fn create_large_cfs(dir: &Arc<dyn Directory>) -> Result<Box<dyn CompoundDirectory>> {
    let mut files = Vec::new();
    let si = new_segment_info(dir, "_123")?;
    for i in 0..20 {
        create_sequence_file(&**dir, &format!("_123.f{i}"), 0, 2000, &si.id(), "suffix")?;
        files.push(format!("_123.f{i}"));
    }

    si.set_files(files)?;
    write_compound(&**dir, &si, &*DEFAULT_IO_CONTEXT)?;
    compound_reader(&**dir, &si)
}

/// Writes the test file of the read-only tests: `writeInt(3)`, no header.
fn write_int_file(dir: &dyn Directory, testfile: &str) -> Result<()> {
    let mut out = dir.create_output(testfile, &*DEFAULT_IO_CONTEXT)?;
    let written = out.write_int(3);
    close_output(out.as_mut(), written)
}

/// Writes `CodecUtil.writeIndexHeader(out, "Foo", 0, id, "suffix");
/// out.writeInt(3); CodecUtil.writeFooter(out);` in a try-with-resources.
fn write_header_int_footer(
    dir: &dyn Directory,
    testfile: &str,
    id: &[u8],
    context: &dyn IOContext,
) -> Result<()> {
    let mut out = dir.create_output(testfile, context)?;
    let written = (|| -> Result<()> {
        codec_util::write_index_header(out.as_mut(), "Foo", 0, id, "suffix")?;
        out.write_int(3)?;
        codec_util::write_footer(out.as_mut())
    })();
    close_output(out.as_mut(), written)
}

/// The body every read-only test shares: an empty compound file over `dir`,
/// on which `op` must throw `UnsupportedOperationException`.
fn assert_read_only(
    write_test_file: bool,
    op: impl FnOnce(&dyn CompoundDirectory, &str) -> Result<()>,
) -> Result<()> {
    let testfile = "_123.test";

    let mut dir = LuceneTestCase::new_directory();
    {
        let directory = dir.as_directory();
        if write_test_file {
            write_int_file(&*directory, testfile)?;
        }

        let si = new_segment_info(&directory, "_123")?;
        si.set_files(Vec::<String>::new())?;
        write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
        let mut cfs = compound_reader(&*directory, &si)?;
        expect_throws_lucene(
            "java.lang.UnsupportedOperationException",
            is_unsupported_operation,
            || op(&*cfs, testfile),
        );

        cfs.close()?;
        drop(directory);
    }
    dir.close()
}

/// Two inputs opened on `_123.f11` and `_123.f3`: the shared body of
/// `testRandomAccess` and `testRandomAccessClones`.
fn assert_independent_positions(
    e1: &mut dyn IndexInput,
    e2: &mut dyn IndexInput,
    a1: &mut dyn IndexInput,
    a2: &mut dyn IndexInput,
) -> Result<()> {
    let byte = |b: u8| i64::from(b as i8);

    // Seek the first pair
    e1.seek(100)?;
    a1.seek(100)?;
    Assert::assert_equals(100, e1.file_pointer());
    Assert::assert_equals(100, a1.file_pointer());
    let mut be1 = e1.read_byte()?;
    let mut ba1 = a1.read_byte()?;
    Assert::assert_equals(byte(be1), byte(ba1));

    // Now seek the second pair
    e2.seek(1027)?;
    a2.seek(1027)?;
    Assert::assert_equals(1027, e2.file_pointer());
    Assert::assert_equals(1027, a2.file_pointer());
    let mut be2 = e2.read_byte()?;
    let mut ba2 = a2.read_byte()?;
    Assert::assert_equals(byte(be2), byte(ba2));

    // Now make sure the first one didn't move
    Assert::assert_equals(101, e1.file_pointer());
    Assert::assert_equals(101, a1.file_pointer());
    be1 = e1.read_byte()?;
    ba1 = a1.read_byte()?;
    Assert::assert_equals(byte(be1), byte(ba1));

    // Now more the first one again, past the buffer length
    e1.seek(1910)?;
    a1.seek(1910)?;
    Assert::assert_equals(1910, e1.file_pointer());
    Assert::assert_equals(1910, a1.file_pointer());
    be1 = e1.read_byte()?;
    ba1 = a1.read_byte()?;
    Assert::assert_equals(byte(be1), byte(ba1));

    // Now make sure the second set didn't move
    Assert::assert_equals(1028, e2.file_pointer());
    Assert::assert_equals(1028, a2.file_pointer());
    be2 = e2.read_byte()?;
    ba2 = a2.read_byte()?;
    Assert::assert_equals(byte(be2), byte(ba2));

    // Move the second set back, again cross the buffer size
    e2.seek(17)?;
    a2.seek(17)?;
    Assert::assert_equals(17, e2.file_pointer());
    Assert::assert_equals(17, a2.file_pointer());
    be2 = e2.read_byte()?;
    ba2 = a2.read_byte()?;
    Assert::assert_equals(byte(be2), byte(ba2));

    // Finally, make sure the first set didn't move
    // Now make sure the first one didn't move
    Assert::assert_equals(1911, e1.file_pointer());
    Assert::assert_equals(1911, a1.file_pointer());
    be1 = e1.read_byte()?;
    ba1 = a1.read_byte()?;
    Assert::assert_equals(byte(be1), byte(ba1));
    Ok(())
}

/// Writes `_123.xyz` of `testCorruptFilesAreCaught` / `testCheckIntegrity`:
/// a header, 1024 bytes and a footer whose checksum is the output's plus
/// `checksum_delta`.
fn write_with_footer_checksum(
    dir: &dyn Directory,
    sub_file: &str,
    id: &[u8],
    checksum_delta: i64,
) -> Result<()> {
    let mut os = dir.create_output(sub_file, &*random_io_context())?;
    let written = (|| -> Result<()> {
        codec_util::write_index_header(os.as_mut(), "Foo", 0, id, "suffix")?;
        for i in 0..1024 {
            os.write_byte(i as u8)?;
        }

        codec_util::write_be_int(os.as_mut(), FOOTER_MAGIC)?;
        codec_util::write_be_int(os.as_mut(), 0)?;
        let checksum = os.checksum()?;
        codec_util::write_be_long(os.as_mut(), checksum.wrapping_add(checksum_delta))
    })();
    close_output(os.as_mut(), written)
}

// -----------------------------------------------------------------------------
// The members
// -----------------------------------------------------------------------------

/// Port of the members of `BaseCompoundFormatTestCase`.
pub trait BaseCompoundFormatTestCase: BaseIndexFileFormatTestCase {
    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn compound_add_random_fields(&self, doc: &mut Document) {
        doc.add(Box::new(
            StoredField::new_string(
                "foobar",
                TestUtil::random_simple_string(&*LuceneTestCase::random()),
            )
            .unwrap_or_else(|e| super::base_index_file_format_test_case::rethrow(e)),
        ));
    }

    /// `testMergeStability()`, overridden.
    fn compound_test_merge_stability(&self) -> Result<()> {
        LuceneTestCase::assume_true("test does not work with CFS", true);
        Ok(())
    }

    /// `testEmpty()`: test that empty CFS is empty.
    fn test_empty(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();

            let si = new_segment_info(&directory, "_123")?;
            si.set_files(Vec::<String>::new())?;
            write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
            let mut cfs = compound_reader(&*directory, &si)?;
            Assert::assert_equals(0, cfs.list_all()?.len() as i64);
            cfs.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testSingleFile()`: this test creates compound file based on a single
    /// file. Files of different sizes are tested: 0, 1, 10, 100 bytes.
    fn test_single_file(&self) -> Result<()> {
        let data = [0, 1, 10, 100];
        for (i, &size) in data.iter().enumerate() {
            let testfile = format!("_{i}.test");
            let mut dir = LuceneTestCase::new_directory();
            {
                let directory = dir.as_directory();
                let si = new_segment_info(&directory, &format!("_{i}"))?;
                create_sequence_file(&*directory, &testfile, 0, size, &si.id(), "suffix")?;

                si.set_files([testfile.clone()])?;
                write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
                let mut cfs = compound_reader(&*directory, &si)?;

                let mut expected = directory.open_input(&testfile, &*random_io_context())?;
                let mut actual = cfs.open_input(&testfile, &*random_io_context())?;
                assert_same_streams(&testfile, expected.as_mut(), actual.as_mut())?;
                assert_same_seek_behavior(&testfile, expected.as_mut(), actual.as_mut())?;
                expected.close()?;
                actual.close()?;
                cfs.close()?;
                drop(directory);
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testTwoFiles()`: this test creates compound file based on two files.
    fn test_two_files(&self) -> Result<()> {
        let files = ["_123.d1".to_string(), "_123.d2".to_string()];
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let si = new_segment_info(&directory, "_123")?;
            create_sequence_file(&*directory, &files[0], 0, 15, &si.id(), "suffix")?;
            create_sequence_file(&*directory, &files[1], 0, 114, &si.id(), "suffix")?;

            si.set_files(files.clone())?;
            write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
            let mut cfs = compound_reader(&*directory, &si)?;

            for file in &files {
                let mut expected = directory.open_input(file, &*random_io_context())?;
                let mut actual = cfs.open_input(file, &*random_io_context())?;
                assert_same_streams(file, expected.as_mut(), actual.as_mut())?;
                assert_same_seek_behavior(file, expected.as_mut(), actual.as_mut())?;
                expected.close()?;
                actual.close()?;
            }

            cfs.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testDoubleClose()`: test that a second call to close() behaves
    /// according to Closeable.
    fn test_double_close(&self) -> Result<()> {
        let testfile = "_123.test";

        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let si = new_segment_info(&directory, "_123")?;
            write_header_int_footer(&*directory, testfile, &si.id(), &*DEFAULT_IO_CONTEXT)?;

            si.set_files([testfile.to_string()])?;
            write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
            let mut cfs = compound_reader(&*directory, &si)?;
            Assert::assert_equals(1, cfs.list_all()?.len() as i64);
            cfs.close()?;
            cfs.close()?; // second close should not throw exception
            drop(directory);
        }
        dir.close()
    }

    /// `testPassIOContext()`: LUCENE-5724: things like NRTCachingDir rely upon
    /// IOContext being properly passed down.
    fn test_pass_io_context(&self) -> Result<()> {
        let testfile = "_123.test";
        let my_context: &dyn IOContext = &*DEFAULT_IO_CONTEXT;

        let dir = Arc::new(OwningTestDirectory::new(
            LuceneTestCase::new_directory(),
            Some(context_address(my_context)),
        ));
        let directory = Arc::clone(&dir) as Arc<dyn Directory>;
        let si = new_segment_info(&directory, "_123")?;
        write_header_int_footer(&*directory, testfile, &si.id(), my_context)?;

        si.set_files([testfile.to_string()])?;
        write_compound(&*directory, &si, my_context)?;
        drop(si);
        drop(directory);
        close_shared(dir)
    }

    /// `testLargeCFS()`: LUCENE-5724: actually test we play nice with
    /// NRTCachingDir and massive file.
    fn test_large_cfs(&self) -> Result<()> {
        let testfile = "_123.test";
        let context = flush_io_context(FlushInfo::new(0, 512 * 1024 * 1024));

        let fs = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
        let dir = Arc::new(NRTCachingDirectory::new(
            Box::new(OwningTestDirectory::new(fs, None)),
            2.0,
            25.0,
        ));
        let directory = Arc::clone(&dir) as Arc<dyn Directory>;

        let si = new_segment_info(&directory, "_123")?;
        let mut out = directory.create_output(testfile, &*context)?;
        let written = (|| -> Result<()> {
            codec_util::write_index_header(out.as_mut(), "Foo", 0, &si.id(), "suffix")?;
            let bytes = vec![0u8; 512];
            for _ in 0..1024 * 1024 {
                out.write_bytes(&bytes, 0, bytes.len())?;
            }
            codec_util::write_footer(out.as_mut())
        })();
        close_output(out.as_mut(), written)?;

        si.set_files([testfile.to_string()])?;
        write_compound(&*directory, &si, &*context)?;

        drop(si);
        drop(directory);
        close_shared(dir)
    }

    /// `testListAll()`: just tests that we can open all files returned by
    /// listAll.
    fn test_list_all(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            // riw should sometimes create docvalues fields, etc
            let riw = RandomIndexWriter::new_with_mock_analyzer(
                &*LuceneTestCase::random(),
                Arc::clone(&directory),
            )?;
            // these fields should sometimes get term vectors, etc
            let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
            let body_field = LuceneTestCase::new_text_field("body", "", Store::NO);
            for i in 0..100 {
                let mut doc = Document::new();
                doc.add(Box::new(rucene::document::Field::new(
                    id_field.name(),
                    i.to_string(),
                    id_field.field_type().clone(),
                )?));
                doc.add(Box::new(rucene::document::Field::new(
                    body_field.name(),
                    TestUtil::random_unicode_string(&*LuceneTestCase::random()),
                    body_field.field_type().clone(),
                )?));
                riw.add_document(&doc)?;
                if LuceneTestCase::random().next_int_bound(7) == 0 {
                    riw.commit()?;
                }
            }
            riw.close()?;
            drop(riw);
            let infos = SegmentInfos::read_latest_commit(Arc::clone(&directory))?;
            for si in infos.iter() {
                if si.info.get_use_compound_file() {
                    let mut cfs_dir = compound_reader(&*directory, &si.info)?;
                    let listed = (|| -> Result<()> {
                        for cfs_file in cfs_dir.list_all()? {
                            let mut cfs_in = cfs_dir.open_input(&cfs_file, &*DEFAULT_IO_CONTEXT)?;
                            cfs_in.close()?;
                        }
                        Ok(())
                    })();
                    let closed = cfs_dir.close();
                    listed?;
                    closed?;
                }
            }
            drop(directory);
        }
        dir.close()
    }

    /// `testCreateOutputDisabled()`: test that cfs reader is read-only.
    fn test_create_output_disabled(&self) -> Result<()> {
        assert_read_only(false, |cfs, _| {
            cfs.create_output("bogus", &*DEFAULT_IO_CONTEXT).map(|_| ())
        })
    }

    /// `testDeleteFileDisabled()`: test that cfs reader is read-only.
    fn test_delete_file_disabled(&self) -> Result<()> {
        assert_read_only(true, |cfs, testfile| cfs.delete_file(testfile))
    }

    /// `testRenameFileDisabled()`: test that cfs reader is read-only.
    fn test_rename_file_disabled(&self) -> Result<()> {
        assert_read_only(true, |cfs, testfile| cfs.rename(testfile, "bogus"))
    }

    /// `testSyncDisabled()`: test that cfs reader is read-only.
    fn test_sync_disabled(&self) -> Result<()> {
        assert_read_only(true, |cfs, testfile| cfs.sync(&[testfile.to_string()]))
    }

    /// `testMakeLockDisabled()`: test that cfs reader is read-only.
    fn test_make_lock_disabled(&self) -> Result<()> {
        assert_read_only(true, |cfs, _| cfs.obtain_lock("foobar").map(|_| ()))
    }

    /// `testRandomFiles()`: this test creates a compound file based on a
    /// large number of files of various length. The file content is
    /// generated randomly. The sizes range from 0 to 1Mb. Some of the sizes
    /// are selected to test the buffering logic in the file reading code.
    /// For this the chunk variable is set to the length of the buffer used
    /// internally by the compound file logic.
    fn test_random_files(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            // Setup the test segment
            let segment = "_123";
            let chunk = 1024; // internal buffer size used by the stream
            let si = new_segment_info(&directory, "_123")?;
            let seg_id = si.id();
            let d = &*directory;
            create_random_file(d, &format!("{segment}.zero"), 0, &seg_id)?;
            create_random_file(d, &format!("{segment}.one"), 1, &seg_id)?;
            create_random_file(d, &format!("{segment}.ten"), 10, &seg_id)?;
            create_random_file(d, &format!("{segment}.hundred"), 100, &seg_id)?;
            create_random_file(d, &format!("{segment}.big1"), chunk, &seg_id)?;
            create_random_file(d, &format!("{segment}.big2"), chunk - 1, &seg_id)?;
            create_random_file(d, &format!("{segment}.big3"), chunk + 1, &seg_id)?;
            create_random_file(d, &format!("{segment}.big4"), 3 * chunk, &seg_id)?;
            create_random_file(d, &format!("{segment}.big5"), 3 * chunk - 1, &seg_id)?;
            create_random_file(d, &format!("{segment}.big6"), 3 * chunk + 1, &seg_id)?;
            create_random_file(d, &format!("{segment}.big7"), 1000 * chunk, &seg_id)?;

            let mut files = Vec::new();
            for file in directory.list_all()? {
                if file.starts_with(segment) {
                    files.push(file);
                }
            }

            si.set_files(files.clone())?;
            write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
            let mut cfs = compound_reader(&*directory, &si)?;

            for file in &files {
                let mut check = directory.open_input(file, &*random_io_context())?;
                let mut test = cfs.open_input(file, &*random_io_context())?;
                assert_same_streams(file, check.as_mut(), test.as_mut())?;
                assert_same_seek_behavior(file, check.as_mut(), test.as_mut())?;
                test.close()?;
                check.close()?;
            }
            cfs.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testManySubFiles()`: make sure we don't somehow use more than 1
    /// descriptor when reading a CFS with many subs.
    fn test_many_sub_files(&self) -> Result<()> {
        let mock = LuceneTestCase::new_mock_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "CFSManySubFiles",
        ));
        let directory = mock.as_directory();

        let file_count = LuceneTestCase::at_least(500);

        let mut files = Vec::new();
        let si = new_segment_info(&directory, "_123")?;
        for file_idx in 0..file_count {
            let file = format!("_123.{file_idx}");
            files.push(file.clone());
            let mut out = directory.create_output(&file, &*random_io_context())?;
            let written = (|| -> Result<()> {
                codec_util::write_index_header(out.as_mut(), "Foo", 0, &si.id(), "suffix")?;
                out.write_byte(file_idx as u8)?;
                codec_util::write_footer(out.as_mut())
            })();
            close_output(out.as_mut(), written)?;
        }

        Assert::assert_equals(0, mock.get_file_handle_count());

        si.set_files(files)?;
        write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
        let mut cfs = compound_reader(&*directory, &si)?;

        let mut ins: Vec<Box<dyn IndexInput>> = Vec::with_capacity(file_count as usize);
        for file_idx in 0..file_count {
            let mut r#in = cfs.open_input(&format!("_123.{file_idx}"), &*random_io_context())?;
            codec_util::check_index_header(r#in.as_mut(), "Foo", 0, 0, &si.id(), "suffix")?;
            ins.push(r#in);
        }

        Assert::assert_equals(1, mock.get_file_handle_count());

        for (file_idx, r#in) in ins.iter_mut().enumerate() {
            Assert::assert_equals(
                i64::from(file_idx as u8 as i8),
                i64::from(r#in.read_byte()? as i8),
            );
        }

        Assert::assert_equals(1, mock.get_file_handle_count());

        for r#in in &mut ins {
            r#in.close()?;
        }
        drop(ins);
        cfs.close()?;

        drop(directory);
        drop(si);
        TestDirectory::Mock(mock).close()
    }

    /// `testClonedStreamsClosing()`.
    fn test_cloned_streams_closing(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let mut cr = create_large_cfs(&directory)?;

            // basic clone
            let mut expected = directory.open_input("_123.f11", &*random_io_context())?;

            let mut one = cr.open_input("_123.f11", &*random_io_context())?;

            let mut two = one.clone_input()?;

            assert_same_streams("basic clone one", expected.as_mut(), one.as_mut())?;
            expected.seek(0)?;
            assert_same_streams("basic clone two", expected.as_mut(), two.as_mut())?;

            // Now close the compound reader
            cr.close()?;
            expected.close()?;
            drop((one, two));
            drop(directory);
        }
        dir.close()
    }

    /// `testRandomAccess()`: this test opens two files from a compound
    /// stream and verifies that their file positions are independent of each
    /// other.
    fn test_random_access(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let mut cr = create_large_cfs(&directory)?;

            // Open two files
            let mut e1 = directory.open_input("_123.f11", &*random_io_context())?;
            let mut e2 = directory.open_input("_123.f3", &*random_io_context())?;

            let mut a1 = cr.open_input("_123.f11", &*random_io_context())?;
            let mut a2 = directory.open_input("_123.f3", &*random_io_context())?;

            assert_independent_positions(e1.as_mut(), e2.as_mut(), a1.as_mut(), a2.as_mut())?;

            e1.close()?;
            e2.close()?;
            a1.close()?;
            a2.close()?;
            cr.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testRandomAccessClones()`: this test opens two files from a compound
    /// stream and verifies that their file positions are independent of each
    /// other.
    fn test_random_access_clones(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let mut cr = create_large_cfs(&directory)?;

            // Open two files
            let mut e1 = cr.open_input("_123.f11", &*random_io_context())?;
            let mut e2 = cr.open_input("_123.f3", &*random_io_context())?;

            let mut a1 = e1.clone_input()?;
            let mut a2 = e2.clone_input()?;

            assert_independent_positions(e1.as_mut(), e2.as_mut(), a1.as_mut(), a2.as_mut())?;

            e1.close()?;
            e2.close()?;
            a1.close()?;
            a2.close()?;
            cr.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testFileNotFound()`.
    fn test_file_not_found(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let mut cr = create_large_cfs(&directory)?;

            // Open bogus file
            expect_throws_lucene("java.io.IOException", LuceneError::is_io_exception, || {
                cr.open_input("bogus", &*random_io_context())
            });

            cr.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testReadPastEOF()`.
    fn test_read_past_eof(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let mut cr = create_large_cfs(&directory)?;
            let mut is = cr.open_input("_123.f2", &*random_io_context())?;
            let length = is.length();
            is.seek(length - 10)?;
            let mut b = vec![0u8; 100];
            is.read_bytes(&mut b, 0, 10)?;

            // Single byte read past end of file
            expect_throws_lucene("java.io.IOException", LuceneError::is_io_exception, || {
                is.read_byte()
            });

            is.seek(length - 10)?;

            // Block read past end of file
            expect_throws_lucene("java.io.IOException", LuceneError::is_io_exception, || {
                is.read_bytes(&mut b, 0, 50)
            });

            is.close()?;
            cr.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testResourceNameInsideCompoundFile()`: LUCENE-6311: make sure the
    /// resource name inside a compound file confesses that it's inside a
    /// compound file.
    fn test_resource_name_inside_compound_file(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let sub_file = "_123.xyz";
            let si = new_segment_info(&directory, "_123")?;
            create_sequence_file(&*directory, sub_file, 0, 10, &si.id(), "suffix")?;

            si.set_files([sub_file.to_string()])?;
            write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT)?;
            let mut cfs = compound_reader(&*directory, &si)?;
            let r#in = cfs.open_input(sub_file, &*DEFAULT_IO_CONTEXT)?;
            let desc = r#in.to_java_string();
            Assert::assert_true_msg(
                &format!("resource description hides that it's inside a compound file: {desc}"),
                desc.contains(&format!("[slice={sub_file}]")),
            );
            drop(r#in);
            cfs.close()?;
            drop(directory);
        }
        dir.close()
    }

    /// `testMissingCodecHeadersAreCaught()`.
    fn test_missing_codec_headers_are_caught(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let sub_file = "_123.xyz";

            // missing codec header
            let mut os = directory.create_output(sub_file, &*random_io_context())?;
            let written = (|| -> Result<()> {
                for i in 0..1024 {
                    os.write_byte(i as u8)?;
                }
                Ok(())
            })();
            close_output(os.as_mut(), written)?;

            let si = new_segment_info(&directory, "_123")?;
            si.set_files([sub_file.to_string()])?;
            let e = expect_throws_lucene(
                "org.apache.lucene.index.CorruptIndexException",
                is_corrupt_index,
                || write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT),
            );
            Assert::assert_true(exception_message(&e).contains("codec header mismatch"));
            drop(directory);
        }
        dir.close()
    }

    /// `testCorruptFilesAreCaught()`.
    fn test_corrupt_files_are_caught(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let sub_file = "_123.xyz";

            // wrong checksum
            let si = new_segment_info(&directory, "_123")?;
            // write footer w/ wrong checksum
            write_with_footer_checksum(&*directory, sub_file, &si.id(), 1)?;

            si.set_files([sub_file.to_string()])?;
            let e = expect_throws_lucene(
                "org.apache.lucene.index.CorruptIndexException",
                is_corrupt_index,
                || write_compound(&*directory, &si, &*DEFAULT_IO_CONTEXT),
            );
            Assert::assert_true(
                exception_message(&e).contains("checksum failed (hardware problem?)"),
            );
            drop(directory);
        }
        dir.close()
    }

    /// `testCheckIntegrity()`.
    fn test_check_integrity(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let directory = dir.as_directory();
            let sub_file = "_123.xyz";
            let si = new_segment_info(&directory, "_123")?;
            write_with_footer_checksum(&*directory, sub_file, &si.id(), 0)?;

            si.set_files([sub_file.to_string()])?;

            let write_tracking_dir = FileTrackingDirectoryWrapper::new(Arc::clone(&directory));
            write_compound(&write_tracking_dir, &si, &*DEFAULT_IO_CONTEXT)?;
            let created_files: HashSet<String> = write_tracking_dir.get_files();

            let read_tracking_dir = ReadBytesDirectoryWrapper::new(Arc::clone(&directory));
            let mut compound_dir = compound_reader(&read_tracking_dir, &si)?;
            compound_dir.check_integrity()?;
            let read_bytes = read_tracking_dir.get_read_bytes();
            Assert::assert_equals(
                JavaStringSet(created_files),
                JavaStringSet(read_bytes.keys().cloned().collect()),
            );
            for (file, bits) in &read_bytes {
                let mut set = bits.clone();
                let len = set.length();
                set.flip_range(0, len);
                let next = set.next_set_bit(0);
                Assert::assert_equals_msg(
                    Some(&format!("Byte at offset {next} of {file} was not read")),
                    NO_MORE_DOCS,
                    next,
                );
            }
            compound_dir.close()?;
            drop(directory);
        }
        dir.close()
    }
}

// -----------------------------------------------------------------------------
// OwningTestDirectory
// -----------------------------------------------------------------------------

/// The address of an `IOContext`, what `assertSame` compares.
fn context_address(context: &dyn IOContext) -> usize {
    context as *const dyn IOContext as *const () as usize
}

/// Closes a directory once every other holder of it is gone (Java's
/// `dir.close()` on the wrapper the test built).
fn close_shared<D: Directory + 'static>(dir: Arc<D>) -> Result<()> {
    match Arc::try_unwrap(dir) {
        Ok(mut dir) => dir.close(),
        Err(_) => Err(LuceneError::illegal_state(
            "the directory is still referenced".to_string(),
        )),
    }
}

/// A `FilterDirectory` over a test directory that owns it (see the module
/// docs): every call goes to the test directory, and `close()` closes it.
/// With `expected_context` set, `createOutput` asserts it receives that
/// very `IOContext`, as the anonymous `FilterDirectory` of
/// `testPassIOContext` does.
struct OwningTestDirectory {
    /// The owned directory as `Arc<dyn Directory>` (`FilterDirectory.in`);
    /// released by `close()` before the owner closes, so the owner's
    /// wrapper is no longer shared.
    r#in: Option<Arc<dyn Directory>>,
    owner: Mutex<Option<TestDirectory>>,
    expected_context: Option<usize>,
}

impl OwningTestDirectory {
    fn new(owner: TestDirectory, expected_context: Option<usize>) -> Self {
        OwningTestDirectory {
            r#in: Some(owner.as_directory()),
            owner: Mutex::new(Some(owner)),
            expected_context,
        }
    }

    /// `in`: the owned directory, `AlreadyClosedException` once closed.
    fn delegate(&self) -> Result<&Arc<dyn Directory>> {
        self.r#in
            .as_ref()
            .ok_or_else(|| LuceneError::already_closed("this Directory is closed"))
    }

    /// Closes the owned test directory.
    fn close_owned(&mut self) -> Result<()> {
        self.r#in = None;
        let owner = self
            .owner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match owner {
            Some(mut owner) => owner.close(),
            None => Ok(()),
        }
    }
}

impl Directory for OwningTestDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.delegate()?.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.delegate()?.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.delegate()?.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if let Some(expected) = self.expected_context {
            if expected != context_address(context) {
                Assert::fail(&format!(
                    "expected same:<{expected:#x}> was not:<{:#x}>",
                    context_address(context)
                ));
            }
        }
        self.delegate()?.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.delegate()?.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.delegate()?.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.delegate()?.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.delegate()?.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.delegate()?.open_input(name, context)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.delegate()?.open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.delegate()?.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.close_owned()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.delegate()?.get_pending_deletions()
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        self.r#in.as_deref()
    }
}

impl fmt::Display for OwningTestDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FilterDirectory({})",
            self.r#in
                .as_ref()
                .map_or("closed", |d| d.directory_type_name())
        )
    }
}
