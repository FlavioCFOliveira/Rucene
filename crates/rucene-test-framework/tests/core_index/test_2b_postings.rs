//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BPostings.java`:
//! test indexes ~82M docs with 26 terms each, so you get > Integer.MAX_VALUE
//! terms/docs pairs.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::IndexOptions;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::{Nightly, SuppressCodecs, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class_annotated, random, test};
use crate::support::{never_throttle, petabyte_max_merge};

/// `Test2BPostings`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BPostings",
        vec![test("test", test_).annotate(Nightly)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec![
                    "SimpleText".to_string(),
                    "Direct".to_string(),
                    "Compressing".to_string(),
                ],
            })
            .annotate(TimeoutSuite {
                millis: 4 * 3_600_000,
            })
        },
    )
}

fn test_() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BPostings"));
    never_throttle(&dir);

    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)?;
    iwc.set_ram_buffer_size_mb(256.0)?;
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_use_cfs_merge_factor(false, 10)?,
    )))?;
    iwc.set_open_mode(OpenMode::CREATE)?;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    petabyte_max_merge(&w);

    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_omit_norms(true)?;
    ft.set_index_options(IndexOptions::DOCS)?;
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(MyTokenStream::new()?));
    let field = Field::new_with_token_stream("field", stream, ft)?;
    doc.add(Box::new(field));

    let num_docs = (i32::MAX / 26) + 1;
    for i in 0..num_docs {
        w.add_document(&doc)?;
        if *VERBOSE && i % 100000 == 0 {
            println!("{i} of {num_docs}...");
        }
    }
    w.force_merge(1)?;
    w.close()?;
    dir.close()
}

/// `public static final class MyTokenStream extends TokenStream`.
#[derive(Debug)]
pub struct MyTokenStream {
    source: AttributeSource,
    term_att: Att,
    index: u32,
}

impl MyTokenStream {
    /// `new MyTokenStream()`.
    pub fn new() -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        Ok(MyTokenStream {
            source,
            term_att,
            index: 0,
        })
    }
}

impl TokenStream for MyTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.index <= 'z' as u32 {
            self.source.clear_attributes();
            {
                let mut term = self.term_att.char_term_mut();
                term.set_length(1);
                // `(char) index++`
                term.buffer_mut()[0] = char::from_u32(self.index).unwrap_or('\0');
            }
            self.index += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn reset(&mut self) -> Result<()> {
        self.index = 'a' as u32;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
