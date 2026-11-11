//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPrefixCodedTerms.java`.

use std::collections::BTreeSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::terms::{PrefixCodedTermsBuilder, Term};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, assert_null, class, random, test};

/// `TestPrefixCodedTerms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPrefixCodedTerms",
        vec![
            test("testEmpty", test_empty),
            test("testOne", test_one),
            test("testRandom", test_random),
        ],
    )
}

/// Port of `TestPrefixCodedTerms.testEmpty`.
fn test_empty() -> Result<()> {
    let b = PrefixCodedTermsBuilder::new();
    let pb = b.finish();
    let mut iter = pb.iterator();
    assert_null(&iter.next()?);
    Ok(())
}

/// Port of `TestPrefixCodedTerms.testOne`.
fn test_one() -> Result<()> {
    let term = Term::from_text("foo", "bogus");
    let mut b = PrefixCodedTermsBuilder::new();
    b.add(&term)?;
    let pb = b.finish();
    let mut iter = pb.iterator();
    let bytes = iter.next()?;
    assert_not_null(&bytes);
    assert_equals(&Some("foo"), &iter.field());
    assert_equals(
        &"bogus".to_string(),
        &bytes.expect("checked by assertNotNull").utf8_to_string()?,
    );
    assert_null(&iter.next()?);
    Ok(())
}

/// Port of `TestPrefixCodedTerms.testRandom`.
fn test_random() -> Result<()> {
    // `new TreeSet<>()`: ordered by `Term.compareTo`.
    let mut terms: BTreeSet<Term> = BTreeSet::new();
    let nterms = LuceneTestCase::at_least(10000);
    for _i in 0..nterms {
        let field = TestUtil::random_unicode_string_max(&*random(), 2);
        let text = TestUtil::random_unicode_string(&*random());
        let term = Term::from_text(field, &text);
        terms.insert(term);
    }

    let mut b = PrefixCodedTermsBuilder::new();
    for r#ref in &terms {
        b.add(r#ref)?;
    }
    let pb = b.finish();

    let mut iter = pb.iterator();
    let mut expected = terms.iter();
    assert_equals(&(terms.len() as i64), &pb.size());
    // System.out.println("TEST: now iter");
    while let Some(bytes) = iter.next()? {
        let next = expected.next();
        Assert::assert_true(next.is_some());
        let field = iter.field().expect("a field after next()").to_string();
        assert_equals(next.expect("checked"), &Term::new(field, bytes));
    }

    Assert::assert_false(expected.next().is_some());
    Ok(())
}
