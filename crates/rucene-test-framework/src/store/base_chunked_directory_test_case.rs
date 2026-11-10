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

//! Port of `org.apache.lucene.tests.store.BaseChunkedDirectoryTestCase`:
//! base class for Directories that "chunk" the input into blocks. It tries
//! to explicitly chunk with different sizes and test boundary conditions
//! around the chunks.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseChunkedDirectoryTestCase`]. Its override of `getDirectory(Path)` is
//! [`chunked_get_directory`], which a subclass's
//! [`BaseDirectoryTestCase::get_directory`] calls, as Java's subclass
//! inherits it; its override of `testSliceOfSlice()` is registered on the
//! class.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, Store};
use rucene::error::Result;
use rucene::store::java_nio_file::Path;
use rucene::store::{Directory, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::{BytesRef, GroupVIntUtil};

use super::base_directory_test_case::{directory_test, BaseDirectoryTestCase};
use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::store::MockDirectoryWrapper;
use crate::util::java_exceptions::expect_throws;
use crate::util::lucene_test_case::TEST_NIGHTLY;
use crate::util::{LuceneTestCase, TestUtil};

/// The declaration of
/// `org.apache.lucene.tests.store.BaseChunkedDirectoryTestCase` (`public
/// abstract class BaseChunkedDirectoryTestCase extends
/// BaseDirectoryTestCase`), with its tests and its override of
/// `testSliceOfSlice()`.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseChunkedDirectoryTestCase) -> Result<()>| {
            directory_test(name, move |c| body(chunked(c)))
        };
        Class::builder("org.apache.lucene.tests.store.BaseChunkedDirectoryTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_directory_test_case::class())
            .method(t("testGroupVIntMultiBlocks", |c| {
                c.test_group_v_int_multi_blocks()
            }))
            .method(t("testCloneClose", |c| c.test_clone_close()))
            .method(t("testCloneSliceClose", |c| c.test_clone_slice_close()))
            .method(t("testSeekZero", |c| c.test_seek_zero()))
            .method(t("testSeekSliceZero", |c| c.test_seek_slice_zero()))
            .method(t("testSeekEnd", |c| c.test_seek_end()))
            .method(t("testSeekSliceEnd", |c| c.test_seek_slice_end()))
            .method(t("testSeeking", |c| c.test_seeking()))
            .method(t("testSlicedSeeking", |c| c.test_sliced_seeking()))
            .method(t("testSliceOfSlice", |c| c.chunked_test_slice_of_slice()))
            .method(t("testRandomChunkSizes", |c| c.test_random_chunk_sizes()))
            .method(t("testBytesCrossBoundary", |c| {
                c.test_bytes_cross_boundary()
            }))
            .method(t("testLittleEndianLongsCrossBoundary", |c| {
                c.test_little_endian_longs_cross_boundary()
            }))
            .method(t("testLittleEndianFloatsCrossBoundary", |c| {
                c.test_little_endian_floats_cross_boundary()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseChunkedDirectoryTestCase`.
fn chunked(c: &dyn BaseDirectoryTestCase) -> &dyn BaseChunkedDirectoryTestCase {
    c.as_chunked().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.store.BaseChunkedDirectoryTestCase",
        )
        .throw()
    })
}

/// `BaseChunkedDirectoryTestCase.getDirectory(Path path)` (`@Override`):
/// `getDirectory(path, 1 << TestUtil.nextInt(random(), 10, 20))`.
pub fn chunked_get_directory(
    c: &dyn BaseChunkedDirectoryTestCase,
    path: &Path,
) -> Result<Box<dyn Directory>> {
    let max_chunk_size = 1 << TestUtil::next_int(&*LuceneTestCase::random(), 10, 20);
    c.get_directory_chunked(path, max_chunk_size)
}

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

fn temp_dir(prefix: &str) -> Path {
    LuceneTestCase::create_temp_dir_prefix(prefix)
}

/// `assertEquals(new BytesRef(expected...), new BytesRef(actual))`.
fn assert_bytes_ref_equals(expected: &[u8], actual: &[u8]) {
    if expected != actual {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            BytesRef::new(expected.to_vec()),
            BytesRef::new(actual.to_vec())
        ));
    }
}

/// Port of the members of `BaseChunkedDirectoryTestCase`.
pub trait BaseChunkedDirectoryTestCase: BaseDirectoryTestCase {
    /// `getDirectory(Path path, int maxChunkSize)`: creates a new directory
    /// with the specified max chunk size.
    fn get_directory_chunked(&self, path: &Path, max_chunk_size: i32)
        -> Result<Box<dyn Directory>>;

    /// `testGroupVIntMultiBlocks()`.
    fn test_group_v_int_multi_blocks(&self) -> Result<()> {
        let max_chunk_size = random().next_int_range(64, 512);
        let mut dir =
            self.get_directory_chunked(&LuceneTestCase::create_temp_dir(), max_chunk_size)?;
        let result = self.do_test_group_v_int(&*dir, 10, 1, 31, 1024);
        result.and(dir.close())
    }

    /// `testCloneClose()`.
    fn test_clone_close(&self) -> Result<()> {
        let mut dir = self.get_directory(&temp_dir("testCloneClose"))?;
        let mut io = dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
        let mut values = [0, 7, 11, 9];
        io.write_v_int(5)?;
        io.write_group_vints(&values, values.len())?;
        io.close()?;
        let mut one = dir.open_input("bytes", &*DEFAULT_IO_CONTEXT)?;
        let mut two = one.clone_input()?;
        let mut three = two.clone_input()?; // clone of clone
        two.close()?;
        Assert::assert_equals(5, one.read_v_int()?);
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            two.read_v_int()
        });
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            GroupVIntUtil::read_group_vints_input(&mut *two, &mut values, 4)
        });
        Assert::assert_equals(5, three.read_v_int()?);
        one.close()?;
        three.close()?;
        dir.close()
    }

    /// `testCloneSliceClose()`.
    fn test_clone_slice_close(&self) -> Result<()> {
        let mut dir = self.get_directory(&temp_dir("testCloneSliceClose"))?;
        let mut io = dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
        let mut values = [0, 7, 11, 9];
        io.write_int(1)?;
        io.write_int(2)?;
        io.write_group_vints(&values, values.len())?; // will write 5 bytes
        io.close()?;
        let mut slicer = dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
        let mut one = slicer.slice("first int", 0, 4 + 5)?;
        let mut two = slicer.slice("second int", 4, 4)?;
        one.close()?;
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            one.read_int()
        });
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            GroupVIntUtil::read_group_vints_input(&mut *one, &mut values, 4)
        });
        Assert::assert_equals(2, two.read_int()?);
        // reopen a new slice "another":
        let mut another = slicer.slice("first int", 0, 4)?;
        Assert::assert_equals(1, another.read_int()?);
        another.close()?;
        two.close()?;
        slicer.close()?;
        dir.close()
    }

    /// `testSeekZero()`.
    fn test_seek_zero(&self) -> Result<()> {
        let upto = if *TEST_NIGHTLY { 31 } else { 3 };
        for i in 0..upto {
            let mut dir = self.get_directory_chunked(&temp_dir("testSeekZero"), 1 << i)?;
            let mut io =
                dir.create_output("zeroBytes", &*LuceneTestCase::new_io_context(&*random()))?;
            io.close()?;
            let mut ii =
                dir.open_input("zeroBytes", &*LuceneTestCase::new_io_context(&*random()))?;
            ii.seek(0)?;
            ii.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSeekSliceZero()`.
    fn test_seek_slice_zero(&self) -> Result<()> {
        let upto = if *TEST_NIGHTLY { 31 } else { 3 };
        for i in 0..upto {
            let mut dir = self.get_directory_chunked(&temp_dir("testSeekSliceZero"), 1 << i)?;
            let mut io =
                dir.create_output("zeroBytes", &*LuceneTestCase::new_io_context(&*random()))?;
            io.close()?;
            let mut slicer =
                dir.open_input("zeroBytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut ii = slicer.slice("zero-length slice", 0, 0)?;
            ii.seek(0)?;
            ii.close()?;
            slicer.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSeekEnd()`.
    fn test_seek_end(&self) -> Result<()> {
        for i in 0..17 {
            let mut dir = self.get_directory_chunked(&temp_dir("testSeekEnd"), 1 << i)?;
            let mut io =
                dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut bytes = vec![0u8; 1 << i];
            random().next_bytes(&mut bytes);
            io.write_bytes(&bytes, 0, bytes.len())?;
            io.close()?;
            let mut ii = dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut actual = vec![0u8; 1 << i];
            let n = actual.len();
            ii.read_bytes(Some(&mut actual), 0, n)?;
            assert_bytes_ref_equals(&bytes, &actual);
            ii.seek(1 << i)?;
            ii.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSeekSliceEnd()`.
    fn test_seek_slice_end(&self) -> Result<()> {
        for i in 0..17 {
            let mut dir = self.get_directory_chunked(&temp_dir("testSeekSliceEnd"), 1 << i)?;
            let mut io =
                dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut bytes = vec![0u8; 1 << i];
            random().next_bytes(&mut bytes);
            io.write_bytes(&bytes, 0, bytes.len())?;
            io.close()?;
            let mut slicer =
                dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut ii = slicer.slice("full slice", 0, bytes.len() as i64)?;
            let mut actual = vec![0u8; 1 << i];
            let n = actual.len();
            ii.read_bytes(Some(&mut actual), 0, n)?;
            assert_bytes_ref_equals(&bytes, &actual);
            ii.seek(1 << i)?;
            ii.close()?;
            slicer.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSeeking()`.
    fn test_seeking(&self) -> Result<()> {
        let num_iters = if *TEST_NIGHTLY { 10 } else { 1 };
        for i in 0..num_iters {
            let mut dir = self.get_directory_chunked(&temp_dir("testSeeking"), 1 << i)?;
            let mut io =
                dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut bytes = vec![0u8; 1 << (i + 1)]; // make sure we switch buffers
            random().next_bytes(&mut bytes);
            io.write_bytes(&bytes, 0, bytes.len())?;
            io.close()?;
            let mut ii = dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut actual = vec![0u8; 1 << (i + 1)]; // first read all bytes
            let n = actual.len();
            ii.read_bytes(Some(&mut actual), 0, n)?;
            assert_bytes_ref_equals(&bytes, &actual);
            for slice_start in 0..bytes.len() {
                for slice_length in 0..bytes.len() - slice_start {
                    let mut slice = vec![0u8; slice_length];
                    ii.seek(slice_start as i64)?;
                    let n = slice.len();
                    ii.read_bytes(Some(&mut slice), 0, n)?;
                    assert_bytes_ref_equals(
                        &bytes[slice_start..slice_start + slice_length],
                        &slice,
                    );
                }
            }
            ii.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSlicedSeeking()`: note instead of seeking to offset and reading
    /// length, this opens slices at the the various offset+length and just
    /// does readBytes.
    fn test_sliced_seeking(&self) -> Result<()> {
        let num_iters = if *TEST_NIGHTLY { 10 } else { 1 };
        for i in 0..num_iters {
            let mut dir = self.get_directory_chunked(&temp_dir("testSlicedSeeking"), 1 << i)?;
            let mut io =
                dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut bytes = vec![0u8; 1 << (i + 1)]; // make sure we switch buffers
            random().next_bytes(&mut bytes);
            io.write_bytes(&bytes, 0, bytes.len())?;
            io.close()?;
            let mut ii = dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut actual = vec![0u8; 1 << (i + 1)]; // first read all bytes
            let n = actual.len();
            ii.read_bytes(Some(&mut actual), 0, n)?;
            ii.close()?;
            assert_bytes_ref_equals(&bytes, &actual);
            let mut slicer =
                dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            for slice_start in 0..bytes.len() {
                for slice_length in 0..bytes.len() - slice_start {
                    assert_slice(&bytes, &*slicer, 0, slice_start, slice_length)?;
                }
            }
            slicer.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testSliceOfSlice()` (`@Override`).
    fn chunked_test_slice_of_slice(&self) -> Result<()> {
        let upto = if *TEST_NIGHTLY { 10 } else { 8 };
        for i in 0..upto {
            let mut dir = self.get_directory_chunked(&temp_dir("testSliceOfSlice"), 1 << i)?;
            let mut io =
                dir.create_output("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut bytes = vec![0u8; 1 << (i + 1)]; // make sure we switch buffers
            random().next_bytes(&mut bytes);
            io.write_bytes(&bytes, 0, bytes.len())?;
            io.close()?;
            let mut ii = dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let mut actual = vec![0u8; 1 << (i + 1)]; // first read all bytes
            let n = actual.len();
            ii.read_bytes(Some(&mut actual), 0, n)?;
            ii.close()?;
            assert_bytes_ref_equals(&bytes, &actual);
            let mut outer_slicer =
                dir.open_input("bytes", &*LuceneTestCase::new_io_context(&*random()))?;
            let outer_slice_start = random().next_int_bound(bytes.len() as i32 / 2) as usize;
            let outer_slice_length =
                random().next_int_bound((bytes.len() - outer_slice_start) as i32) as usize;
            let mut inner_slicer = outer_slicer.slice(
                "parentBytesSlice",
                outer_slice_start as i64,
                outer_slice_length as i64,
            )?;
            for slice_start in 0..outer_slice_length {
                for slice_length in 0..outer_slice_length - slice_start {
                    assert_slice(
                        &bytes,
                        &*inner_slicer,
                        outer_slice_start,
                        slice_start,
                        slice_length,
                    )?;
                }
            }
            inner_slicer.close()?;
            outer_slicer.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testRandomChunkSizes()`.
    fn test_random_chunk_sizes(&self) -> Result<()> {
        let num = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(10)
        } else {
            3
        };
        for _ in 0..num {
            let chunk_size = TestUtil::next_int(&*random(), 20, 100);
            self.assert_chunking(&*random(), chunk_size)?;
        }
        Ok(())
    }

    /// `assertChunking(Random random, int chunkSize)` (`private`).
    fn assert_chunking(&self, random: &dyn Random, chunk_size: i32) -> Result<()> {
        let path = LuceneTestCase::create_temp_dir_prefix(&format!("mmap{chunk_size}"));
        let chunked_dir = self.get_directory_chunked(&path, chunk_size)?;
        let mut dir = MockDirectoryWrapper::new(random, Arc::from(chunked_dir));
        let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
            MockAnalyzer::new(random),
        )));
        config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
        let writer = RandomIndexWriter::new_with_config(random, dir.as_directory(), config)?;
        let mut doc = Document::new();
        let docid = LuceneTestCase::new_string_field("docid", "0", Store::YES);
        let junk = LuceneTestCase::new_string_field("junk", "", Store::YES);
        doc.add(Box::new(docid));
        doc.add(Box::new(junk));

        let num_docs = 100;
        for i in 0..num_docs {
            doc.get_field_mut_as::<Field>("docid")
                .expect("docid")
                .set_string_value(i.to_string())?;
            doc.get_field_mut_as::<Field>("junk")
                .expect("junk")
                .set_string_value(TestUtil::random_unicode_string(random))?;
            writer.add_document(&doc)?;
        }
        let reader = writer.get_reader()?;
        writer.close()?;
        drop(writer);

        let stored_fields = reader.stored_fields()?;
        let num_asserts = LuceneTestCase::at_least(100);
        for _ in 0..num_asserts {
            let doc_id = random.next_int_bound(num_docs);
            let stored = stored_fields
                .document(doc_id)?
                .get_field("docid")
                .and_then(|f| f.string_value());
            Assert::assert_equals_str(None, Some(&doc_id.to_string()), stored.as_deref());
        }
        reader.close()?;
        drop(stored_fields);
        drop(reader);
        dir.close()
    }

    /// `testBytesCrossBoundary()`.
    fn test_bytes_cross_boundary(&self) -> Result<()> {
        let num = if *TEST_NIGHTLY {
            TestUtil::next_int(&*random(), 100, 1000)
        } else {
            TestUtil::next_int(&*random(), 50, 100)
        };
        let mut bytes = vec![0u8; num as usize];
        random().next_bytes(&mut bytes);
        let mut dir = self.get_directory_chunked(&temp_dir("testBytesCrossBoundary"), 16)?;
        let result = (|| -> Result<()> {
            let mut out = dir.create_output(
                "bytesCrossBoundary",
                &*LuceneTestCase::new_io_context(&*random()),
            )?;
            let written = out.write_bytes(&bytes, 0, bytes.len());
            written.and(out.close())?;
            let mut input = dir.open_input(
                "bytesCrossBoundary",
                &*LuceneTestCase::new_io_context(&*random()),
            )?;
            let body = (|| -> Result<()> {
                let mut slice = input.random_access_slice(0, input.length())?;
                Assert::assert_equals(input.length(), slice.length());
                self.assert_bytes(&mut *slice, &bytes, 0)?;

                // subslices
                for offset in 1..bytes.len() {
                    let mut subslice =
                        input.random_access_slice(offset as i64, input.length() - offset as i64)?;
                    Assert::assert_equals(input.length() - offset as i64, subslice.length());
                    self.assert_bytes(&mut *subslice, &bytes, offset)?;
                }

                // with padding
                for i in 1..7 {
                    let name = format!("bytes-{i}");
                    let mut o =
                        dir.create_output(&name, &*LuceneTestCase::new_io_context(&*random()))?;
                    let mut junk = vec![0u8; i];
                    random().next_bytes(&mut junk);
                    o.write_bytes(&junk, 0, junk.len())?;
                    input.seek(0)?;
                    let len = input.length();
                    o.copy_bytes(&mut *input, len)?;
                    o.close()?;
                    let mut padded =
                        dir.open_input(&name, &*LuceneTestCase::new_io_context(&*random()))?;
                    let mut whole =
                        padded.random_access_slice(i as i64, padded.length() - i as i64)?;
                    Assert::assert_equals(padded.length() - i as i64, whole.length());
                    self.assert_bytes(&mut *whole, &bytes, 0)?;
                    padded.close()?;
                }
                Ok(())
            })();
            body.and(input.close())
        })();
        result.and(dir.close())
    }

    /// `testLittleEndianLongsCrossBoundary()`.
    fn test_little_endian_longs_cross_boundary(&self) -> Result<()> {
        let mut dir =
            self.get_directory_chunked(&temp_dir("testLittleEndianLongsCrossBoundary"), 16)?;
        let result = (|| -> Result<()> {
            let mut out = dir.create_output(
                "littleEndianLongs",
                &*LuceneTestCase::new_io_context(&*random()),
            )?;
            out.write_byte(2)?;
            out.write_long(3)?;
            out.write_long(i64::MAX)?;
            out.write_long(-3)?;
            out.close()?;
            let mut input = dir.open_input(
                "littleEndianLongs",
                &*LuceneTestCase::new_io_context(&*random()),
            )?;
            Assert::assert_equals(25, input.length());
            Assert::assert_equals(2, input.read_byte()?);
            let mut l = [0i64; 4];
            input.read_longs(Some(&mut l), 1, 3)?;
            crate::index::base_index_file_format_test_case::assert_array_equals(
                &[0i64, 3, i64::MAX, -3],
                &l,
            );
            Assert::assert_equals(25, input.file_pointer());
            input.close()
        })();
        result.and(dir.close())
    }

    /// `testLittleEndianFloatsCrossBoundary()`.
    fn test_little_endian_floats_cross_boundary(&self) -> Result<()> {
        let mut dir = self.get_directory_chunked(&temp_dir("testFloatsCrossBoundary"), 8)?;
        let result = (|| -> Result<()> {
            let mut out =
                dir.create_output("Floats", &*LuceneTestCase::new_io_context(&*random()))?;
            out.write_byte(2)?;
            out.write_int(3f32.to_bits() as i32)?;
            out.write_int(f32::MAX.to_bits() as i32)?;
            out.write_int((-3f32).to_bits() as i32)?;
            out.close()?;
            let mut input =
                dir.open_input("Floats", &*LuceneTestCase::new_io_context(&*random()))?;
            Assert::assert_equals(13, input.length());
            Assert::assert_equals(2, input.read_byte()?);
            let mut ff = [0f32; 4];
            input.read_floats(Some(&mut ff), 1, 3)?;
            for (e, a) in [0f32, 3.0, f32::MAX, -3.0].iter().zip(ff.iter()) {
                Assert::assert_equals_float_msg(None, *e, *a, 0.0);
            }
            Assert::assert_equals(13, input.file_pointer());
            input.close()
        })();
        result.and(dir.close())
    }
}

/// `assertSlice(byte[] bytes, IndexInput slicer, int outerSliceStart, int
/// sliceStart, int sliceLength)` (`private`).
fn assert_slice(
    bytes: &[u8],
    slicer: &dyn IndexInput,
    outer_slice_start: usize,
    slice_start: usize,
    slice_length: usize,
) -> Result<()> {
    let mut slice = vec![0u8; slice_length];
    let mut input = slicer.slice("bytesSlice", slice_start as i64, slice.len() as i64)?;
    let n = slice.len();
    input.read_bytes(Some(&mut slice), 0, n)?;
    input.close()?;
    let from = outer_slice_start + slice_start;
    assert_bytes_ref_equals(&bytes[from..from + slice_length], &slice);
    Ok(())
}
