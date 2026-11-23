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

//! Port of `org.apache.lucene.benchmark.byTask.feeds.TestHtmlParser`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_benchmark::by_task::feeds::demo_html_parser::Parser;
use rucene_test_framework::jdk::locale;

use crate::benchmark_support::core_support::{class, test};

/// `new Parser(new StringReader(text))`.
fn parser(text: &str) -> Result<Parser> {
    Parser::from_reader(text).map_err(|e| rucene_benchmark::jdk::exception(format!("{e:?}")))
}

/// `TestHtmlParser`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.benchmark.byTask.feeds.TestHtmlParser",
        vec![
            test("testUnicode", test_unicode),
            test("testEntities", test_entities),
            test("testComments", test_comments),
            test("testScript", test_script),
            test("testStyle", test_style),
            test("testDoctype", test_doctype),
            test("testMeta", test_meta),
            test("testTitle", test_title),
            test("testTurkish", test_turkish),
            test("testSampleTRECDoc", test_sample_trec_doc),
            test("testNoHTML", test_no_html),
            test("testivalid", test_ivalid),
        ],
    )
}

fn test_unicode() -> Result<()> {
    let p = parser("<html><body>汉语</body></html>")?;
    Assert::assert_equals("汉语", p.body.as_str());
    Ok(())
}

fn test_entities() -> Result<()> {
    let p = parser("<html><body>&#x6C49;&#x8BED;&yen;</body></html>")?;
    Assert::assert_equals("汉语¥", p.body.as_str());
    Ok(())
}

fn test_comments() -> Result<()> {
    let p = parser("<html><body>foo<!-- bar --><! baz --></body></html>")?;
    Assert::assert_equals("foo", p.body.as_str());
    Ok(())
}

fn test_script() -> Result<()> {
    let text = String::from("<html><body><script type=\"text/javascript\">")
        + "document.write(\"test\")</script>foo</body></html>";
    let p = parser(&text)?;
    Assert::assert_equals("foo", p.body.as_str());
    Ok(())
}

fn test_style() -> Result<()> {
    let text = String::from("<html><head><style type=\"text/css\">")
        + "body{background-color:blue;}</style>"
        + "</head><body>foo</body></html>";
    let p = parser(&text)?;
    Assert::assert_equals("foo", p.body.as_str());
    Ok(())
}

fn test_doctype() -> Result<()> {
    let text = String::from("<!DOCTYPE HTML PUBLIC ")
        + "\"-//W3C//DTD HTML 4.01 Transitional//EN\""
        + "\"http://www.w3.org/TR/html4/loose.dtd\">"
        + "<html><body>foo</body></html>";
    let p = parser(&text)?;
    Assert::assert_equals("foo", p.body.as_str());
    Ok(())
}

fn test_meta() -> Result<()> {
    let text = String::from("<html><head>")
        + "<meta name=\"a\" content=\"1\" />"
        + "<meta name=\"b\" content=\"2\" />"
        + "<meta name=\"keywords\" content=\"this is a test\" />"
        + "<meta http-equiv=\"Content-Type\" content=\"text/html;charset=UTF-8\" />"
        + "</head><body>foobar</body></html>";
    let p = parser(&text)?;
    let tags = &p.meta_tags;
    Assert::assert_equals(4usize, tags.size());
    Assert::assert_equals_nullable(Some("1"), tags.get_property("a"));
    Assert::assert_equals_nullable(Some("2"), tags.get_property("b"));
    Assert::assert_equals_nullable(Some("this is a test"), tags.get_property("keywords"));
    Assert::assert_equals_nullable(Some("text/html;charset=UTF-8"), tags.get_property("content-type"));
    Ok(())
}

fn test_title() -> Result<()> {
    let p = parser("<html><head><TITLE>foo</TITLE><head><body>bar</body></html>")?;
    Assert::assert_equals("foo", p.title.as_str());
    Ok(())
}

// LUCENE-2246
fn test_turkish() -> Result<()> {
    let saved = locale::get_default();
    locale::set_default(Some(locale::Locale::builder_from_language_tag("tr-TR")));
    let result = (|| -> Result<()> {
        let text = String::from("<html><HEAD><TITLE>ııı</TITLE></head><body>")
            + "<IMG SRC=\"../images/head.jpg\" WIDTH=570 HEIGHT=47 BORDER=0 ALT=\"ş\">"
            + "<a title=\"(ııı)\"></body></html>";
        let p = parser(&text)?;
        Assert::assert_equals("ııı", p.title.as_str());
        Assert::assert_equals("[ş]", p.body.as_str());
        Ok(())
    })();
    locale::set_default(saved);
    result
}

fn test_sample_trec_doc() -> Result<()> {
    let text = String::from("<html>\r\n")
        + "\r\n"
        + "<head>\r\n"
        + "<title>\r\n"
        + "TEST-000 title\r\n"
        + "</title>\r\n"
        + "</head>\r\n"
        + "\r\n"
        + "<body>\r\n"
        + "TEST-000 text\r\n"
        + "\r\n"
        + "</body>\r\n"
        + "\r\n";
    let p = parser(&text)?;
    Assert::assert_equals("TEST-000 title", p.title.as_str());
    Assert::assert_equals("TEST-000 text", rucene_benchmark::jdk::java_trim(&p.body));
    Ok(())
}

fn test_no_html() -> Result<()> {
    let p = parser("hallo")?;
    Assert::assert_equals("", p.title.as_str());
    Assert::assert_equals("hallo", p.body.as_str());
    Ok(())
}

fn test_ivalid() -> Result<()> {
    let p = parser("<title>foo</title>bar")?;
    Assert::assert_equals("foo", p.title.as_str());
    Assert::assert_equals("bar", p.body.as_str());
    Ok(())
}
