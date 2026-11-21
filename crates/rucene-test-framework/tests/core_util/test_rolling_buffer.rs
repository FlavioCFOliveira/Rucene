//! Port of `lucene/core/src/test/org/apache/lucene/util/TestRollingBuffer.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::{FixedBitSet, Resettable, RollingBuffer};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, verbose};

/// `TestRollingBuffer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestRollingBuffer",
        vec![test("test", test_rolling)],
    )
}

/// `Position implements RollingBuffer.Resettable`.
struct Position {
    pos: i32,
}

impl Resettable for Position {
    fn reset(&mut self) {
        self.pos = -1;
    }
}

fn test_rolling() -> Result<()> {
    let mut buffer = RollingBuffer::new(|| Position { pos: -1 });

    for _ in 0..100 * *RANDOM_MULTIPLIER {
        let mut free_before_pos = 0i32;
        let max_pos = LuceneTestCase::at_least(10000);
        let mut pos_set = FixedBitSet::new((max_pos + 1000) as usize);
        let mut pos_upto = 0i32;
        let random = random();
        while free_before_pos < max_pos {
            if random.next_int_bound(4) == 1 {
                let limit = if LuceneTestCase::rarely() { 1000 } else { 20 };
                let inc = random.next_int_bound(limit);
                let pos = free_before_pos + inc;
                pos_upto = pos_upto.max(pos);
                if verbose() {
                    println!("  check pos={pos} posUpto={pos_upto}");
                }
                let pos_data = buffer.get(pos);
                if !pos_set.get_and_set(pos as usize) {
                    assert_equals(&-1, &pos_data.pos);
                    pos_data.pos = pos;
                } else {
                    assert_equals(&pos, &pos_data.pos);
                }
            } else {
                if pos_upto > free_before_pos {
                    free_before_pos += random.next_int_bound(pos_upto - free_before_pos);
                }
                if verbose() {
                    println!("  freeBeforePos={free_before_pos}");
                }
                buffer.free_before(free_before_pos);
            }
        }

        buffer.reset();
    }
    Ok(())
}
