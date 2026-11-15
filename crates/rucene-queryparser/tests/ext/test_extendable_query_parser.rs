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

//! Port of `TestExtendableQueryParser.java`: testcase for the class
//! `ExtendableQueryParser`. It extends `TestQueryParser`, overriding
//! `getParser(Analyzer)`, so it also runs every inherited test method on an
//! `ExtendableQueryParser`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::term_query::TermQuery;
use rucene_queryparser::ext::extensions::{Extensions, DEFAULT_EXTENSION_FIELD_DELIMITER};

use super::extension_stub::ExtensionStub;
use crate::classic::test_query_parser::{methods, TestQueryParserFixture};
use crate::qp_support::query_parser_test_base::QueryParserTestBase;
use crate::qp_support::search_support::{fixture_class, ftest};
use crate::qp_support::{cast, expect_throws, is, CLASSIC_PARSE_EXCEPTION};

type Fixture = TestQueryParserFixture<true>;

/// `private static final char[] DELIMITERS`.
const DELIMITERS: [char; 3] = [DEFAULT_EXTENSION_FIELD_DELIMITER, '-', '|'];

/// `TestExtendableQueryParser`.
pub fn class_def() -> Arc<Class> {
    let mut m = methods::<true>();
    m.extend([
        ftest("testUnescapedExtDelimiter", test_unescaped_ext_delimiter),
        ftest("testExtFieldUnqoted", test_ext_field_unqoted),
        ftest("testExtDefaultField", test_ext_default_field),
        ftest("testExtField", test_ext_field),
    ]);
    fixture_class::<Fixture>(
        "org.apache.lucene.queryparser.ext.TestExtendableQueryParser",
        m,
    )
}

/// `newExtensions(char delimiter)`.
fn new_extensions(delimiter: char) -> Arc<Extensions> {
    Arc::new(Extensions::with_delimiter(delimiter))
}

fn class_name(q: &dyn rucene::search::Query) -> String {
    format!("class {}", q.java_class_name())
}

fn test_unescaped_ext_delimiter(t: &mut Fixture) -> Result<()> {
    let ext = new_extensions(':');
    ext.add(Some("testExt"), Some(Arc::new(ExtensionStub)));
    let mut parser = t.get_ext_parser(None, Some(ext))?;
    expect_throws(CLASSIC_PARSE_EXCEPTION, || {
        parser.parse("aField:testExt:\"foo \\& bar\"")
    });
    Ok(())
}

fn test_ext_field_unqoted(t: &mut Fixture) -> Result<()> {
    for delimiter in DELIMITERS {
        let ext = new_extensions(delimiter);
        ext.add(Some("testExt"), Some(Arc::new(ExtensionStub)));
        let field = ext.build_extension_field_with_field("testExt", "aField");
        let mut parser = t.get_ext_parser(None, Some(ext))?;
        let query = parser.parse(&format!("{field}:foo bar"))?;
        Assert::assert_true_msg(
            &format!(
                "expected instance of BooleanQuery but was {}",
                class_name(&*query)
            ),
            is::<BooleanQuery>(&*query),
        );
        let bquery = cast::<BooleanQuery>(&*query, "org.apache.lucene.search.BooleanQuery");
        let clauses = bquery.clauses();
        Assert::assert_equals(2, clauses.len());
        let query = Arc::clone(clauses[0].query());
        Assert::assert_true_msg(
            &format!(
                "expected instance of TermQuery but was {}",
                class_name(&*query)
            ),
            is::<TermQuery>(&*query),
        );
        let tquery = cast::<TermQuery>(&*query, "org.apache.lucene.search.TermQuery");
        Assert::assert_equals("aField".to_string(), tquery.get_term().field().to_string());
        Assert::assert_equals("foo".to_string(), tquery.get_term().text());

        let query = Arc::clone(clauses[1].query());
        Assert::assert_true_msg(
            &format!(
                "expected instance of TermQuery but was {}",
                class_name(&*query)
            ),
            is::<TermQuery>(&*query),
        );
        let tquery = cast::<TermQuery>(&*query, "org.apache.lucene.search.TermQuery");
        Assert::assert_equals(t.get_default_field(), tquery.get_term().field().to_string());
        Assert::assert_equals("bar".to_string(), tquery.get_term().text());
    }
    Ok(())
}

fn test_ext_default_field(t: &mut Fixture) -> Result<()> {
    for delimiter in DELIMITERS {
        let ext = new_extensions(delimiter);
        ext.add(Some("testExt"), Some(Arc::new(ExtensionStub)));
        let field = ext.build_extension_field("testExt");
        let mut parser = t.get_ext_parser(None, Some(ext))?;
        let parse = parser.parse(&format!("{field}:\"foo \\& bar\""))?;
        Assert::assert_true_msg(
            &format!(
                "expected instance of TermQuery but was {}",
                class_name(&*parse)
            ),
            is::<TermQuery>(&*parse),
        );
        let tquery = cast::<TermQuery>(&*parse, "org.apache.lucene.search.TermQuery");
        Assert::assert_equals(t.get_default_field(), tquery.get_term().field().to_string());
        Assert::assert_equals("foo & bar".to_string(), tquery.get_term().text());
    }
    Ok(())
}

fn test_ext_field(t: &mut Fixture) -> Result<()> {
    for delimiter in DELIMITERS {
        let ext = new_extensions(delimiter);
        ext.add(Some("testExt"), Some(Arc::new(ExtensionStub)));
        let field = ext.build_extension_field_with_field("testExt", "afield");
        let mut parser = t.get_ext_parser(None, Some(ext))?;
        let parse = parser.parse(&format!("{field}:\"foo \\& bar\""))?;
        Assert::assert_true_msg(
            &format!(
                "expected instance of TermQuery but was {}",
                class_name(&*parse)
            ),
            is::<TermQuery>(&*parse),
        );
        let tquery = cast::<TermQuery>(&*parse, "org.apache.lucene.search.TermQuery");
        Assert::assert_equals("afield".to_string(), tquery.get_term().field().to_string());
        Assert::assert_equals("foo & bar".to_string(), tquery.get_term().text());
    }
    Ok(())
}
