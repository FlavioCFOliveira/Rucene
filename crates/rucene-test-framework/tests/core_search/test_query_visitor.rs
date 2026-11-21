//! Port of `lucene/core/src/test/org/apache/lucene/search/TestQueryVisitor.java`.
//!
//! Java collects terms into `HashSet<Term>` and `HashMap<Term, Float>`; the
//! port's `Term` is ordered but not hashed, so the sets and maps are
//! `BTreeSet` and `BTreeMap`, which compare equal exactly when Java's do.

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query_visitor::{term_collector, EmptyQueryVisitor, QueryVisitor};
use rucene::search::term_query::TermQuery;
use rucene::search::{BooleanQuery, BooleanQueryBuilder, BoostQuery, Occur, Query};

use crate::core_search::support::{term, tq};
use crate::core_support::{assert_equals, class, test};

/// `TestQueryVisitor`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestQueryVisitor",
        vec![
            test("testExtractTermsEquivalent", test_extract_terms_equivalent),
            // Public, but neither `test*` nor `@Test`: the runner does not
            // run them, as Java's does not.
            test("extractAllTerms", extract_all_terms),
            test("extractTermsFromField", extract_terms_from_field),
            test("testExtractTermsAndBoosts", test_extract_terms_and_boosts),
            test("testLeafQueryTypeCounts", test_leaf_query_type_counts),
            test("testExtractMatchingTermSet", test_extract_matching_term_set),
        ],
    )
}

fn boost(q: Arc<dyn Query>, b: f32) -> Arc<dyn Query> {
    Arc::new(BoostQuery::new(q, b).expect("a finite boost"))
}

/// `private static final Query query`.
static QUERY: LazyLock<Arc<dyn Query>> = LazyLock::new(|| build_query().expect("the query builds"));

fn build_query() -> Result<Arc<dyn Query>> {
    Ok(Arc::new(
        BooleanQueryBuilder::new()
            .add(tq("field1", "t1"), Occur::MUST)?
            .add(
                Arc::new(
                    BooleanQueryBuilder::new()
                        .add(tq("field1", "tm2"), Occur::SHOULD)?
                        .add(boost(tq("field1", "tm3"), 2.0), Occur::SHOULD)?
                        .build(),
                ),
                Occur::MUST,
            )?
            .add(
                boost(
                    Arc::new(
                        PhraseQueryBuilder::new()
                            .add(term("field1", "term4"))?
                            .add(term("field1", "term5"))?
                            .build()?,
                    ),
                    3.0,
                ),
                Occur::MUST,
            )?
            .add(tq("field1", "term8"), Occur::MUST_NOT)?
            .add(
                Arc::new(PrefixQuery::new(term("field1", "term9"))?),
                Occur::SHOULD,
            )?
            .add(
                boost(
                    Arc::new(
                        BooleanQueryBuilder::new()
                            .add(boost(tq("field2", "term10"), 3.0), Occur::MUST)?
                            .build(),
                    ),
                    2.0,
                ),
                Occur::SHOULD,
            )?
            .build(),
    ))
}

fn term_set(terms: &[(&str, &str)]) -> BTreeSet<Term> {
    terms.iter().map(|(f, t)| term(f, t)).collect()
}

/// Port of `TestQueryVisitor.testExtractTermsEquivalent`.
fn test_extract_terms_equivalent() -> Result<()> {
    let mut terms = BTreeSet::new();
    let expected = term_set(&[
        ("field1", "t1"),
        ("field1", "tm2"),
        ("field1", "tm3"),
        ("field1", "term4"),
        ("field1", "term5"),
        ("field2", "term10"),
    ]);
    QUERY.visit(&mut term_collector(&mut terms));
    assert_equals(&expected, &terms);
    Ok(())
}

/// The anonymous visitor of `extractAllTerms`.
struct AllTermsVisitor<'a> {
    terms: &'a mut BTreeSet<Term>,
}

impl QueryVisitor for AllTermsVisitor<'_> {
    fn consume_terms(&mut self, _query: &dyn Query, ts: &[Term]) {
        self.terms.extend(ts.iter().cloned());
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        _occur: Occur,
        _parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        Box::new(self)
    }
}

/// Port of `TestQueryVisitor.extractAllTerms`.
fn extract_all_terms() -> Result<()> {
    let mut terms = BTreeSet::new();
    let expected = term_set(&[
        ("field1", "t1"),
        ("field1", "tm2"),
        ("field1", "tm3"),
        ("field1", "term4"),
        ("field1", "term5"),
        ("field1", "term8"),
        ("field2", "term10"),
    ]);
    QUERY.visit(&mut AllTermsVisitor { terms: &mut terms });
    assert_equals(&expected, &terms);
    Ok(())
}

/// The anonymous visitor of `extractTermsFromField`.
struct FieldTermsVisitor<'a> {
    actual: &'a mut BTreeSet<Term>,
}

impl QueryVisitor for FieldTermsVisitor<'_> {
    fn accept_field(&self, field: &str) -> bool {
        "field2" == field
    }

    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        self.actual.extend(terms.iter().cloned());
    }
}

/// Port of `TestQueryVisitor.extractTermsFromField`.
fn extract_terms_from_field() -> Result<()> {
    let mut actual = BTreeSet::new();
    let expected = term_set(&[("field2", "term10")]);
    QUERY.visit(&mut FieldTermsVisitor {
        actual: &mut actual,
    });
    assert_equals(&expected, &actual);
    Ok(())
}

/// `static class BoostedTermExtractor extends QueryVisitor`.
///
/// Java's sub-visitors share the one `termsToBoosts` map; here the map is
/// lent to each sub-visitor in turn.
struct BoostedTermExtractor<'m> {
    boost: f32,
    terms_to_boosts: &'m mut BTreeMap<Term, f32>,
}

impl QueryVisitor for BoostedTermExtractor<'_> {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        for t in terms {
            self.terms_to_boosts.insert(t.clone(), self.boost);
        }
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        if let Some(b) = parent.as_any().downcast_ref::<BoostQuery>() {
            return Box::new(BoostedTermExtractor {
                boost: self.boost * b.get_boost(),
                terms_to_boosts: &mut *self.terms_to_boosts,
            });
        }
        // `super.getSubVisitor(occur, parent)`
        if occur == Occur::MUST_NOT {
            return Box::new(EmptyQueryVisitor);
        }
        Box::new(self)
    }
}

/// Port of `TestQueryVisitor.testExtractTermsAndBoosts`.
fn test_extract_terms_and_boosts() -> Result<()> {
    let mut terms_to_boosts = BTreeMap::new();
    QUERY.visit(&mut BoostedTermExtractor {
        boost: 1.0,
        terms_to_boosts: &mut terms_to_boosts,
    });
    let mut expected = BTreeMap::new();
    expected.insert(term("field1", "t1"), 1.0f32);
    expected.insert(term("field1", "tm2"), 1.0);
    expected.insert(term("field1", "tm3"), 2.0);
    expected.insert(term("field1", "term4"), 3.0);
    expected.insert(term("field1", "term5"), 3.0);
    expected.insert(term("field2", "term10"), 6.0);
    assert_equals(&expected, &terms_to_boosts);
    Ok(())
}

/// The anonymous visitor of `testLeafQueryTypeCounts`, which counts the
/// leaves by their class.
struct LeafCounter<'a> {
    query_counts: &'a mut HashMap<TypeId, i32>,
}

impl LeafCounter<'_> {
    fn count_query(&mut self, q: &dyn Query) {
        *self.query_counts.entry(q.as_any().type_id()).or_insert(0) += 1;
    }
}

impl QueryVisitor for LeafCounter<'_> {
    fn consume_terms(&mut self, query: &dyn Query, _terms: &[Term]) {
        self.count_query(query);
    }

    fn visit_leaf(&mut self, query: &dyn Query) {
        self.count_query(query);
    }
}

/// Port of `TestQueryVisitor.testLeafQueryTypeCounts`.
fn test_leaf_query_type_counts() -> Result<()> {
    let mut query_counts: HashMap<TypeId, i32> = HashMap::new();
    QUERY.visit(&mut LeafCounter {
        query_counts: &mut query_counts,
    });
    assert_equals(&4, &query_counts[&TypeId::of::<TermQuery>()]);
    assert_equals(&1, &query_counts[&TypeId::of::<PhraseQuery>()]);
    Ok(())
}

/// `abstract static class QueryNode extends QueryVisitor` and its subclasses
/// `TermNode`, `ConjunctionNode` and `DisjunctionNode`.
enum QueryNode {
    Term(Term),
    Conjunction(Vec<QueryNode>),
    Disjunction(Vec<QueryNode>),
}

impl QueryNode {
    fn children_mut(&mut self) -> &mut Vec<QueryNode> {
        match self {
            QueryNode::Conjunction(c) | QueryNode::Disjunction(c) => c,
            QueryNode::Term(_) => {
                unreachable!("a TermNode visits no sub-query and adds no child")
            }
        }
    }

    /// `children.sort(Comparator.comparingInt(QueryNode::getWeight))`, or
    /// its `.reversed()`: a stable sort on each child's weight, which
    /// `getWeight` computes (sorting the child's own children first).
    fn sort_children(children: &mut Vec<QueryNode>, reversed: bool) {
        let weights: Vec<i32> = children.iter_mut().map(|c| c.get_weight()).collect();
        let mut paired: Vec<(i32, QueryNode)> =
            weights.into_iter().zip(children.drain(..)).collect();
        if reversed {
            paired.sort_by_key(|a| std::cmp::Reverse(a.0));
        } else {
            paired.sort_by_key(|a| a.0);
        }
        children.extend(paired.into_iter().map(|(_, c)| c));
    }

    fn get_weight(&mut self) -> i32 {
        match self {
            QueryNode::Term(t) => t.text().encode_utf16().count() as i32,
            QueryNode::Conjunction(children) => {
                Self::sort_children(children, false);
                children[0].get_weight()
            }
            QueryNode::Disjunction(children) => {
                Self::sort_children(children, true);
                children[0].get_weight()
            }
        }
    }

    fn collect_terms(&mut self, terms: &mut BTreeSet<Term>) {
        match self {
            QueryNode::Term(t) => {
                terms.insert(t.clone());
            }
            QueryNode::Conjunction(children) => {
                Self::sort_children(children, false);
                children[0].collect_terms(terms);
            }
            QueryNode::Disjunction(children) => {
                for child in children.iter_mut() {
                    child.collect_terms(terms);
                }
            }
        }
    }

    fn next_term_set(&mut self) -> bool {
        match self {
            QueryNode::Term(_) => false,
            QueryNode::Conjunction(children) => {
                Self::sort_children(children, false);
                if children[0].next_term_set() {
                    return true;
                }
                if children.len() == 1 {
                    return false;
                }
                children.remove(0);
                true
            }
            QueryNode::Disjunction(children) => {
                let mut next = false;
                for child in children.iter_mut() {
                    next |= child.next_term_set();
                }
                next
            }
        }
    }
}

impl std::fmt::Display for QueryNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (prefix, children) = match self {
            QueryNode::Term(t) => return write!(f, "TERM({t})"),
            QueryNode::Conjunction(c) => ("AND(", c),
            QueryNode::Disjunction(c) => ("OR(", c),
        };
        let joined: Vec<String> = children.iter().map(|c| c.to_string()).collect();
        write!(f, "{prefix}{})", joined.join(","))
    }
}

impl QueryVisitor for QueryNode {
    fn consume_terms(&mut self, _query: &dyn Query, terms: &[Term]) {
        // `TermNode` inherits `QueryVisitor.consumeTerms`, which does
        // nothing.
        if let QueryNode::Term(_) = self {
            return;
        }
        let children = self.children_mut();
        for t in terms {
            children.push(QueryNode::Term(t.clone()));
        }
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        if occur == Occur::MUST || occur == Occur::FILTER {
            let children = self.children_mut();
            children.push(QueryNode::Conjunction(Vec::new()));
            return Box::new(children.last_mut().expect("just pushed"));
        }
        if occur == Occur::MUST_NOT {
            return Box::new(EmptyQueryVisitor);
        }
        if let Some(bq) = parent.as_any().downcast_ref::<BooleanQuery>() {
            if !bq.get_clauses(Occur::MUST).is_empty() || !bq.get_clauses(Occur::FILTER).is_empty()
            {
                return Box::new(EmptyQueryVisitor);
            }
        }
        let children = self.children_mut();
        children.push(QueryNode::Disjunction(Vec::new()));
        Box::new(children.last_mut().expect("just pushed"))
    }
}

/// Port of `TestQueryVisitor.testExtractMatchingTermSet`.
fn test_extract_matching_term_set() -> Result<()> {
    let mut extractor = QueryNode::Conjunction(Vec::new());
    QUERY.visit(&mut extractor);
    let mut minimum_term_set = BTreeSet::new();
    extractor.collect_terms(&mut minimum_term_set);

    let expected1 = term_set(&[("field1", "t1")]);
    assert_equals(&expected1, &minimum_term_set);
    Assert::assert_true(extractor.next_term_set());
    let expected2 = term_set(&[("field1", "tm2"), ("field1", "tm3")]);
    minimum_term_set.clear();
    extractor.collect_terms(&mut minimum_term_set);
    assert_equals(&expected2, &minimum_term_set);

    let bq: Arc<dyn Query> = Arc::new(
        BooleanQueryBuilder::new()
            .add(
                Arc::new(
                    BooleanQueryBuilder::new()
                        .add(tq("f", "1"), Occur::MUST)?
                        .add(tq("f", "61"), Occur::MUST)?
                        .add(tq("f", "211"), Occur::FILTER)?
                        .add(tq("f", "5"), Occur::SHOULD)?
                        .build(),
                ),
                Occur::SHOULD,
            )?
            .add(
                Arc::new(PhraseQuery::from_strings("f", &["3333", "44444"])?),
                Occur::SHOULD,
            )?
            .build(),
    );

    let mut ex2 = QueryNode::Conjunction(Vec::new());
    bq.visit(&mut ex2);
    let expected3 = term_set(&[("f", "1"), ("f", "3333")]);
    minimum_term_set.clear();
    ex2.collect_terms(&mut minimum_term_set);
    assert_equals(&expected3, &minimum_term_set);
    ex2.get_weight(); // force sort order
    assert_equals(
        &"AND(AND(OR(AND(TERM(f:3333),TERM(f:44444)),AND(TERM(f:1),TERM(f:61),AND(TERM(f:211))))))"
            .to_string(),
        &ex2.to_string(),
    );
    Ok(())
}
