//! Port of `lucene/core/src/test/org/apache/lucene/document/column/TestColumnBatchTokenStreamColumn.java`:
//! tests for `TokenStreamColumn` batch indexing.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::OffsetAttribute;
use rucene::document::column::SharedTokenStreamHandle;
use rucene::document::{Document, Field, FieldType, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::postings_enum::{POSTINGS_ENUM_FREQS, POSTINGS_ENUM_POSITIONS};
use rucene::index::{DocValuesType, IndexOptions, IndexReader, LeafReader};
use rucene::search::{PhraseQuery, NO_MORE_DOCS};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::{CannedTokenStream, MockAnalyzer, Token};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use super::column_batch_test_util::{
    count_term, frozen, ft, nn, only_leaf, searcher, simple_batch, ArrayLongColumn,
    ArrayTokenStreamColumn, Ctx,
};
use crate::core_support::{assert_equals, assert_equals_msg, class, random, test, IAE};

/// `TestColumnBatchTokenStreamColumn`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.column.TestColumnBatchTokenStreamColumn",
        vec![
            test("testTokenStreamInversion", test_token_stream_inversion),
            test("testParityWithAddDocument", test_parity_with_add_document),
            test(
                "testMultiValuedAccumulatesFreq",
                test_multi_valued_accumulates_freq,
            ),
            test(
                "testMixedBatchKeepsOtherColumnsColumnar",
                test_mixed_batch_keeps_other_columns_columnar,
            ),
            test(
                "testConstructorRejectsNonIndexed",
                test_constructor_rejects_non_indexed,
            ),
            test(
                "testConstructorRejectsNonTokenized",
                test_constructor_rejects_non_tokenized,
            ),
            test("testRejectsStored", test_rejects_stored),
            test("testRejectsDocValues", test_rejects_doc_values),
            test("testRejectsPoints", test_rejects_points),
        ],
    )
}

/// `invertedType()`.
fn inverted_type() -> Result<FieldType> {
    frozen(|t| {
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        t.set_tokenized(true)
    })
}

/// `canned(terms...)`.
fn canned(terms: &[&str]) -> SharedTokenStreamHandle {
    let mut tokens = Vec::with_capacity(terms.len());
    let mut offset = 0i32;
    for t in terms {
        let len = t.encode_utf16().count() as i32;
        tokens.push(Token::with_offsets(t, offset, offset + len));
        offset += len + 1;
    }
    let final_offset = if terms.is_empty() {
        0
    } else {
        tokens[terms.len() - 1].end_offset()
    };
    Rc::new(RefCell::new(CannedTokenStream::with_final(
        0,
        final_offset,
        tokens,
    )))
}

/// `positions(leaf, field, term)`.
fn positions(leaf: &Arc<dyn LeafReader>, field: &str, term: &str) -> Result<Vec<i32>> {
    let mut all = Vec::new();
    let terms = match leaf.terms(field)? {
        None => return Ok(all),
        Some(t) => t,
    };
    let mut te = terms.iterator()?;
    if !te.seek_exact(&BytesRef::from_char_sequence(term))? {
        return Ok(all);
    }
    let mut pe = te.postings(None, POSTINGS_ENUM_POSITIONS)?;
    loop {
        let doc = pe.next_doc()?;
        if doc == NO_MORE_DOCS {
            break;
        }
        let freq = pe.freq()?;
        for _i in 0..freq {
            all.push(doc * 1000 + pe.next_position()?);
        }
    }
    Ok(all)
}

fn mock_ctx() -> Result<Ctx> {
    Ctx::open_mock_analyzer()
}

/// Port of `testTokenStreamInversion`.
fn test_token_stream_inversion() -> Result<()> {
    let c = mock_ctx()?;
    let streams = vec![
        canned(&["quick", "brown", "fox"]),
        canned(&["lazy", "brown", "dog"]),
    ];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayTokenStreamColumn::new("body", ft(&inverted_type()?), vec![0, 1], streams)?.into(),
        ],
    ))?;
    let r = c.reader()?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "body", "brown")?);
    assert_equals(&1, &count_term(&s, "body", "quick")?);
    assert_equals(&1, &count_term(&s, "body", "dog")?);
    assert_equals(&0, &count_term(&s, "body", "missing")?);
    // Positions are honored: "quick brown" matches doc 0, "brown quick" does not.
    assert_equals(
        &1,
        &s.count(Arc::new(PhraseQuery::from_strings(
            "body",
            &["quick", "brown"],
        )?))?,
    );
    assert_equals(
        &0,
        &s.count(Arc::new(PhraseQuery::from_strings(
            "body",
            &["brown", "quick"],
        )?))?,
    );
    c.close(Some(r))
}

/// Port of `testParityWithAddDocument`: the inverted postings must be
/// identical to adding the same token stream via `addDocument`.
fn test_parity_with_add_document() -> Result<()> {
    let docs: [&[&str]; 2] = [
        &["the", "quick", "brown", "fox"],
        &["a", "lazy", "brown", "dog"],
    ];
    let batch = mock_ctx()?;
    let streams = vec![canned(docs[0]), canned(docs[1])];
    batch.add_batch(&simple_batch(
        2,
        vec![
            ArrayTokenStreamColumn::new("body", ft(&inverted_type()?), vec![0, 1], streams)?.into(),
        ],
    ))?;
    let br: Arc<dyn IndexReader> = batch.reader()?;
    let batch_leaf = only_leaf(&br)?;

    let docw = Ctx::open_with(LuceneTestCase::new_index_writer_config_analyzer(Some(
        Arc::new(MockAnalyzer::new(&*random())),
    )))?;
    for doc in docs {
        let mut d = Document::new();
        d.add(Box::new(Field::new_with_token_stream(
            "body",
            canned(doc),
            inverted_type()?,
        )?));
        docw.w.add_document(&d)?;
    }
    let dr = docw.reader()?;
    let doc_leaf = only_leaf(&dr)?;
    for term in ["the", "quick", "brown", "fox", "a", "lazy", "dog"] {
        assert_equals_msg(
            Some(&format!("positions mismatch for term {term}")),
            &positions(&doc_leaf, "body", term)?,
            &positions(&batch_leaf, "body", term)?,
        );
    }
    batch.close(Some(br))?;
    docw.close(Some(dr))
}

/// Port of `testMultiValuedAccumulatesFreq`.
fn test_multi_valued_accumulates_freq() -> Result<()> {
    let c = mock_ctx()?;
    // Doc 0 has two values (repeated batch doc-id), doc 1 has one value.
    let streams = vec![
        canned(&["brown", "fox"]),
        canned(&["brown", "dog"]),
        canned(&["red", "cat"]),
    ];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayTokenStreamColumn::new("body", ft(&inverted_type()?), vec![0, 0, 1], streams)?
                .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    // "brown" appears once in each of doc 0's two values → freq 2 in doc 0.
    let mut te = nn(leaf.terms("body")?).iterator()?;
    Assert::assert_true(te.seek_exact(&BytesRef::from_char_sequence("brown"))?);
    let mut pe = te.postings(None, POSTINGS_ENUM_FREQS)?;
    assert_equals(&0, &pe.next_doc()?);
    assert_equals(&2, &pe.freq()?);
    assert_equals(&NO_MORE_DOCS, &pe.next_doc()?);
    let s = searcher(&r)?;
    assert_equals(&1, &count_term(&s, "body", "fox")?);
    assert_equals(&1, &count_term(&s, "body", "cat")?);
    c.close(Some(r))
}

/// Port of `testMixedBatchKeepsOtherColumnsColumnar`.
fn test_mixed_batch_keeps_other_columns_columnar() -> Result<()> {
    let c = mock_ctx()?;
    let streams = vec![canned(&["hello", "world"]), canned(&["goodbye", "world"])];
    c.add_batch(&simple_batch(
        2,
        vec![
            ArrayTokenStreamColumn::new("body", ft(&inverted_type()?), vec![0, 1], streams)?.into(),
            ArrayLongColumn::new(
                "num",
                ft(NumericDocValuesField::type_()),
                vec![0, 1],
                vec![10, 20],
            )?
            .into(),
        ],
    ))?;
    let r = c.reader()?;
    let leaf = only_leaf(&r)?;
    let s = searcher(&r)?;
    assert_equals(&2, &count_term(&s, "body", "world")?);
    let mut dv = nn(leaf.get_numeric_doc_values("num")?);
    assert_equals(&0, &dv.next_doc()?);
    assert_equals(&10i64, &dv.long_value()?);
    assert_equals(&1, &dv.next_doc()?);
    assert_equals(&20i64, &dv.long_value()?);
    c.close(Some(r))
}

/// Port of `testConstructorRejectsNonIndexed`.
fn test_constructor_rejects_non_indexed() -> Result<()> {
    let t = frozen(|t| t.set_tokenized(true))?;
    expect_throws(IAE, || {
        ArrayTokenStreamColumn::new("body", ft(&t), vec![0], vec![canned(&["x"])])
    });
    Ok(())
}

/// Port of `testConstructorRejectsNonTokenized`.
fn test_constructor_rejects_non_tokenized() -> Result<()> {
    let t = frozen(|t| {
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        t.set_tokenized(false)
    })?;
    expect_throws(IAE, || {
        ArrayTokenStreamColumn::new("body", ft(&t), vec![0], vec![canned(&["x"])])
    });
    Ok(())
}

/// `new FieldType()` with `DOCS_AND_FREQS_AND_POSITIONS`, tokenized, then `extra`, frozen.
fn tokenized_with(extra: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    frozen(|t| {
        t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        t.set_tokenized(true)?;
        extra(t)
    })
}

/// Port of `testRejectsStored`.
fn test_rejects_stored() -> Result<()> {
    assert_add_batch_rejected(&tokenized_with(|t| t.set_stored(true))?)
}

/// Port of `testRejectsDocValues`.
fn test_rejects_doc_values() -> Result<()> {
    assert_add_batch_rejected(&tokenized_with(|t| {
        t.set_doc_values_type(DocValuesType::SORTED)
    })?)
}

/// Port of `testRejectsPoints`.
fn test_rejects_points() -> Result<()> {
    assert_add_batch_rejected(&tokenized_with(|t| t.set_dimensions(1, 4))?)
}

/// `assertAddBatchRejected(type)`.
fn assert_add_batch_rejected(t: &FieldType) -> Result<()> {
    let c = mock_ctx()?;
    expect_throws(IAE, || {
        c.add_batch(&simple_batch(
            1,
            vec![ArrayTokenStreamColumn::new("body", ft(t), vec![0], vec![canned(&["x"])])?.into()],
        ))
    });
    c.close(None)
}
