//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocIDMerger.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::merge::{DocIDMerger, DocIDMergerSub, DocMap};
use rucene::search::NO_MORE_DOCS;
use rucene::util::bit_set::BitSet;
use rucene::util::FixedBitSet;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestDocIDMerger`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocIDMerger",
        vec![
            test("testNoSort", test_no_sort),
            test("testWithSort", test_with_sort),
        ],
    )
}

/// `private static class TestSubUnsorted extends DocIDMerger.Sub`.
struct TestSubUnsorted {
    doc_map: DocMap,
    mapped_doc_id: i32,
    doc_id: i32,
    value_start: i32,
    max_doc: i32,
}

impl TestSubUnsorted {
    fn new(doc_map: DocMap, max_doc: i32, value_start: i32) -> Self {
        TestSubUnsorted {
            doc_map,
            mapped_doc_id: 0,
            doc_id: -1,
            value_start,
            max_doc,
        }
    }

    fn get_value(&self) -> i32 {
        self.value_start + self.doc_id
    }
}

impl DocIDMergerSub for TestSubUnsorted {
    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == self.max_doc {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(self.doc_id)
        }
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// Port of `TestDocIDMerger.testNoSort`.
fn test_no_sort() -> Result<()> {
    let sub_count = TestUtil::next_int(&*random(), 1, 20);
    let mut subs = Vec::new();
    let mut value_start = 0;
    for _ in 0..sub_count {
        let max_doc = TestUtil::next_int(&*random(), 1, 1000);
        let doc_base = value_start;
        subs.push(TestSubUnsorted::new(
            Arc::new(move |doc_id| doc_base + doc_id),
            max_doc,
            value_start,
        ));
        value_start += max_doc;
    }

    let mut merger = DocIDMerger::new(subs, false)?;

    let mut count = 0;
    loop {
        let Some(sub) = merger.next_sub()? else {
            break;
        };
        assert_equals(&count, &sub.mapped_doc_id);
        assert_equals(&count, &sub.get_value());
        count += 1;
    }

    assert_equals(&value_start, &count);
    Ok(())
}

/// `private static class TestSubSorted extends DocIDMerger.Sub`.
struct TestSubSorted {
    doc_map: DocMap,
    mapped_doc_id: i32,
    doc_id: i32,
    max_doc: i32,
    index: i32,
}

impl std::fmt::Display for TestSubSorted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TestSubSorted(index={}, mappedDocID={})",
            self.index, self.mapped_doc_id
        )
    }
}

impl DocIDMergerSub for TestSubSorted {
    fn next_doc(&mut self) -> Result<i32> {
        self.doc_id += 1;
        if self.doc_id == self.max_doc {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(self.doc_id)
        }
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// Port of `TestDocIDMerger.testWithSort`.
fn test_with_sort() -> Result<()> {
    let sub_count = TestUtil::next_int(&*random(), 1, 20);
    let mut old_to_new: Vec<Vec<i32>> = Vec::new();
    // how many docs we've written to each sub:
    let mut uptos: Vec<usize> = Vec::new();
    let mut tot_doc_count = 0;
    for _ in 0..sub_count {
        let max_doc = TestUtil::next_int(&*random(), 1, 1000);
        uptos.push(0);
        old_to_new.push(vec![0; max_doc as usize]);
        tot_doc_count += max_doc;
    }

    let mut completed_subs: Vec<Vec<i32>> = Vec::new();

    // randomly distribute target docIDs into the segments:
    for doc_id in 0..tot_doc_count {
        let sub = random().next_int_bound(old_to_new.len() as i32) as usize;
        let mut upto = uptos[sub];
        old_to_new[sub][upto] = doc_id;
        upto += 1;
        if upto == old_to_new[sub].len() {
            completed_subs.push(old_to_new.remove(sub));
            uptos.remove(sub);
        } else {
            uptos[sub] = upto;
        }
    }
    assert_equals(&0, &old_to_new.len());

    // sometimes do some deletions:
    let live_docs: Option<Arc<FixedBitSet>> = if random().next_boolean() {
        let mut live_docs = FixedBitSet::new(tot_doc_count as usize);
        live_docs.set_range(0, tot_doc_count as usize);
        let delete_attempt_count = TestUtil::next_int(&*random(), 1, tot_doc_count);
        for _ in 0..delete_attempt_count {
            live_docs.clear(random().next_int_bound(tot_doc_count) as usize);
        }
        Some(Arc::new(live_docs))
    } else {
        None
    };

    let mut subs = Vec::new();
    for i in 0..sub_count as usize {
        let doc_map = Arc::new(completed_subs[i].clone());
        let live = live_docs.clone();
        let len = doc_map.len() as i32;
        subs.push(TestSubSorted {
            doc_map: Arc::new(move |doc_id| {
                let mapped = doc_map[doc_id as usize];
                if live.as_ref().is_none_or(|l| l.get(mapped as usize)) {
                    mapped
                } else {
                    -1
                }
            }),
            mapped_doc_id: 0,
            doc_id: -1,
            max_doc: len,
            index: i as i32,
        });
    }

    let mut merger = DocIDMerger::new(subs, true)?;

    let mut count = 0;
    loop {
        let Some(sub) = merger.next_sub()? else {
            break;
        };
        if let Some(live_docs) = &live_docs {
            count = live_docs.next_set_bit(count);
        }
        assert_equals(&count, &sub.mapped_doc_id);
        count += 1;
    }

    match &live_docs {
        Some(live_docs) => {
            if count < tot_doc_count {
                assert_equals(&NO_MORE_DOCS, &live_docs.next_set_bit(count));
            } else {
                assert_equals(&tot_doc_count, &count);
            }
        }
        None => assert_equals(&tot_doc_count, &count),
    }
    Ok(())
}
