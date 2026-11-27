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

//! The types declared in `AbstractPrefixTreeQuery.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::postings_enum::POSTINGS_ENUM_NONE;
use rucene::index::{LeafReaderContext, PostingsEnum, Terms, TermsEnum};
use rucene::search::doc_id_set::DocIdSet;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::{
    ConstantScoreScorer, ConstantScoreWeight, ConstantScoreWeightImpl, DefaultScorerSupplier,
    Query, QueryVisitor, ScoreMode, ScorerSupplier, Weight,
};
use rucene::util::bit_set::BitSet;
use rucene::util::doc_id_set::DocIdSetBuilder;
use spatial4j::shape::shape::Shape;

use crate::jdk::string_hash;
use crate::prefix::contains_prefix_tree_query::SmallDocSet;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractPrefixTreeQuery`
/// (`public abstract class AbstractPrefixTreeQuery extends Query`): base
/// class for Lucene Queries on SpatialPrefixTree fields.
///
/// This struct holds the fields; a concrete query embeds it and implements
/// [`PrefixTreeQuery`] (Java's abstract `getDocIdSet`), and its `Query`
/// methods call the bodies here (`equals`, `hashCode`, `visit`,
/// `createWeight`).
#[derive(Clone)]
pub struct AbstractPrefixTreeQuery {
    /// `queryShape`.
    pub query_shape: Arc<dyn Shape>,
    /// `fieldName`.
    pub field_name: String,
    /// `grid`: not in equals/hashCode since it's implied for a specific
    /// field.
    pub grid: Arc<dyn SpatialPrefixTree>,
    /// `detailLevel`.
    pub detail_level: i32,
}

impl fmt::Debug for AbstractPrefixTreeQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbstractPrefixTreeQuery")
            .field("query_shape", &self.query_shape)
            .field("field_name", &self.field_name)
            .field("detail_level", &self.detail_level)
            .finish()
    }
}

impl AbstractPrefixTreeQuery {
    /// `AbstractPrefixTreeQuery(Shape queryShape, String fieldName,
    /// SpatialPrefixTree grid, int detailLevel)`.
    pub fn new(
        query_shape: Arc<dyn Shape>,
        field_name: &str,
        grid: Arc<dyn SpatialPrefixTree>,
        detail_level: i32,
    ) -> Self {
        Self {
            query_shape,
            field_name: field_name.to_string(),
            grid,
            detail_level,
        }
    }

    /// `equalsTo(AbstractPrefixTreeQuery other)` (after `sameClassAs`).
    ///
    /// Equivalent to `AbstractPrefixTreeQuery.equals(Object)` with each
    /// subclass's `query_eq`: its downcast to its own type is `sameClassAs(o)`,
    /// then this is `equalsTo(getClass().cast(o))`.
    pub fn equals_to(&self, other: &AbstractPrefixTreeQuery) -> bool {
        self.detail_level == other.detail_level
            && self.field_name == other.field_name
            && self.query_shape.equals(&*other.query_shape)
    }

    /// `hashCode()`, from the query's `classHash()`.
    pub fn hash_code(&self, class_hash: i32) -> i32 {
        let mut result = class_hash;
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.query_shape.hash_code());
        result = result
            .wrapping_mul(31)
            .wrapping_add(string_hash(&self.field_name));
        result = result.wrapping_mul(31).wrapping_add(self.detail_level);
        result
    }

    /// `visit(QueryVisitor visitor)`.
    pub fn visit(&self, query: &dyn Query, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field_name) {
            visitor.visit_leaf(query);
        }
    }
}

/// The `DocIdSet` a prefix tree query computes: one of the core's, or the
/// `SmallDocSet` of `ContainsPrefixTreeQuery`, whose `iterator()` is `null`
/// when it is empty.
pub enum TreeDocIdSet {
    /// A core `DocIdSet` (a `DocIdSetBuilder`'s or a `BitDocIdSet`).
    Core(Arc<dyn DocIdSet + Send + Sync>),
    /// A `SmallDocSet`.
    Small(SmallDocSet),
}

impl TreeDocIdSet {
    /// `iterator()`: `None` (Java `null`) when there are no docs.
    ///
    /// # Errors
    ///
    /// The errors of the set.
    pub fn iterator(&self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        match self {
            TreeDocIdSet::Core(set) => Ok(Some(set.iterator()?)),
            TreeDocIdSet::Small(set) => set.java_iterator(),
        }
    }
}

/// The abstract method of `AbstractPrefixTreeQuery`, and the
/// `toString(String)` its subclasses implement.
pub trait PrefixTreeQuery: Query + Clone {
    /// The `AbstractPrefixTreeQuery` fields.
    fn tree_query(&self) -> &AbstractPrefixTreeQuery;

    /// `getDocIdSet(LeafReaderContext context)`: `None` (Java `null`) when
    /// no document matches.
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the terms.
    fn get_doc_id_set(&self, context: &LeafReaderContext) -> Result<Option<TreeDocIdSet>>;
}

/// The anonymous `ConstantScoreWeight` of `AbstractPrefixTreeQuery.createWeight`.
#[derive(Debug)]
struct PrefixTreeWeight<Q> {
    query: Q,
    score_mode: ScoreMode,
}

impl<Q: PrefixTreeQuery> ConstantScoreWeightImpl for PrefixTreeWeight<Q> {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(doc_set) = self.query.get_doc_id_set(context)? else {
            return Ok(None);
        };
        let Some(disi) = doc_set.iterator()? else {
            return Ok(None);
        };
        let scorer = ConstantScoreScorer::from_iterator(score, self.score_mode, disi);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

/// The body of `AbstractPrefixTreeQuery.createWeight(IndexSearcher
/// searcher, ScoreMode scoreMode, float boost)`.
pub fn create_weight<Q: PrefixTreeQuery>(
    query: &Q,
    score_mode: ScoreMode,
    boost: f32,
) -> Arc<dyn Weight> {
    Arc::new(ConstantScoreWeight::new(
        Arc::new(query.clone()),
        boost,
        PrefixTreeWeight {
            query: query.clone(),
            score_mode,
        },
    ))
}

/// Equivalent to `org.apache.lucene.spatial.prefix.AbstractPrefixTreeQuery.BaseTermsEnumTraverser`
/// (`public abstract class BaseTermsEnumTraverser`): holds transient state
/// and docid collecting utility methods as part of traversing a
/// `TermsEnum` for a `LeafReaderContext`.
pub struct BaseTermsEnumTraverser {
    /// `context`.
    pub context: LeafReaderContext,
    /// `maxDoc`.
    pub max_doc: i32,
    /// `terms`: maybe null.
    pub terms: Option<Arc<dyn Terms>>,
    /// `termsEnum`: remember to check for null!
    pub terms_enum: Option<Box<dyn TermsEnum>>,
    /// `postingsEnum`.
    pub postings_enum: Option<Box<dyn PostingsEnum>>,
}

impl BaseTermsEnumTraverser {
    /// `BaseTermsEnumTraverser(LeafReaderContext context)`, over the
    /// enclosing query's `fieldName`.
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the terms.
    pub fn new(context: &LeafReaderContext, field_name: &str) -> Result<Self> {
        let reader = context.leaf_reader();
        let max_doc = reader.max_doc();
        let terms = reader.terms(field_name)?;
        let terms_enum = match &terms {
            Some(terms) => Some(terms.iterator()?),
            None => None,
        };
        Ok(Self {
            context: context.clone(),
            max_doc,
            terms,
            terms_enum,
            postings_enum: None,
        })
    }

    /// `termsEnum`, which the callers have checked for null.
    pub fn terms_enum(&mut self) -> &mut Box<dyn TermsEnum> {
        self.terms_enum.as_mut().expect("termsEnum != null")
    }

    /// `postingsEnum = termsEnum.postings(postingsEnum, PostingsEnum.NONE)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the postings.
    pub fn postings(&mut self) -> Result<&mut Box<dyn PostingsEnum>> {
        let reuse = self.postings_enum.take();
        let postings = self
            .terms_enum
            .as_mut()
            .expect("termsEnum != null")
            .postings(reuse, POSTINGS_ENUM_NONE)?
            .expect("postings of a positioned TermsEnum");
        Ok(self.postings_enum.insert(postings))
    }

    /// `collectDocs(BitSet bitSet)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the postings.
    pub fn collect_docs_bit_set(&mut self, bit_set: &mut dyn BitSet) -> Result<()> {
        debug_assert!(self.terms_enum.is_some());
        let postings = self.postings()?;
        bit_set.or(&mut **postings)
    }

    /// `collectDocs(DocIdSetBuilder docSetBuilder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the postings.
    pub fn collect_docs_builder(&mut self, doc_set_builder: &mut DocIdSetBuilder) -> Result<()> {
        debug_assert!(self.terms_enum.is_some());
        let postings = self.postings()?;
        doc_set_builder.add(&mut **postings)
    }
}
