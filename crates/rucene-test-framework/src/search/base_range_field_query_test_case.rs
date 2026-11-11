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

//! Port of `org.apache.lucene.tests.search.BaseRangeFieldQueryTestCase`:
//! abstract class to do basic tests for a RangeField query. Testing rigor
//! inspired by `BaseGeoPointTestCase`.
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseRangeFieldQueryTestCase`]; its nested `Range` class is the trait
//! [`Range`] (the subclass's box) and [`RangeSlot`] (the `isMissing` flag
//! of the base class beside the box).
//!
//! **Divergence** (`Range[][] ranges`): Java's arrays hold references to
//! boxes that later statements mutate (`isMissing`, `setMin`, `setMax`)
//! and share (`Arrays.fill`); the port holds each box in a shared,
//! mutable cell ([`RangeRef`]) so that the same sharing and mutations take
//! place.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, IndexableField};
use rucene::search::query::Query;
use rucene::util::BytesRef;

use crate::search::FixedBitSetCollector;
use crate::util::java_exceptions::rethrow_java;
use crate::util::lucene_test_case::{Nightly, VERBOSE};
use crate::util::{LuceneTestCase, TestUtil};

/// The declaration of `org.apache.lucene.tests.search.BaseRangeFieldQueryTestCase`
/// (`public abstract class BaseRangeFieldQueryTestCase extends
/// LuceneTestCase`) with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseRangeFieldQueryTestCase) -> Result<()>) -> Method {
            range_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.search.BaseRangeFieldQueryTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(t("testRandomTiny", |c| c.test_random_tiny()))
            .method(t("testRandomMedium", |c| c.test_random_medium()))
            .method(t("testRandomBig", |c| c.test_random_big()).annotate(Nightly))
            .method(t("testMultiValued", |c| c.test_multi_valued()))
            .method(t("testAllEqual", |c| c.test_all_equal()))
            .method(t("testLowCardinality", |c| c.test_low_cardinality()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseRangeFieldQueryTestCase` subclass.
pub fn range_test(
    name: &str,
    body: impl Fn(&dyn BaseRangeFieldQueryTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut RangeTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow_java(e))
        })
}

/// The instance of a `BaseRangeFieldQueryTestCase` subclass.
pub struct RangeTestInstance {
    /// The subclass.
    pub case: Arc<dyn BaseRangeFieldQueryTestCase>,
}

impl RangeTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseRangeFieldQueryTestCase>(case: T) -> Self {
        RangeTestInstance {
            case: Arc::new(case),
        }
    }
}

/// A value of a range's bound: what `getMin`/`getMax` return and
/// `setMin`/`setMax` take (`Object`).
#[derive(Clone, Debug, PartialEq)]
pub enum RangeValue {
    /// `Integer`.
    Int(i32),
    /// `Long`.
    Long(i64),
    /// `Float`.
    Float(f32),
    /// `Double`.
    Double(f64),
    /// `InetAddress`, by its address bytes (`getAddress()`).
    Inet(Vec<u8>),
}

/// `Range.QueryType`: supported query relations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryType {
    /// `INTERSECTS`.
    Intersects,
    /// `WITHIN`.
    Within,
    /// `CONTAINS`.
    Contains,
    /// `CROSSES`.
    Crosses,
}

impl fmt::Display for QueryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            QueryType::Intersects => "INTERSECTS",
            QueryType::Within => "WITHIN",
            QueryType::Contains => "CONTAINS",
            QueryType::Crosses => "CROSSES",
        })
    }
}

/// `Range` (`protected abstract static class`): base class for range
/// verification; a subclass's box.
pub trait Range: fmt::Display {
    /// `numDimensions()`.
    fn num_dimensions(&self) -> i32;
    /// `getMin(int dim)`.
    fn get_min(&self, dim: i32) -> RangeValue;
    /// `setMin(int dim, Object val)`.
    fn set_min(&mut self, dim: i32, val: RangeValue);
    /// `getMax(int dim)`.
    fn get_max(&self, dim: i32) -> RangeValue;
    /// `setMax(int dim, Object val)`.
    fn set_max(&mut self, dim: i32, val: RangeValue);
    /// `isEqual(Range other)`.
    fn is_equal(&self, other: &dyn Range) -> bool;
    /// `isDisjoint(Range other)`.
    fn is_disjoint(&self, other: &dyn Range) -> bool;
    /// `isWithin(Range other)`.
    fn is_within(&self, other: &dyn Range) -> bool;
    /// `contains(Range other)`.
    fn contains(&self, other: &dyn Range) -> bool;
    /// The box as its own type, for the casts of the subclass (`(IntTestRange)
    /// other`).
    fn as_any(&self) -> &dyn std::any::Any;

    /// `relate(Range other)`: `None` (Java's `null`) when disjoint.
    fn relate(&self, other: &dyn Range) -> Option<QueryType> {
        if self.is_disjoint(other) {
            // if disjoint; return null:
            None
        } else if self.is_within(other) {
            Some(QueryType::Within)
        } else if self.contains(other) {
            Some(QueryType::Contains)
        } else {
            Some(QueryType::Crosses)
        }
    }
}

/// A `Range` object: the base class's `isMissing` beside the subclass's
/// box.
pub struct RangeSlot {
    /// `isMissing`.
    pub is_missing: bool,
    /// The box.
    pub range: Box<dyn Range>,
}

/// A reference to a `Range` object, shared and mutable as in Java.
pub type RangeRef = Rc<RefCell<RangeSlot>>;

/// A new `Range` object for `range`.
pub fn range_ref(range: Box<dyn Range>) -> RangeRef {
    Rc::new(RefCell::new(RangeSlot {
        is_missing: false,
        range,
    }))
}

/// Port of the members of `BaseRangeFieldQueryTestCase`.
pub trait BaseRangeFieldQueryTestCase: Send + Sync + 'static {
    /// `newRangeField(Range box)`.
    fn new_range_field(&self, r#box: &dyn Range) -> Result<Box<dyn IndexableField>>;
    /// `newIntersectsQuery(Range box)`.
    fn new_intersects_query(&self, r#box: &dyn Range) -> Result<Arc<dyn Query>>;
    /// `newContainsQuery(Range box)`.
    fn new_contains_query(&self, r#box: &dyn Range) -> Result<Arc<dyn Query>>;
    /// `newWithinQuery(Range box)`.
    fn new_within_query(&self, r#box: &dyn Range) -> Result<Arc<dyn Query>>;
    /// `newCrossesQuery(Range box)`.
    fn new_crosses_query(&self, r#box: &dyn Range) -> Result<Arc<dyn Query>>;
    /// `nextRange(int dimensions)`.
    fn next_range(&self, dimensions: i32) -> Result<Box<dyn Range>>;

    /// `dimension()`.
    fn dimension(&self) -> i32 {
        LuceneTestCase::random().next_int_bound(4) + 1
    }

    /// `testRandomTiny()`.
    fn test_random_tiny(&self) -> Result<()> {
        // Make sure single-leaf-node case is OK:
        for _ in 0..10 {
            self.do_test_random(10, false)?;
        }
        Ok(())
    }

    /// `testRandomMedium()`.
    fn test_random_medium(&self) -> Result<()> {
        self.do_test_random(1000, false)
    }

    /// `testRandomBig()` (`@Nightly`).
    fn test_random_big(&self) -> Result<()> {
        self.do_test_random(200000, false)
    }

    /// `testMultiValued()`.
    fn test_multi_valued(&self) -> Result<()> {
        self.do_test_random(1000, true)
    }

    /// `testAllEqual()`.
    fn test_all_equal(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1000) as usize;
        let dimensions = self.dimension();
        let the_range: Rc<Vec<RangeRef>> = Rc::new(vec![range_ref(self.next_range(dimensions)?)]);
        let ranges: Vec<Rc<Vec<RangeRef>>> = vec![the_range; num_docs];
        self.verify(&ranges)
    }

    /// `testLowCardinality()`: force low cardinality leaves.
    fn test_low_cardinality(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1000) as usize;
        let dimensions = self.dimension();
        let cardinality = TestUtil::next_int(&*LuceneTestCase::random(), 2, 20);
        let mut diff_ranges: Vec<Rc<Vec<RangeRef>>> = Vec::new();
        for _ in 0..cardinality {
            diff_ranges.push(Rc::new(vec![range_ref(self.next_range(dimensions)?)]));
        }
        let mut ranges: Vec<Rc<Vec<RangeRef>>> = Vec::with_capacity(num_docs);
        for _ in 0..num_docs {
            ranges.push(Rc::clone(
                &diff_ranges[LuceneTestCase::random().next_int_bound(cardinality) as usize],
            ));
        }
        self.verify(&ranges)
    }

    /// `doTestRandom(int count, boolean multiValued)` (`private`).
    fn do_test_random(&self, count: i32, multi_valued: bool) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(count) as usize;
        let dimensions = self.dimension();

        if *VERBOSE {
            println!("TEST: numDocs={num_docs}");
        }

        let mut ranges: Vec<Option<Rc<Vec<RangeRef>>>> = vec![None; num_docs];

        let have_real_doc = true;

        'nextdoc: for id in 0..num_docs {
            let x = LuceneTestCase::random().next_int_bound(20);
            if ranges[id].is_none() {
                ranges[id] = Some(Rc::new(vec![range_ref(self.next_range(dimensions)?)]));
            }
            if x == 17 {
                // some docs don't have a box:
                ranges[id].as_ref().expect("set above")[0]
                    .borrow_mut()
                    .is_missing = true;
                if *VERBOSE {
                    println!("  id={id} is missing");
                }
                continue;
            }

            if multi_valued && LuceneTestCase::random().next_boolean() {
                // randomly add multi valued documents (up to 2 fields)
                let n = LuceneTestCase::random().next_int_bound(2) + 1;
                let mut boxes = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    boxes.push(range_ref(self.next_range(dimensions)?));
                }
                ranges[id] = Some(Rc::new(boxes));
            }

            if id > 0 && x < 9 && have_real_doc {
                let mut i = 0;
                // don't step on missing ranges:
                let old_id = loop {
                    let old_id = LuceneTestCase::random().next_int_bound(id as i32) as usize;
                    if !ranges[old_id].as_ref().expect("filled")[0]
                        .borrow()
                        .is_missing
                    {
                        break old_id;
                    } else {
                        i += 1;
                        if i > id {
                            continue 'nextdoc;
                        }
                    }
                };

                let current = Rc::clone(ranges[id].as_ref().expect("set above"));
                let old = Rc::clone(ranges[old_id].as_ref().expect("filled"));
                if x == dimensions * 2 {
                    // Fully identical box (use first box in case current is multivalued but old is
                    // not)
                    for d in 0..dimensions {
                        let min = old[0].borrow().range.get_min(d);
                        current[0].borrow_mut().range.set_min(d, min);
                        let max = old[0].borrow().range.get_max(d);
                        current[0].borrow_mut().range.set_max(d, max);
                    }
                    if *VERBOSE {
                        println!(
                            "  id={id} box={} (same box as doc={old_id})",
                            boxes_to_string(&current)
                        );
                    }
                } else {
                    let even = dimensions % 2;
                    for m in 0..dimensions * 2 {
                        if x == m {
                            let d = (m as f32 / 2.0).floor() as i32;
                            // current could be multivalue but old may not be, so use first box
                            if even == 0 {
                                // even is min
                                let min = old[0].borrow().range.get_min(d);
                                current[0].borrow_mut().range.set_min(d, min);
                                if *VERBOSE {
                                    println!(
                                        "  id={id} box={} (same min[{d}] as doc={old_id})",
                                        boxes_to_string(&current)
                                    );
                                }
                            } else {
                                // odd is max
                                let max = old[0].borrow().range.get_max(d);
                                current[0].borrow_mut().range.set_max(d, max);
                                if *VERBOSE {
                                    println!(
                                        "  id={id} box={} (same max[{d}] as doc={old_id})",
                                        boxes_to_string(&current)
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        let ranges: Vec<Rc<Vec<RangeRef>>> =
            ranges.into_iter().map(|r| r.expect("filled")).collect();
        self.verify(&ranges)
    }

    /// `verify(Range[][] ranges)` (`private`).
    fn verify(&self, ranges: &[Rc<Vec<RangeRef>>]) -> Result<()> {
        let iwc = LuceneTestCase::new_index_writer_config();
        // Else seeds may not reproduce:
        iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
        // Else we can get O(N^2) merging
        let mbd = iwc.max_buffered_docs();
        if mbd != -1 && mbd < (ranges.len() / 100) as i32 {
            iwc.set_max_buffered_docs((ranges.len() / 100) as i32)?;
        }
        let dir = if ranges.len() > 50000 {
            // Avoid slow codecs like SimpleText
            iwc.set_codec(TestUtil::get_default_codec())?;
            let simple_name = LuceneTestCase::get_test_class().simple_name().to_string();
            LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(&simple_name))
        } else {
            LuceneTestCase::new_directory()
        };

        let result = (|| -> Result<()> {
            let mut deleted: HashSet<usize> = HashSet::new();
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            for (id, boxes) in ranges.iter().enumerate() {
                let mut doc = Document::new();
                doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
                doc.add(Box::new(NumericDocValuesField::new("id", id as i64)));
                if !boxes[0].borrow().is_missing {
                    for b in boxes.iter() {
                        self.add_range(&mut doc, &*b.borrow().range)?;
                    }
                }
                w.add_document(&doc)?;
                if id > 0 && LuceneTestCase::random().next_int_bound(100) == 1 {
                    let id_to_delete = LuceneTestCase::random().next_int_bound(id as i32) as usize;
                    w.delete_documents_by_terms(vec![Term::new(
                        "id",
                        BytesRef::from_char_sequence(&id_to_delete.to_string()),
                    )])?;
                    deleted.insert(id_to_delete);
                    if *VERBOSE {
                        println!("  delete id={id_to_delete}");
                    }
                }
            }

            if LuceneTestCase::random().next_boolean() {
                w.force_merge(1)?;
            }
            let dr = directory_reader::open_with_writer(&*w)?;
            let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
            w.close()?;
            drop(w);
            let s = LuceneTestCase::new_searcher(dr);

            let dimensions = ranges[0][0].borrow().range.num_dimensions();
            let iters = LuceneTestCase::at_least(25);
            let live_docs = MultiBits::get_live_docs(s.get_index_reader())?;
            let max_doc = s.get_index_reader().max_doc();

            let outcome = (|| -> Result<()> {
                for iter in 0..iters {
                    if *VERBOSE {
                        println!("\nTEST: iter={iter} s={s:?}");
                    }

                    // occasionally test open ended bounding ranges
                    let query_range = self.next_range(dimensions)?;
                    let rv = LuceneTestCase::random().next_int_bound(4);
                    let (query_type, query) = match rv {
                        0 => (
                            QueryType::Intersects,
                            self.new_intersects_query(&*query_range)?,
                        ),
                        1 => (QueryType::Contains, self.new_contains_query(&*query_range)?),
                        2 => (QueryType::Within, self.new_within_query(&*query_range)?),
                        _ => (QueryType::Crosses, self.new_crosses_query(&*query_range)?),
                    };

                    if *VERBOSE {
                        println!("  query={}", query.to_query_string(""));
                    }

                    let hits = s.search_with_manager(
                        query,
                        &FixedBitSetCollector::create_manager(max_doc),
                    )?;
                    let mut doc_id_to_id = MultiDocValues::get_numeric_values(&r, "id")?
                        .unwrap_or_else(|| {
                            randomizedtesting::jdk::exception::throw_without_message(
                                "java.lang.NullPointerException",
                            )
                        });
                    for doc_id in 0..max_doc {
                        Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
                        let id = doc_id_to_id.long_value()? as usize;
                        let boxes = &ranges[id];
                        let deleted_doc =
                            live_docs.as_ref().is_some_and(|l| !l.get(doc_id as usize));
                        let expected = if deleted_doc {
                            // document is deleted
                            false
                        } else if boxes[0].borrow().is_missing {
                            false
                        } else {
                            self.expected_result(&*query_range, boxes, query_type)
                        };

                        if hits.get(doc_id as usize) != expected {
                            let mut b = String::new();
                            b.push_str(&format!("FAIL (iter {iter}): "));
                            if expected {
                                b.push_str(&format!(
                                    "id={id}{}should match but did not\n",
                                    if boxes.len() > 1 {
                                        " (MultiValue) "
                                    } else {
                                        " "
                                    }
                                ));
                            } else {
                                b.push_str(&format!("id={id} should not match but did\n"));
                            }
                            b.push_str(&format!(" queryRange={query_range}\n"));
                            b.push_str(&format!(
                                " box{}{}",
                                if boxes.len() > 1 { "es=" } else { "=" },
                                boxes[0].borrow().range
                            ));
                            for bx in boxes.iter().skip(1) {
                                b.push_str(", ");
                                b.push_str(&bx.borrow().range.to_string());
                            }
                            b.push_str(&format!("\n queryType={query_type}\n"));
                            b.push_str(&format!(" deleted?={deleted_doc}"));
                            Assert::fail(&format!("wrong hit (first of possibly more):\n\n{b}"));
                        }
                    }
                }
                Ok(())
            })();
            drop(s);
            drop(live_docs);
            let closed = r.close();
            outcome.and(closed)
        })();
        result.and(dir.close())
    }

    /// `addRange(Document doc, Range box)` (`protected`).
    fn add_range(&self, doc: &mut Document, r#box: &dyn Range) -> Result<()> {
        doc.add(self.new_range_field(r#box)?);
        Ok(())
    }

    /// `expectedResult(Range queryRange, Range[] range, Range.QueryType
    /// queryType)` (`protected`).
    fn expected_result(
        &self,
        query_range: &dyn Range,
        range: &[RangeRef],
        query_type: QueryType,
    ) -> bool {
        range
            .iter()
            .any(|r| self.expected_bbox_query_result(query_range, &*r.borrow().range, query_type))
    }

    /// `expectedBBoxQueryResult(Range queryRange, Range range,
    /// Range.QueryType queryType)` (`protected`).
    fn expected_bbox_query_result(
        &self,
        query_range: &dyn Range,
        range: &dyn Range,
        query_type: QueryType,
    ) -> bool {
        if query_range.is_equal(range) && query_type != QueryType::Crosses {
            return true;
        }
        let relation = range.relate(query_range);
        if query_type == QueryType::Intersects {
            return relation.is_some();
        } else if query_type == QueryType::Crosses {
            // by definition, RangeFields that CONTAIN the query are also considered to cross
            return relation == Some(query_type) || relation == Some(QueryType::Contains);
        }
        relation == Some(query_type)
    }
}

/// `Arrays.toString(ranges[id])`.
fn boxes_to_string(boxes: &[RangeRef]) -> String {
    let parts: Vec<String> = boxes.iter().map(|b| b.borrow().range.to_string()).collect();
    format!("[{}]", parts.join(", "))
}
