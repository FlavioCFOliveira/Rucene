//! Port of `lucene/core/src/test/org/apache/lucene/codecs/lucene103/blocktree/TestTrie.java`.
//!
//! Java's `TreeMap<BytesRef, TrieBuilder.Output>` is a `BTreeMap` keyed by
//! the key bytes (`BytesRef.compareTo` is the unsigned lexicographic order of
//! `Vec<u8>`), whose values are `TrieBuilder.Output` records; `assertEquals`
//! of two such maps compares their entries with `Output.equals` (`fp`,
//! `hasTerms`, and `floorData` by content).
//!
//! Rucene's `TrieReader` takes the random-access view of the slice Java
//! passes it (`TrieReader.java:78-83`), and `Node.floorData(reader)` reads
//! the floor bytes out of it instead of returning the seeked input
//! (`src/codecs/lucene103/trie_reader.rs`, `TrieReader::floor_data`).

use std::collections::BTreeMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene103::blocktree::{Output, TrieBuilder};
use rucene::codecs::lucene103::{ChildSaveStrategy, Node, TrieReader};
use rucene::error::Result;
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene::util::BytesRef;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_array_equals, assert_equals, assert_not_null, assert_null, class, random, test,
};

/// `TestTrie`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.lucene103.blocktree.TestTrie",
        vec![
            test("testStrategyChoose", test_strategy_choose),
            test("testRandomTerms", test_random_terms),
            test("testVeryLongTerms", test_very_long_terms),
            test("testOneByteTerms", test_one_byte_terms),
        ],
    )
}

/// `TrieBuilder.Output` as `Output.equals` compares it.
type OutputValue = (i64, bool, Option<Vec<u8>>);

fn output_value(o: &Output) -> OutputValue {
    (
        o.fp,
        o.has_terms,
        o.floor_data.as_ref().map(|b| b.slice().to_vec()),
    )
}

/// `new BytesRef(bytes)`.
fn bytes_ref(bytes: &[u8]) -> BytesRef {
    BytesRef::new(bytes.to_vec())
}

/// `new TrieBuilder.Output(0L, false, new BytesRef("emptyOutput"))`.
fn empty_output() -> Output {
    Output {
        fp: 0,
        has_terms: false,
        floor_data: Some(bytes_ref(b"emptyOutput")),
    }
}

/// Port of `TestTrie.testStrategyChoose`.
fn test_strategy_choose() -> Result<()> {
    // bits use 32 bytes while reverse_array use 31 bytes, choose reverse_array
    assert_equals(
        &ChildSaveStrategy::ReverseArray,
        &ChildSaveStrategy::choose(0, 255, 226),
    );
    // bits use 32 bytes while array use 31 bytes, choose array
    assert_equals(
        &ChildSaveStrategy::Array,
        &ChildSaveStrategy::choose(0, 255, 32),
    );
    // array and bits both use 32 position bytes, we choose bits.
    assert_equals(
        &ChildSaveStrategy::Bits,
        &ChildSaveStrategy::choose(0, 255, 33),
    );
    // reverse_array and bits both use 32 position bytes, we choose bits.
    assert_equals(
        &ChildSaveStrategy::Bits,
        &ChildSaveStrategy::choose(0, 255, 225),
    );
    Ok(())
}

/// Port of `TestTrie.testRandomTerms`.
fn test_random_terms() -> Result<()> {
    let supplier: &dyn Fn() -> Vec<u8> = &random_bytes;
    test_trie_builder(supplier, LuceneTestCase::at_least(1000))?;
    test_trie_lookup(supplier, 12)
}

/// Port of `TestTrie.testVeryLongTerms`.
fn test_very_long_terms() -> Result<()> {
    let supplier = || {
        let mut bytes = vec![0u8; 65535];
        for i in 1..bytes.len() {
            bytes[i] = random().next_int_bound((i / 256 + 1) as i32) as u8;
        }
        bytes
    };
    test_trie_lookup(&supplier, 5)
}

/// Port of `TestTrie.testOneByteTerms`.
fn test_one_byte_terms() -> Result<()> {
    // heavily test single byte terms to generate various label distribution.
    let supplier = || vec![random().next_int() as u8];
    let round = LuceneTestCase::at_least(50);
    for _ in 0..round {
        test_trie_lookup(&supplier, 10)?;
    }
    Ok(())
}

/// Port of `TestTrie.testTrieBuilder`.
fn test_trie_builder(random_bytes_supplier: &dyn Fn() -> Vec<u8>, count: i32) -> Result<()> {
    let mut expected: BTreeMap<Vec<u8>, Output> = BTreeMap::new();
    expected.insert(Vec::new(), empty_output());
    for _ in 0..count {
        let key = random_bytes_supplier();
        let fp = random().next_long_bound(1i64 << 62);
        let has_terms = random().next_boolean();
        let value = Output {
            fp,
            has_terms,
            floor_data: Some(BytesRef::new(random_bytes_supplier())),
        };
        expected.insert(key, value);
    }

    let mut trie_builder = TrieBuilder::bytes_ref_to_trie(&bytes_ref(b""), empty_output());
    for (key, value) in &expected {
        if key.is_empty() {
            continue;
        }
        let add = TrieBuilder::bytes_ref_to_trie(&bytes_ref(key), value.clone());
        trie_builder.append(&add)?;
    }
    let mut actual: BTreeMap<Vec<u8>, OutputValue> = BTreeMap::new();
    trie_builder.visit(|k, v| {
        actual.insert(k.slice().to_vec(), output_value(v));
    })?;
    let expected: BTreeMap<Vec<u8>, OutputValue> = expected
        .iter()
        .map(|(k, v)| (k.clone(), output_value(v)))
        .collect();
    assert_equals(&expected, &actual);
    Ok(())
}

/// Port of `TestTrie.testTrieLookup`.
fn test_trie_lookup(random_bytes_supplier: &dyn Fn() -> Vec<u8>, round: i32) -> Result<()> {
    for iter in 1..=round {
        let mut expected: BTreeMap<Vec<u8>, Output> = BTreeMap::new();
        expected.insert(Vec::new(), empty_output());
        let n = 1 << iter;
        for _ in 0..n {
            let key = random_bytes_supplier();
            let fp = random().next_long_bound(1i64 << 62);
            let has_terms = random().next_boolean();
            let floor_data = if random().next_boolean() {
                None
            } else {
                Some(BytesRef::new(random_bytes_supplier()))
            };
            let value = Output {
                fp,
                has_terms,
                floor_data,
            };
            expected.insert(key, value);
        }

        let mut trie_builder = TrieBuilder::bytes_ref_to_trie(&bytes_ref(b""), empty_output());
        for (key, value) in &expected {
            if key.is_empty() {
                continue;
            }
            let add = TrieBuilder::bytes_ref_to_trie(&bytes_ref(key), value.clone());
            trie_builder.append(&add)?;
        }

        let mut directory = LuceneTestCase::new_directory();
        {
            let dir = directory.as_directory();
            {
                let mut index = dir.create_output("index", &*DEFAULT_IO_CONTEXT)?;
                let mut meta = dir.create_output("meta", &*DEFAULT_IO_CONTEXT)?;
                trie_builder.save(&mut *meta, &mut *index)?;
                index.close()?;
                meta.close()?;
            }

            let mut index_in = dir.open_input("index", &*DEFAULT_IO_CONTEXT)?;
            let mut meta_in = dir.open_input("meta", &*DEFAULT_IO_CONTEXT)?;
            let start = meta_in.read_v_long()?;
            let root_fp = meta_in.read_v_long()?;
            let end = meta_in.read_v_long()?;
            let outputs = index_in.slice("outputs", start, end - start)?;
            let mut reader =
                TrieReader::new(outputs.random_access_slice(0, outputs.length())?, root_fp)?;

            for (key, value) in &expected {
                assert_result(&mut reader, key, value)?;
            }

            let test_not_found = LuceneTestCase::at_least(100);
            for _ in 0..test_not_found {
                let mut key = random_bytes();
                while expected.contains_key(&key) {
                    key = random_bytes();
                }
                let mut last_k: &[u8] = &[];
                for k in expected.keys() {
                    if k.as_slice() > key.as_slice() {
                        debug_assert!(last_k < key.as_slice());
                        let mismatch1 = mismatch(last_k, &key);
                        let mismatch2 = mismatch(k, &key);
                        assert_not_found_on_level_n(&mut reader, &key, mismatch1.max(mismatch2))?;
                        break;
                    }
                    last_k = k;
                }
            }
            index_in.close()?;
            meta_in.close()?;
        }
        directory.close()?;
    }
    Ok(())
}

/// `Arrays.mismatch(a, 0, a.length, b, 0, b.length)`: the first index where
/// the two differ, the shorter length when one is a proper prefix of the
/// other, or -1 when they are equal.
fn mismatch(a: &[u8], b: &[u8]) -> i32 {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => i as i32,
        None if a.len() == b.len() => -1,
        None => a.len().min(b.len()) as i32,
    }
}

/// Port of `TestTrie.randomBytes`.
fn random_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; (random().next_int_bound(256) + 1) as usize];
    for i in 1..bytes.len() {
        bytes[i] = random().next_int_bound(1 << (i % 9)) as u8;
    }
    bytes
}

/// Port of `TestTrie.assertResult`.
fn assert_result(reader: &mut TrieReader, term: &[u8], expected: &Output) -> Result<()> {
    let mut parent: Node = reader.root;
    for &b in term {
        let found = reader.lookup_child(b as i32, &parent)?;
        assert_not_null(&found);
        parent = found.unwrap_or_default();
    }
    Assert::assert_true(parent.has_output());
    assert_equals(&expected.fp, &parent.output_fp);
    assert_equals(&expected.has_terms, &parent.has_terms);
    match &expected.floor_data {
        None => Assert::assert_false(parent.is_floor()),
        Some(floor_data) => {
            let bytes = reader.floor_data(&parent, floor_data.length)?;
            assert_array_equals(floor_data.slice(), &bytes);
        }
    }
    Ok(())
}

/// Port of `TestTrie.assertNotFoundOnLevelN`.
fn assert_not_found_on_level_n(reader: &mut TrieReader, term: &[u8], n: i32) -> Result<()> {
    let mut parent: Node = reader.root;
    for (i, &b) in term.iter().enumerate() {
        let found = reader.lookup_child(b as i32, &parent)?;
        if i as i32 == n {
            assert_null(&found);
            break;
        }
        assert_not_null(&found);
        parent = found.unwrap_or_default();
    }
    Ok(())
}
