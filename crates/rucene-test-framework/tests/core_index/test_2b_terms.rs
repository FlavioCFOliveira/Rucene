//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BTerms.java`:
//! crazy test that indexes > 2.1B unique terms.
//!
//! * `private static final BytesRef bytes`: the term buffer the stream fills
//!   and `MyTermAttributeImpl.getBytesRef()` returns, a process-wide
//!   `Mutex<BytesRef>` (Rust's `get_bytes_ref` returns the bytes by value).
//! * `CharTermAttribute.class.isAssignableFrom(attClass)`: the requested
//!   interface is `CharTermAttribute` (Rust attribute interfaces do not
//!   extend one another).
//! * `if (true) { … }` and `if (savedTerms == null)`: kept as in Java.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::any::{Any, TypeId};
use std::sync::{Arc, Mutex, OnceLock};

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{CharTermAttribute, TermToBytesRefAttribute};
use rucene::analysis::{default_attribute_factory, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::{SeekStatus, Term};
use rucene::index::{IndexOptions, IndexReader};
use rucene::search::TermQuery;
use rucene::util::attribute::{
    Attribute, AttributeFactory, AttributeImpl, AttributeReflector, AttributeSource,
};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, SuppressSysoutChecks,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class_annotated, random, test};
use crate::support::{
    never_throttle, npe, open_reader, petabyte_max_merge, set_check_index_on_close, two_b_config,
};

/// `Test2BTerms`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BTerms",
        vec![test("test2BTerms", test_2b_terms)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(Monster {
                value: "very slow, use 5g minimum heap".to_string(),
            })
            .annotate(TimeoutSuite {
                millis: 80 * 3_600_000, // effectively no limit
            })
            .annotate(SuppressSysoutChecks {
                bug_url: "Stuff gets printed".to_string(),
            })
        },
    )
}

const TOKEN_LEN: usize = 5;

/// `private static final BytesRef bytes = new BytesRef(TOKEN_LEN)`.
fn bytes() -> &'static Mutex<BytesRef> {
    static BYTES: OnceLock<Mutex<BytesRef>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let mut b = BytesRef::new(vec![0u8; TOKEN_LEN]);
        b.length = 0;
        Mutex::new(b)
    })
}

fn lock_bytes() -> std::sync::MutexGuard<'static, BytesRef> {
    bytes().lock().unwrap_or_else(|e| e.into_inner())
}

/// `private static final class MyTokenStream extends TokenStream`.
struct MyTokenStream {
    source: AttributeSource,
    tokens_per_doc: i32,
    token_count: i32,
    /// `public final List<BytesRef> savedTerms`.
    saved_terms: Vec<BytesRef>,
    next_save: i32,
    term_counter: i64,
    random: Arc<dyn Random>,
}

impl std::fmt::Debug for MyTokenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MyTokenStream")
            .field("tokens_per_doc", &self.tokens_per_doc)
            .field("token_count", &self.token_count)
            .finish_non_exhaustive()
    }
}

impl MyTokenStream {
    fn new(random: Arc<dyn Random>, tokens_per_doc: i32) -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(Arc::new(MyAttributeFactory {
            delegate: default_attribute_factory(),
        }));
        source.add_attribute_by_id(TypeId::of::<dyn TermToBytesRefAttribute>())?;
        lock_bytes().length = TOKEN_LEN;
        let next_save = TestUtil::next_int(&*random, 500000, 1000000);
        Ok(MyTokenStream {
            source,
            tokens_per_doc,
            token_count: 0,
            saved_terms: Vec::new(),
            next_save,
            term_counter: 0,
            random,
        })
    }
}

impl TokenStream for MyTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.source.clear_attributes();
        if self.token_count >= self.tokens_per_doc {
            return Ok(false);
        }
        let mut shift = 32;
        {
            let mut bytes = lock_bytes();
            for i in 0..5 {
                bytes.bytes[i] = ((self.term_counter >> shift) & 0xFF) as u8;
                shift -= 8;
            }
        }
        self.term_counter += 1;
        self.token_count += 1;
        self.next_save -= 1;
        if self.next_save == 0 {
            let copy = BytesRef::deep_copy_of(&lock_bytes());
            println!("TEST: save term={copy}");
            self.saved_terms.push(copy);
            self.next_save = TestUtil::next_int(&*self.random, 500000, 1000000);
        }
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.token_count = 0;
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// `private static final class MyTermAttributeImpl extends AttributeImpl
/// implements TermToBytesRefAttribute`.
#[derive(Debug, Default)]
struct MyTermAttributeImpl;

impl Attribute for MyTermAttributeImpl {}

impl TermToBytesRefAttribute for MyTermAttributeImpl {
    fn get_bytes_ref(&self) -> BytesRef {
        lock_bytes().clone()
    }
}

impl AttributeImpl for MyTermAttributeImpl {
    fn as_term_to_bytes_ref_attribute(&self) -> Option<&dyn TermToBytesRefAttribute> {
        Some(self)
    }

    fn as_term_to_bytes_ref_attribute_mut(&mut self) -> Option<&mut dyn TermToBytesRefAttribute> {
        Some(self)
    }

    fn clear(&mut self) {}

    fn copy_to(&self, _target: &mut dyn AttributeImpl) {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn TermToBytesRefAttribute>(),
            "TermToBytesRefAttribute",
            "bytes",
            &self.get_bytes_ref(),
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Throwable::new("java.lang.UnsupportedOperationException", "").throw()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| vec![TypeId::of::<dyn TermToBytesRefAttribute>()])
    }
}

/// `private static final class MyAttributeFactory extends AttributeFactory`.
#[derive(Debug)]
struct MyAttributeFactory {
    delegate: Arc<dyn AttributeFactory>,
}

impl AttributeFactory for MyAttributeFactory {
    fn create_attribute_instance(
        &self,
        att_class: TypeId,
    ) -> std::result::Result<Box<dyn AttributeImpl>, LuceneError> {
        if att_class == TypeId::of::<dyn TermToBytesRefAttribute>() {
            return Ok(Box::new(MyTermAttributeImpl));
        }
        if att_class == TypeId::of::<dyn CharTermAttribute>() {
            return Err(LuceneError::illegal_argument("no".to_string()));
        }
        self.delegate.create_attribute_instance(att_class)
    }
}

fn test_2b_terms() -> Result<()> {
    println!("Starting Test2B");
    let term_count: i64 = (i32::MAX as i64) + 100000000;

    let terms_per_doc = TestUtil::next_int(&*random(), 100000, 1000000);

    let mut saved_terms: Option<Vec<BytesRef>> = None;

    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BTerms"));
    // MockDirectoryWrapper dir = newFSDirectory(new File("/p/lucene/indices/2bindex"));
    never_throttle(&dir);
    set_check_index_on_close(&dir, false); // don't double-checkindex

    #[allow(clippy::overly_complex_bool_expr)]
    if true {
        let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

        petabyte_max_merge(&w);

        let mut doc = Document::new();
        let ts = Arc::new(AtomicRefCell::new(MyTokenStream::new(
            random(),
            terms_per_doc,
        )?));

        let custom_type = FieldType::new_from(TextField::type_not_stored());
        custom_type.set_index_options(IndexOptions::DOCS)?;
        custom_type.set_omit_norms(true)?;
        let stream: Arc<AtomicRefCell<dyn TokenStream>> = ts.clone();
        let field = Field::new_with_token_stream("field", stream, custom_type)?;
        doc.add(Box::new(field));
        // w.setInfoStream(System.out);
        let num_docs = (term_count / terms_per_doc as i64) as i32;

        println!("TERMS_PER_DOC={terms_per_doc}");
        println!("numDocs={num_docs}");

        for i in 0..num_docs {
            let t0 = std::time::Instant::now();
            w.add_document(&doc)?;
            println!("{i} of {num_docs} {} ms", t0.elapsed().as_millis());
        }
        saved_terms = Some(ts.borrow().saved_terms.clone());

        println!("TEST: full merge");
        w.force_merge(1)?;
        println!("TEST: close writer");
        w.close()?;
    }

    println!("TEST: open reader");
    let dr = open_reader(&dir)?;
    let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
    let mut saved_terms = match saved_terms {
        Some(terms) => terms,
        None => find_terms(&r)?,
    };
    let num_saved_terms = saved_terms.len();
    let mut big_ord_terms: Vec<BytesRef> =
        saved_terms[num_saved_terms - 10..num_saved_terms].to_vec();
    println!("TEST: test big ord terms...");
    test_saved_terms(&dr, &r, &mut big_ord_terms)?;
    println!("TEST: test all saved terms...");
    test_saved_terms(&dr, &r, &mut saved_terms)?;
    r.close()?;

    println!("TEST: now CheckIndex...");
    let status = TestUtil::check_index(dir.as_directory())?;
    let tc = status.segment_infos[0]
        .term_index_status
        .as_ref()
        .unwrap_or_else(|| npe("termIndexStatus"))
        .term_count;
    Assert::assert_true_msg(
        &format!("count {tc} is not > {}", i32::MAX),
        tc > i32::MAX as i64,
    );

    dir.close()?;
    println!("TEST: done!");
    Ok(())
}

fn find_terms(r: &Arc<dyn IndexReader>) -> Result<Vec<BytesRef>> {
    println!("TEST: findTerms");
    let mut terms_enum = MultiTerms::get_terms(r, "field")?
        .unwrap_or_else(|| npe("getTerms"))
        .iterator()?;
    let mut saved_terms = Vec::new();
    let mut next_save = TestUtil::next_int(&*random(), 500000, 1000000);
    while let Some(term) = terms_enum.next()? {
        next_save -= 1;
        if next_save == 0 {
            saved_terms.push(BytesRef::deep_copy_of(&term));
            println!("TEST: add {term}");
            next_save = TestUtil::next_int(&*random(), 500000, 1000000);
        }
    }
    Ok(saved_terms)
}

/// `testSavedTerms(r, terms)`; `dr` is `r` as the `DirectoryReader` it is.
fn test_saved_terms(
    dr: &Arc<dyn rucene::index::directory_reader::DirectoryReader>,
    r: &Arc<dyn IndexReader>,
    terms: &mut [BytesRef],
) -> Result<()> {
    println!("TEST: run {} terms on reader={r}", terms.len());
    let s = LuceneTestCase::new_searcher(Arc::clone(dr));
    shuffle(terms, &*random());
    let mut terms_enum = MultiTerms::get_terms(r, "field")?
        .unwrap_or_else(|| npe("getTerms"))
        .iterator()?;
    let mut failed = false;
    for _ in 0..10 * terms.len() {
        let term = terms[random().next_int_bound(terms.len() as i32) as usize].clone();
        println!("TEST: search {term}");
        let t0 = std::time::Instant::now();
        let count = s.count(Arc::new(TermQuery::new(Term::new("field", term.clone()))))? as i64;
        if count <= 0 {
            println!("  FAILED: count={count}");
            failed = true;
        }
        println!("  took {} ms", t0.elapsed().as_millis());

        let result = terms_enum.seek_ceil(&term)?;
        if result != SeekStatus::FOUND {
            if result == SeekStatus::END {
                println!("  FAILED: got END");
            } else {
                println!(
                    "  FAILED: wrong term: got {}",
                    terms_enum
                        .term()?
                        .map_or_else(|| "null".to_string(), |t| t.to_string())
                );
            }
            failed = true;
        }
    }
    Assert::assert_false(failed);
    Ok(())
}
