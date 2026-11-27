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

//! Port of `TestSimpleFragmentsBuilder.java`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene_highlighter::highlight::simple_html_encoder::SimpleHTMLEncoder;
use rucene_highlighter::vectorhighlight::field_frag_list::FieldFragList;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::fragments_builder::FragmentsBuilder;
use rucene_highlighter::vectorhighlight::simple_frag_list_builder::SimpleFragListBuilder;
use rucene_highlighter::vectorhighlight::simple_fragments_builder::SimpleFragmentsBuilder;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use super::abstract_test_case::{
    make1d1f_index, make1dmf_index, make_index_long_mv, make_index_long_mvb, make_index_short_mv,
    make_index_str_mv, pq_f, reopen, tq, tv_type, vh_class, Fixture, F,
};
use crate::highlighter_support::core_support::{random, JavaHashSet};
use crate::highlighter_support::search_support::ftest;

/// `TestSimpleFragmentsBuilder`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestSimpleFragmentsBuilder",
        vec![
            ftest("test1TermIndex", test_1_term_index),
            ftest("test2Frags", test_2_frags),
            ftest("test3Frags", test_3_frags),
            ftest("testTagsAndEncoder", test_tags_and_encoder),
            ftest("test1PhraseShortMV", test_1_phrase_short_mv),
            ftest("test1PhraseLongMV", test_1_phrase_long_mv),
            ftest("test1PhraseLongMVB", test_1_phrase_long_mvb),
            ftest("testUnstoredField", test_unstored_field),
            ftest("test1StrMV", test_1_str_mv),
            ftest("testMVSeparator", test_mv_separator),
            ftest(
                "testDiscreteMultiValueHighlighting",
                test_discrete_multi_value_highlighting,
            ),
            ftest(
                "testRandomDiscreteMultiValueHighlighting",
                test_random_discrete_multi_value_highlighting,
            ),
        ],
    )
}

fn tqf(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(F, text)))
}

fn strings(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|s| s.to_string()).collect()
}

fn fragment(
    sfb: &SimpleFragmentsBuilder,
    f: &Fixture,
    ffl: &mut dyn FieldFragList,
) -> Result<Option<String>> {
    sfb.create_fragment(f.reader(), 0, F, ffl)
}

fn test_1_term_index(f: &mut Fixture) -> Result<()> {
    let mut ffl = ffl(f, tqf("a"), "a")?;
    let sfb = SimpleFragmentsBuilder::new();
    Assert::assert_equals_str(
        None,
        Some("<b>a</b>"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );

    // change tags
    let sfb = SimpleFragmentsBuilder::with_tags(strings(&["["]), strings(&["]"]));
    Assert::assert_equals_str(None, Some("[a]"), fragment(&sfb, f, &mut *ffl)?.as_deref());
    Ok(())
}

fn assert_frags(actual: Option<Vec<String>>, expected: &[&str]) {
    let actual = actual.expect("fragments");
    Assert::assert_equals(expected.len(), actual.len());
    for (e, a) in expected.iter().zip(&actual) {
        Assert::assert_equals(e.to_string(), a.clone());
    }
}

fn test_2_frags(f: &mut Fixture) -> Result<()> {
    let mut ffl = ffl(f, tqf("a"), "a b b b b b b b b b b b a b a b")?;
    let sfb = SimpleFragmentsBuilder::new();
    let frags = sfb.create_fragments(f.reader(), 0, F, &mut *ffl, 3)?;
    // 3 snippets requested, but should be 2
    assert_frags(
        frags,
        &["<b>a</b> b b b b b b b b b b", "b b <b>a</b> b <b>a</b> b"],
    );
    Ok(())
}

fn test_3_frags(f: &mut Fixture) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tqf("a"), Occur::SHOULD)?;
    boolean_query.add(tqf("c"), Occur::SHOULD)?;

    let mut ffl = ffl(
        f,
        Arc::new(boolean_query.build()),
        "a b b b b b b b b b b b a b a b b b b b c a a b b",
    )?;
    let sfb = SimpleFragmentsBuilder::new();
    let frags = sfb.create_fragments(f.reader(), 0, F, &mut *ffl, 3)?;
    assert_frags(
        frags,
        &[
            "<b>a</b> b b b b b b b b b b",
            "b b <b>a</b> b <b>a</b> b b b b b c",
            "<b>c</b> <b>a</b> <b>a</b> b b",
        ],
    );
    Ok(())
}

fn test_tags_and_encoder(f: &mut Fixture) -> Result<()> {
    let mut ffl = ffl(f, tqf("a"), "<h1> a </h1>")?;
    let sfb = SimpleFragmentsBuilder::new();
    let pre_tags = strings(&["["]);
    let post_tags = strings(&["]"]);
    Assert::assert_equals_str(
        None,
        Some("&lt;h1&gt; [a] &lt;&#x2F;h1&gt;"),
        sfb.create_fragment_with_tags(
            f.reader(),
            0,
            F,
            &mut *ffl,
            &pre_tags,
            &post_tags,
            &SimpleHTMLEncoder::new(),
        )?
        .as_deref(),
    );
    Ok(())
}

fn ffl(
    f: &mut Fixture,
    query: Arc<dyn Query>,
    index_value: &str,
) -> Result<Box<dyn FieldFragList>> {
    make1d1f_index(f, index_value)?;
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    let fpl = FieldPhraseList::new(&mut stack, &fq);
    SimpleFragListBuilder::new().create_field_frag_list(&fpl, 20)
}

/// The frag list of `query` over the index `f` holds, as `SimpleFragListBuilder`
/// builds it with `frag_char_size`.
fn ffl_of(
    f: &Fixture,
    query: Arc<dyn Query>,
    frag_char_size: i32,
) -> Result<Box<dyn FieldFragList>> {
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    let fpl = FieldPhraseList::new(&mut stack, &fq);
    let sflb = SimpleFragListBuilder::new();
    sflb.create_field_frag_list(&fpl, frag_char_size)
}

fn test_1_phrase_short_mv(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    let mut ffl = ffl_of(f, tq("d"), 100)?;
    let sfb = SimpleFragmentsBuilder::new();
    // Should we probably be trimming?
    Assert::assert_equals_str(
        None,
        Some("  a b c  <b>d</b> e"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );
    Ok(())
}

fn test_1_phrase_long_mv(f: &mut Fixture) -> Result<()> {
    make_index_long_mv(f)?;
    let mut ffl = ffl_of(f, pq_f(&["search", "engines"]), 100)?;
    let sfb = SimpleFragmentsBuilder::new();
    Assert::assert_equals_str(
        None,
        Some("customization: The most <b>search engines</b> use only one of these methods. Even the <b>search engines</b> that says they can"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );
    Ok(())
}

fn test_1_phrase_long_mvb(f: &mut Fixture) -> Result<()> {
    make_index_long_mvb(f)?;
    // "speed" -(2gram)-> "sp","pe","ee","ed"
    let mut ffl = ffl_of(f, pq_f(&["sp", "pe", "ee", "ed"]), 100)?;
    let sfb = SimpleFragmentsBuilder::new();
    Assert::assert_equals_str(
        None,
        Some("additional hardware. \nWhen you talk about processing <b>speed</b>, the"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );
    Ok(())
}

fn test_unstored_field(f: &mut Fixture) -> Result<()> {
    make_unstored_index(f)?;
    let mut ffl = ffl_of(f, tq("aaa"), 100)?;
    let sfb = SimpleFragmentsBuilder::new();
    Assert::assert_true(fragment(&sfb, f, &mut *ffl)?.is_none());
    Ok(())
}

/// `makeUnstoredIndex()`.
fn make_unstored_index(f: &mut Fixture) -> Result<()> {
    let config = IndexWriterConfig::with_analyzer(f.analyzer_w.clone());
    config.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(f.dir(), config)?;
    let mut doc = Document::new();
    let custom_type = tv_type(false)?;
    doc.add(Field::new(F, "aaa".to_string(), custom_type)?);
    // doc.add( new Field( F, "aaa", Store.NO, Index.ANALYZED, TermVector.WITH_POSITIONS_OFFSETS )
    // );
    writer.add_document(&doc)?;
    writer.close()?;
    reopen(f, false)
}

fn test_1_str_mv(f: &mut Fixture) -> Result<()> {
    make_index_str_mv(f)?;
    let mut ffl = ffl_of(f, tq("defg"), 100)?;
    let mut sfb = SimpleFragmentsBuilder::new();
    sfb.set_multi_valued_separator(u16::from(b'/'));
    Assert::assert_equals_str(
        None,
        Some("abc/<b>defg</b>/hijkl"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );
    Ok(())
}

fn test_mv_separator(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    let mut ffl = ffl_of(f, tq("d"), 100)?;
    let mut sfb = SimpleFragmentsBuilder::new();
    sfb.set_multi_valued_separator(u16::from(b'/'));
    Assert::assert_equals_str(
        None,
        Some("//a b c//<b>d</b> e"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );
    Ok(())
}

fn test_discrete_multi_value_highlighting(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    let mut ffl = ffl_of(f, tq("d"), 100)?;
    let mut sfb = SimpleFragmentsBuilder::new();
    sfb.set_discrete_multi_value_highlighting(true);
    Assert::assert_equals_str(
        None,
        Some("<b>d</b> e"),
        fragment(&sfb, f, &mut *ffl)?.as_deref(),
    );

    make1dmf_index(f, &["some text to highlight", "highlight other text"])?;
    let mut ffl = ffl_of(f, tq("text"), 32)?;
    let result = sfb.create_fragments(f.reader(), 0, F, &mut *ffl, 3)?;
    assert_frags(
        result,
        &[
            "some <b>text</b> to highlight",
            "highlight other <b>text</b>",
        ],
    );

    let mut ffl = ffl_of(f, tq("highlight"), 32)?;
    let result = sfb.create_fragments(f.reader(), 0, F, &mut *ffl, 3)?;
    assert_frags(
        result,
        &["text to <b>highlight</b>", "<b>highlight</b> other text"],
    );
    Ok(())
}

fn test_random_discrete_multi_value_highlighting(_f: &mut Fixture) -> Result<()> {
    let n = 3 + random().next_int_bound(10 * *RANDOM_MULTIPLIER) as usize;
    let mut random_values: Vec<String> = Vec::with_capacity(n);
    for _i in 0..n {
        let mut random_value;
        loop {
            random_value = TestUtil::random_simple_string(&*random());
            if !random_value.is_empty() {
                break;
            }
        }
        random_values.push(random_value);
    }

    let dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;

    let custom_type = super::abstract_test_case::tv_type(true)?;

    let num_docs = random_values.len() * 5;
    let num_fields = 2 + random().next_int_bound(5) as usize;
    let num_terms = 2 + random().next_int_bound(3) as usize;
    let mut docs: Vec<Vec<Vec<String>>> = Vec::with_capacity(num_docs);
    let mut documents: Vec<Document> = Vec::with_capacity(num_docs);
    // `HashMap<String, Set<Integer>>`: only `get` is used, so the map's own
    // order does not matter; the sets iterate as `java.util.HashSet`
    let mut value_to_doc_id: HashMap<String, JavaHashSet<i32>> = HashMap::new();
    for i in 0..num_docs {
        let mut document = Document::new();
        let mut fields: Vec<Vec<String>> = Vec::with_capacity(num_fields);
        for _j in 0..num_fields {
            let mut field_values: Vec<String> = Vec::with_capacity(num_terms);
            field_values.push(get_random_value(
                &random_values,
                &mut value_to_doc_id,
                i as i32,
            ));
            let mut builder = field_values[0].clone();
            for _k in 1..num_terms {
                let v = get_random_value(&random_values, &mut value_to_doc_id, i as i32);
                builder.push(' ');
                builder.push_str(&v);
                field_values.push(v);
            }
            document.add(Field::new(F, builder, Arc::clone(&custom_type))?);
            fields.push(field_values);
        }
        docs.push(fields);
        documents.push(document);
    }
    writer.add_documents(&documents)?;
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    let result = (|| -> Result<()> {
        let highlight_iters = 1 + random().next_int_bound(120 * *RANDOM_MULTIPLIER);
        for _highlight_iter in 0..highlight_iters {
            let query_term =
                &random_values[random().next_int_bound(random_values.len() as i32) as usize];
            let random_hit = *value_to_doc_id[query_term].iter().next().expect("a doc id");
            let mut builders: Vec<String> = Vec::new();
            for field_values in &docs[random_hit as usize] {
                let mut builder = String::new();
                let mut hit = false;
                for (i, value) in field_values.iter().enumerate() {
                    if query_term == value {
                        builder.push_str("<b>");
                        builder.push_str(query_term);
                        builder.push_str("</b>");
                        hit = true;
                    } else {
                        builder.push_str(value);
                    }
                    if i != field_values.len() - 1 {
                        builder.push(' ');
                    }
                }
                if hit {
                    builders.push(builder);
                }
            }

            let query = tq(query_term);
            let fq = FieldQuery::without_reader(&query, true, true)?;
            let mut stack = FieldTermStack::new(&*reader, random_hit, F, &fq)?;

            let fpl = FieldPhraseList::new(&mut stack, &fq);
            let sflb = SimpleFragListBuilder::with_margin(100)?;
            let mut ffl = sflb.create_field_frag_list(&fpl, 300)?;

            let mut sfb = SimpleFragmentsBuilder::new();
            sfb.set_discrete_multi_value_highlighting(true);
            let actual_fragments = sfb
                .create_fragments(&*reader, random_hit, F, &mut *ffl, num_fields as i32)?
                .expect("fragments");
            Assert::assert_equals(builders.len(), actual_fragments.len());
            for (i, actual) in actual_fragments.iter().enumerate() {
                Assert::assert_equals(builders[i].clone(), actual.clone());
            }
        }
        Ok(())
    })();
    // finally
    reader.close()?;
    dir.close()?;
    result
}

fn get_random_value(
    random_values: &[String],
    value_to_doc_id: &mut HashMap<String, JavaHashSet<i32>>,
    doc_id: i32,
) -> String {
    let value = random_values[random().next_int_bound(random_values.len() as i32) as usize].clone();
    value_to_doc_id
        .entry(value.clone())
        .or_default()
        .add(doc_id);
    value
}
