//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BPostingsBytes.java`:
//! test indexes 2B docs with 65k freqs each, so you get >
//! Integer.MAX_VALUE postings data for the term.
//!
//! * `Pattern.compile("maxDocsPerChunk=(\\d+), blockSize=(\\d+)")`: no regex
//!   engine; [`find_chunk_params`] scans for the same pattern.
//! * `tokenStream.n = …` on the stream the field holds: the stream is shared
//!   through the `Arc` the field keeps.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{default_token_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexOptions;
use rucene::util::attribute::AttributeSource;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::codecs::compressing::CompressingCodec;
use rucene_test_framework::util::lucene_test_case::{Monster, SuppressCodecs};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, test};
use crate::support::{never_throttle, open_reader, petabyte_max_merge, two_b_config};

/// `Test2BPostingsBytes`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BPostingsBytes",
        vec![test("test", test_)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(Monster {
                value: "takes ~20GB-30GB of space and 10 minutes".to_string(),
            })
        },
    )
}

/// `regex.matcher(s)`, `matcher.find()`, `group(1)`, `group(2)`.
fn find_chunk_params(s: &str) -> Option<(String, String)> {
    let digits = |t: &str| -> usize { t.chars().take_while(|c| c.is_ascii_digit()).count() };
    let mut from = 0;
    while let Some(pos) = s[from..].find("maxDocsPerChunk=") {
        let start = from + pos + "maxDocsPerChunk=".len();
        let n1 = digits(&s[start..]);
        if n1 > 0 {
            let rest = &s[start + n1..];
            if let Some(tail) = rest.strip_prefix(", blockSize=") {
                let n2 = digits(tail);
                if n2 > 0 {
                    return Some((s[start..start + n1].to_string(), tail[..n2].to_string()));
                }
            }
        }
        from = from + pos + 1;
    }
    None
}

fn test_() -> Result<()> {
    let default_config = IndexWriterConfig::with_analyzer(None);
    let default_codec = default_config.codec();
    if IndexWriterConfig::with_analyzer(None)
        .codec()
        .as_any()
        .and_then(|a| a.downcast_ref::<CompressingCodec>())
        .is_some()
    {
        let codec_string = default_codec.to_java_string();
        let matcher = find_chunk_params(&codec_string);
        Assert::assert_true_msg(
            &format!("Unexpected CompressingCodec toString() output: {codec_string}"),
            matcher.is_some(),
        );
        let (g1, g2) = matcher.unwrap_or_default();
        let max_docs_per_chunk: i32 = parse_int(&g1);
        let block_size: i32 = parse_int(&g2);
        let product = max_docs_per_chunk.wrapping_mul(block_size);
        LuceneTestCase::assume_true(
            &format!(
                "{} maxDocsPerChunk ({max_docs_per_chunk}) * blockSize ({block_size}) < 16 - this can trigger OOM with -Dtests.heapsize=30g",
                default_codec.name()
            ),
            product >= 16,
        );
    }

    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "2BPostingsBytes1",
    ));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    petabyte_max_merge(&w);

    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    ft.set_omit_norms(true)?;
    let token_stream = Arc::new(AtomicRefCell::new(MyTokenStream::new()?));
    let stream: Arc<AtomicRefCell<dyn TokenStream>> = token_stream.clone();
    let field = Field::new_with_token_stream("field", stream, ft)?;
    doc.add(Box::new(field));

    let num_docs = 1000;
    for i in 0..num_docs {
        if i % 2 == 1 {
            // trick blockPF's little optimization
            token_stream.borrow_mut().n = 65536;
        } else {
            token_stream.borrow_mut().n = 65537;
        }
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;

    let one_thousand = open_reader(&dir)?;
    let sub_readers: Vec<Arc<dyn DirectoryReader>> = vec![Arc::clone(&one_thousand); 1000];
    let dir2 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "2BPostingsBytes2",
    ));
    never_throttle(&dir2);
    let w2 = IndexWriter::new(dir2.as_directory(), IndexWriterConfig::with_analyzer(None))?;
    TestUtil::add_indexes_slowly(&w2, &sub_readers)?;
    w2.force_merge(1)?;
    w2.close()?;
    one_thousand.close()?;

    let one_million = open_reader(&dir2)?;
    let sub_readers: Vec<Arc<dyn DirectoryReader>> = vec![Arc::clone(&one_million); 2000];
    let dir3 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "2BPostingsBytes3",
    ));
    never_throttle(&dir3);
    let w3 = IndexWriter::new(dir3.as_directory(), IndexWriterConfig::with_analyzer(None))?;
    TestUtil::add_indexes_slowly(&w3, &sub_readers)?;
    w3.force_merge(1)?;
    w3.close()?;
    one_million.close()?;

    dir.close()?;
    dir2.close()?;
    dir3.close()
}

/// `Integer.parseInt(s)`.
fn parse_int(s: &str) -> i32 {
    s.parse().unwrap_or_else(|_| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// `public static final class MyTokenStream extends TokenStream`.
#[derive(Debug)]
pub struct MyTokenStream {
    source: AttributeSource,
    term_att: Att,
    index: i32,
    /// `int n`.
    pub n: i32,
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
            n: 0,
        })
    }
}

impl TokenStream for MyTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.index < self.n {
            self.source.clear_attributes();
            {
                let mut term = self.term_att.char_term_mut();
                term.buffer_mut()[0] = 'a';
                term.set_length(1);
            }
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
