//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSameTokenSamePosition.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestSameTokenSamePosition`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSameTokenSamePosition",
        vec![test("test", test_), test("testMoreDocs", test_more_docs)],
    )
}

/// `new TextField("eng", new BugReproTokenStream())`.
fn eng_field() -> Result<TextField> {
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(BugReproTokenStream::new()?));
    TextField::new_with_token_stream("eng", stream)
}

/// Attempt to reproduce an assertion error that happens only with the trunk
/// version around April 2011.
///
/// Port of `TestSameTokenSamePosition.test`.
fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let riw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(eng_field()?));
    riw.add_document(&doc)?;
    riw.close()?;
    dir.close()
}

/// Same as the above, but with more docs.
///
/// Port of `TestSameTokenSamePosition.testMoreDocs`.
fn test_more_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let riw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _i in 0..100 {
        let mut doc = Document::new();
        doc.add(Box::new(eng_field()?));
        riw.add_document(&doc)?;
    }
    riw.close()?;
    dir.close()
}

/// `TOKEN_COUNT`.
const TOKEN_COUNT: usize = 4;

/// `final class BugReproTokenStream extends TokenStream`.
#[derive(Debug)]
struct BugReproTokenStream {
    source: AttributeSource,
    term_att: Att,
    offset_att: Att,
    pos_inc_att: Att,
    next_token_index: usize,
}

impl BugReproTokenStream {
    const TERMS: [&'static str; 4] = ["six", "six", "drunken", "drunken"];
    const STARTS: [i32; 4] = [0, 0, 4, 4];
    const ENDS: [i32; 4] = [3, 3, 11, 11];
    const INCS: [i32; 4] = [1, 0, 1, 0];

    fn new() -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        let pos_inc_att = Att::add(&mut source, pos_inc_id())?;
        Ok(BugReproTokenStream {
            source,
            term_att,
            offset_att,
            pos_inc_att,
            next_token_index: 0,
        })
    }
}

impl TokenStream for BugReproTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.next_token_index < TOKEN_COUNT {
            self.term_att.set_term(Self::TERMS[self.next_token_index]);
            self.offset_att.offset_mut().set_offset(
                Self::STARTS[self.next_token_index],
                Self::ENDS[self.next_token_index],
            );
            self.pos_inc_att
                .pos_inc_mut()
                .set_position_increment(Self::INCS[self.next_token_index]);
            self.next_token_index += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self) -> Result<()> {
        self.next_token_index = 0;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
