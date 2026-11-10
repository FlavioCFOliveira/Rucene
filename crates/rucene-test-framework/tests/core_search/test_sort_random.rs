//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSortRandom.java`.
//!
//! random sorting tests

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    Document, NumericDocValuesField, NumericValue, SortedDocValuesField, StoredField,
};
use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::index::{IndexReaderContext as _, LeafReaderContext};
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::field_comparator::SortValue;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{Query, ScoreMode};
use rucene::util::{BitSet, BitSetIterator, BytesRef, FixedBitSet};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test, verbose};

/// `TestSortRandom`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSortRandom",
        vec![test("testRandomStringSort", test_random_string_sort)],
    )
}

/// Port of `TestSortRandom.testRandomStringSort`.
fn test_random_string_sort() -> Result<()> {
    random_string_sort(SortFieldType::String)
}

/// `private void testRandomStringSort(SortField.Type type)`.
fn random_string_sort(t: SortFieldType) -> Result<()> {
    let rnd = JdkRandom::new(random().next_long());

    let num_docs_target = LuceneTestCase::at_least(100);
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&rnd, dir.as_directory())?;
    let allow_dups = rnd.next_boolean();
    let mut seen: HashSet<String> = HashSet::new();
    let max_length = TestUtil::next_int(&rnd, 5, 100);
    if verbose() {
        println!("TEST: NUM_DOCS={num_docs_target} maxLength={max_length} allowDups={allow_dups}");
    }

    let mut num_docs = 0;
    let mut doc_values: Vec<Option<BytesRef>> = Vec::new();
    // TODO: deletions
    while num_docs < num_docs_target {
        let mut doc = Document::new();

        // 10% of the time, the document is missing the value:
        if random().next_int_bound(10) != 7 {
            let s = if rnd.next_boolean() {
                TestUtil::random_simple_string_max(&rnd, max_length)
            } else {
                TestUtil::random_unicode_string_max(&rnd, max_length)
            };

            if !allow_dups {
                if seen.contains(&s) {
                    continue;
                }
                seen.insert(s.clone());
            }

            if verbose() {
                println!("  {num_docs}: s={s}");
            }

            let br = BytesRef::from_char_sequence(&s);
            doc.add(Box::new(SortedDocValuesField::new("stringdv", br.clone())));
            doc_values.push(Some(br));
        } else {
            if verbose() {
                println!("  {num_docs}: <missing>");
            }
            doc_values.push(None);
        }

        doc.add(Box::new(NumericDocValuesField::new(
            "id",
            i64::from(num_docs),
        )));
        doc.add(Box::new(StoredField::new_number(
            "id",
            NumericValue::Int(num_docs),
        )?));
        writer.add_document(&doc)?;
        num_docs += 1;

        if rnd.next_int_bound(40) == 17 {
            // force flush
            writer.get_reader()?.close()?;
        }
    }

    let r = writer.get_reader()?;
    writer.close()?;
    if verbose() {
        println!("  reader={r}");
    }

    let doc_values = Arc::new(doc_values);
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    let iters = LuceneTestCase::at_least(100);
    for iter in 0..iters {
        let reverse = rnd.next_boolean();

        let sort_missing_last = random().next_boolean();
        let sf = SortField::new_with_missing(
            Some("stringdv".to_string()),
            t,
            reverse,
            if sort_missing_last {
                Some(MissingValue::StringLast)
            } else {
                None
            },
        )?;

        let sort = if rnd.next_boolean() {
            Sort::new_fields(vec![sf])?
        } else {
            Sort::new_fields(vec![sf, SortField::FIELD_DOC])?
        };
        let hit_count = TestUtil::next_int(&rnd, 1, r.max_doc() + 20);
        let f = RandomQuery::new(rnd.next_long(), rnd.next_float(), Arc::clone(&doc_values));
        let hits = s.search_with_sort_and_scores(Arc::new(f.clone()), hit_count, &sort, false)?;

        if verbose() {
            println!(
                "\nTEST: iter={iter} {} hits; topN={hit_count}; reverse={reverse}; sortMissingLast={sort_missing_last} sort={sort}",
                hits.total_hits
            );
        }

        // Compute expected results:
        let mut expected = f.match_values.lock().expect("matchValues").clone();
        expected.sort_by(|a, b| match (a, b) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => {
                if sort_missing_last {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                }
            }
            (Some(_), None) => {
                if sort_missing_last {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            }
            (Some(a), Some(b)) => a.cmp(b),
        });

        if reverse {
            expected.reverse();
        }
        if verbose() {
            println!("  expected:");
            for (idx, br) in expected.iter().enumerate() {
                println!(
                    "    {idx}: {}",
                    br.as_ref().map_or("<missing>".to_string(), |b| b
                        .utf8_to_string()
                        .expect("UTF-8"))
                );
                if idx as i32 == hit_count - 1 {
                    break;
                }
            }
        }

        if verbose() {
            println!("  actual:");
            let stored_fields = s.stored_fields()?;
            for (hit_idx, fd) in hits.score_docs.iter().enumerate() {
                let br = bytes_of(&fd.fields.as_ref().expect("fields")[0]);
                println!(
                    "    {hit_idx}: {} id={:?}",
                    br.map_or("<missing>".to_string(), |b| b
                        .utf8_to_string()
                        .expect("UTF-8")),
                    stored_fields.document(fd.doc())?.get("id")
                );
            }
        }
        for (hit_idx, fd) in hits.score_docs.iter().enumerate() {
            let br = expected[hit_idx].clone();

            let br2 = bytes_of(&fd.fields.as_ref().expect("fields")[0]);

            assert_equals(&br, &br2);
        }
    }

    drop(s);
    r.close()?;
    dir.close()
}

/// `(BytesRef) fd.fields[0]`.
fn bytes_of(value: &SortValue) -> Option<BytesRef> {
    match value {
        SortValue::Null => None,
        SortValue::Bytes(b) => Some(b.clone()),
        other => {
            panic!("java.lang.ClassCastException: {other:?} cannot be cast to org.apache.lucene.util.BytesRef")
        }
    }
}

/// `private static class RandomQuery extends Query`.
#[derive(Debug, Clone)]
struct RandomQuery {
    seed: i64,
    density: f32,
    doc_values: Arc<Vec<Option<BytesRef>>>,
    /// `Collections.synchronizedList(new ArrayList<BytesRef>())`.
    match_values: Arc<Mutex<Vec<Option<BytesRef>>>>,
    /// `new ConcurrentHashMap<LeafReaderContext, FixedBitSet>()`, keyed by
    /// the context's identity as Java's identity-equal contexts are.
    bitsets: Arc<Mutex<HashMap<usize, Arc<FixedBitSet>>>>,
}

impl RandomQuery {
    // density should be 0.0 ... 1.0
    fn new(seed: i64, density: f32, doc_values: Arc<Vec<Option<BytesRef>>>) -> RandomQuery {
        RandomQuery {
            seed,
            density,
            doc_values,
            match_values: Arc::new(Mutex::new(Vec::new())),
            bitsets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// `private boolean equalsTo(RandomQuery other)`.
    fn equals_to(&self, other: &RandomQuery) -> bool {
        self.seed == other.seed && Arc::ptr_eq(&self.doc_values, &other.doc_values)
    }
}

/// The anonymous `ConstantScoreWeight` of `RandomQuery.createWeight`.
#[derive(Debug)]
struct RandomWeight {
    query: RandomQuery,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for RandomWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let q = &self.query;
        let bits = {
            let mut bitsets = q.bitsets.lock().expect("bitsets");
            match bitsets.get(&context.id()) {
                Some(bits) => Arc::clone(bits),
                None => {
                    let random = JdkRandom::new(i64::from(context.doc_base()) ^ q.seed);
                    let reader = context.leaf_reader();
                    let max_doc = reader.max_doc();
                    // `assertNotNull(idSource)`: `DocValues.getNumeric` never
                    // returns null.
                    let mut id_source = DocValues::get_numeric(&*reader, "id")?;
                    let mut bitset = FixedBitSet::new(max_doc as usize);
                    for doc_id in 0..max_doc {
                        assert_equals(&doc_id, &id_source.next_doc()?);
                        if random.next_float() <= q.density {
                            bitset.set(doc_id as usize);
                            // System.out.println("  acc id=" + idSource.getInt(docID) + " docID=" +
                            // docID);
                            q.match_values.lock().expect("matchValues").push(
                                q.doc_values[id_source.long_value()? as i32 as usize].clone(),
                            );
                        }
                    }
                    let bitset = Arc::new(bitset);
                    bitsets.insert(context.id(), Arc::clone(&bitset));
                    bitset
                }
            }
        };
        // The bitset is built for the whole segment, the first time each leaf is seen. Every
        // partition iterates through its own set of doc ids, using a separate iterator backed by
        // the shared bitset.
        let cost = bits.approximate_cardinality() as i64;
        let scorer = ConstantScoreScorer::from_iterator(
            score,
            self.score_mode,
            Box::new(BitSetIterator::new(bits as Arc<dyn BitSet>, cost)?),
        );
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Query for RandomQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            RandomWeight {
                query: self.clone(),
                score_mode,
            },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "RandomFilter(density={})",
            rucene::util::java_fmt::float_to_string(self.density)
        )
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<RandomQuery>()
                .is_some_and(|o| self.equals_to(o))
    }

    fn query_hash(&self) -> u64 {
        // `Objects.hash(seed, density)`, then the identity of `docValues` and
        // the class hash.
        let mut h: i32 = 31;
        h = h.wrapping_add((self.seed ^ (self.seed >> 32)) as i32);
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.density.to_bits() as i32);
        let mut identity = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(&self.doc_values) as usize).hash(&mut identity);
        h = h.wrapping_mul(31).wrapping_add(identity.finish() as i32);
        h = h.wrapping_mul(31).wrapping_add(self.class_hash() as i32);
        h as u32 as u64
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
