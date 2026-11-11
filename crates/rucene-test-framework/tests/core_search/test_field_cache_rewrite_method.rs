//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFieldCacheRewriteMethod.java`.
//!
//! Tests the FieldcacheRewriteMethod with random regular expressions
//! (`extends TestRegexpRandom2`, overriding `assertSame`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::doc_values_rewrite_method::DocValuesRewriteMethod;
use rucene::search::multi_term_query::{constant_score_blended_rewrite, constant_score_rewrite};
use rucene::search::{DefaultProvider, Query, RegexpQuery};
use rucene::util::automaton::{RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;

use crate::core_search::support::{fixture_class, ftest, term};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_search::test_regexp_random2::{
    methods, search25, AssertSame, RegexpRandom2Fixture,
};

/// `TestFieldCacheRewriteMethod`.
pub struct FieldCacheRewriteMethod;

type Fixture = RegexpRandom2Fixture<FieldCacheRewriteMethod>;

/// `TestFieldCacheRewriteMethod`: the inherited `setUp`, `tearDown` and
/// `testRegexps`, and its own `testEquals`.
pub fn class_def() -> Arc<Class> {
    let mut methods = methods::<FieldCacheRewriteMethod>();
    methods.push(ftest("testEquals", test_equals));
    fixture_class::<Fixture>(
        "org.apache.lucene.search.TestFieldCacheRewriteMethod",
        methods,
    )
}

/// `new RegexpQuery(new Term(fieldName, regexp), RegExp.NONE, 0, name ->
/// null, Operations.DEFAULT_DETERMINIZE_WORK_LIMIT, rewriteMethod)`.
fn regexp_with(
    field: &str,
    regexp: &str,
    rewrite_method: Arc<dyn rucene::search::multi_term_query::RewriteMethod>,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(RegexpQuery::with_everything(
        term(field, regexp),
        RegExp::NONE,
        0,
        &DefaultProvider,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        rewrite_method,
        true,
    )?))
}

impl AssertSame for FieldCacheRewriteMethod {
    /// Test fieldcache rewrite against filter rewrite
    fn assert_same(f: &Fixture, regexp: &str) -> Result<()> {
        let field_cache = regexp_with(&f.field_name, regexp, Arc::new(DocValuesRewriteMethod))?;

        let filter = regexp_with(&f.field_name, regexp, constant_score_rewrite())?;

        let filter2 = regexp_with(&f.field_name, regexp, constant_score_blended_rewrite())?;

        let field_cache_docs = search25(f.searcher1(), &field_cache)?;
        let filter_docs = search25(f.searcher2(), &filter)?;
        let filter2_docs = search25(f.searcher2(), &filter2)?;

        CheckHits::check_equal(
            &*field_cache,
            &field_cache_docs.score_docs,
            &filter_docs.score_docs,
        );
        CheckHits::check_equal(
            &*field_cache,
            &field_cache_docs.score_docs,
            &filter2_docs.score_docs,
        );
        Ok(())
    }
}

fn test_equals(f: &mut Fixture) -> Result<()> {
    {
        let a1: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[aA]"),
            RegExp::NONE,
        )?);
        let a2: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[aA]"),
            RegExp::NONE,
        )?);
        let b: Arc<dyn Query> = Arc::new(RegexpQuery::with_syntax_flags(
            term(&f.field_name, "[bB]"),
            RegExp::NONE,
        )?);
        assert_query_equals(&a1, &a2);
        Assert::assert_false(a1.query_eq(&*b));
        QueryUtils::check(&*a1);
    }

    {
        let a1 = regexp_with(&f.field_name, "[aA]", Arc::new(DocValuesRewriteMethod))?;
        let a2 = regexp_with(&f.field_name, "[aA]", Arc::new(DocValuesRewriteMethod))?;
        let b = regexp_with(&f.field_name, "[bB]", Arc::new(DocValuesRewriteMethod))?;
        assert_query_equals(&a1, &a2);
        Assert::assert_false(a1.query_eq(&*b));
        QueryUtils::check(&*a1);
    }
    Ok(())
}
