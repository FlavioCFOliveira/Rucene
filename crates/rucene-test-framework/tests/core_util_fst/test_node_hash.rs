//! Port of `lucene/core/src/test/org/apache/lucene/util/fst/TestNodeHash.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomBytes;
use randomizedtesting::jdk::reflect::{Class, Method};
use rucene::error::Result;
use rucene::util::fst::{NoOutputs, NodeHash, PagedGrowableHash};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, class, random, test};

/// `TestNodeHash`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![test(
        "testCopyFallbackNodeBytes",
        test_copy_fallback_node_bytes,
    )];
    class("org.apache.lucene.util.fst.TestNodeHash", methods)
}

fn test_copy_fallback_node_bytes() -> Result<()> {
    // `new NodeHash<>(null, 1)`: the compiler is not used by the tables.
    let _node_hash: NodeHash<NoOutputs> = NodeHash::new(1.0);
    let mut primary_hash_table = PagedGrowableHash::new();
    let mut fallback_hash_table = PagedGrowableHash::new();
    let node_length = LuceneTestCase::at_least(500) as usize;
    let fallback_hash_slot = 1;
    let fallback_bytes = RandomBytes::random_bytes_of_length(&*random(), node_length as i32);
    fallback_hash_table.copy_node_bytes(fallback_hash_slot, &fallback_bytes, node_length)?;
    let stored_bytes = fallback_hash_table.bytes(fallback_hash_slot, node_length);
    for i in 0..node_length {
        assert_equals_msg(
            Some(&format!("byte @ index={i}")),
            &(fallback_bytes[i] as i8),
            &(stored_bytes[i] as i8),
        );
    }

    let primary_hash_slot = 2;
    primary_hash_table.copy_fallback_node_bytes(
        primary_hash_slot,
        &fallback_hash_table,
        fallback_hash_slot,
        node_length,
    )?;
    let copied_bytes = primary_hash_table.bytes(primary_hash_slot, node_length);
    for i in 0..node_length {
        assert_equals_msg(
            Some(&format!("byte @ index={i}")),
            &(fallback_bytes[i] as i8),
            &(copied_bytes[i] as i8),
        );
    }
    Ok(())
}
