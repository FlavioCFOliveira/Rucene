//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BPositions.java`:
//! test indexes ~82M docs with 52 positions each, so you get > Integer.MAX_VALUE
//! positions.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::util::lucene_test_case::{Monster, SuppressCodecs, VERBOSE};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class_annotated, test};
use crate::support::{never_throttle, petabyte_max_merge, two_b_config};

/// `Test2BPositions`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BPositions",
        vec![test("test", test_)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(Monster {
                value: "uses lots of space and takes a few minutes".to_string(),
            })
        },
    )
}

fn test_() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BPositions"));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    petabyte_max_merge(&w);

    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_omit_norms(true)?;
    let stream: Rc<RefCell<dyn TokenStream>> = Rc::new(RefCell::new(MyTokenStream::new()?));
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
    pos_inc_att: Att,
    index: i32,
}

impl MyTokenStream {
    /// `new MyTokenStream()`.
    pub fn new() -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let pos_inc_att = Att::add(&mut source, pos_inc_id())?;
        Ok(MyTokenStream {
            source,
            term_att,
            pos_inc_att,
            index: 0,
        })
    }
}

impl TokenStream for MyTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.index < 52 {
            self.source.clear_attributes();
            {
                let mut term = self.term_att.char_term_mut();
                term.set_length(1);
                term.buffer_mut()[0] = 'a';
            }
            self.pos_inc_att
                .pos_inc_mut()
                .set_position_increment(1 + self.index);
            self.index += 1;
            return Ok(true);
        }
        Ok(false)
    }

    fn reset(&mut self) -> Result<()> {
        self.index = 0;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
