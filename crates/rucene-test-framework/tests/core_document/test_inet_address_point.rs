//! Port of `lucene/core/src/test/org/apache/lucene/document/TestInetAddressPoint.java`:
//! simple tests for `InetAddressPoint`.
//!
//! **Divergence from Lucene 10.5.0 (whole class).** Java passes
//! `java.net.InetAddress` objects; the port's `InetAddressPoint` takes an
//! address as its 4- or 16-byte form (`InetAddress.getAddress()`).
//! [`get_by_name`] stands for `InetAddress.getByName(literal)`, including
//! Java's conversion of an IPv4-mapped IPv6 literal to an `Inet4Address`, and
//! two addresses are equal when their byte forms are, as
//! `Inet4Address.equals`/`Inet6Address.equals` decide for these literals.

use std::net::IpAddr;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::geo_fields::InetAddressPoint;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::Query;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};
use rucene_test_framework::util::java_exceptions::expect_throws;

/// `TestInetAddressPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestInetAddressPoint",
        vec![
            test("testBasics", test_basics),
            test("testBasicsV6", test_basics_v6),
            test("testToString", test_to_string),
            test("testQueryEquals", test_query_equals),
            test("testPrefixQuery", test_prefix_query),
            test("testNextUp", test_next_up),
            test("testNextDown", test_next_down),
        ],
    )
}

const ARITHMETIC: &str = "java.lang.ArithmeticException";

/// `InetAddress.getByName(literal)` as its `getAddress()` bytes.
fn get_by_name(literal: &str) -> Vec<u8> {
    let ip: IpAddr = literal
        .parse()
        .expect("INVARIANT: the test passes address literals");
    match ip {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            // Java's `InetAddress.getByName` returns an `Inet4Address` for an
            // IPv4-mapped literal.
            Some(v4) => v4.octets().to_vec(),
            None => v6.octets().to_vec(),
        },
    }
}

/// `assertEquals(q1, q2)` over queries (`Query.equals`, `toString` message).
fn assert_query_equals(q1: &Arc<dyn Query>, q2: &Arc<dyn Query>) {
    if !q1.query_eq(q2.as_ref()) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            q1.to_query_string(""),
            q2.to_query_string("")
        ))
    }
}

/// Indexes one `InetAddressPoint("field", address)` and returns the writer,
/// the reader and a searcher over it.
fn index_one(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    address: &[u8],
) -> Result<(RandomIndexWriter, Arc<dyn rucene::index::DirectoryReader>)> {
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    // add a doc with an address
    let mut document = Document::new();
    document.add(Box::new(InetAddressPoint::new("field", address)?));
    writer.add_document(&document)?;
    // search and verify we found our doc
    let reader = writer.get_reader()?;
    Ok((writer, reader))
}

/// Port of `testBasics`: add a single address and search for it.
fn test_basics() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let address = get_by_name("1.2.3.4");
    let (writer, reader) = index_one(&dir, &address)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_exact_query("field", &address)?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_prefix_query("field", &address, 24)?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_range_query(
            "field",
            &get_by_name("1.2.3.3"),
            &get_by_name("1.2.3.5"),
        )?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_set_query(
            "field",
            &[&get_by_name("1.2.3.4")],
        )?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_set_query(
            "field",
            &[&get_by_name("1.2.3.4"), &get_by_name("1.2.3.5")],
        )?)?,
    );
    assert_equals(
        &0,
        &searcher.count(InetAddressPoint::new_set_query(
            "field",
            &[&get_by_name("1.2.3.3")],
        )?)?,
    );
    assert_equals(
        &0,
        &searcher.count(InetAddressPoint::new_set_query("field", &[])?)?,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    writer.close()?;
    dir.close()
}

/// Port of `testBasicsV6`: add a single address and search for it.
fn test_basics_v6() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let address = get_by_name("fec0::f66d");
    let (writer, reader) = index_one(&dir, &address)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_exact_query("field", &address)?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_prefix_query("field", &address, 64)?)?,
    );
    assert_equals(
        &1,
        &searcher.count(InetAddressPoint::new_range_query(
            "field",
            &get_by_name("fec0::f66c"),
            &get_by_name("fec0::f66e"),
        )?)?,
    );

    (reader as Arc<dyn IndexReader>).close()?;
    writer.close()?;
    dir.close()
}

/// Port of `testToString`.
fn test_to_string() -> Result<()> {
    assert_equals(
        "InetAddressPoint <field:1.2.3.4>",
        format!(
            "{}",
            InetAddressPoint::new("field", &get_by_name("1.2.3.4"))?
        )
        .as_str(),
    );
    assert_equals(
        "InetAddressPoint <field:1.2.3.4>",
        format!(
            "{}",
            InetAddressPoint::new("field", &get_by_name("::FFFF:1.2.3.4"))?
        )
        .as_str(),
    );
    assert_equals(
        "InetAddressPoint <field:[fdc8:57ed:f042:ad1:f66d:4ff:fe90:ce0c]>",
        format!(
            "{}",
            InetAddressPoint::new(
                "field",
                &get_by_name("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c")
            )?
        )
        .as_str(),
    );

    assert_equals(
        "field:[1.2.3.4 TO 1.2.3.4]",
        InetAddressPoint::new_exact_query("field", &get_by_name("1.2.3.4"))?
            .to_query_string("")
            .as_str(),
    );
    assert_equals(
        "field:[0:0:0:0:0:0:0:1 TO 0:0:0:0:0:0:0:1]",
        InetAddressPoint::new_exact_query("field", &get_by_name("::1"))?
            .to_query_string("")
            .as_str(),
    );

    assert_equals(
        "field:[1.2.3.0 TO 1.2.3.255]",
        InetAddressPoint::new_prefix_query("field", &get_by_name("1.2.3.4"), 24)?
            .to_query_string("")
            .as_str(),
    );
    assert_equals(
        "field:[fdc8:57ed:f042:ad1:0:0:0:0 TO fdc8:57ed:f042:ad1:ffff:ffff:ffff:ffff]",
        InetAddressPoint::new_prefix_query(
            "field",
            &get_by_name("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c"),
            64,
        )?
        .to_query_string("")
        .as_str(),
    );
    assert_equals(
        "field:{fdc8:57ed:f042:ad1:f66d:4ff:fe90:ce0c}",
        InetAddressPoint::new_set_query(
            "field",
            &[&get_by_name("fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c")],
        )?
        .to_query_string("")
        .as_str(),
    );
    Ok(())
}

/// Port of `testQueryEquals`.
fn test_query_equals() -> Result<()> {
    let g = get_by_name;
    let q1 = InetAddressPoint::new_range_query("a", &g("1.2.3.3"), &g("1.2.3.5"))?;
    let q2 = InetAddressPoint::new_range_query("a", &g("1.2.3.3"), &g("1.2.3.5"))?;
    assert_query_equals(&q1, &q2);
    assert_equals(&q1.query_hash(), &q2.query_hash());
    Assert::assert_false(
        q1.query_eq(InetAddressPoint::new_range_query("a", &g("1.2.3.3"), &g("1.2.3.7"))?.as_ref()),
    );
    Assert::assert_false(
        q1.query_eq(InetAddressPoint::new_range_query("b", &g("1.2.3.3"), &g("1.2.3.5"))?.as_ref()),
    );

    let q1 = InetAddressPoint::new_prefix_query("a", &g("1.2.3.3"), 16)?;
    let q2 = InetAddressPoint::new_prefix_query("a", &g("1.2.3.3"), 16)?;
    assert_query_equals(&q1, &q2);
    assert_equals(&q1.query_hash(), &q2.query_hash());
    Assert::assert_false(
        q1.query_eq(InetAddressPoint::new_prefix_query("a", &g("1.1.3.5"), 16)?.as_ref()),
    );
    Assert::assert_false(
        q1.query_eq(InetAddressPoint::new_prefix_query("a", &g("1.2.3.5"), 24)?.as_ref()),
    );

    let q1 = InetAddressPoint::new_exact_query("a", &g("1.2.3.3"))?;
    let q2 = InetAddressPoint::new_exact_query("a", &g("1.2.3.3"))?;
    assert_query_equals(&q1, &q2);
    assert_equals(&q1.query_hash(), &q2.query_hash());
    Assert::assert_false(
        q1.query_eq(InetAddressPoint::new_exact_query("a", &g("1.2.3.5"))?.as_ref()),
    );

    let q1 = InetAddressPoint::new_set_query("a", &[&g("1.2.3.3"), &g("1.2.3.5")])?;
    let q2 = InetAddressPoint::new_set_query("a", &[&g("1.2.3.3"), &g("1.2.3.5")])?;
    assert_query_equals(&q1, &q2);
    assert_equals(&q1.query_hash(), &q2.query_hash());
    Assert::assert_false(
        q1.query_eq(
            InetAddressPoint::new_set_query("a", &[&g("1.2.3.3"), &g("1.2.3.7")])?.as_ref(),
        ),
    );
    Ok(())
}

/// Port of `testPrefixQuery`.
fn test_prefix_query() -> Result<()> {
    let g = get_by_name;
    assert_query_equals(
        &InetAddressPoint::new_range_query("a", &g("1.2.3.0"), &g("1.2.3.255"))?,
        &InetAddressPoint::new_prefix_query("a", &g("1.2.3.127"), 24)?,
    );
    assert_query_equals(
        &InetAddressPoint::new_range_query("a", &g("1.2.3.128"), &g("1.2.3.255"))?,
        &InetAddressPoint::new_prefix_query("a", &g("1.2.3.213"), 25)?,
    );
    assert_query_equals(
        &InetAddressPoint::new_range_query("a", &g("2001::a000:0"), &g("2001::afff:ffff"))?,
        &InetAddressPoint::new_prefix_query("a", &g("2001::a6bd:fc80"), 100)?,
    );
    Ok(())
}

/// Port of `testNextUp`.
///
fn test_next_up() -> Result<()> {
    let g = get_by_name;
    assert_equals(&g("::1"), &InetAddressPoint::next_up(&g("::"))?);

    assert_equals(&g("::1:0"), &InetAddressPoint::next_up(&g("::ffff"))?);

    assert_equals(&g("1.2.4.0"), &InetAddressPoint::next_up(&g("1.2.3.255"))?);

    assert_equals(
        &g("0.0.0.0"),
        &InetAddressPoint::next_up(&g("::fffe:ffff:ffff"))?,
    );

    assert_equals(
        &g("::1:0:0:0"),
        &InetAddressPoint::next_up(&g("255.255.255.255"))?,
    );

    let e = expect_throws(ARITHMETIC, || {
        InetAddressPoint::next_up(&g("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"))
    });
    assert_equals(
        "Overflow: there is no greater InetAddress than ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        e.message().unwrap_or_default(),
    );
    Ok(())
}

/// Port of `testNextDown`.
fn test_next_down() -> Result<()> {
    let g = get_by_name;
    assert_equals(
        &g("ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe"),
        &InetAddressPoint::next_down(&g("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"))?,
    );

    assert_equals(&g("::ffff"), &InetAddressPoint::next_down(&g("::1:0"))?);

    assert_equals(
        &g("1.2.3.255"),
        &InetAddressPoint::next_down(&g("1.2.4.0"))?,
    );

    assert_equals(
        &g("::fffe:ffff:ffff"),
        &InetAddressPoint::next_down(&g("0.0.0.0"))?,
    );

    assert_equals(
        &g("255.255.255.255"),
        &InetAddressPoint::next_down(&g("::1:0:0:0"))?,
    );

    let e = expect_throws(ARITHMETIC, || InetAddressPoint::next_down(&g("::")));
    assert_equals(
        "Underflow: there is no smaller InetAddress than 0:0:0:0:0:0:0:0",
        e.message().unwrap_or_default(),
    );
    Ok(())
}
