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

//! Port of `FacetUtils.java`, of the Java package `org.apache.lucene.facet`.

use rucene::error::Result;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::util::Bits;

/// Utility class with a single method for getting a DocIdSetIterator that
/// skips deleted docs.
///
/// Equivalent to `org.apache.lucene.facet.FacetUtils`.
pub struct FacetUtils;

impl FacetUtils {
    /// Wrap the given DocIdSetIterator and liveDocs into another
    /// DocIdSetIterator that returns non-deleted documents during iteration.
    ///
    /// Equivalent to `liveDocsDISI(DocIdSetIterator, Bits)`.
    pub fn live_docs_disi<'a>(
        it: Box<dyn DocIdSetIterator + 'a>,
        live_docs: Box<dyn Bits + 'a>,
    ) -> Box<dyn DocIdSetIterator + 'a> {
        Box::new(LiveDocsDisi { it, live_docs })
    }
}

/// The anonymous `FilterDocIdSetIterator` of `liveDocsDISI`.
struct LiveDocsDisi<'a> {
    it: Box<dyn DocIdSetIterator + 'a>,
    live_docs: Box<dyn Bits + 'a>,
}

impl LiveDocsDisi<'_> {
    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        debug_assert!(doc == self.it.doc_id());
        while doc != NO_MORE_DOCS && !self.live_docs.get(doc as usize) {
            doc = self.it.next_doc()?;
        }
        Ok(doc)
    }
}

impl DocIdSetIterator for LiveDocsDisi<'_> {
    fn doc_id(&self) -> i32 {
        self.it.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.it.next_doc()?;
        self.do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.it.advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.it.cost()
    }
}
/// The two-iterator `ConjunctionDISI` that
/// `ConjunctionUtils.intersectIterators(List.of(a, b))` builds, over borrowed
/// iterators so that the caller keeps reading the doc values it intersects.
///
/// As `ConjunctionDISI`, the cheaper iterator (by `cost()`, the first on a
/// tie: `List.sort` is stable) leads and the other is advanced to it
/// (`ConjunctionDISI.java:128-160`).
pub(crate) struct Intersect2<'a> {
    lead1: &'a mut dyn DocIdSetIterator,
    lead2: &'a mut dyn DocIdSetIterator,
}

impl<'a> Intersect2<'a> {
    /// Equivalent to `ConjunctionUtils.intersectIterators(List.of(a, b))`.
    pub(crate) fn new(a: &'a mut dyn DocIdSetIterator, b: &'a mut dyn DocIdSetIterator) -> Self {
        if b.cost() < a.cost() {
            Self { lead1: b, lead2: a }
        } else {
            Self { lead1: a, lead2: b }
        }
    }

    /// Equivalent to `ConjunctionDISI.doNext(int)`.
    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            debug_assert!(doc == self.lead1.doc_id());
            let next2 = self.lead2.advance(doc)?;
            if next2 != doc {
                doc = self.lead1.advance(next2)?;
                if next2 != doc {
                    continue;
                }
            }
            return Ok(doc);
        }
    }

    /// Equivalent to `ConjunctionDISI.nextDoc()`.
    pub(crate) fn next_doc(&mut self) -> Result<i32> {
        let doc = self.lead1.next_doc()?;
        self.do_next(doc)
    }

    /// Equivalent to `ConjunctionDISI.advance(int)`.
    pub(crate) fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.lead1.advance(target)?;
        self.do_next(doc)
    }

    /// Equivalent to `ConjunctionDISI.docID()`.
    pub(crate) fn doc_id(&self) -> i32 {
        self.lead1.doc_id()
    }
}

/// The result of `DocValues.unwrapSingleton(SortedNumericDocValues)`: the
/// single-valued view, or the multi-valued instance when it is not one.
pub(crate) enum NumericValuesView {
    Single(Box<dyn rucene::index::NumericDocValues>),
    Multi(Box<dyn rucene::index::SortedNumericDocValues>),
}

/// `DocValues.unwrapSingleton(SortedNumericDocValues)`, keeping the
/// multi-valued instance when there is no single-valued view.
pub(crate) fn unwrap_singleton_numeric(
    dv: Box<dyn rucene::index::SortedNumericDocValues>,
) -> Result<NumericValuesView> {
    if dv.is_singleton() {
        if let Some(single) = dv.into_singleton()? {
            return Ok(NumericValuesView::Single(single));
        }
        unreachable!("is_singleton() promised a single-valued view");
    }
    Ok(NumericValuesView::Multi(dv))
}

/// The result of `DocValues.unwrapSingleton(SortedSetDocValues)`.
pub(crate) enum SortedValuesView {
    Single(Box<dyn rucene::index::SortedDocValues>),
    Multi(Box<dyn rucene::index::SortedSetDocValues>),
}

/// `DocValues.unwrapSingleton(SortedSetDocValues)`, keeping the multi-valued
/// instance when there is no single-valued view.
pub(crate) fn unwrap_singleton_sorted(
    dv: Box<dyn rucene::index::SortedSetDocValues>,
) -> Result<SortedValuesView> {
    if dv.is_singleton() {
        if let Some(single) = dv.into_singleton()? {
            return Ok(SortedValuesView::Single(single));
        }
        unreachable!("is_singleton() promised a single-valued view");
    }
    Ok(SortedValuesView::Multi(dv))
}
