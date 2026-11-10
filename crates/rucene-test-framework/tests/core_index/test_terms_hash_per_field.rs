//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermsHashPerField.java`.
//!
//! The anonymous `TermsHashPerField` subclass is [`TestHash`]: the port's
//! `TermsHashPerField` leaves the `newTerm`/`addTerm` dispatch of
//! `add(BytesRef, int)` to its caller (`TermsHashPerField::intern`), so the
//! subclass's overrides are that dispatch, over the table's
//! `FreqProxPostingsArray` columns (`TermsHashPerField::posting_mut`). The
//! table owns no pools in this port; `TestHash` holds the three pools Java's
//! constructor receives.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::{RandomPicks, RandomStrings};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::freq_prox_terms_writer::{ByteSliceReader, FreqProxPosting, TermsHashPerField};
use rucene::index::IndexOptions;
use rucene::util::byte_block_pool::{ByteBlockPool, DirectAllocator};
use rucene::util::int_block_pool::IntBlockPool;
use rucene::util::BytesRef;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::java_hash_set::JavaHashSet;
use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};
use crate::support::npe;

/// `TestTermsHashPerField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermsHashPerField",
        vec![
            test("testAddAndUpdateTerm", test_add_and_update_term),
            test("testAddAndUpdateRandom", test_add_and_update_random),
            test("testWriteBytes", test_write_bytes),
        ],
    )
}

/// The anonymous `TermsHashPerField` of `createNewHash`, with the pools its
/// constructor receives.
struct TestHash {
    hash: TermsHashPerField,
    int_block_pool: IntBlockPool,
    byte_block_pool: ByteBlockPool,
    term_block_pool: ByteBlockPool,
    new_called: Arc<AtomicI32>,
    add_called: Arc<AtomicI32>,
}

impl TestHash {
    /// `void newTerm(int termID, int docID)`.
    fn new_term(&mut self, term_id: i32, doc_id: i32) {
        self.new_called.fetch_add(1, Ordering::SeqCst);
        let postings = self.hash.posting_mut(term_id);
        postings.last_doc_id = doc_id;
        postings.last_doc_code = doc_id << 1;
        postings.term_freq = 1;
    }

    /// `void addTerm(int termID, int docID)`.
    fn add_term(&mut self, term_id: i32, doc_id: i32) -> Result<()> {
        self.add_called.fetch_add(1, Ordering::SeqCst);
        let postings = *self.hash.posting(term_id);
        if doc_id != postings.last_doc_id {
            if 1 == postings.term_freq {
                self.write_v_int(term_id, 0, postings.last_doc_code | 1)?;
            } else {
                self.write_v_int(term_id, 0, postings.last_doc_code)?;
                self.write_v_int(term_id, 0, postings.term_freq)?;
            }
            let postings = self.hash.posting_mut(term_id);
            postings.term_freq = 1;
            postings.last_doc_code = (doc_id - postings.last_doc_id) << 1;
            postings.last_doc_id = doc_id;
        } else {
            let postings = self.hash.posting_mut(term_id);
            postings.term_freq = postings.term_freq.checked_add(1).unwrap_or_else(|| {
                randomizedtesting::jdk::exception::Throwable::new(
                    "java.lang.ArithmeticException",
                    "integer overflow",
                )
                .throw()
            });
        }
        Ok(())
    }

    /// `writeVInt(stream, i)`.
    fn write_v_int(&mut self, term_id: i32, stream: usize, value: i32) -> Result<()> {
        self.hash.write_v_int(
            &mut self.byte_block_pool,
            &mut self.int_block_pool,
            term_id,
            stream,
            value,
        )
    }

    /// `boolean start(IndexableField field, boolean first)`: no next per-field.
    fn start(&mut self, _field: Option<()>, _first: bool) -> bool {
        true
    }

    /// `void add(BytesRef termBytes, int docID)`.
    fn add(&mut self, term_bytes: &BytesRef, doc_id: i32) -> Result<()> {
        let slot = self.hash.intern_into(
            &mut self.term_block_pool,
            &mut self.byte_block_pool,
            &mut self.int_block_pool,
            term_bytes.slice(),
            doc_id,
        )?;
        if slot.is_new {
            self.new_term(slot.term_id, doc_id);
            Ok(())
        } else {
            self.add_term(slot.term_id, doc_id)
        }
    }

    /// `void finish()`: no next per-field.
    fn finish(&mut self) {}

    /// `void writeBytes(int stream, byte[] b, int offset, int len)`, on the
    /// term the last `add` positioned the table on (term 0 in the test).
    fn write_bytes(
        &mut self,
        term_id: i32,
        stream: usize,
        b: &[u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        self.hash.write_bytes(
            &mut self.byte_block_pool,
            &mut self.int_block_pool,
            term_id,
            stream,
            &b[offset..offset + len],
        )
    }

    /// `void initReader(ByteSliceReader reader, int termID, int stream)`.
    fn init_reader(&self, reader: &mut ByteSliceReader, term_id: i32, stream: usize) {
        self.hash
            .init_reader(&self.int_block_pool, reader, term_id, stream);
    }

    /// `(FreqProxPostingsArray) hash.postingsArray`, the slot of `term_id`.
    fn postings_array(&self, term_id: i32) -> FreqProxPosting {
        *self.hash.posting(term_id)
    }
}

/// `private static TermsHashPerField createNewHash(AtomicInteger newCalled, AtomicInteger
/// addCalled)`.
fn create_new_hash(new_called: &Arc<AtomicI32>, add_called: &Arc<AtomicI32>) -> TestHash {
    let int_block_pool = IntBlockPool::new();
    let byte_block_pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    let term_block_pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));

    let hash = TermsHashPerField::new(
        1,
        "testfield".to_string(),
        IndexOptions::DOCS_AND_FREQS,
        Arc::new(AtomicI64::new(0)), // Counter.newCounter()
    );
    TestHash {
        hash,
        int_block_pool,
        byte_block_pool,
        term_block_pool,
        new_called: Arc::clone(new_called),
        add_called: Arc::clone(add_called),
    }
}

/// `boolean assertDocAndFreq(ByteSliceReader reader, FreqProxPostingsArray postingsArray, int
/// prevDoc, int termId, int doc, int frequency)`.
fn assert_doc_and_freq(
    reader: &mut ByteSliceReader,
    hash: &TestHash,
    prev_doc: i32,
    term_id: i32,
    doc: i32,
    frequency: i32,
) -> Result<bool> {
    let postings_array = hash.postings_array(term_id);
    let mut doc_id = prev_doc;
    let freq;
    let eof = reader.eof();
    if eof {
        doc_id = postings_array.last_doc_id;
        freq = postings_array.term_freq;
    } else {
        let code = reader.read_v_int(&hash.byte_block_pool)?;
        doc_id += ((code as u32) >> 1) as i32;
        if (code & 1) != 0 {
            freq = 1;
        } else {
            freq = reader.read_v_int(&hash.byte_block_pool)?;
        }
    }
    assert_equals_msg(Some(&format!("docID mismatch eof: {eof}")), &doc, &doc_id);
    assert_equals_msg(
        Some(&format!("freq mismatch eof: {eof}")),
        &frequency,
        &freq,
    );
    Ok(eof)
}

/// `newBytesRef(s)`.
fn b(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// Port of `TestTermsHashPerField.testAddAndUpdateTerm`.
fn test_add_and_update_term() -> Result<()> {
    let new_called = Arc::new(AtomicI32::new(0));
    let add_called = Arc::new(AtomicI32::new(0));
    let mut hash = create_new_hash(&new_called, &add_called);
    hash.start(None, true);

    hash.add(&b("start"), 0)?; // tid = 0;
    hash.add(&b("foo"), 0)?; // tid = 1;
    hash.add(&b("bar"), 0)?; // tid = 2;
    hash.finish();
    hash.add(&b("bar"), 1)?;
    hash.add(&b("foobar"), 1)?; // tid = 3;
    hash.add(&b("bar"), 1)?;
    hash.add(&b("bar"), 1)?;
    hash.add(&b("foobar"), 1)?;
    hash.add(&b("verylongfoobarbaz"), 1)?; // tid = 4;
    hash.finish();
    hash.add(&b("verylongfoobarbaz"), 2)?;
    hash.add(&b("boom"), 2)?; // tid = 5;
    hash.finish();
    hash.add(&b("verylongfoobarbaz"), 3)?;
    hash.add(&b("end"), 3)?; // tid = 6;
    hash.finish();

    assert_equals(&7, &new_called.load(Ordering::SeqCst));
    assert_equals(&6, &add_called.load(Ordering::SeqCst));
    let mut reader = ByteSliceReader::new();
    hash.init_reader(&mut reader, 0, 0);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 0, 0, 0, 1)?);
    hash.init_reader(&mut reader, 1, 0);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 0, 1, 0, 1)?);
    hash.init_reader(&mut reader, 2, 0);
    Assert::assert_false(assert_doc_and_freq(&mut reader, &hash, 0, 2, 0, 1)?);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 2, 2, 1, 3)?);
    hash.init_reader(&mut reader, 3, 0);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 0, 3, 1, 2)?);
    hash.init_reader(&mut reader, 4, 0);
    Assert::assert_false(assert_doc_and_freq(&mut reader, &hash, 0, 4, 1, 1)?);
    Assert::assert_false(assert_doc_and_freq(&mut reader, &hash, 1, 4, 2, 1)?);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 2, 4, 3, 1)?);
    hash.init_reader(&mut reader, 5, 0);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 0, 5, 2, 1)?);
    hash.init_reader(&mut reader, 6, 0);
    Assert::assert_true(assert_doc_and_freq(&mut reader, &hash, 0, 6, 3, 1)?);
    Ok(())
}

/// The local `class Posting` of `testAddAndUpdateRandom`.
#[derive(Debug, Clone)]
struct Posting {
    term_id: i32,
    doc_and_freq: BTreeMap<i32, i32>,
}

impl Posting {
    fn new() -> Self {
        Self {
            term_id: -1,
            doc_and_freq: BTreeMap::new(),
        }
    }
}

/// Port of `TestTermsHashPerField.testAddAndUpdateRandom`.
fn test_add_and_update_random() -> Result<()> {
    let new_called = Arc::new(AtomicI32::new(0));
    let add_called = Arc::new(AtomicI32::new(0));
    let mut hash = create_new_hash(&new_called, &add_called);
    hash.start(None, true);
    // `Map<BytesRef, Posting> postingMap = new HashMap<>()`: the keys in
    // Java's iteration order, and the values by key.
    let mut posting_keys: JavaHashSet<BytesRef> = JavaHashSet::new();
    let mut posting_map: HashMap<BytesRef, Posting> = HashMap::new();
    let num_strings = 1 + random().next_int_bound(200);
    for _ in 0..num_strings {
        let random_string =
            RandomStrings::random_realistic_unicode_of_codepoint_length_between(&*random(), 1, 10);
        let key = b(&random_string);
        if posting_keys.add(key.clone()) {
            posting_map.insert(key, Posting::new());
        }
    }
    let mut bytes_refs: Vec<BytesRef> = posting_keys.to_vec();
    bytes_refs.sort();
    let num_docs = 1 + random().next_int_bound(200);
    let mut term_ord = 0;
    for doc in 0..num_docs {
        let num_terms = 1 + random().next_int_bound(200);
        for _ in 0..num_terms {
            let r#ref = RandomPicks::random_from_list(&*random(), &bytes_refs).clone();
            let posting = posting_map
                .get_mut(&r#ref)
                .unwrap_or_else(|| npe("posting"));
            if posting.term_id == -1 {
                posting.term_id = term_ord;
                term_ord += 1;
            }
            *posting.doc_and_freq.entry(doc).or_insert(0) += 1;
            hash.add(&r#ref, doc)?;
        }
        hash.finish();
    }
    let mut values: Vec<Posting> = posting_keys
        .iter()
        .map(|k| posting_map[k].clone())
        .filter(|x| x.term_id != -1)
        .collect();
    shuffle(&mut values, &*random()); // term order doesn't matter
    let mut reader = ByteSliceReader::new();
    for p in &values {
        hash.init_reader(&mut reader, p.term_id, 0);
        let mut eof = false;
        let mut pref_doc = 0;
        for (&key, &value) in &p.doc_and_freq {
            Assert::assert_false_msg("the reader must not be EOF here", eof);
            eof = assert_doc_and_freq(&mut reader, &hash, pref_doc, p.term_id, key, value)?;
            pref_doc = key;
        }
        Assert::assert_true_msg("the last posting must be EOF on the reader", eof);
    }
    Ok(())
}

/// Port of `TestTermsHashPerField.testWriteBytes`.
fn test_write_bytes() -> Result<()> {
    for _ in 0..100 {
        let new_called = Arc::new(AtomicI32::new(0));
        let add_called = Arc::new(AtomicI32::new(0));
        let mut hash = create_new_hash(&new_called, &add_called);
        hash.start(None, true);
        hash.add(&b("start"), 0)?; // tid = 0;
        let size = TestUtil::next_int(&*random(), 50000, 100000) as usize;
        let mut random_data = vec![0u8; size];
        random().next_bytes(&mut random_data);
        let mut offset = 0usize;
        while offset < random_data.len() {
            let write_length =
                (random_data.len() - offset).min(TestUtil::next_int(&*random(), 1, 200) as usize);
            hash.write_bytes(0, 0, &random_data, offset, write_length)?;
            offset += write_length;
        }
        let mut reader = ByteSliceReader::new();
        reader.init(
            0,
            hash.byte_block_pool.byte_offset() + hash.byte_block_pool.byte_upto() as i32,
        );
        for &expected in &random_data {
            assert_equals(&expected, &reader.read_byte(&hash.byte_block_pool));
        }
    }
    Ok(())
}
