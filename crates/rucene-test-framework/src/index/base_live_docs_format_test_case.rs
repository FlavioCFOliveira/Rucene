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

//! Port of `org.apache.lucene.tests.index.BaseLiveDocsFormatTestCase`:
//! abstract class that performs basic testing of a codec's `LiveDocsFormat`.
//!
//! The class is declared by [`class()`] (`public abstract class
//! BaseLiveDocsFormatTestCase extends LuceneTestCase`) and its members are
//! the trait [`BaseLiveDocsFormatTestCase`]; a subclass's reflective
//! constructor builds a [`LiveDocsTestInstance`] holding the subclass, as
//! [`FormatTestInstance`](super::base_index_file_format_test_case::FormatTestInstance)
//! does for the suites of `BaseIndexFileFormatTestCase`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::Codec;
use rucene::error::Result;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use rucene::search::Sort;
use rucene::store::{DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{Bits, FixedBitSet, Version};

use super::base_index_file_format_test_case::rethrow;
use crate::util::lucene_test_case::{LuceneTestCase, Monster};
use crate::util::TestUtil;

/// The test instance of a subclass of `BaseLiveDocsFormatTestCase`: the
/// subclass and the codec `setUp()` saves.
pub struct LiveDocsTestInstance {
    /// The subclass, as the trait its members are reached through.
    pub case: Arc<dyn BaseLiveDocsFormatTestCase>,
    /// `savedCodec`.
    saved_codec: Option<Arc<dyn Codec>>,
}

impl LiveDocsTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseLiveDocsFormatTestCase>(case: Arc<T>) -> Self {
        LiveDocsTestInstance {
            case,
            saved_codec: None,
        }
    }
}

/// A test method of the class: `public void <name>() throws IOException`.
fn live_docs_test(
    name: &str,
    body: impl Fn(&dyn BaseLiveDocsFormatTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.io.IOException"]).body(
        move |inst: &mut LiveDocsTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow(e))
        },
    )
}

/// The declaration of `org.apache.lucene.tests.index.BaseLiveDocsFormatTestCase`,
/// with its overrides of `setUp()`/`tearDown()` and its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.index.BaseLiveDocsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
                |inst: &mut LiveDocsTestInstance| {
                    LuceneTestCase::super_set_up();
                    // set the default codec, so adding test cases to this isn't fragile
                    inst.saved_codec = rucene::codecs::get_default();
                    rucene::codecs::set_default(inst.case.get_codec());
                },
            ))
            .method(
                Method::new("tearDown")
                    .throws(&["java.lang.Exception"])
                    .body(tear_down),
            )
            .method(live_docs_test("testDenseLiveDocs", |c| {
                c.test_dense_live_docs()
            }))
            .method(live_docs_test("testEmptyLiveDocs", |c| {
                c.test_empty_live_docs()
            }))
            .method(live_docs_test("testSparseLiveDocs", |c| {
                c.test_sparse_live_docs()
            }))
            .method(
                live_docs_test("testOverflow", |c| c.test_overflow()).annotate(Monster {
                    value: "Uses lots of memory".to_string(),
                }),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

/// `BaseLiveDocsFormatTestCase.tearDown()`, which a subclass overriding
/// `tearDown()` calls as `super.tearDown()`: restores the default codec
/// `setUp()` saved, then `LuceneTestCase.tearDown()`.
pub fn tear_down(inst: &mut LiveDocsTestInstance) {
    if let Some(saved) = inst.saved_codec.take() {
        rucene::codecs::set_default(saved); // restore
    }
    LuceneTestCase::super_tear_down();
}

/// The anonymous `Bits` of `testSerialization`: the live docs seen through
/// `get`/`length` only ("make sure the impl doesn't only work with a
/// `FixedBitSet`").
#[derive(Debug)]
struct WrappedBits<'a>(&'a FixedBitSet);

impl Bits for WrappedBits<'_> {
    fn get(&self, index: usize) -> bool {
        self.0.get(index)
    }

    fn length(&self) -> usize {
        self.0.length()
    }
}

/// Port of the members of `BaseLiveDocsFormatTestCase`.
pub trait BaseLiveDocsFormatTestCase: Send + Sync + 'static {
    /// `getCodec()`: returns the codec to run tests against.
    fn get_codec(&self) -> Arc<dyn Codec>;

    /// `testDenseLiveDocs()`.
    fn test_dense_live_docs(&self) -> Result<()> {
        let max_doc = TestUtil::next_int(&*LuceneTestCase::random(), 3, 1000);
        self.test_serialization(max_doc, max_doc - 1, false)?;
        self.test_serialization(max_doc, max_doc - 1, true)
    }

    /// `testEmptyLiveDocs()`.
    fn test_empty_live_docs(&self) -> Result<()> {
        let max_doc = TestUtil::next_int(&*LuceneTestCase::random(), 3, 1000);
        self.test_serialization(max_doc, 0, false)?;
        self.test_serialization(max_doc, 0, true)
    }

    /// `testSparseLiveDocs()`.
    fn test_sparse_live_docs(&self) -> Result<()> {
        let max_doc = TestUtil::next_int(&*LuceneTestCase::random(), 3, 1000);
        self.test_serialization(max_doc, 1, false)?;
        self.test_serialization(max_doc, 1, true)
    }

    /// `testOverflow()` (`@Monster("Uses lots of memory")`):
    /// `IndexWriter.MAX_DOCS` documents.
    fn test_overflow(&self) -> Result<()> {
        self.test_serialization(MAX_DOCS, MAX_DOCS - 7, false)
    }

    /// `testSerialization(maxDoc, numLiveDocs, fixedBitSet)` (`private`).
    fn test_serialization(
        &self,
        max_doc: i32,
        num_live_docs: i32,
        fixed_bit_set: bool,
    ) -> Result<()> {
        let codec = rucene::codecs::get_default().unwrap_or_else(|| self.get_codec());
        let format = codec.live_docs_format();

        let random = LuceneTestCase::random();
        let mut live_docs = FixedBitSet::new(max_doc as usize);
        if num_live_docs > max_doc / 2 {
            live_docs.set_range(0, max_doc as usize);
            for _ in 0..max_doc - num_live_docs {
                let mut clear_bit;
                loop {
                    clear_bit = random.next_int_bound(max_doc);
                    if live_docs.get(clear_bit as usize) {
                        break;
                    }
                }
                live_docs.clear(clear_bit as usize);
            }
        } else {
            for _ in 0..num_live_docs {
                let mut set_bit;
                loop {
                    set_bit = random.next_int_bound(max_doc);
                    if !live_docs.get(set_bit as usize) {
                        break;
                    }
                }
                live_docs.set(set_bit as usize);
            }
        }

        let wrapped = WrappedBits(&live_docs);
        let bits: &dyn Bits = if fixed_bit_set { &live_docs } else { &wrapped };

        let mut dir = LuceneTestCase::new_directory();
        let directory = dir.as_directory();
        let si = SegmentInfo::new(
            Arc::clone(&directory),
            Version::LATEST,
            Some(Version::LATEST),
            "foo".to_string(),
            max_doc,
            random.next_boolean(),
            false,
            Arc::clone(&codec),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            Sort::default(),
        )?;
        let sci = SegmentCommitInfo::new(si.clone(), 0, 0, 0, -1, -1, StringHelper::random_id())?;
        format.write_live_docs(
            bits,
            &*directory,
            &sci,
            max_doc - num_live_docs,
            &*DEFAULT_IO_CONTEXT,
        )?;

        let sci = SegmentCommitInfo::new(
            si,
            max_doc - num_live_docs,
            0,
            1,
            -1,
            -1,
            StringHelper::random_id(),
        )?;
        let bits2 = format.read_live_docs(&*directory, &sci, &*READONCE_IO_CONTEXT)?;
        Assert::assert_equals(max_doc as i64, bits2.length() as i64);
        for i in 0..max_doc as usize {
            Assert::assert_equals(bits.get(i), bits2.get(i));
        }
        drop(bits2);
        drop(directory);
        dir.close()
    }
}
