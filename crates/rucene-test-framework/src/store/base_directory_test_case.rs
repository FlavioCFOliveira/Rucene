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

//! Port of `org.apache.lucene.tests.store.BaseDirectoryTestCase`: base class
//! for `Directory` implementations.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseDirectoryTestCase`]. A subclass implements `getDirectory(Path)` and
//! builds its instances with [`DirectoryTestInstance::new`].
//!
//! **The directory a test owns.** Java's `getDirectory` returns a `Directory`
//! the test closes (try-with-resources); here it is a `Box<dyn Directory>`,
//! closed with `close(&mut self)`. A test that shares it with threads or with
//! a reader turns it into an `Arc` and closes it once the sharers are gone.
//! [`OwnedTestDirectory`] makes a `LuceneTestCase.newDirectory()` result such
//! a box, transparent to every question asked of it.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testSetOfStrings`, `testMapOfStrings`: Java checks that the returned
//!   set/map rejects `add`/`put` with `UnsupportedOperationException`; the
//!   port's `JavaStringSet`/`JavaStringMap` have no mutators at all, so the
//!   immutability is a property of the type and the three `expectThrows`
//!   calls of each test are not made.
//! * `assertThat(…, hasItemInArray(…))`, `containsString`,
//!   `containsInAnyOrder`: the Hamcrest port has no such matchers; each is a
//!   direct check that fails with Hamcrest's `Expected: …` text.
//! * `testIndexOutputToString`: Java's `IndexOutput.toString()` is its
//!   resource description, which the port answers with
//!   `resource_description()`.
//! * `testIsLoaded`'s `wrapperClasses`: Java collects the classes of the
//!   `FilterDirectory` chain and tests their names (`DirectIO`) and
//!   `NRTCachingDirectory.class.isAssignableFrom`; the port walks the same
//!   chain through `as_filter_directory()` and asks each directory its type
//!   name and whether it is an `NRTCachingDirectory` (`as_any()`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, LazyLock};

use randomizedtesting::generators::{RandomBytes, RandomNumbers};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_file_names::segment_file_name;
use rucene::store::java_nio_file::{Files, Path};
use rucene::store::mmap::MMapDirectory;
use rucene::store::{
    BufferedChecksum, BufferedChecksumIndexInput, ChecksumIndexInput, DataInput, Directory,
    FileSwitchDirectory, FilterDirectory, IOContext, IndexInput, IndexOutput, Lock,
    NRTCachingDirectory, RandomAccessInput, DEFAULT_IO_CONTEXT,
};
use rucene::util::packed::PackedInts;
use rucene::util::{GroupVIntUtil, IOUtils};

use crate::index::base_index_file_format_test_case::assert_array_equals;
use crate::mockfile::ExtrasFS;
use crate::store::{MockDirectoryWrapper, RawDirectoryWrapper, Throttling};
use crate::util::java_exceptions::{expect_throws, expect_throws_any_of, is_instance};
use crate::util::lucene_test_case::{TestDirectory, TEST_NIGHTLY};
use crate::util::{LuceneTestCase, TestUtil};

/// The declaration of `org.apache.lucene.tests.store.BaseDirectoryTestCase`
/// (`public abstract class BaseDirectoryTestCase extends LuceneTestCase`)
/// with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseDirectoryTestCase) -> Result<()>) -> Method {
            directory_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.store.BaseDirectoryTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testCopyFrom", |c| c.test_copy_from()))
            .method(t("testRename", |c| c.test_rename()))
            .method(t("testDeleteFile", |c| c.test_delete_file()))
            .method(t("testByte", |c| c.test_byte()))
            .method(t("testShort", |c| c.test_short()))
            .method(t("testInt", |c| c.test_int()))
            .method(t("testLong", |c| c.test_long()))
            .method(t("testAlignedLittleEndianLongs", |c| {
                c.test_aligned_little_endian_longs()
            }))
            .method(t("testUnalignedLittleEndianLongs", |c| {
                c.test_unaligned_little_endian_longs()
            }))
            .method(t("testLittleEndianLongsUnderflow", |c| {
                c.test_little_endian_longs_underflow()
            }))
            .method(t("testAlignedInts", |c| c.test_aligned_ints()))
            .method(t("testUnalignedInts", |c| c.test_unaligned_ints()))
            .method(t("testIntsUnderflow", |c| c.test_ints_underflow()))
            .method(t("testAlignedFloats", |c| c.test_aligned_floats()))
            .method(t("testUnalignedFloats", |c| c.test_unaligned_floats()))
            .method(t("testFloatsUnderflow", |c| c.test_floats_underflow()))
            .method(t("testString", |c| c.test_string()))
            .method(t("testVInt", |c| c.test_v_int()))
            .method(t("testVLong", |c| c.test_v_long()))
            .method(t("testZInt", |c| c.test_z_int()))
            .method(t("testZLong", |c| c.test_z_long()))
            .method(t("testSetOfStrings", |c| c.test_set_of_strings()))
            .method(t("testMapOfStrings", |c| c.test_map_of_strings()))
            .method(t("testChecksum", |c| c.test_checksum()))
            .method(t("testDetectClose", |c| c.test_detect_close()))
            .method(t("testThreadSafetyInListAll", |c| {
                c.test_thread_safety_in_list_all()
            }))
            .method(t("testFileExistsInListAfterCreated", |c| {
                c.test_file_exists_in_list_after_created()
            }))
            .method(t("testSeekToEOFThenBack", |c| {
                c.test_seek_to_eof_then_back()
            }))
            .method(t("testIllegalEOF", |c| c.test_illegal_eof()))
            .method(t("testSeekPastEOF", |c| c.test_seek_past_eof()))
            .method(t("testSliceOutOfBounds", |c| c.test_slice_out_of_bounds()))
            .method(t("testNoDir", |c| c.test_no_dir()))
            .method(t("testCopyBytes", |c| c.test_copy_bytes()))
            .method(t("testCopyBytesWithThreads", |c| {
                c.test_copy_bytes_with_threads()
            }))
            .method(t("testFsyncDoesntCreateNewFiles", |c| {
                c.test_fsync_doesnt_create_new_files()
            }))
            .method(t("testRandomLong", |c| c.test_random_long()))
            .method(t("testRandomInt", |c| c.test_random_int()))
            .method(t("testRandomShort", |c| c.test_random_short()))
            .method(t("testRandomByte", |c| c.test_random_byte()))
            .method(t("testSliceOfSlice", |c| c.test_slice_of_slice()))
            .method(t("testLargeWrites", |c| c.test_large_writes()))
            .method(t("testIndexOutputToString", |c| {
                c.test_index_output_to_string()
            }))
            .method(t("testDoubleCloseOutput", |c| c.test_double_close_output()))
            .method(t("testDoubleCloseInput", |c| c.test_double_close_input()))
            .method(t("testCreateTempOutput", |c| c.test_create_temp_output()))
            .method(t("testCreateOutputForExistingFile", |c| {
                c.test_create_output_for_existing_file()
            }))
            .method(t("testSeekToEndOfFile", |c| c.test_seek_to_end_of_file()))
            .method(t("testSeekBeyondEndOfFile", |c| {
                c.test_seek_beyond_end_of_file()
            }))
            .method(t("testPendingDeletions", |c| c.test_pending_deletions()))
            .method(t("testListAllIsSorted", |c| c.test_list_all_is_sorted()))
            .method(t("testDataTypes", |c| c.test_data_types()))
            .method(t("testGroupVIntOverflow", |c| {
                c.test_group_v_int_overflow()
            }))
            .method(t("testGroupVInt", |c| c.test_group_v_int()))
            .method(t("testPrefetch", |c| c.test_prefetch()))
            .method(t("testPrefetchOnSlice", |c| c.test_prefetch_on_slice()))
            .method(t("testIsLoaded", |c| c.test_is_loaded()))
            .method(t("testIsLoadedOnSlice", |c| c.test_is_loaded_on_slice()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseDirectoryTestCase` subclass: `body` runs on the
/// instance's subclass, and an error it returns is thrown as the Java
/// exception it stands for.
pub fn directory_test(
    name: &str,
    body: impl Fn(&dyn BaseDirectoryTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut DirectoryTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| crate::util::java_exceptions::rethrow_java(e))
        },
    )
}

/// The instance of a `BaseDirectoryTestCase` subclass the runner creates.
pub struct DirectoryTestInstance {
    /// The subclass, as the trait its members are reached through.
    pub case: Arc<dyn BaseDirectoryTestCase>,
}

impl DirectoryTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseDirectoryTestCase>(case: T) -> Self {
        DirectoryTestInstance {
            case: Arc::new(case),
        }
    }
}

/// `newIOContext(random())`.
fn io_context() -> Box<dyn IOContext> {
    LuceneTestCase::new_io_context(&*LuceneTestCase::random())
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `createTempDir(prefix)`.
fn temp_dir(prefix: &str) -> Path {
    LuceneTestCase::create_temp_dir_prefix(prefix)
}

/// `assertThat(dir.listAll(), hasItemInArray(file))` (`present`) or its
/// `not(...)`.
fn assert_has_item(files: &[String], file: &str, present: bool) {
    let has = files.iter().any(|f| f == file);
    if has != present {
        let not = if present { "" } else { "not " };
        Assert::fail(&format!(
            "\nExpected: {not}an array containing \"{file}\"\n     but: was {files:?}"
        ));
    }
}

/// `value(idx)` (`private static`).
fn value(idx: i32) -> u8 {
    ((idx % 256) * (1 + (idx / 256))) as u8
}

/// The bytes of `bytes[from..to]` as Java `byte`s, for an array comparison.
fn java_bytes(bytes: &[u8]) -> Vec<i8> {
    bytes.iter().map(|&b| b as i8).collect()
}

/// The try-with-resources of a directory: `body` runs on it, then it is
/// closed; the body's failure comes first.
pub fn with_directory(
    mut dir: Box<dyn Directory>,
    body: impl FnOnce(&dyn Directory) -> Result<()>,
) -> Result<()> {
    // Java's try-with-resources closes the resource on any throwable, an
    // assumption failure included, then rethrows it.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&*dir)));
    let closed = dir.close();
    match result {
        Ok(result) => result.and(closed),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Closes a directory shared through an `Arc` once every sharer is gone.
///
/// # Errors
///
/// What `close()` throws; `IllegalStateException` while a sharer remains.
pub fn close_shared(mut dir: Arc<dyn Directory>) -> Result<()> {
    match Arc::get_mut(&mut dir) {
        Some(dir) => dir.close(),
        None => Err(LuceneError::illegal_state(
            "the directory is still shared and cannot be closed",
        )),
    }
}

/// The try-with-resources of an input or output: closed after `body`.
fn with_input(
    mut input: Box<dyn IndexInput>,
    body: impl FnOnce(&mut dyn IndexInput) -> Result<()>,
) -> Result<()> {
    // Java's try-with-resources closes the resource on any throwable, an
    // assumption failure included, then rethrows it.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut *input)));
    let closed = input.close();
    match result {
        Ok(result) => result.and(closed),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn with_output(
    mut output: Box<dyn IndexOutput>,
    body: impl FnOnce(&mut dyn IndexOutput) -> Result<()>,
) -> Result<()> {
    // Java's try-with-resources closes the resource on any throwable, an
    // assumption failure included, then rethrows it.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut *output)));
    let closed = output.close();
    match result {
        Ok(result) => result.and(closed),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Port of the members of `BaseDirectoryTestCase`: the hook a subclass
/// implements and the tests (each overridable, as a subclass may override
/// one).
pub trait BaseDirectoryTestCase: Send + Sync + 'static {
    /// `getDirectory(Path path)`: a subclass returns the Directory to be
    /// tested; if it's an FS-based directory it should point to the
    /// specified path, else it can ignore it.
    fn get_directory(&self, path: &Path) -> Result<Box<dyn Directory>>;

    /// The subclass as a `BaseChunkedDirectoryTestCase`, when it is one.
    fn as_chunked(
        &self,
    ) -> Option<&dyn super::base_chunked_directory_test_case::BaseChunkedDirectoryTestCase> {
        None
    }

    /// `testCopyFrom()`.
    fn test_copy_from(&self) -> Result<()> {
        {
            let source = self.get_directory(&temp_dir("testCopy"))?;
            let dest = OwnedTestDirectory::new(LuceneTestCase::new_directory());
            let mut dest: Box<dyn Directory> = Box::new(dest);
            let result = run_copy_from(&*source, &*dest);
            let closed = dest.close();
            let mut source = source;
            let closed_source = source.close();
            result.and(closed).and(closed_source)?;
        }
        {
            let mut source: Box<dyn Directory> =
                Box::new(OwnedTestDirectory::new(LuceneTestCase::new_directory()));
            let mut dest = self.get_directory(&temp_dir("testCopyDestination"))?;
            let result = run_copy_from(&*source, &*dest);
            let closed = dest.close();
            let closed_source = source.close();
            result.and(closed).and(closed_source)
        }
    }

    /// `testRename()`.
    fn test_rename(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testRename"))?, |dir| {
            let mut output = dir.create_output("foobar", &*io_context())?;
            let num_bytes = random().next_int_bound(20000) as usize;
            let mut bytes = vec![0u8; num_bytes];
            random().next_bytes(&mut bytes);
            output.write_bytes(&bytes, 0, bytes.len())?;
            output.close()?;

            dir.rename("foobar", "foobaz")?;

            let mut input = dir.open_input("foobaz", &*io_context())?;
            let mut bytes2 = vec![0u8; num_bytes];
            input.read_bytes(Some(&mut bytes2), 0, num_bytes)?;
            Assert::assert_equals(input.length(), num_bytes as i64);
            input.close()?;

            assert_array_equals(&java_bytes(&bytes), &java_bytes(&bytes2));
            Ok(())
        })
    }

    /// `testDeleteFile()`.
    fn test_delete_file(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testDeleteFile"))?, |dir| {
            let file = "foo.txt";
            assert_has_item(&dir.list_all()?, file, false);

            dir.create_output("foo.txt", &*DEFAULT_IO_CONTEXT)?
                .close()?;
            assert_has_item(&dir.list_all()?, file, true);

            dir.delete_file("foo.txt")?;
            assert_has_item(&dir.list_all()?, file, false);

            expect_throws_any_of(
                &[
                    "java.nio.file.NoSuchFileException",
                    "java.io.FileNotFoundException",
                ],
                || dir.delete_file("foo.txt"),
            );
            Ok(())
        })
    }

    /// `testByte()`.
    fn test_byte(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testByte"))?, |dir| {
            let mut output = dir.create_output("byte", &*io_context())?;
            output.write_byte(128)?;
            output.close()?;

            let mut input = dir.open_input("byte", &*io_context())?;
            Assert::assert_equals(1, input.length());
            Assert::assert_equals(128u8 as i8, input.read_byte()? as i8);
            input.close()
        })
    }

    /// `testShort()`.
    fn test_short(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testShort"))?, |dir| {
            let mut output = dir.create_output("short", &*io_context())?;
            output.write_short(-20)?;
            output.close()?;

            let mut input = dir.open_input("short", &*io_context())?;
            Assert::assert_equals(2, input.length());
            Assert::assert_equals(-20i16, input.read_short()?);
            input.close()
        })
    }

    /// `testInt()`.
    fn test_int(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testInt"))?, |dir| {
            let mut output = dir.create_output("int", &*io_context())?;
            output.write_int(-500)?;
            output.close()?;

            let mut input = dir.open_input("int", &*io_context())?;
            Assert::assert_equals(4, input.length());
            Assert::assert_equals(-500, input.read_int()?);
            input.close()
        })
    }

    /// `testLong()`.
    fn test_long(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testLong"))?, |dir| {
            let mut output = dir.create_output("long", &*io_context())?;
            output.write_long(-5000)?;
            output.close()?;

            let mut input = dir.open_input("long", &*io_context())?;
            Assert::assert_equals(8, input.length());
            Assert::assert_equals(-5000i64, input.read_long()?);
            input.close()
        })
    }

    /// `testAlignedLittleEndianLongs()`.
    fn test_aligned_little_endian_longs(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testAlignedLittleEndianLongs"))?,
            |dir| {
                with_output(
                    dir.create_output("littleEndianLongs", &*io_context())?,
                    |out| {
                        out.write_long(3)?;
                        out.write_long(i64::MAX)?;
                        out.write_long(-3)
                    },
                )?;
                with_input(
                    dir.open_input("littleEndianLongs", &*io_context())?,
                    |input| {
                        Assert::assert_equals(24, input.length());
                        let mut l = [0i64; 4];
                        input.read_longs(Some(&mut l), 1, 3)?;
                        assert_array_equals(&[0i64, 3, i64::MAX, -3], &l);
                        Assert::assert_equals(24, input.file_pointer());
                        Ok(())
                    },
                )
            },
        )
    }

    /// `testUnalignedLittleEndianLongs()`.
    fn test_unaligned_little_endian_longs(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testUnalignedLittleEndianLongs"))?,
            |dir| {
                with_output(
                    dir.create_output("littleEndianLongs", &*io_context())?,
                    |out| {
                        out.write_byte(2)?;
                        out.write_long(3)?;
                        out.write_long(i64::MAX)?;
                        out.write_long(-3)
                    },
                )?;
                with_input(
                    dir.open_input("littleEndianLongs", &*io_context())?,
                    |input| {
                        Assert::assert_equals(25, input.length());
                        Assert::assert_equals(2, input.read_byte()?);
                        let mut l = [0i64; 4];
                        input.read_longs(Some(&mut l), 1, 3)?;
                        assert_array_equals(&[0i64, 3, i64::MAX, -3], &l);
                        Assert::assert_equals(25, input.file_pointer());
                        Ok(())
                    },
                )
            },
        )
    }

    /// `testLittleEndianLongsUnderflow()`.
    fn test_little_endian_longs_underflow(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testLittleEndianLongsUnderflow"))?,
            |dir| {
                let offset = random().next_int_bound(8);
                let length = TestUtil::next_int(&*random(), 1, 16);
                with_output(
                    dir.create_output("littleEndianLongs", &*io_context())?,
                    |out| {
                        let mut b = vec![
                            0u8;
                            (offset + length * 8 - TestUtil::next_int(&*random(), 1, 8))
                                as usize
                        ];
                        random().next_bytes(&mut b);
                        out.write_bytes(&b, 0, b.len())
                    },
                )?;
                with_input(
                    dir.open_input("littleEndianLongs", &*io_context())?,
                    |input| {
                        input.seek(i64::from(offset))?;
                        expect_throws("java.io.EOFException", || {
                            input.read_longs(
                                Some(&mut vec![0i64; length as usize]),
                                0,
                                length as usize,
                            )
                        });
                        Ok(())
                    },
                )
            },
        )
    }

    /// `testAlignedInts()`.
    fn test_aligned_ints(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testAlignedInts"))?, |dir| {
            with_output(dir.create_output("Ints", &*io_context())?, |out| {
                out.write_int(3)?;
                out.write_int(i32::MAX)?;
                out.write_int(-3)
            })?;
            with_input(dir.open_input("Ints", &*io_context())?, |input| {
                Assert::assert_equals(12, input.length());
                let mut i = [0i32; 4];
                input.read_ints(Some(&mut i), 1, 3)?;
                assert_array_equals(&[0, 3, i32::MAX, -3], &i);
                Assert::assert_equals(12, input.file_pointer());
                Ok(())
            })
        })
    }

    /// `testUnalignedInts()`.
    fn test_unaligned_ints(&self) -> Result<()> {
        let padding = random().next_int_bound(3) + 1;
        with_directory(self.get_directory(&temp_dir("testUnalignedInts"))?, |dir| {
            with_output(dir.create_output("Ints", &*io_context())?, |out| {
                for _ in 0..padding {
                    out.write_byte(2)?;
                }
                out.write_int(3)?;
                out.write_int(i32::MAX)?;
                out.write_int(-3)
            })?;
            with_input(dir.open_input("Ints", &*io_context())?, |input| {
                Assert::assert_equals(12 + i64::from(padding), input.length());
                for _ in 0..padding {
                    Assert::assert_equals(2, input.read_byte()?);
                }
                let mut i = [0i32; 4];
                input.read_ints(Some(&mut i), 1, 3)?;
                assert_array_equals(&[0, 3, i32::MAX, -3], &i);
                Assert::assert_equals(12 + i64::from(padding), input.file_pointer());
                Ok(())
            })
        })
    }

    /// `testIntsUnderflow()`.
    fn test_ints_underflow(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testIntsUnderflow"))?, |dir| {
            let offset = random().next_int_bound(4);
            let length = TestUtil::next_int(&*random(), 1, 16);
            with_output(dir.create_output("Ints", &*io_context())?, |out| {
                let mut b = vec![
                    0u8;
                    (offset + length * 4 - TestUtil::next_int(&*random(), 1, 4))
                        as usize
                ];
                random().next_bytes(&mut b);
                out.write_bytes(&b, 0, b.len())
            })?;
            with_input(dir.open_input("Ints", &*io_context())?, |input| {
                input.seek(i64::from(offset))?;
                expect_throws("java.io.EOFException", || {
                    input.read_ints(Some(&mut vec![0i32; length as usize]), 0, length as usize)
                });
                Ok(())
            })
        })
    }

    /// `testAlignedFloats()`.
    fn test_aligned_floats(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testAlignedFloats"))?, |dir| {
            with_output(dir.create_output("Floats", &*io_context())?, |out| {
                out.write_int(3f32.to_bits() as i32)?;
                out.write_int(f32::MAX.to_bits() as i32)?;
                out.write_int((-3f32).to_bits() as i32)
            })?;
            with_input(dir.open_input("Floats", &*io_context())?, |input| {
                Assert::assert_equals(12, input.length());
                let mut ff = [0f32; 4];
                input.read_floats(Some(&mut ff), 1, 3)?;
                assert_float_array_equals(&[0.0, 3.0, f32::MAX, -3.0], &ff);
                Assert::assert_equals(12, input.file_pointer());
                Ok(())
            })
        })
    }

    /// `testUnalignedFloats()`.
    fn test_unaligned_floats(&self) -> Result<()> {
        let padding = random().next_int_bound(3) + 1;
        with_directory(
            self.get_directory(&temp_dir("testUnalignedFloats"))?,
            |dir| {
                with_output(dir.create_output("Floats", &*io_context())?, |out| {
                    for _ in 0..padding {
                        out.write_byte(2)?;
                    }
                    out.write_int(3f32.to_bits() as i32)?;
                    out.write_int(f32::MAX.to_bits() as i32)?;
                    out.write_int((-3f32).to_bits() as i32)
                })?;
                with_input(dir.open_input("Floats", &*io_context())?, |input| {
                    Assert::assert_equals(12 + i64::from(padding), input.length());
                    for _ in 0..padding {
                        Assert::assert_equals(2, input.read_byte()?);
                    }
                    let mut ff = [0f32; 4];
                    input.read_floats(Some(&mut ff), 1, 3)?;
                    assert_float_array_equals(&[0.0, 3.0, f32::MAX, -3.0], &ff);
                    Assert::assert_equals(12 + i64::from(padding), input.file_pointer());
                    Ok(())
                })
            },
        )
    }

    /// `testFloatsUnderflow()`.
    fn test_floats_underflow(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testFloatsUnderflow"))?,
            |dir| {
                let offset = random().next_int_bound(4);
                let length = TestUtil::next_int(&*random(), 1, 16);
                with_output(dir.create_output("Floats", &*io_context())?, |out| {
                    let mut b = vec![
                        0u8;
                        (offset + length * 4 - TestUtil::next_int(&*random(), 1, 4))
                            as usize
                    ];
                    random().next_bytes(&mut b);
                    out.write_bytes(&b, 0, b.len())
                })?;
                with_input(dir.open_input("Floats", &*io_context())?, |input| {
                    input.seek(i64::from(offset))?;
                    expect_throws("java.io.EOFException", || {
                        input.read_floats(
                            Some(&mut vec![0f32; length as usize]),
                            0,
                            length as usize,
                        )
                    });
                    Ok(())
                })
            },
        )
    }

    /// `testString()`.
    fn test_string(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testString"))?, |dir| {
            let mut output = dir.create_output("string", &*io_context())?;
            output.write_string("hello!")?;
            output.close()?;

            let mut input = dir.open_input("string", &*io_context())?;
            Assert::assert_equals("hello!".to_string(), input.read_string()?);
            Assert::assert_equals(7, input.length());
            input.close()
        })
    }

    /// `testVInt()`.
    fn test_v_int(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testVInt"))?, |dir| {
            let mut output = dir.create_output("vint", &*io_context())?;
            output.write_v_int(500)?;
            output.close()?;

            let mut input = dir.open_input("vint", &*io_context())?;
            Assert::assert_equals(2, input.length());
            Assert::assert_equals(500, input.read_v_int()?);
            input.close()
        })
    }

    /// `testVLong()`.
    fn test_v_long(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testVLong"))?, |dir| {
            let mut output = dir.create_output("vlong", &*io_context())?;
            output.write_v_long(i64::MAX)?;
            output.close()?;

            let mut input = dir.open_input("vlong", &*io_context())?;
            Assert::assert_equals(9, input.length());
            Assert::assert_equals(i64::MAX, input.read_v_long()?);
            input.close()
        })
    }

    /// `testZInt()`.
    fn test_z_int(&self) -> Result<()> {
        let mut ints = vec![0i32; random().next_int_bound(10) as usize];
        for v in ints.iter_mut() {
            *v = match random().next_int_bound(3) {
                0 => random().next_int(),
                1 => {
                    if random().next_boolean() {
                        i32::MIN
                    } else {
                        i32::MAX
                    }
                }
                2 => {
                    let sign = if random().next_boolean() { -1 } else { 1 };
                    sign * random().next_int_bound(1024)
                }
                _ => Throwable::new("java.lang.AssertionError", "").throw(),
            };
        }
        with_directory(self.get_directory(&temp_dir("testZInt"))?, |dir| {
            let mut output = dir.create_output("zint", &*io_context())?;
            for &i in &ints {
                output.write_z_int(i)?;
            }
            output.close()?;

            let mut input = dir.open_input("zint", &*io_context())?;
            for &i in &ints {
                Assert::assert_equals(i, input.read_z_int()?);
            }
            Assert::assert_equals(input.length(), input.file_pointer());
            input.close()
        })
    }

    /// `testZLong()`.
    fn test_z_long(&self) -> Result<()> {
        let mut longs = vec![0i64; random().next_int_bound(10) as usize];
        for v in longs.iter_mut() {
            *v = match random().next_int_bound(3) {
                0 => random().next_long(),
                1 => {
                    if random().next_boolean() {
                        i64::MIN
                    } else {
                        i64::MAX
                    }
                }
                2 => {
                    let sign: i64 = if random().next_boolean() { -1 } else { 1 };
                    sign * i64::from(random().next_int_bound(1024))
                }
                _ => Throwable::new("java.lang.AssertionError", "").throw(),
            };
        }
        with_directory(self.get_directory(&temp_dir("testZLong"))?, |dir| {
            let mut output = dir.create_output("zlong", &*io_context())?;
            for &l in &longs {
                output.write_z_long(l)?;
            }
            output.close()?;

            let mut input = dir.open_input("zlong", &*io_context())?;
            for &l in &longs {
                Assert::assert_equals(l, input.read_z_long()?);
            }
            Assert::assert_equals(input.length(), input.file_pointer());
            input.close()
        })
    }

    /// `testSetOfStrings()` (see the module's divergences).
    fn test_set_of_strings(&self) -> Result<()> {
        use rucene::util::java_collections::JavaStringSet;
        with_directory(self.get_directory(&temp_dir("testSetOfStrings"))?, |dir| {
            let as_set = |values: &[&str]| {
                JavaStringSet::from_read(values.iter().map(|s| s.to_string()).collect())
            };
            let mut output = dir.create_output("stringset", &*io_context())?;
            output.write_set_of_strings(&as_set(&["test1", "test2"]))?;
            output.write_set_of_strings(&as_set(&[]))?;
            output.write_set_of_strings(&as_set(&["test3"]))?;
            output.close()?;

            let mut input = dir.open_input("stringset", &*io_context())?;
            let set1 = input.read_set_of_strings()?;
            assert_sets_equal(&["test1", "test2"], &set1);
            let set2 = input.read_set_of_strings()?;
            assert_sets_equal(&[], &set2);
            let set3 = input.read_set_of_strings()?;
            assert_sets_equal(&["test3"], &set3);
            Assert::assert_equals(input.length(), input.file_pointer());
            input.close()
        })
    }

    /// `testMapOfStrings()` (see the module's divergences).
    fn test_map_of_strings(&self) -> Result<()> {
        use rucene::util::java_collections::JavaStringMap;
        let m = JavaStringMap::from_read(vec![
            ("test1".to_string(), "value1".to_string()),
            ("test2".to_string(), "value2".to_string()),
        ]);
        with_directory(self.get_directory(&temp_dir("testMapOfStrings"))?, |dir| {
            let mut output = dir.create_output("stringmap", &*io_context())?;
            output.write_map_of_strings(&m)?;
            output.write_map_of_strings(&JavaStringMap::from_read(Vec::new()))?;
            output.write_map_of_strings(&JavaStringMap::from_read(vec![(
                "key".to_string(),
                "value".to_string(),
            )]))?;
            output.close()?;

            let mut input = dir.open_input("stringmap", &*io_context())?;
            let map1 = input.read_map_of_strings()?;
            assert_maps_equal(&[("test1", "value1"), ("test2", "value2")], &map1);
            let map2 = input.read_map_of_strings()?;
            assert_maps_equal(&[], &map2);
            let map3 = input.read_map_of_strings()?;
            assert_maps_equal(&[("key", "value")], &map3);
            Assert::assert_equals(input.length(), input.file_pointer());
            input.close()
        })
    }

    /// `testChecksum()`.
    fn test_checksum(&self) -> Result<()> {
        let mut expected = BufferedChecksum::new();
        let num_bytes = random().next_int_bound(20000) as usize;
        let mut bytes = vec![0u8; num_bytes];
        random().next_bytes(&mut bytes);
        expected.update_bytes(&bytes, 0, bytes.len())?;

        with_directory(self.get_directory(&temp_dir("testChecksum"))?, |dir| {
            let mut output = dir.create_output("checksum", &*io_context())?;
            output.write_bytes(&bytes, 0, bytes.len())?;
            output.close()?;

            let mut input: Box<BufferedChecksumIndexInput> = dir.open_checksum_input("checksum")?;
            input.skip_bytes(num_bytes as i64)?;
            Assert::assert_equals(expected.get_value(), input.get_checksum()?);
            input.close()
        })
    }

    /// `testDetectClose()`: make sure directory throws AlreadyClosedException
    /// if you try to createOutput after closing.
    fn test_detect_close(&self) -> Result<()> {
        let mut dir = self.get_directory(&temp_dir("testDetectClose"))?;
        dir.close()?;
        expect_throws("org.apache.lucene.store.AlreadyClosedException", || {
            dir.create_output("test", &*io_context())
        });
        Ok(())
    }

    /// `testThreadSafetyInListAll()`.
    fn test_thread_safety_in_list_all(&self) -> Result<()> {
        let dir: Arc<dyn Directory> = Arc::from(self.get_directory(&temp_dir("testThreadSafety"))?);
        let result = (|| -> Result<()> {
            if let Some(any) = dir.as_any() {
                if let Some(mock) = any.downcast_ref::<MockDirectoryWrapper>() {
                    // we are not making a real index, just writing, reading files.
                    mock.set_check_index_on_close(false);
                    // makes this test really slow
                    mock.set_throttling(Throttling::Never);
                } else if let Some(raw) = any.downcast_ref::<RawDirectoryWrapper>() {
                    raw.base().set_check_index_on_close(false);
                }
            }

            let stop = Arc::new(AtomicBool::new(false));
            let writer = {
                let dir = Arc::clone(&dir);
                let stop = Arc::clone(&stop);
                Thread::unnamed(move || {
                    let run = || -> Result<()> {
                        let max = RandomizedTest::random_int_between(500, 1000);
                        for i in 0..max {
                            let file_name = format!("file-{i}");
                            let mut output = dir.create_output(&file_name, &*io_context())?;
                            // Add some lags so that the other thread can read the content of the
                            // directory.
                            std::thread::yield_now();
                            output.close()?;
                            Assert::assert_true(LuceneTestCase::slow_file_exists(
                                &*dir, &file_name,
                            )?);
                        }
                        Ok(())
                    };
                    let outcome = run();
                    stop.store(true, Ordering::SeqCst);
                    if let Err(e) = outcome {
                        Throwable::new(
                            "java.io.UncheckedIOException",
                            crate::util::java_exceptions::java_throwable(&e).to_string(),
                        )
                        .throw()
                    }
                })
            };

            let reader = {
                let dir = Arc::clone(&dir);
                let stop = Arc::clone(&stop);
                Thread::unnamed(move || {
                    let run = || -> Result<()> {
                        let rnd = JdkRandom::new(RandomizedTest::random_long());
                        while !stop.load(Ordering::SeqCst) {
                            let files: Vec<String> = dir
                                .list_all()?
                                .into_iter()
                                .filter(|name| !ExtrasFS::is_extra(name)) // Ignore anything from ExtraFS.
                                .collect();

                            if !files.is_empty() {
                                loop {
                                    let file =
                                        &files[rnd.next_int_bound(files.len() as i32) as usize];
                                    match dir.open_input(file, &*io_context()) {
                                        Ok(mut input) => {
                                            // Just open, nothing else.
                                            input.close()?;
                                        }
                                        Err(e)
                                            if is_instance(
                                                &e,
                                                "java.nio.file.AccessDeniedException",
                                            ) =>
                                        {
                                            // Access denied is allowed for files for which the
                                            // output is still open (MockDirectoryWriter enforces
                                            // this, for example). Since we don't synchronize with
                                            // the writer thread, just ignore it.
                                        }
                                        Err(e) => {
                                            Throwable::with_cause(
                                                "java.io.UncheckedIOException",
                                                Some(format!(
                                                    "Something went wrong when opening: {file}"
                                                )),
                                                crate::util::java_exceptions::java_throwable(&e),
                                            )
                                            .throw();
                                        }
                                    }
                                    if rnd.next_int_bound(3) == 0 {
                                        break; // Sometimes break and list files again.
                                    }
                                }
                            }
                        }
                        Ok(())
                    };
                    if let Err(e) = run() {
                        Throwable::new(
                            "java.io.UncheckedIOException",
                            crate::util::java_exceptions::java_throwable(&e).to_string(),
                        )
                        .throw()
                    }
                })
            };

            reader.start();
            writer.start();

            let joined_writer = writer.join();
            let joined_reader = reader.join();
            if let Err(payload) = joined_writer {
                std::panic::resume_unwind(payload);
            }
            if let Err(payload) = joined_reader {
                std::panic::resume_unwind(payload);
            }
            Ok(())
        })();
        let closed = close_shared(dir);
        result.and(closed)
    }

    /// `testFileExistsInListAfterCreated()`: LUCENE-1468: once we create an
    /// output, we should see it in the dir listing.
    fn test_file_exists_in_list_after_created(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testFileExistsInListAfterCreated"))?,
            |dir| {
                let name = "file";
                dir.create_output(name, &*io_context())?.close()?;
                Assert::assert_true(LuceneTestCase::slow_file_exists(dir, name)?);
                assert_has_item(&dir.list_all()?, name, true);
                Ok(())
            },
        )
    }

    /// `testSeekToEOFThenBack()` (LUCENE-2852).
    fn test_seek_to_eof_then_back(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testSeekToEOFThenBack"))?,
            |dir| {
                let buffer_length = 1024usize;
                let mut o = dir.create_output("out", &*io_context())?;
                let mut bytes = vec![0u8; 3 * buffer_length];
                o.write_bytes(&bytes, 0, bytes.len())?;
                o.close()?;

                let mut i = dir.open_input("out", &*io_context())?;
                i.seek(2 * buffer_length as i64 - 1)?;
                i.seek(3 * buffer_length as i64)?;
                i.seek(buffer_length as i64)?;
                i.read_bytes(Some(&mut bytes), 0, 2 * buffer_length)?;
                i.close()
            },
        )
    }

    /// `testIllegalEOF()` (LUCENE-1196).
    fn test_illegal_eof(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testIllegalEOF"))?, |dir| {
            let mut o = dir.create_output("out", &*io_context())?;
            let b = [0u8; 1024];
            o.write_bytes(&b, 0, 1024)?;
            o.close()?;
            let mut i = dir.open_input("out", &*io_context())?;
            i.seek(1024)?;
            i.close()
        })
    }

    /// `testSeekPastEOF()`.
    fn test_seek_past_eof(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testSeekPastEOF"))?, |dir| {
            let mut o = dir.create_output("out", &*io_context())?;
            let len = random().next_int_bound(2048) as usize;
            let b = vec![0u8; len];
            o.write_bytes(&b, 0, len)?;
            o.close()?;
            let mut i = dir.open_input("out", &*io_context())?;

            // Seeking past EOF should always throw EOFException
            expect_throws("java.io.EOFException", || {
                i.seek(len as i64 + i64::from(RandomizedTest::random_int_between(1, 2048)))
            });

            // Seeking exactly to EOF should never throw any exception.
            i.seek(len as i64)?;

            // But any read following the seek(len) should throw an EOFException.
            expect_throws("java.io.EOFException", || i.read_byte());
            expect_throws("java.io.EOFException", || {
                i.read_bytes(Some(&mut [0u8; 1]), 0, 1)
            });

            i.close()
        })
    }

    /// `testSliceOutOfBounds()`.
    fn test_slice_out_of_bounds(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testSliceOutOfBounds"))?,
            |dir| {
                let mut o = dir.create_output("out", &*io_context())?;
                let len = (random().next_int_bound(2040) + 8) as i64;
                let b = vec![0u8; len as usize];
                o.write_bytes(&b, 0, len as usize)?;
                o.close()?;
                let mut i = dir.open_input("out", &*io_context())?;
                expect_throws("java.lang.IllegalArgumentException", || {
                    i.slice("slice1", 0, len + 1)
                });
                expect_throws("java.lang.IllegalArgumentException", || {
                    i.slice("slice2", -1, len)
                });
                let slice = i.slice("slice3", 4, len / 2)?;
                expect_throws("java.lang.IllegalArgumentException", || {
                    slice.slice("slice3sub", 1, len / 2)
                });
                expect_throws("java.lang.IllegalArgumentException", || {
                    i.slice("slice4", i64::MAX - 1, 10)
                });
                i.close()
            },
        )
    }

    /// `testNoDir()`: LUCENE-3382 -- make sure we get exception if the
    /// directory really does not exist.
    fn test_no_dir(&self) -> Result<()> {
        let temp_dir = temp_dir("doesnotexist");
        IOUtils::rm([&temp_dir])?;
        let dir: Arc<dyn Directory> = Arc::from(self.get_directory(&temp_dir)?);
        expect_throws_any_of(
            &[
                "java.nio.file.NoSuchFileException",
                "org.apache.lucene.index.IndexNotFoundException",
            ],
            || directory_reader::open(Arc::clone(&dir)),
        );
        close_shared(dir)
    }

    /// `testCopyBytes()`.
    fn test_copy_bytes(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testCopyBytes"))?, |dir| {
            let mut out = dir.create_output("test", &*io_context())?;
            let mut bytes = vec![0u8; TestUtil::next_int(&*random(), 1, 77777) as usize];
            let size = TestUtil::next_int(&*random(), 1, 1777777);
            let mut upto = 0;
            let mut byte_upto = 0usize;
            while upto < size {
                bytes[byte_upto] = value(upto);
                byte_upto += 1;
                upto += 1;
                if byte_upto == bytes.len() {
                    out.write_bytes(&bytes, 0, bytes.len())?;
                    byte_upto = 0;
                }
            }

            out.write_bytes(&bytes, 0, byte_upto)?;
            Assert::assert_equals(i64::from(size), out.file_pointer());
            out.close()?;
            Assert::assert_equals(i64::from(size), dir.file_length("test")?);

            // copy from test -> test2
            let mut input = dir.open_input("test", &*io_context())?;

            let mut out = dir.create_output("test2", &*io_context())?;

            upto = 0;
            while upto < size {
                if random().next_boolean() {
                    out.write_byte(input.read_byte()?)?;
                    upto += 1;
                } else {
                    let chunk =
                        TestUtil::next_int(&*random(), 1, bytes.len() as i32).min(size - upto);
                    out.copy_bytes(&mut *input, i64::from(chunk))?;
                    upto += chunk;
                }
            }
            Assert::assert_equals(size, upto);
            out.close()?;
            input.close()?;

            // verify
            let mut in2 = dir.open_input("test2", &*io_context())?;
            upto = 0;
            while upto < size {
                if random().next_boolean() {
                    let v = in2.read_byte()?;
                    Assert::assert_equals(value(upto) as i8, v as i8);
                    upto += 1;
                } else {
                    let limit =
                        TestUtil::next_int(&*random(), 1, bytes.len() as i32).min(size - upto);
                    in2.read_bytes(Some(&mut bytes), 0, limit as usize)?;
                    for &b in bytes.iter().take(limit as usize) {
                        Assert::assert_equals(value(upto) as i8, b as i8);
                        upto += 1;
                    }
                }
            }
            in2.close()?;

            dir.delete_file("test")?;
            dir.delete_file("test2")
        })
    }

    /// `testCopyBytesWithThreads()` (LUCENE-3541).
    fn test_copy_bytes_with_threads(&self) -> Result<()> {
        let d: Arc<dyn Directory> =
            Arc::from(self.get_directory(&temp_dir("testCopyBytesWithThreads"))?);
        let result = (|| -> Result<()> {
            let header_len = 100;
            let data =
                RandomBytes::random_bytes_of_length_between(&*random(), header_len + 1, 10000);

            let mut output = d.create_output("data", &*DEFAULT_IO_CONTEXT)?;
            output.write_bytes(&data, 0, data.len())?;
            output.close()?;

            let mut input = d.open_input("data", &*DEFAULT_IO_CONTEXT)?;
            let mut output_header = d.create_output("header", &*DEFAULT_IO_CONTEXT)?;
            // copy our header
            output_header.copy_bytes(&mut *input, i64::from(header_len))?;
            output_header.close()?;

            // now make N copies of the remaining bytes
            let threads = 10;
            let start = Arc::new(Barrier::new(threads));
            let mut copies = Vec::new();
            for i in 0..threads {
                let mut src = input.clone_input()?;
                let d = Arc::clone(&d);
                let start = Arc::clone(&start);
                let t = Thread::unnamed(move || {
                    let mut run = || -> Result<()> {
                        start.wait();
                        let mut dst = d.create_output(&format!("copy{i}"), &*DEFAULT_IO_CONTEXT)?;
                        let n = src.length() - i64::from(header_len);
                        dst.copy_bytes(&mut *src, n)?;
                        dst.close()
                    };
                    if let Err(e) = run() {
                        Throwable::new(
                            "java.lang.RuntimeException",
                            crate::util::java_exceptions::java_throwable(&e).to_string(),
                        )
                        .throw()
                    }
                });
                t.start();
                copies.push(t);
            }
            for t in copies {
                if let Err(payload) = t.join() {
                    std::panic::resume_unwind(payload);
                }
            }

            for i in 0..threads {
                with_input(
                    d.open_input(&format!("copy{i}"), &*DEFAULT_IO_CONTEXT)?,
                    |copied| {
                        let mut data_copy = vec![0u8; data.len()];
                        data_copy[..header_len as usize]
                            .copy_from_slice(&data[..header_len as usize]);
                        copied.read_bytes(
                            Some(&mut data_copy),
                            header_len as usize,
                            data.len() - header_len as usize,
                        )?;
                        assert_array_equals(&java_bytes(&data), &java_bytes(&data_copy));
                        Ok(())
                    },
                )?;
            }
            input.close()
        })();
        let closed = close_shared(d);
        result.and(closed)
    }

    /// `testFsyncDoesntCreateNewFiles()`: this test backdoors the directory
    /// via the filesystem. so it must actually use the filesystem.
    fn test_fsync_doesnt_create_new_files(&self) -> Result<()> {
        let path = temp_dir("nocreate");
        with_directory(self.get_directory(&path)?, |fsdir| {
            // this test backdoors the directory via the filesystem. so it must be an FSDir (for now)
            if fsdir.as_fs_directory().is_none() {
                LuceneTestCase::assume_true("test only works for FSDirectory subclasses", false);
                return Ok(());
            }

            // create a file
            let mut out = fsdir.create_output("afile", &*io_context())?;
            out.write_string("boo")?;
            out.close()?;

            // delete it in the file system.
            Files::delete(&path.resolve_str("afile"))?;

            let file_count = fsdir.list_all()?.len();

            // fsync it
            expect_throws_any_of(
                &[
                    "java.io.FileNotFoundException",
                    "java.nio.file.NoSuchFileException",
                ],
                || fsdir.sync(&["afile".to_string()]),
            );

            // no new files created
            Assert::assert_equals(file_count, fsdir.list_all()?.len());
            Ok(())
        })
    }

    /// `testRandomLong()`: random access APIs.
    fn test_random_long(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testLongs"))?, |dir| {
            let mut output = dir.create_output("longs", &*io_context())?;
            let num = TestUtil::next_int(&*random(), 50, 3000);
            let mut longs = vec![0i64; num as usize];
            for l in longs.iter_mut() {
                *l = TestUtil::next_long(&*random(), i64::MIN, i64::MAX);
                output.write_long(*l)?;
            }
            output.close()?;

            // slice
            let mut input = dir.open_input("longs", &*io_context())?;
            let mut slice = input.random_access_slice(0, input.length())?;
            Assert::assert_equals(input.length(), slice.length());
            for (i, &l) in longs.iter().enumerate() {
                Assert::assert_equals(l, slice.read_long_at(i as i64 * 8)?);
            }

            // subslices
            for i in 1..longs.len() {
                let offset = i as i64 * 8;
                let mut subslice = input.random_access_slice(offset, input.length() - offset)?;
                Assert::assert_equals(input.length() - offset, subslice.length());
                for j in i..longs.len() {
                    Assert::assert_equals(longs[j], subslice.read_long_at((j - i) as i64 * 8)?);
                }
            }

            // with padding
            for i in 0..7 {
                let name = format!("longs-{i}");
                let mut o = dir.create_output(&name, &*io_context())?;
                let mut junk = vec![0u8; i];
                random().next_bytes(&mut junk);
                o.write_bytes(&junk, 0, junk.len())?;
                input.seek(0)?;
                let len = input.length();
                o.copy_bytes(&mut *input, len)?;
                o.close()?;
                let mut padded = dir.open_input(&name, &*io_context())?;
                let mut whole = padded.random_access_slice(i as i64, padded.length() - i as i64)?;
                Assert::assert_equals(padded.length() - i as i64, whole.length());
                for (j, &l) in longs.iter().enumerate() {
                    Assert::assert_equals(l, whole.read_long_at(j as i64 * 8)?);
                }
                padded.close()?;
            }

            input.close()
        })
    }

    /// `testRandomInt()`.
    fn test_random_int(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testInts"))?, |dir| {
            let mut output = dir.create_output("ints", &*io_context())?;
            let num = TestUtil::next_int(&*random(), 50, 3000);
            let mut ints = vec![0i32; num as usize];
            for v in ints.iter_mut() {
                *v = random().next_int();
                output.write_int(*v)?;
            }
            output.close()?;

            // slice
            let mut input = dir.open_input("ints", &*io_context())?;
            let mut slice = input.random_access_slice(0, input.length())?;
            Assert::assert_equals(input.length(), slice.length());
            for (i, &v) in ints.iter().enumerate() {
                Assert::assert_equals(v, slice.read_int_at(i as i64 * 4)?);
            }

            // subslices
            for i in 1..ints.len() {
                let offset = i as i64 * 4;
                let mut subslice = input.random_access_slice(offset, input.length() - offset)?;
                Assert::assert_equals(input.length() - offset, subslice.length());
                for j in i..ints.len() {
                    Assert::assert_equals(ints[j], subslice.read_int_at((j - i) as i64 * 4)?);
                }
            }

            // with padding
            for i in 0..7 {
                let name = format!("ints-{i}");
                let mut o = dir.create_output(&name, &*io_context())?;
                let mut junk = vec![0u8; i];
                random().next_bytes(&mut junk);
                o.write_bytes(&junk, 0, junk.len())?;
                input.seek(0)?;
                let len = input.length();
                o.copy_bytes(&mut *input, len)?;
                o.close()?;
                let mut padded = dir.open_input(&name, &*io_context())?;
                let mut whole = padded.random_access_slice(i as i64, padded.length() - i as i64)?;
                Assert::assert_equals(padded.length() - i as i64, whole.length());
                for (j, &v) in ints.iter().enumerate() {
                    Assert::assert_equals(v, whole.read_int_at(j as i64 * 4)?);
                }
                padded.close()?;
            }
            input.close()
        })
    }

    /// `testRandomShort()`.
    fn test_random_short(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testShorts"))?, |dir| {
            let mut output = dir.create_output("shorts", &*io_context())?;
            let num = TestUtil::next_int(&*random(), 50, 3000);
            let mut shorts = vec![0i16; num as usize];
            for v in shorts.iter_mut() {
                *v = random().next_int() as i16;
                output.write_short(*v)?;
            }
            output.close()?;

            // slice
            let mut input = dir.open_input("shorts", &*io_context())?;
            let mut slice = input.random_access_slice(0, input.length())?;
            Assert::assert_equals(input.length(), slice.length());
            for (i, &v) in shorts.iter().enumerate() {
                Assert::assert_equals(v, slice.read_short_at(i as i64 * 2)?);
            }

            // subslices
            for i in 1..shorts.len() {
                let offset = i as i64 * 2;
                let mut subslice = input.random_access_slice(offset, input.length() - offset)?;
                Assert::assert_equals(input.length() - offset, subslice.length());
                for j in i..shorts.len() {
                    Assert::assert_equals(shorts[j], subslice.read_short_at((j - i) as i64 * 2)?);
                }
            }

            // with padding
            for i in 0..7 {
                let name = format!("shorts-{i}");
                let mut o = dir.create_output(&name, &*io_context())?;
                let mut junk = vec![0u8; i];
                random().next_bytes(&mut junk);
                o.write_bytes(&junk, 0, junk.len())?;
                input.seek(0)?;
                let len = input.length();
                o.copy_bytes(&mut *input, len)?;
                o.close()?;
                let mut padded = dir.open_input(&name, &*io_context())?;
                let mut whole = padded.random_access_slice(i as i64, padded.length() - i as i64)?;
                Assert::assert_equals(padded.length() - i as i64, whole.length());
                for (j, &v) in shorts.iter().enumerate() {
                    Assert::assert_equals(v, whole.read_short_at(j as i64 * 2)?);
                }
                padded.close()?;
            }
            input.close()
        })
    }

    /// `testRandomByte()`.
    fn test_random_byte(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testBytes"))?, |dir| {
            let mut output = dir.create_output("bytes", &*io_context())?;
            let num = if *TEST_NIGHTLY {
                TestUtil::next_int(&*random(), 1000, 3000)
            } else {
                TestUtil::next_int(&*random(), 50, 1000)
            };
            let mut bytes = vec![0u8; num as usize];
            random().next_bytes(&mut bytes);
            for &b in &bytes {
                output.write_byte(b)?;
            }
            output.close()?;

            // slice
            let mut input = dir.open_input("bytes", &*io_context())?;
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
                let mut o = dir.create_output(&name, &*io_context())?;
                let mut junk = vec![0u8; i];
                random().next_bytes(&mut junk);
                o.write_bytes(&junk, 0, junk.len())?;
                input.seek(0)?;
                let len = input.length();
                o.copy_bytes(&mut *input, len)?;
                o.close()?;
                let mut padded = dir.open_input(&name, &*io_context())?;
                let mut whole = padded.random_access_slice(i as i64, padded.length() - i as i64)?;
                Assert::assert_equals(padded.length() - i as i64, whole.length());
                self.assert_bytes(&mut *whole, &bytes, 0)?;
                padded.close()?;
            }
            input.close()
        })
    }

    /// `assertBytes(RandomAccessInput slice, byte[] bytes, int bytesOffset)`
    /// (`protected`).
    fn assert_bytes(
        &self,
        slice: &mut dyn RandomAccessInput,
        bytes: &[u8],
        bytes_offset: usize,
    ) -> Result<()> {
        let to_read = bytes.len() - bytes_offset;
        for i in 0..to_read {
            Assert::assert_equals(
                bytes[bytes_offset + i] as i8,
                slice.read_byte_at(i as i64)? as i8,
            );
            let offset = random().next_int_bound(1000) as usize;
            let mut sub1 = vec![0u8; offset + i];
            slice.read_bytes_at(0, &mut sub1, offset, i)?;
            assert_array_equals(
                &java_bytes(&bytes[bytes_offset..bytes_offset + i]),
                &java_bytes(&sub1[offset..]),
            );
            let mut sub2 = vec![0u8; offset + to_read - i];
            slice.read_bytes_at(i as i64, &mut sub2, offset, to_read - i)?;
            assert_array_equals(
                &java_bytes(&bytes[bytes_offset + i..]),
                &java_bytes(&sub2[offset..]),
            );
        }
        Ok(())
    }

    /// `testSliceOfSlice()`: try to stress slices of slices.
    fn test_slice_of_slice(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("sliceOfSlice"))?, |dir| {
            let mut output = dir.create_output("bytes", &*io_context())?;
            let num = if *TEST_NIGHTLY {
                TestUtil::next_int(&*random(), 250, 2500)
            } else {
                TestUtil::next_int(&*random(), 50, 250)
            } as usize;
            let mut bytes = vec![0u8; num];
            random().next_bytes(&mut bytes);
            for &b in &bytes {
                output.write_byte(b)?;
            }
            output.close()?;

            let mut input = dir.open_input("bytes", &*io_context())?;
            // seek to a random spot should not impact slicing.
            let to = TestUtil::next_long(&*random(), 0, input.length());
            input.seek(to)?;
            let mut i = 0;
            while i < num {
                let mut slice1 = input.slice("slice1", i as i64, (num - i) as i64)?;
                Assert::assert_equals(0, slice1.file_pointer());
                Assert::assert_equals((num - i) as i64, slice1.length());

                // seek to a random spot shouldnt impact slicing.
                let to = TestUtil::next_long(&*random(), 0, slice1.length());
                slice1.seek(to)?;
                let mut j = 0;
                while (j as i64) < slice1.length() {
                    let mut slice2 = slice1.slice("slice2", j as i64, (num - i - j) as i64)?;
                    if random().next_boolean() {
                        slice2 = slice2.clone_input()?; // clone shouldn't impact slice data
                    }
                    Assert::assert_equals(0, slice2.file_pointer());
                    Assert::assert_equals((num - i - j) as i64, slice2.length());
                    let mut data = vec![0u8; num];
                    data[..i + j].copy_from_slice(&bytes[..i + j]);
                    if random().next_boolean() {
                        // read the bytes for this slice-of-slice
                        slice2.read_bytes(Some(&mut data), i + j, num - i - j)?;
                    } else {
                        // seek to a random spot in between, read some, seek back and read the rest
                        let seek = TestUtil::next_long(&*random(), 0, slice2.length());
                        slice2.seek(seek)?;
                        slice2.read_bytes(
                            Some(&mut data),
                            i + j + seek as usize,
                            num - i - j - seek as usize,
                        )?;
                        slice2.seek(0)?;
                        slice2.read_bytes(Some(&mut data), i + j, seek as usize)?;
                    }
                    assert_array_equals(&java_bytes(&bytes), &java_bytes(&data));
                    j += 16;
                }
                i += 16;
            }

            input.close()
        })
    }

    /// `testLargeWrites()`: this test that writes larger than the size of
    /// the buffer output will correctly increment the file pointer.
    fn test_large_writes(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("largeWrites"))?, |dir| {
            let mut os = dir.create_output("testBufferStart.txt", &*io_context())?;

            let mut large_buf = vec![0u8; 2048];
            random().next_bytes(&mut large_buf);

            let current_pos = os.file_pointer();
            os.write_bytes(&large_buf, 0, large_buf.len())?;

            let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Assert::assert_equals(current_pos + large_buf.len() as i64, os.file_pointer());
            }));
            os.close()?;
            if let Err(payload) = checked {
                std::panic::resume_unwind(payload);
            }
            Ok(())
        })
    }

    /// `testIndexOutputToString()` (LUCENE-6084).
    fn test_index_output_to_string(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let mut out = dir.create_output("camelCase.txt", &*io_context())?;
                let s = out.resource_description().to_string();
                if !s.contains("camelCase.txt") {
                    Assert::fail(&format!(
                        "\nExpected: a string containing \"camelCase.txt\"\n     but: was \"{s}\""
                    ));
                }
                out.close()
            },
        )
    }

    /// `testDoubleCloseOutput()`.
    fn test_double_close_output(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let mut out = dir.create_output("foobar", &*io_context())?;
                out.write_string("testing")?;
                out.close()?;
                out.close() // close again
            },
        )
    }

    /// `testDoubleCloseInput()`.
    fn test_double_close_input(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let mut out = dir.create_output("foobar", &*io_context())?;
                out.write_string("testing")?;
                out.close()?;
                let mut input = dir.open_input("foobar", &*io_context())?;
                Assert::assert_equals("testing".to_string(), input.read_string()?);
                input.close()?;
                input.close() // close again
            },
        )
    }

    /// `testCreateTempOutput()`.
    fn test_create_temp_output(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let mut names = Vec::new();
                let iters = LuceneTestCase::at_least(50);
                for iter in 0..iters {
                    let mut out = dir.create_temp_output("foo", "bar", &*io_context())?;
                    names.push(out.name().to_string());
                    out.write_v_int(iter)?;
                    out.close()?;
                }
                for (iter, name) in names.iter().enumerate() {
                    let mut input = dir.open_input(name, &*io_context())?;
                    Assert::assert_equals(iter as i32, input.read_v_int()?);
                    input.close()?;
                }

                let files: HashSet<String> = dir
                    .list_all()?
                    .into_iter()
                    .filter(|file| !ExtrasFS::is_extra(file)) // remove any ExtrasFS stuff.
                    .collect();
                let expected: HashSet<String> = names.iter().cloned().collect();
                if files != expected || files.len() != names.len() {
                    Assert::fail(&format!(
                    "\nExpected: iterable with items {names:?} in any order\n     but: was {files:?}"
                ));
                }
                Ok(())
            },
        )
    }

    /// `testCreateOutputForExistingFile()`.
    fn test_create_output_for_existing_file(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let name = "file";
                dir.create_output(name, &*DEFAULT_IO_CONTEXT)?.close()?;

                // Try to create an existing file should fail.
                expect_throws("java.nio.file.FileAlreadyExistsException", || {
                    dir.create_output(name, &*DEFAULT_IO_CONTEXT)
                        .and_then(|mut out| out.close())
                });

                // Delete file and try to recreate it.
                dir.delete_file(name)?;
                dir.create_output(name, &*DEFAULT_IO_CONTEXT)?.close()
            },
        )
    }

    /// `testSeekToEndOfFile()`.
    fn test_seek_to_end_of_file(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                with_output(dir.create_output("a", &*DEFAULT_IO_CONTEXT)?, |out| {
                    for _ in 0..1024 {
                        out.write_byte(0)?;
                    }
                    Ok(())
                })?;
                with_input(dir.open_input("a", &*DEFAULT_IO_CONTEXT)?, |input| {
                    input.seek(100)?;
                    Assert::assert_equals(100, input.file_pointer());
                    input.seek(1024)?;
                    Assert::assert_equals(1024, input.file_pointer());
                    Ok(())
                })
            },
        )
    }

    /// `testSeekBeyondEndOfFile()`.
    fn test_seek_beyond_end_of_file(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                with_output(dir.create_output("a", &*DEFAULT_IO_CONTEXT)?, |out| {
                    for _ in 0..1024 {
                        out.write_byte(0)?;
                    }
                    Ok(())
                })?;
                with_input(dir.open_input("a", &*DEFAULT_IO_CONTEXT)?, |input| {
                    input.seek(100)?;
                    Assert::assert_equals(100, input.file_pointer());
                    expect_throws("java.io.EOFException", || input.seek(1025));
                    Ok(())
                })
            },
        )
    }

    /// `testPendingDeletions()`: make sure the FSDirectory impl properly
    /// "emulates" deletions on filesystems (Windows) with buggy deleteFile.
    fn test_pending_deletions(&self) -> Result<()> {
        let path = LuceneTestCase::add_virus_checker(&LuceneTestCase::create_temp_dir());
        with_directory(self.get_directory(&path)?, |dir| {
            LuceneTestCase::assume_true(
                "we can only install VirusCheckingFS on an FSDirectory",
                dir.as_fs_directory().is_some(),
            );
            let fs_dir = dir;

            // Keep trying until virus checker refuses to delete:
            let file_name = loop {
                // create a random filename (segment file name style), so it cannot hit windows
                // problem with special filenames ("con", "com1",...):
                let candidate = segment_file_name(
                    &TestUtil::random_simple_string_between(&*random(), 1, 6),
                    &TestUtil::random_simple_string(&*random()),
                    "test",
                );
                with_output(
                    dir.create_output(&candidate, &*DEFAULT_IO_CONTEXT)?,
                    |out| {
                        out.file_pointer(); // just fake access to prevent compiler warning
                        Ok(())
                    },
                )?;
                fs_dir.delete_file(&candidate)?;
                if !fs_dir.get_pending_deletions()?.is_empty() {
                    // good: virus checker struck and prevented deletion of fileName
                    break candidate;
                }
            };

            // Make sure listAll does NOT include the file:
            assert_has_item(&fs_dir.list_all()?, &file_name, false);

            // Make sure fileLength claims it's deleted:
            expect_throws("java.nio.file.NoSuchFileException", || {
                fs_dir.file_length(&file_name)
            });

            // Make sure rename fails:
            expect_throws("java.nio.file.NoSuchFileException", || {
                fs_dir.rename(&file_name, "file2")
            });

            // Make sure delete fails:
            expect_throws("java.nio.file.NoSuchFileException", || {
                fs_dir.delete_file(&file_name)
            });

            // Make sure we cannot open it for reading:
            expect_throws_any_of(
                &[
                    "java.nio.file.NoSuchFileException",
                    "java.io.FileNotFoundException",
                ],
                || fs_dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT),
            );
            Ok(())
        })
    }

    /// `testListAllIsSorted()`.
    fn test_list_all_is_sorted(&self) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let count = LuceneTestCase::at_least(20) as usize;
                let mut names: HashSet<String> = HashSet::new();
                while names.len() < count {
                    // create a random filename (segment file name style), so it cannot hit windows
                    // problem with special filenames ("con", "com1",...):
                    let name = segment_file_name(
                        &TestUtil::random_simple_string_between(&*random(), 1, 6),
                        &TestUtil::random_simple_string(&*random()),
                        "test",
                    );
                    if random().next_int_bound(5) == 1 {
                        let mut out = dir.create_temp_output(&name, "foo", &*DEFAULT_IO_CONTEXT)?;
                        names.insert(out.name().to_string());
                        out.close()?;
                    } else if !names.contains(&name) {
                        let mut out = dir.create_output(&name, &*DEFAULT_IO_CONTEXT)?;
                        names.insert(out.name().to_string());
                        out.close()?;
                    }
                }
                let actual = dir.list_all()?;
                let mut expected = actual.clone();
                // Arrays.sort(String[]): UTF-16 code unit order
                expected.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                if expected != actual {
                    Assert::fail(&format!(
                        "arrays differ; expected:<{expected:?}> but was:<{actual:?}>"
                    ));
                }
                Ok(())
            },
        )
    }

    /// `testDataTypes()`.
    fn test_data_types(&self) -> Result<()> {
        let values = [43, 12345, 123456, 1234567890];
        with_directory(self.get_directory(&temp_dir("testDataTypes"))?, |dir| {
            let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
            out.write_byte(43)?;
            out.write_short(12345)?;
            out.write_int(1234567890)?;
            out.write_group_vints(&values, 4)?;
            out.write_long(1234567890123456789)?;
            out.close()?;

            let mut restored = [0i32; 4];

            let mut input = dir.open_input("test", &*DEFAULT_IO_CONTEXT)?;
            Assert::assert_equals(43, input.read_byte()?);
            Assert::assert_equals(12345i16, input.read_short()?);
            Assert::assert_equals(1234567890, input.read_int()?);
            GroupVIntUtil::read_group_vints_input(&mut *input, &mut restored, 4)?;
            assert_array_equals(&values, &restored);
            Assert::assert_equals(1234567890123456789i64, input.read_long()?);
            input.close()
        })
    }

    /// `testGroupVIntOverflow()` (`@Deprecated`).
    #[allow(deprecated)]
    fn test_group_v_int_overflow(&self) -> Result<()> {
        with_directory(
            self.get_directory(&temp_dir("testGroupVIntOverflow"))?,
            |dir| {
                let size = 32usize;
                let mut values = vec![0i64; size];
                let mut restore = vec![0i64; size];
                values[0] = 1i64 << 31; // values[0] = 2147483648 as long, but as int it is -2147483648

                for i in 0..size {
                    if random().next_boolean() {
                        values[i] = values[0];
                    }
                }

                // a smaller limit value cover default implementation of readGroupVInts
                // and a bigger limit value cover the faster implementation.
                let limit = random().next_int_range(1, size as i32) as usize;
                let mut out = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
                out.write_group_vints_long(&values, limit)?;
                out.close()?;
                with_input(dir.open_input("test", &*DEFAULT_IO_CONTEXT)?, |input| {
                    GroupVIntUtil::read_group_vints_input_long(&mut *input, &mut restore, limit)?;
                    for i in 0..limit {
                        Assert::assert_equals(values[i], restore[i]);
                    }
                    Ok(())
                })?;

                values[0] = 0xFFFF_FFFFi64 + 1;
                expect_throws("java.lang.ArithmeticException", || {
                    out.write_group_vints_long(&values, 4)
                });
                Ok(())
            },
        )
    }

    /// `testGroupVInt()`.
    fn test_group_v_int(&self) -> Result<()> {
        with_directory(self.get_directory(&temp_dir("testGroupVInt"))?, |dir| {
            // test fallback to default implementation of readGroupVInt
            self.do_test_group_v_int(dir, 5, 1, 6, 8)?;

            // use more iterations to covers all bpv
            self.do_test_group_v_int(dir, LuceneTestCase::at_least(100), 1, 31, 128)

            // we use BaseChunkedDirectoryTestCase#testGroupVIntMultiBlocks cover multiple blocks for
            // ByteBuffersDataInput and MMapDirectory
        })
    }

    /// `doTestGroupVInt(Directory dir, int iterations, int minBpv, int
    /// maxBpv, int maxNumValues)` (`protected`).
    fn do_test_group_v_int(
        &self,
        dir: &dyn Directory,
        iterations: i32,
        min_bpv: i32,
        max_bpv: i32,
        max_num_values: i32,
    ) -> Result<()> {
        let mut values = vec![0i32; max_num_values as usize];
        let mut num_values_array = vec![0i32; iterations as usize];
        let mut group_v_int_out = dir.create_output("group-varint", &*DEFAULT_IO_CONTEXT)?;
        let mut v_int_out = dir.create_output("vint", &*DEFAULT_IO_CONTEXT)?;

        // encode
        for iter in 0..iterations as usize {
            let bpv = TestUtil::next_int(&*random(), min_bpv, max_bpv);
            num_values_array[iter] = TestUtil::next_int(&*random(), 1, max_num_values);
            for j in 0..num_values_array[iter] as usize {
                values[j] = RandomNumbers::random_int_between(
                    &*random(),
                    0,
                    PackedInts::max_value(bpv) as i32,
                );
                v_int_out.write_v_int(values[j])?;
            }
            group_v_int_out.write_group_vints(&values, num_values_array[iter] as usize)?;
        }
        group_v_int_out.close()?;
        v_int_out.close()?;

        // decode
        let mut group_v_int_in = dir.open_input("group-varint", &*DEFAULT_IO_CONTEXT)?;
        let mut v_int_in = dir.open_input("vint", &*DEFAULT_IO_CONTEXT)?;
        for iter in 0..iterations as usize {
            GroupVIntUtil::read_group_vints_input(
                &mut *group_v_int_in,
                &mut values,
                num_values_array[iter] as usize,
            )?;
            for &v in values.iter().take(num_values_array[iter] as usize) {
                Assert::assert_equals(v_int_in.read_v_int()?, v);
            }
        }
        group_v_int_in.close()?;
        v_int_in.close()?;
        dir.delete_file("group-varint")?;
        dir.delete_file("vint")
    }

    /// `testPrefetch()`.
    fn test_prefetch(&self) -> Result<()> {
        self.do_test_prefetch(0)
    }

    /// `testPrefetchOnSlice()`.
    fn test_prefetch_on_slice(&self) -> Result<()> {
        self.do_test_prefetch(TestUtil::next_int(&*random(), 1, 1024))
    }

    /// `doTestPrefetch(int startOffset)` (`private`).
    fn do_test_prefetch(&self, start_offset: i32) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let start_offset = start_offset as usize;
                let total_length =
                    start_offset + TestUtil::next_int(&*random(), 16384, 65536) as usize;
                let mut arr = vec![0u8; total_length];
                random().next_bytes(&mut arr);
                with_output(
                    dir.create_output("temp.bin", &*DEFAULT_IO_CONTEXT)?,
                    |out| out.write_bytes(&arr, 0, arr.len()),
                )?;
                let mut temp = vec![0u8; 2048];

                with_input(dir.open_input("temp.bin", &*DEFAULT_IO_CONTEXT)?, |orig| {
                    let mut input = if start_offset == 0 {
                        orig.clone_input()?
                    } else {
                        orig.slice(
                            "slice",
                            start_offset as i64,
                            (total_length - start_offset) as i64,
                        )?
                    };
                    for _ in 0..10_000 {
                        let offset = TestUtil::next_int(&*random(), 0, input.length() as i32 - 1);
                        if random().next_boolean() {
                            let prefetch_length = TestUtil::next_long(
                                &*random(),
                                1,
                                input.length() - i64::from(offset),
                            );
                            input.prefetch(i64::from(offset), prefetch_length)?;
                        }
                        input.seek(i64::from(offset))?;
                        Assert::assert_equals(i64::from(offset), input.file_pointer());
                        let at = start_offset + offset as usize;
                        match random().next_int_bound(100) {
                            0 => Assert::assert_equals(arr[at] as i8, input.read_byte()? as i8),
                            1 => {
                                if input.length() - i64::from(offset) >= 8 {
                                    let mut le = [0u8; 8];
                                    le.copy_from_slice(&arr[at..at + 8]);
                                    Assert::assert_equals(
                                        i64::from_le_bytes(le),
                                        input.read_long()?,
                                    );
                                }
                            }
                            _ => {
                                let read_length = TestUtil::next_int(
                                    &*random(),
                                    1,
                                    (temp.len() as i64).min(input.length() - i64::from(offset))
                                        as i32,
                                ) as usize;
                                input.read_bytes(Some(&mut temp), 0, read_length)?;
                                assert_array_equals(
                                    &java_bytes(&arr[at..at + read_length]),
                                    &java_bytes(&temp[..read_length]),
                                );
                            }
                        }
                    }
                    input.close()
                })
            },
        )
    }

    /// `testIsLoaded()`.
    fn test_is_loaded(&self) -> Result<()> {
        self.do_test_is_loaded(0)
    }

    /// `testIsLoadedOnSlice()`.
    fn test_is_loaded_on_slice(&self) -> Result<()> {
        self.do_test_is_loaded(TestUtil::next_int(&*random(), 1, 1024))
    }

    /// `testIsLoaded(int startOffset)` (`private`).
    fn do_test_is_loaded(&self, start_offset: i32) -> Result<()> {
        with_directory(
            self.get_directory(&LuceneTestCase::create_temp_dir())?,
            |dir| {
                let mut inner = FilterDirectory::unwrap(dir);
                if let Some(fsd) = inner
                    .as_any()
                    .and_then(|a| a.downcast_ref::<FileSwitchDirectory>())
                {
                    // We check the secondary directory based on the assumption that the file in this
                    // test will always route there.
                    inner = FilterDirectory::unwrap(fsd.secondary_dir());
                }
                let m_map_dir = inner
                    .as_any()
                    .and_then(|a| a.downcast_ref::<MMapDirectory>());
                if let Some(m) = m_map_dir {
                    m.set_preload(MMapDirectory::all_files());
                }
                let start_offset = start_offset as usize;
                let total_length =
                    start_offset + TestUtil::next_int(&*random(), 16384, 65536) as usize;
                let mut arr = vec![0u8; total_length];
                random().next_bytes(&mut arr);
                with_output(
                    dir.create_output("temp.bin", &*DEFAULT_IO_CONTEXT)?,
                    |out| out.write_bytes(&arr, 0, arr.len()),
                )?;

                with_input(dir.open_input("temp.bin", &*DEFAULT_IO_CONTEXT)?, |orig| {
                    let mut input = if start_offset == 0 {
                        orig.clone_input()?
                    } else {
                        orig.slice(
                            "slice",
                            start_offset as i64,
                            (total_length - start_offset) as i64,
                        )?
                    };
                    let loaded = input.is_loaded();
                    if rucene::util::Constants::windows() {
                        // On Windows, we temporarily don't care until this is fixed: #14050
                    } else if m_map_dir.is_some() {
                        // Direct IO wraps MMap but does not support isLoaded.
                        // NRTCachingDirectory may return index inputs from cache.
                        if !wrapper_classes_exclude_is_loaded(dir) {
                            Assert::assert_true(loaded.is_some());
                            Assert::assert_true(loaded.unwrap_or(false));
                        } else {
                            Assert::assert_false(loaded.is_some());
                        }
                    } else {
                        Assert::assert_false(loaded.is_some());
                    }
                    input.close()
                })
            },
        )
    }
}

/// `wrapperClasses(dir).stream().anyMatch(clz -> clz.getName().contains("DirectIO") ||
/// NRTCachingDirectory.class.isAssignableFrom(clz))`, over the
/// `FilterDirectory` chain of `dir`.
fn wrapper_classes_exclude_is_loaded(dir: &dyn Directory) -> bool {
    let mut current = dir;
    while let Some(delegate) = current.as_filter_directory() {
        let is_nrt = current
            .as_any()
            .is_some_and(|a| a.downcast_ref::<NRTCachingDirectory>().is_some());
        if is_nrt || current.directory_type_name().contains("DirectIO") {
            return true;
        }
        current = delegate;
    }
    false
}

/// `runCopyFrom(Directory source, Directory dest)` (`private`).
fn run_copy_from(source: &dyn Directory, dest: &dyn Directory) -> Result<()> {
    let mut output = source.create_output("foobar", &*io_context())?;

    let bytes = RandomBytes::random_bytes_of_length(&*random(), 20000);
    output.write_bytes(&bytes, 0, bytes.len())?;
    output.close()?;

    dest.copy_from(source, "foobar", "foobaz", &*io_context())?;
    Assert::assert_true(LuceneTestCase::slow_file_exists(dest, "foobaz")?);

    let mut input = dest.open_input("foobaz", &*io_context())?;
    let mut bytes2 = vec![0u8; bytes.len()];
    let n = bytes2.len();
    input.read_bytes(Some(&mut bytes2), 0, n)?;
    input.close()?;

    assert_array_equals(&java_bytes(&bytes), &java_bytes(&bytes2));
    Ok(())
}

/// `assertArrayEquals(float[], float[], 0)`.
fn assert_float_array_equals(expected: &[f32], actual: &[f32]) {
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if e.to_bits() != a.to_bits() && e != a {
            Assert::fail(&format!(
                "arrays first differed at element [{i}]; expected:<{e}> but was:<{a}>"
            ));
        }
    }
    Assert::assert_equals(expected.len(), actual.len());
}

/// `assertEquals(Set, Set)` over a read set of strings.
fn assert_sets_equal(expected: &[&str], actual: &rucene::util::java_collections::JavaStringSet) {
    let same = actual.len() == expected.len() && expected.iter().all(|s| actual.contains(s));
    if !same {
        let actual: Vec<&String> = actual.iter().collect();
        Assert::fail(&format!("expected:<{expected:?}> but was:<{actual:?}>"));
    }
}

/// `assertEquals(Map, Map)` over a read map of strings.
fn assert_maps_equal(
    expected: &[(&str, &str)],
    actual: &rucene::util::java_collections::JavaStringMap,
) {
    let same = actual.len() == expected.len()
        && expected
            .iter()
            .all(|(k, v)| actual.get(k).map(String::as_str) == Some(*v));
    if !same {
        let actual: Vec<(&String, &String)> = actual.iter().collect();
        Assert::fail(&format!("expected:<{expected:?}> but was:<{actual:?}>"));
    }
}

// -----------------------------------------------------------------------------
// The directory a test owns
// -----------------------------------------------------------------------------

/// A `LuceneTestCase.newDirectory()` / `newFSDirectory()` result as a
/// `Directory` the holder owns: every call, and every question asked of the
/// directory (`instanceof`, `toString()`, the `FilterDirectory` chain), goes
/// to the wrapped `MockDirectoryWrapper` or `RawDirectoryWrapper`; `close()`
/// closes it. It stands for the same Java object, so it adds no behaviour of
/// its own.
pub struct OwnedTestDirectory {
    inner: TestDirectory,
}

impl OwnedTestDirectory {
    /// Takes ownership of `inner`.
    pub fn new(inner: TestDirectory) -> Self {
        OwnedTestDirectory { inner }
    }

    /// The wrapper, as the directory it is.
    pub fn get(&self) -> &dyn Directory {
        match &self.inner {
            TestDirectory::Mock(mock) => mock,
            TestDirectory::Raw(raw) => &**raw,
        }
    }

    /// The `TestDirectory`.
    pub fn test_directory(&self) -> &TestDirectory {
        &self.inner
    }
}

impl Directory for OwnedTestDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.get().list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.get().delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.get().file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.get().create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.get().create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.get().sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.get().sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.get().rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.get().open_input(name, context)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.get().open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.get().obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn copy_from(
        &self,
        from: &dyn Directory,
        src: &str,
        dest: &str,
        context: &dyn IOContext,
    ) -> Result<()> {
        self.get().copy_from(from, src, dest, context)
    }

    fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
        self.get().get_pending_deletions()
    }

    fn as_fs_directory(&self) -> Option<&rucene::store::FSDirectory> {
        self.get().as_fs_directory()
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        self.get().as_filter_directory()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        match &self.inner {
            TestDirectory::Mock(mock) => Some(mock),
            TestDirectory::Raw(raw) => Some(&**raw),
        }
    }

    fn directory_type_name(&self) -> &'static str {
        self.get().directory_type_name()
    }

    fn directory_to_string(&self) -> String {
        self.get().directory_to_string()
    }

    fn ensure_open(&self) -> Result<()> {
        self.get().ensure_open()
    }
}
