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

//! Port of `FieldQuery.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::{RewriteMethod, TopTermsScoringBooleanQueryRewrite};
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::query::Query;
use rucene::search::synonym_query::SynonymQuery;
use rucene::search::term_query::TermQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;

use crate::vectorhighlight::field_term_stack::TermInfo;

// The maximum number of different matching terms accumulated from any one
// MultiTermQuery
const MAX_MTQ_TERMS: i32 = 1024;

fn must_be_flatten_first(query: &dyn Query) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.RuntimeException",
        format!("query \"{}\" must be flatten first.", query.to_query_string("")),
    )
}

/// `LinkedHashSet.add`: appends `query` unless an equal query is present.
fn linked_add(set: &mut Vec<Arc<dyn Query>>, query: Arc<dyn Query>) {
    if !set.iter().any(|q| q.query_eq(&*query)) {
        set.push(query);
    }
}

/// The protected overridable `flatten` of `FieldQuery` a Java subclass
/// overrides (`FieldQuery.java:104-170`). Java calls it from the
/// constructor with virtual dispatch, so an override takes part in the
/// construction; the default is the base class's body, reachable as
/// [`FieldQuery::super_flatten`].
pub trait FieldQueryOverrides: Send + Sync {
    /// Equivalent to an override of the protected `flatten(Query,
    /// IndexSearcher, Collection, float)`; `flat_queries` is the
    /// `LinkedHashSet` the constructor passes.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn flatten(
        &self,
        this: &FieldQuery,
        source_query: &Arc<dyn Query>,
        searcher: Option<&IndexSearcher>,
        flat_queries: &mut Vec<Arc<dyn Query>>,
        boost: f32,
    ) -> Result<()> {
        this.super_flatten(source_query, searcher, flat_queries, boost)
    }
}

/// The overrides slot of a `FieldQuery` (Debug-printable).
#[derive(Clone, Default)]
struct FieldQueryOverridesSlot(Option<Arc<dyn FieldQueryOverrides>>);

impl std::fmt::Debug for FieldQueryOverridesSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", if self.0.is_some() { "Some(..)" } else { "None" })
    }
}

/// FieldQuery breaks down query object into terms/phrases and keeps them in
/// a QueryPhraseMap structure.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldQuery`; a
/// subclass's override of `flatten` is a [`FieldQueryOverrides`], given to
/// [`FieldQuery::with_overrides`].
#[derive(Debug)]
pub struct FieldQuery {
    pub(crate) field_match: bool,

    /// The subclass's overrides; empty for the base class.
    overrides: FieldQueryOverridesSlot,

    // fieldMatch==true,  Map<fieldName,QueryPhraseMap>
    // fieldMatch==false, Map<null,QueryPhraseMap>
    pub(crate) root_maps: HashMap<Option<String>, QueryPhraseMap>,

    // fieldMatch==true,  Map<fieldName,setOfTermsInQueries>
    // fieldMatch==false, Map<null,setOfTermsInQueries>
    pub(crate) term_set_map: HashMap<Option<String>, HashSet<String>>,

    pub(crate) term_or_phrase_number: Arc<AtomicI32>, // used for colored tag support
}

impl FieldQuery {
    /// Equivalent to `new FieldQuery(Query, IndexReader, boolean, boolean)`;
    /// `reader` is `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and Java's `RuntimeException` for a query that
    /// could not be flattened.
    pub fn new(
        query: &Arc<dyn Query>,
        reader: Option<&Arc<dyn IndexReader>>,
        phrase_highlight: bool,
        field_match: bool,
    ) -> Result<Self> {
        Self::construct(query, reader, phrase_highlight, field_match, None)
    }

    /// Equivalent to `new FieldQuery(Query, IndexReader, boolean, boolean)`
    /// of a subclass whose `flatten` override is `overrides`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_overrides(
        query: &Arc<dyn Query>,
        reader: Option<&Arc<dyn IndexReader>>,
        phrase_highlight: bool,
        field_match: bool,
        overrides: Arc<dyn FieldQueryOverrides>,
    ) -> Result<Self> {
        Self::construct(query, reader, phrase_highlight, field_match, Some(overrides))
    }

    fn construct(
        query: &Arc<dyn Query>,
        reader: Option<&Arc<dyn IndexReader>>,
        phrase_highlight: bool,
        field_match: bool,
        overrides: Option<Arc<dyn FieldQueryOverrides>>,
    ) -> Result<Self> {
        let mut this = Self {
            overrides: FieldQueryOverridesSlot(overrides),
            field_match,
            root_maps: HashMap::new(),
            term_set_map: HashMap::new(),
            term_or_phrase_number: Arc::new(AtomicI32::new(0)),
        };
        let mut flat_queries: Vec<Arc<dyn Query>> = Vec::new();
        let searcher = match reader {
            None => None,
            Some(reader) => Some(IndexSearcher::new(Arc::clone(reader))?),
        };
        this.flatten(query, searcher.as_ref(), &mut flat_queries, 1.0)?;
        this.save_terms(&flat_queries, searcher.as_ref())?;
        let expand_queries = this.expand(flat_queries)?;

        for flat_query in expand_queries {
            let counter = Arc::clone(&this.term_or_phrase_number);
            let root_map = this.get_root_map_for_query(&*flat_query, counter)?;
            root_map.add(&*flat_query, reader)?;
            let mut boost = 1.0f32;
            let mut flat_query: Arc<dyn Query> = flat_query;
            while let Some(bq) = flat_query.as_any().downcast_ref::<BoostQuery>() {
                let inner = bq.get_query();
                boost *= bq.get_boost();
                flat_query = inner;
            }
            if !phrase_highlight {
                if let Some(pq) = flat_query.as_any().downcast_ref::<PhraseQuery>() {
                    if pq.get_terms().len() > 1 {
                        for term in pq.get_terms() {
                            root_map.add_term(term, boost);
                        }
                    }
                }
            }
        }
        Ok(this)
    }

    /// For backward compatibility of
    /// `FastVectorHighlighter.getFieldQuery(Query)`.
    ///
    /// Equivalent to the package-private `new FieldQuery(Query, boolean,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn without_reader(
        query: &Arc<dyn Query>,
        phrase_highlight: bool,
        field_match: bool,
    ) -> Result<Self> {
        Self::new(query, None, phrase_highlight, field_match)
    }

    /// Equivalent to the protected `flatten(Query, IndexSearcher,
    /// Collection, float)`; `flat_queries` is Java's `LinkedHashSet`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn flatten(
        &self,
        source_query: &Arc<dyn Query>,
        searcher: Option<&IndexSearcher>,
        flat_queries: &mut Vec<Arc<dyn Query>>,
        boost: f32,
    ) -> Result<()> {
        match &self.overrides.0 {
            Some(o) => o.flatten(self, source_query, searcher, flat_queries, boost),
            None => self.super_flatten(source_query, searcher, flat_queries, boost),
        }
    }

    /// The base class's `flatten(Query, IndexSearcher, Collection, float)`;
    /// its recursive calls dispatch to [`flatten`](Self::flatten), as
    /// Java's virtual calls do.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn super_flatten(
        &self,
        source_query: &Arc<dyn Query>,
        searcher: Option<&IndexSearcher>,
        flat_queries: &mut Vec<Arc<dyn Query>>,
        boost: f32,
    ) -> Result<()> {
        let mut boost = boost;
        let mut source_query: Arc<dyn Query> = Arc::clone(source_query);
        while let Some(bq) = source_query.as_any().downcast_ref::<BoostQuery>() {
            let inner = bq.get_query();
            boost *= bq.get_boost();
            source_query = inner;
        }
        let any = source_query.as_any();
        if let Some(bq) = any.downcast_ref::<BooleanQuery>() {
            for clause in bq.clauses() {
                if !clause.is_prohibited() {
                    self.flatten(clause.query(), searcher, flat_queries, boost)?;
                }
            }
        } else if let Some(dmq) = any.downcast_ref::<DisjunctionMaxQuery>() {
            for query in dmq.get_disjuncts() {
                self.flatten(query, searcher, flat_queries, boost)?;
            }
        } else if any.is::<TermQuery>() {
            if boost != 1.0 {
                source_query = Arc::new(BoostQuery::new(source_query, boost)?);
            }
            linked_add(flat_queries, source_query);
        } else if let Some(syn_query) = any.downcast_ref::<SynonymQuery>() {
            for term in syn_query.get_terms() {
                let q: Arc<dyn Query> = Arc::new(TermQuery::new(term));
                self.flatten(&q, searcher, flat_queries, boost)?;
            }
        } else if let Some(pq) = any.downcast_ref::<PhraseQuery>() {
            if pq.get_terms().len() == 1 {
                source_query = Arc::new(TermQuery::new(pq.get_terms()[0].clone()));
            }
            if boost != 1.0 {
                source_query = Arc::new(BoostQuery::new(source_query, boost)?);
            }
            linked_add(flat_queries, source_query);
        } else if let Some(csq) = any.downcast_ref::<ConstantScoreQuery>() {
            let q = csq.get_query();
            self.flatten(&q, searcher, flat_queries, boost)?;
        } else if let Some(fsq) = any.downcast_ref::<FunctionScoreQuery>() {
            let q = Arc::clone(fsq.get_wrapped_query());
            self.flatten(&q, searcher, flat_queries, boost)?;
        } else if let Some(searcher) = searcher {
            let rewritten: Option<Arc<dyn Query>> = match source_query.as_multi_term_query() {
                Some(mtq) => Some(
                    TopTermsScoringBooleanQueryRewrite::new(MAX_MTQ_TERMS)
                        .rewrite(searcher, mtq.to_multi_term_query_arc())?,
                ),
                None => source_query.rewrite(searcher)?,
            };
            if let Some(rewritten) = rewritten {
                if !Arc::ptr_eq(&rewritten, &source_query) {
                    self.flatten(&rewritten, Some(searcher), flat_queries, boost)?;
                }
            }
        }
        // else discard queries
        Ok(())
    }

    /// Create expandQueries from flatQueries.
    ///
    /// ```text
    /// expandQueries := flatQueries + overlapped phrase queries
    ///
    /// ex1) flatQueries={a,b,c}
    ///      =&gt; expandQueries={a,b,c}
    /// ex2) flatQueries={a,"b c","c d"}
    ///      =&gt; expandQueries={a,"b c","c d","b c d"}
    /// ```
    ///
    /// Equivalent to the package-private `expand(Collection)`, which empties
    /// `flat_queries` (Java removes each query as it reads it).
    ///
    /// # Errors
    ///
    /// What building an overlapped phrase raises.
    pub(crate) fn expand(&self, flat_queries: Vec<Arc<dyn Query>>) -> Result<Vec<Arc<dyn Query>>> {
        let mut expand_queries: Vec<Arc<dyn Query>> = Vec::new();
        for (i, query) in flat_queries.iter().enumerate() {
            linked_add(&mut expand_queries, Arc::clone(query));
            let mut query_boost = 1.0f32;
            let mut query: Arc<dyn Query> = Arc::clone(query);
            while let Some(bq) = query.as_any().downcast_ref::<BoostQuery>() {
                query_boost *= bq.get_boost();
                let inner = bq.get_query();
                query = inner;
            }
            let Some(query) = query.as_any().downcast_ref::<PhraseQuery>() else {
                continue;
            };
            for qj in &flat_queries[i + 1..] {
                let mut qj_boost = 1.0f32;
                let mut qj: Arc<dyn Query> = Arc::clone(qj);
                while let Some(bq) = qj.as_any().downcast_ref::<BoostQuery>() {
                    qj_boost *= bq.get_boost();
                    let inner = bq.get_query();
                    qj = inner;
                }
                let Some(qj) = qj.as_any().downcast_ref::<PhraseQuery>() else {
                    continue;
                };
                self.check_overlap_phrases(&mut expand_queries, query, query_boost, qj, qj_boost)?;
            }
        }
        Ok(expand_queries)
    }

    /// Check if PhraseQuery A and B have overlapped part.
    ///
    /// ```text
    /// ex1) A="a b", B="b c" =&gt; overlap; expandQueries={"a b c"}
    /// ex2) A="b c", B="a b" =&gt; overlap; expandQueries={"a b c"}
    /// ex3) A="a b", B="c d" =&gt; no overlap; expandQueries={}
    /// ```
    ///
    /// Equivalent to the private `checkOverlap(Collection, PhraseQuery,
    /// float, PhraseQuery, float)`.
    fn check_overlap_phrases(
        &self,
        expand_queries: &mut Vec<Arc<dyn Query>>,
        a: &PhraseQuery,
        a_boost: f32,
        b: &PhraseQuery,
        b_boost: f32,
    ) -> Result<()> {
        if a.get_slop() != b.get_slop() {
            return Ok(());
        }
        let ats = a.get_terms();
        let bts = b.get_terms();
        if self.field_match && ats[0].field() != bts[0].field() {
            return Ok(());
        }
        Self::check_overlap(expand_queries, ats, bts, a.get_slop(), a_boost)?;
        Self::check_overlap(expand_queries, bts, ats, b.get_slop(), b_boost)
    }

    /// Check if src and dest have overlapped part and if it is, create
    /// PhraseQueries and add expandQueries.
    ///
    /// ```text
    /// ex1) src="a b", dest="c d"       =&gt; no overlap
    /// ex2) src="a b", dest="a b c"     =&gt; no overlap
    /// ex3) src="a b", dest="b c"       =&gt; overlap; expandQueries={"a b c"}
    /// ex4) src="a b c", dest="b c d"   =&gt; overlap; expandQueries={"a b c d"}
    /// ex5) src="a b c", dest="b c"     =&gt; no overlap
    /// ex6) src="a b c", dest="b"       =&gt; no overlap
    /// ex7) src="a a a a", dest="a a a" =&gt; overlap;
    ///                                     expandQueries={"a a a a a","a a a a a a"}
    /// ex8) src="a b c d", dest="b c"   =&gt; no overlap
    /// ```
    ///
    /// Equivalent to the private `checkOverlap(Collection, Term[], Term[],
    /// int, float)`.
    fn check_overlap(
        expand_queries: &mut Vec<Arc<dyn Query>>,
        src: &[Term],
        dest: &[Term],
        slop: i32,
        boost: f32,
    ) -> Result<()> {
        // beginning from 1 (not 0) is safe because that the PhraseQuery has
        // multiple terms should guarantee that the length of src is more than
        // 1
        for i in 1..src.len() {
            let mut overlap = true;
            for j in i..src.len() {
                if (j - i) < dest.len() && src[j].text() != dest[j - i].text() {
                    overlap = false;
                    break;
                }
            }
            if overlap && src.len() - i < dest.len() {
                let mut pq_builder = phrase_query::Builder::new();
                for src_term in src {
                    pq_builder.add(src_term.clone())?;
                }
                for dest_term in &dest[src.len() - i..] {
                    pq_builder.add(Term::from_text_nullable_field(
                        src[0].field_opt(),
                        &dest_term.text(),
                    ))?;
                }
                pq_builder.set_slop(slop);
                let mut pq: Arc<dyn Query> = Arc::new(pq_builder.build()?);
                if boost != 1.0 {
                    pq = Arc::new(BoostQuery::new(pq, 1.0)?);
                }
                linked_add(expand_queries, pq);
            }
        }
        Ok(())
    }

    /// Equivalent to the package-private `getRootMap(Query)`.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` for a query that is not flattened.
    pub(crate) fn get_root_map_for_query(
        &mut self,
        query: &dyn Query,
        counter: Arc<AtomicI32>,
    ) -> Result<&mut QueryPhraseMap> {
        let key = self.get_key(query)?;
        Ok(self
            .root_maps
            .entry(key)
            .or_insert_with(|| QueryPhraseMap::with_counter(counter)))
    }

    /// Return 'key' string. 'key' is the field name of the Query. If not
    /// fieldMatch, 'key' will be null.
    ///
    /// Equivalent to the private `getKey(Query)`.
    fn get_key(&self, query: &dyn Query) -> Result<Option<String>> {
        if !self.field_match {
            return Ok(None);
        }
        let mut held: Option<Arc<dyn Query>> = None;
        loop {
            let current: &dyn Query = held.as_deref().unwrap_or(query);
            match current.as_any().downcast_ref::<BoostQuery>() {
                Some(bq) => {
                    let inner = bq.get_query();
                    held = Some(inner);
                }
                None => break,
            }
        }
        let query: &dyn Query = held.as_deref().unwrap_or(query);
        let any = query.as_any();
        if let Some(tq) = any.downcast_ref::<TermQuery>() {
            Ok(tq.get_term().field_opt().map(str::to_string))
        } else if let Some(pq) = any.downcast_ref::<PhraseQuery>() {
            let terms = pq.get_terms();
            let Some(first) = terms.first() else {
                return Err(LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    "Index 0 out of bounds for length 0",
                ));
            };
            Ok(first.field_opt().map(str::to_string))
        } else if let Some(mtq) = query.as_multi_term_query() {
            Ok(Some(mtq.get_field().to_string()))
        } else {
            Err(must_be_flatten_first(query))
        }
    }

    /// Save the set of terms in the queries to termSetMap.
    ///
    /// ```text
    /// ex1) q=name:john
    ///      - fieldMatch==true
    ///          termSetMap=Map<"name",Set<"john">>
    ///      - fieldMatch==false
    ///          termSetMap=Map<null,Set<"john">>
    ///
    /// ex2) q=name:john title:manager
    ///      - fieldMatch==true
    ///          termSetMap=Map<"name",Set<"john">,
    ///                         "title",Set<"manager">>
    ///      - fieldMatch==false
    ///          termSetMap=Map<null,Set<"john","manager">>
    ///
    /// ex3) q=name:"john lennon"
    ///      - fieldMatch==true
    ///          termSetMap=Map<"name",Set<"john","lennon">>
    ///      - fieldMatch==false
    ///          termSetMap=Map<null,Set<"john","lennon">>
    /// ```
    ///
    /// Equivalent to the package-private `saveTerms(Collection,
    /// IndexSearcher)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors, and Java's `RuntimeException` for a query that
    /// is not flattened.
    pub(crate) fn save_terms(
        &mut self,
        flat_queries: &[Arc<dyn Query>],
        searcher: Option<&IndexSearcher>,
    ) -> Result<()> {
        for query in flat_queries {
            let mut query: Arc<dyn Query> = Arc::clone(query);
            while let Some(bq) = query.as_any().downcast_ref::<BoostQuery>() {
                let inner = bq.get_query();
                query = inner;
            }
            let key = self.get_key(&*query)?;
            let any = query.as_any();
            let term_set = self.term_set_map.entry(key).or_default();
            if let Some(tq) = any.downcast_ref::<TermQuery>() {
                term_set.insert(tq.get_term().text());
            } else if let Some(pq) = any.downcast_ref::<PhraseQuery>() {
                for term in pq.get_terms() {
                    term_set.insert(term.text());
                }
            } else if let (true, Some(searcher)) = (query.is_multi_term_query(), searcher) {
                let rewritten = query.rewrite(searcher)?.unwrap_or_else(|| Arc::clone(&query));
                let Some(mtq_terms) = rewritten.as_any().downcast_ref::<BooleanQuery>() else {
                    return Err(class_cast(&*rewritten, "org.apache.lucene.search.BooleanQuery"));
                };
                for clause in mtq_terms.clauses() {
                    let Some(tq) = clause.query().as_any().downcast_ref::<TermQuery>() else {
                        return Err(class_cast(&**clause.query(), "org.apache.lucene.search.TermQuery"));
                    };
                    term_set.insert(tq.get_term().text());
                }
            } else {
                return Err(must_be_flatten_first(&*query));
            }
        }
        Ok(())
    }

    /// Equivalent to the package-private `getTermSet(String)`.
    pub(crate) fn get_term_set(&self, field: &str) -> Option<&HashSet<String>> {
        let key = if self.field_match {
            Some(field.to_string())
        } else {
            None
        };
        self.term_set_map.get(&key)
    }

    /// Equivalent to `getFieldTermMap(String, String)`: `field_name` and the
    /// term; returns QueryPhraseMap.
    pub fn get_field_term_map(&self, field_name: &str, term: &str) -> Option<&QueryPhraseMap> {
        let root_map = self.get_root_map(field_name)?;
        root_map.sub_map.get(term)
    }

    /// Equivalent to `searchPhrase(String, List)`: `field_name` and the
    /// phrase candidate; returns QueryPhraseMap.
    pub fn search_phrase(
        &self,
        field_name: &str,
        phrase_candidate: &[TermInfo],
    ) -> Option<&QueryPhraseMap> {
        let root = self.get_root_map(field_name)?;
        root.search_phrase(phrase_candidate)
    }

    /// Equivalent to the private `getRootMap(String)`.
    fn get_root_map(&self, field_name: &str) -> Option<&QueryPhraseMap> {
        let key = if self.field_match {
            Some(field_name.to_string())
        } else {
            None
        };
        self.root_maps.get(&key)
    }

    /// Equivalent to the package-private `nextTermOrPhraseNumber()`.
    pub(crate) fn next_term_or_phrase_number(&self) -> i32 {
        self.term_or_phrase_number.fetch_add(1, Ordering::Relaxed)
    }

    /// Equivalent to reading the package-private field `rootMaps`.
    pub fn root_maps(&self) -> &HashMap<Option<String>, QueryPhraseMap> {
        &self.root_maps
    }

    /// Equivalent to reading the package-private field `termSetMap`.
    pub fn term_set_map(&self) -> &HashMap<Option<String>, HashSet<String>> {
        &self.term_set_map
    }

    /// Equivalent to reading the package-private field `termOrPhraseNumber`.
    pub fn term_or_phrase_number(&self) -> i32 {
        self.term_or_phrase_number.load(Ordering::Relaxed)
    }
}

/// Java's `ClassCastException` of a cast of `query` to `class`.
fn class_cast(query: &dyn Query, class: &str) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class {}",
            query.java_class_name(),
            class
        ),
    )
}

/// Internal structure of a query for highlighting: represents a nested
/// query structure.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldQuery.QueryPhraseMap`.
#[derive(Debug, Clone)]
pub struct QueryPhraseMap {
    pub(crate) terminal: bool,
    pub(crate) slop: i32, // valid if terminal == true and phraseHighlight == true
    pub(crate) boost: f32, // valid if terminal == true
    pub(crate) term_or_phrase_number: i32, // valid if terminal == true
    /// The `FieldQuery`'s phrase-number counter (Java keeps the
    /// `FieldQuery` itself, for `nextTermOrPhraseNumber()`).
    field_query_counter: Arc<AtomicI32>,
    pub(crate) sub_map: HashMap<String, QueryPhraseMap>,
}

impl QueryPhraseMap {
    /// Equivalent to `new QueryPhraseMap(FieldQuery)`.
    pub fn new(field_query: &FieldQuery) -> Self {
        Self::with_counter(Arc::clone(&field_query.term_or_phrase_number))
    }

    fn with_counter(field_query_counter: Arc<AtomicI32>) -> Self {
        Self {
            terminal: false,
            slop: 0,
            boost: 0.0,
            term_or_phrase_number: 0,
            field_query_counter,
            sub_map: HashMap::new(),
        }
    }

    /// Equivalent to the package-private `addTerm(Term, float)`.
    pub(crate) fn add_term(&mut self, term: &Term, boost: f32) {
        let counter = Arc::clone(&self.field_query_counter);
        let map = Self::get_or_new_map(&mut self.sub_map, &term.text(), counter);
        map.mark_terminal(boost);
    }

    fn get_or_new_map<'a>(
        sub_map: &'a mut HashMap<String, QueryPhraseMap>,
        term: &str,
        counter: Arc<AtomicI32>,
    ) -> &'a mut QueryPhraseMap {
        sub_map
            .entry(term.to_string())
            .or_insert_with(|| QueryPhraseMap::with_counter(counter))
    }

    /// Equivalent to the package-private `add(Query, IndexReader)`.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` for a query that is not flattened.
    pub(crate) fn add(
        &mut self,
        query: &dyn Query,
        _reader: Option<&Arc<dyn IndexReader>>,
    ) -> Result<()> {
        let mut boost = 1.0f32;
        let mut held: Option<Arc<dyn Query>> = None;
        loop {
            let current: &dyn Query = held.as_deref().unwrap_or(query);
            match current.as_any().downcast_ref::<BoostQuery>() {
                Some(bq) => {
                    boost = bq.get_boost();
                    let inner = bq.get_query();
                    held = Some(inner);
                }
                None => break,
            }
        }
        let current: &dyn Query = held.as_deref().unwrap_or(query);
        let any = current.as_any();
        if let Some(tq) = any.downcast_ref::<TermQuery>() {
            self.add_term(tq.get_term(), boost);
        } else if let Some(pq) = any.downcast_ref::<PhraseQuery>() {
            let terms = pq.get_terms();
            let counter = Arc::clone(&self.field_query_counter);
            let mut qpm: &mut QueryPhraseMap = self;
            for term in terms {
                qpm = Self::get_or_new_map(&mut qpm.sub_map, &term.text(), Arc::clone(&counter));
            }
            qpm.mark_terminal_with_slop(pq.get_slop(), boost);
        } else {
            return Err(must_be_flatten_first(current));
        }
        Ok(())
    }

    /// Equivalent to `getTermMap(String)`.
    pub fn get_term_map(&self, term: &str) -> Option<&QueryPhraseMap> {
        self.sub_map.get(term)
    }

    fn mark_terminal(&mut self, boost: f32) {
        self.mark_terminal_with_slop(0, boost);
    }

    fn mark_terminal_with_slop(&mut self, slop: i32, boost: f32) {
        self.terminal = true;
        self.slop = slop;
        self.boost = boost;
        self.term_or_phrase_number = self.field_query_counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Equivalent to `isTerminal()`.
    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    /// Equivalent to `getSlop()`.
    pub fn get_slop(&self) -> i32 {
        self.slop
    }

    /// Equivalent to `getBoost()`.
    pub fn get_boost(&self) -> f32 {
        self.boost
    }

    /// Equivalent to `getTermOrPhraseNumber()`.
    pub fn get_term_or_phrase_number(&self) -> i32 {
        self.term_or_phrase_number
    }

    /// Equivalent to `searchPhrase(List)`.
    pub fn search_phrase(&self, phrase_candidate: &[TermInfo]) -> Option<&QueryPhraseMap> {
        let mut curr_map = self;
        for ti in phrase_candidate {
            curr_map = curr_map.sub_map.get(ti.get_text())?;
        }
        if curr_map.is_valid_term_or_phrase(phrase_candidate) {
            Some(curr_map)
        } else {
            None
        }
    }

    /// Equivalent to `isValidTermOrPhrase(List)`.
    pub fn is_valid_term_or_phrase(&self, phrase_candidate: &[TermInfo]) -> bool {
        // check terminal
        if !self.terminal {
            return false;
        }

        // if the candidate is a term, it is valid
        if phrase_candidate.len() == 1 {
            return true;
        }

        // else check whether the candidate is valid phrase
        // compare position-gaps between terms to slop
        let mut pos = phrase_candidate[0].get_position();
        for ti in &phrase_candidate[1..] {
            let next_pos = ti.get_position();
            if (next_pos - pos - 1).wrapping_abs() > self.slop {
                return false;
            }
            pos = next_pos;
        }
        true
    }
}
