//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BDocs.java`.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, StringField};
use rucene::error::Result;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::IndexWriter;
use rucene::index::POSTINGS_ENUM_FREQS;
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, SuppressSysoutChecks,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class_annotated, random, test};
use crate::support::{never_throttle, open_reader, println_flush, two_b_config};

/// `Test2BDocs`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BDocs",
        vec![test("test2BDocs", test_2b_docs)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(TimeoutSuite {
                millis: 80 * 3_600_000, // effectively no limit
            })
            .annotate(Monster {
                value: "Takes ~30min".to_string(),
            })
            .annotate(SuppressSysoutChecks {
                bug_url: "Stuff gets printed".to_string(),
            })
        },
    )
}

/// Indexes `Integer.MAX_VALUE` docs with indexed field(s).
fn test_2b_docs() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BDocs"));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    let mut doc = Document::new();
    let field = Field::new(
        "f1",
        "a".to_string(),
        StringField::type_not_stored().clone(),
    )?;
    doc.add(Box::new(field));

    for i in 0..MAX_DOCS {
        w.add_document(&doc)?;
        if i % (10 * 1000 * 1000) == 0 {
            println_flush(&format!("indexed: {i}"));
        }
    }

    w.force_merge(1)?;
    w.close()?;

    println_flush("verifying...");

    let r = open_reader(&dir)?;

    let term = BytesRef::new(vec![b'a']);

    let mut skips: i64 = 0;

    let rnd = random();

    let start = std::time::Instant::now();

    for context in Arc::clone(&r).leaves()? {
        let reader = context.leaf_reader();
        let lim = reader.max_doc();

        let terms = reader.terms("f1")?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "terms",
            )
            .throw()
        });
        for _ in 0..10000 {
            let mut te = terms.iterator()?;
            Assert::assert_true(te.seek_exact(&term)?);
            let mut docs = te.postings(None, POSTINGS_ENUM_FREQS)?;

            // skip randomly through the term
            let mut target: i32 = -1;
            loop {
                let mut max_skip_size = lim - target + 1;
                // do a smaller skip half of the time
                if rnd.next_boolean() {
                    max_skip_size = max_skip_size.min(256);
                }
                let mut new_target = target + rnd.next_int_bound(max_skip_size) + 1;
                if new_target >= lim {
                    if target + 1 >= lim {
                        break; // we already skipped to end, so break.
                    }
                    new_target = lim - 1; // skip to end
                }
                target = new_target;

                let res = docs.advance(target)?;
                if res == NO_MORE_DOCS {
                    break;
                }

                Assert::assert_true(res >= target);

                skips += 1;
                target = res;
            }
        }
    }

    r.close()?;
    dir.close()?;

    let elapsed = start.elapsed();

    println!("Skip count={skips} seconds={}", elapsed.as_secs());
    // `assert skips > 0` (Java assertions are enabled under test).
    if skips <= 0 {
        randomizedtesting::jdk::exception::Throwable::new("java.lang.AssertionError", "").throw()
    }
    Ok(())
}
