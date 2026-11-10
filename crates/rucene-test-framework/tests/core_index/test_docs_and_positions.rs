//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocsAndPositions.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::postings_enum::{
    PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
};
use rucene::index::terms::Term;
use rucene::index::{IndexReader, LeafReader};
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    arrays_to_string, assert_equals, assert_equals_msg, assert_not_null, assert_null, random,
};

/// `TestDocsAndPositions`: its field `fieldName`, set by `setUp()`.
#[derive(Default)]
struct Instance {
    field_name: String,
}

/// A test method of the instance.
fn ti(name: &str, body: fn(&Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestDocsAndPositions`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestDocsAndPositions")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut Instance| {
                LuceneTestCase::super_set_up();
                inst.field_name = format!("field{}", random().next_int());
            },
        ))
        .method(ti("testPositionsSimple", test_positions_simple))
        .method(ti("testRandomPositions", test_random_positions))
        .method(ti("testRandomDocs", test_random_docs))
        .method(ti(
            "testLargeNumberOfPositions",
            test_large_number_of_positions,
        ))
        .method(ti("testDocsEnumStart", test_docs_enum_start))
        .method(ti(
            "testDocsAndPositionsEnumStart",
            test_docs_and_positions_enum_start,
        ))
        .build()
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `setOmitNorms(true)`.
fn omit_norms_type() -> Result<FieldType> {
    let mut custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_omit_norms(true)?;
    Ok(custom_type)
}

/// `new RandomIndexWriter(random(), dir, newIndexWriterConfig(new
/// MockAnalyzer(random())))`, with `merge_policy` when given.
fn writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    log_merge_policy: bool,
) -> Result<RandomIndexWriter> {
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    if log_merge_policy {
        iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    }
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)
}

/// Simple testcase for `PostingsEnum`.
///
/// Port of `TestDocsAndPositions.testPositionsSimple`.
fn test_positions_simple(inst: &Instance) -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let writer = writer(&directory, false)?;
    for _ in 0..39 {
        let mut doc = Document::new();
        let custom_type = omit_norms_type()?;
        doc.add(Box::new(LuceneTestCase::new_field(
            &inst.field_name,
            &format!(
                "{}{}{}{}",
                "1 2 3 4 5 6 7 8 9 10 ",
                "1 2 3 4 5 6 7 8 9 10 ",
                "1 2 3 4 5 6 7 8 9 10 ",
                "1 2 3 4 5 6 7 8 9 10"
            ),
            &custom_type,
        )));
        writer.add_document(&doc)?;
    }
    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    let num = LuceneTestCase::at_least(13);
    for _ in 0..num {
        let bytes = LuceneTestCase::new_bytes_ref_str("1");
        for leaf_reader_context in Arc::clone(&reader).leaves()? {
            let leaf = leaf_reader_context.leaf_reader();
            let docs_and_pos_enum = get_docs_and_positions(inst, &leaf, &bytes)?;
            assert_not_null(&docs_and_pos_enum);
            let mut docs_and_pos_enum = docs_and_pos_enum.expect("checked by assertNotNull");
            if leaf.max_doc() == 0 {
                continue;
            }
            let advance = docs_and_pos_enum.advance(random().next_int_bound(leaf.max_doc()))?;
            loop {
                let msg = format!(
                    "Advanced to: {advance} current doc: {}",
                    docs_and_pos_enum.doc_id()
                ); // TODO: + " usePayloads: " + usePayload;
                for expected in [0, 10, 20, 30] {
                    assert_equals_msg(Some(&msg), &4, &docs_and_pos_enum.freq()?);
                    assert_equals_msg(Some(&msg), &expected, &docs_and_pos_enum.next_position()?);
                }
                if docs_and_pos_enum.next_doc()? == NO_MORE_DOCS {
                    break;
                }
            }
        }
    }
    reader.close()?;
    directory.close()
}

/// `public PostingsEnum getDocsAndPositions(LeafReader reader, BytesRef
/// bytes)`.
fn get_docs_and_positions(
    inst: &Instance,
    reader: &Arc<dyn LeafReader>,
    bytes: &BytesRef,
) -> Result<Option<Box<dyn PostingsEnum>>> {
    if let Some(terms) = reader.terms(&inst.field_name)? {
        let mut te = terms.iterator()?;
        if te.seek_exact(bytes)? {
            return Ok(Some(te.postings(None, POSTINGS_ENUM_ALL)?));
        }
    }
    Ok(None)
}

/// this test indexes random numbers within a range into a field and checks
/// their occurrences by searching for a number from that range selected at
/// random. All positions for that number are saved up front and compared to
/// the enums positions.
///
/// Port of `TestDocsAndPositions.testRandomPositions`.
fn test_random_positions(inst: &Instance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir, true)?;
    let num_docs = LuceneTestCase::at_least(47);
    let max = 1051;
    let term = random().next_int_bound(max);
    let mut positions_in_doc: Vec<Vec<i32>> = vec![Vec::new(); num_docs as usize];
    let custom_type = omit_norms_type()?;
    for i in 0..num_docs as usize {
        let mut doc = Document::new();
        let mut positions = Vec::new();
        let mut builder = String::new();
        let num = LuceneTestCase::at_least(131);
        for j in 0..num {
            let next_int = random().next_int_bound(max);
            builder.push_str(&next_int.to_string());
            builder.push(' ');
            if next_int == term {
                positions.push(j);
            }
        }
        if positions.is_empty() {
            builder.push_str(&term.to_string());
            positions.push(num);
        }
        doc.add(Box::new(LuceneTestCase::new_field(
            &inst.field_name,
            &builder,
            &custom_type,
        )));
        positions_in_doc[i] = positions;
        writer.add_document(&doc)?;
    }

    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    let num = LuceneTestCase::at_least(13);
    for i in 0..num {
        let bytes = LuceneTestCase::new_bytes_ref_str(&term.to_string());
        for leaf_reader_context in Arc::clone(&reader).leaves()? {
            let leaf = leaf_reader_context.leaf_reader();
            let docs_and_pos_enum = get_docs_and_positions(inst, &leaf, &bytes)?;
            assert_not_null(&docs_and_pos_enum);
            let mut docs_and_pos_enum = docs_and_pos_enum.expect("checked by assertNotNull");
            let max_doc = leaf.max_doc();
            // initially advance or do next doc
            let init_doc = if random().next_boolean() {
                docs_and_pos_enum.next_doc()?
            } else {
                docs_and_pos_enum.advance(random().next_int_bound(max_doc))?
            };
            // now run through the scorer and check if all positions are there...
            loop {
                let doc_id = docs_and_pos_enum.doc_id();
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                let pos = &positions_in_doc[(leaf_reader_context.doc_base() + doc_id) as usize];
                assert_equals(&(pos.len() as i32), &docs_and_pos_enum.freq()?);
                // number of positions read should be random - don't read all of them
                // allways
                let how_many = if random().next_int_bound(20) == 0 {
                    pos.len() as i32 - random().next_int_bound(pos.len() as i32)
                } else {
                    pos.len() as i32
                };
                for j in 0..how_many as usize {
                    assert_equals_msg(
                        Some(&format!(
                            "iteration: {i} initDoc: {init_doc} doc: {doc_id} base: {} positions: {}",
                            leaf_reader_context.doc_base(),
                            arrays_to_string(pos)
                        )),
                        &pos[j],
                        &docs_and_pos_enum.next_position()?,
                    );
                }

                if random().next_int_bound(10) == 0 {
                    // once is a while advance
                    let target = doc_id + 1 + random().next_int_bound(max_doc - doc_id);
                    if docs_and_pos_enum.advance(target)? == NO_MORE_DOCS {
                        break;
                    }
                }

                if docs_and_pos_enum.next_doc()? == NO_MORE_DOCS {
                    break;
                }
            }
        }
    }
    reader.close()?;
    dir.close()
}

/// Port of `TestDocsAndPositions.testRandomDocs`.
fn test_random_docs(inst: &Instance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir, true)?;
    let num_docs = LuceneTestCase::at_least(49);
    let max = 15678;
    let term = random().next_int_bound(max);
    let mut freq_in_doc = vec![0i32; num_docs as usize];
    let custom_type = omit_norms_type()?;
    for i in 0..num_docs as usize {
        let mut doc = Document::new();
        let mut builder = String::new();
        for _ in 0..199 {
            let next_int = random().next_int_bound(max);
            builder.push_str(&next_int.to_string());
            builder.push(' ');
            if next_int == term {
                freq_in_doc[i] += 1;
            }
        }
        doc.add(Box::new(LuceneTestCase::new_field(
            &inst.field_name,
            &builder,
            &custom_type,
        )));
        writer.add_document(&doc)?;
    }

    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    let num = LuceneTestCase::at_least(13);
    for i in 0..num {
        let bytes = LuceneTestCase::new_bytes_ref_str(&term.to_string());
        for context in Arc::clone(&reader).leaves()? {
            let leaf = context.leaf_reader();
            let max_doc = leaf.max_doc();
            let postings_enum = TestUtil::docs(
                &*random(),
                &Arc::clone(&leaf).as_index_reader(),
                &inst.field_name,
                &bytes,
                None,
                POSTINGS_ENUM_FREQS,
            )?;
            if find_next(
                &freq_in_doc,
                context.doc_base(),
                context.doc_base() + max_doc,
            ) == i32::MAX
            {
                assert_null(&postings_enum.map(|p| p.to_string()));
                continue;
            }
            assert_not_null(&postings_enum);
            let mut postings_enum = postings_enum.expect("checked by assertNotNull");
            postings_enum.next_doc()?;
            for j in 0..max_doc {
                if freq_in_doc[(context.doc_base() + j) as usize] != 0 {
                    assert_equals(&j, &postings_enum.doc_id());
                    assert_equals(
                        &postings_enum.freq()?,
                        &freq_in_doc[(context.doc_base() + j) as usize],
                    );
                    if i % 2 == 0 && random().next_int_bound(10) == 0 {
                        let next = find_next(
                            &freq_in_doc,
                            context.doc_base() + j + 1,
                            context.doc_base() + max_doc,
                        )
                        .wrapping_sub(context.doc_base());
                        let advanced_to = postings_enum.advance(next)?;
                        if next >= max_doc {
                            assert_equals(&NO_MORE_DOCS, &advanced_to);
                        } else {
                            Assert::assert_true_msg(
                                &format!("advanced to: {advanced_to} but should be <= {next}"),
                                next >= advanced_to,
                            );
                        }
                    } else {
                        postings_enum.next_doc()?;
                    }
                }
            }
            assert_equals_msg(
                Some(&format!(
                    "docBase: {} maxDoc: {max_doc} {postings_enum}",
                    context.doc_base()
                )),
                &NO_MORE_DOCS,
                &postings_enum.doc_id(),
            );
        }
    }

    reader.close()?;
    dir.close()
}

/// `private static int findNext(int[] docs, int pos, int max)`.
fn find_next(docs: &[i32], pos: i32, max: i32) -> i32 {
    for i in pos..max {
        if docs[i as usize] != 0 {
            return i;
        }
    }
    i32::MAX
}

/// tests retrieval of positions for terms that have a large number of
/// occurrences to force test of buffer refill during positions iteration.
///
/// Port of `TestDocsAndPositions.testLargeNumberOfPositions`.
fn test_large_number_of_positions(inst: &Instance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = writer(&dir, false)?;
    let how_many = 1000;
    let custom_type = omit_norms_type()?;
    for _ in 0..39 {
        let mut doc = Document::new();
        let mut builder = String::new();
        for j in 0..how_many {
            if j % 2 == 0 {
                builder.push_str("even ");
            } else {
                builder.push_str("odd ");
            }
        }
        doc.add(Box::new(LuceneTestCase::new_field(
            &inst.field_name,
            &builder,
            &custom_type,
        )));
        writer.add_document(&doc)?;
    }

    // now do searches
    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    let num = LuceneTestCase::at_least(13);
    for i in 0..num {
        let bytes = LuceneTestCase::new_bytes_ref_str("even");

        for leaf_reader_context in Arc::clone(&reader).leaves()? {
            let leaf = leaf_reader_context.leaf_reader();
            let docs_and_pos_enum = get_docs_and_positions(inst, &leaf, &bytes)?;
            assert_not_null(&docs_and_pos_enum);
            let mut docs_and_pos_enum = docs_and_pos_enum.expect("checked by assertNotNull");

            let max_doc = leaf.max_doc();
            // initially advance or do next doc
            let init_doc = if random().next_boolean() {
                docs_and_pos_enum.next_doc()?
            } else {
                docs_and_pos_enum.advance(random().next_int_bound(max_doc))?
            };
            let msg = format!("Iteration: {i} initDoc: {init_doc}"); // TODO: + " payloads: " + usePayload;
            assert_equals(&(how_many / 2), &docs_and_pos_enum.freq()?);
            for j in (0..how_many).step_by(2) {
                assert_equals_msg(
                    Some(&format!(
                        "position missmatch index: {j} with freq: {} -- {msg}",
                        docs_and_pos_enum.freq()?
                    )),
                    &j,
                    &docs_and_pos_enum.next_position()?,
                );
            }
        }
    }
    reader.close()?;
    dir.close()
}

/// Port of `TestDocsAndPositions.testDocsEnumStart`.
fn test_docs_enum_start(_inst: &Instance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "foo",
        "bar",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let r = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let disi = TestUtil::docs(
        &*random(),
        &Arc::clone(&r).as_index_reader(),
        "foo",
        &LuceneTestCase::new_bytes_ref_str("bar"),
        None,
        POSTINGS_ENUM_NONE,
    )?
    .unwrap_or_else(|| crate::support::npe("TestUtil.docs"));
    let mut disi = disi;
    let docid = disi.doc_id();
    assert_equals(&-1, &docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);

    // now reuse and check again
    let mut te = r
        .terms("foo")?
        .unwrap_or_else(|| crate::support::npe("r.terms(\"foo\")"))
        .iterator()?;
    Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("bar"))?);
    let mut disi = TestUtil::docs_of(&*random(), &mut *te, Some(disi), POSTINGS_ENUM_NONE)?;
    let docid = disi.doc_id();
    assert_equals(&-1, &docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);
    writer.close()?;
    Arc::clone(&r).as_index_reader().close()?;
    dir.close()
}

/// Port of `TestDocsAndPositions.testDocsAndPositionsEnumStart`.
fn test_docs_and_positions_enum_start(_inst: &Instance) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "foo",
        "bar",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let r = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let mut disi = r
        .postings(&Term::from_text("foo", "bar"), POSTINGS_ENUM_ALL)?
        .unwrap_or_else(|| crate::support::npe("r.postings"));
    let docid = disi.doc_id();
    assert_equals(&-1, &docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);

    // now reuse and check again
    let mut te = r
        .terms("foo")?
        .unwrap_or_else(|| crate::support::npe("r.terms(\"foo\")"))
        .iterator()?;
    Assert::assert_true(te.seek_exact(&LuceneTestCase::new_bytes_ref_str("bar"))?);
    let mut disi = te.postings(Some(disi), POSTINGS_ENUM_ALL)?;
    let docid = disi.doc_id();
    assert_equals(&-1, &docid);
    Assert::assert_true(disi.next_doc()? != NO_MORE_DOCS);
    writer.close()?;
    Arc::clone(&r).as_index_reader().close()?;
    dir.close()
}
