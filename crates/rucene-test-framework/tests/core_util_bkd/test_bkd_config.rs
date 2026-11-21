//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestBKDConfig.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::bkd::BKDConfig;
use rucene::util::ArrayUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, test};

/// `TestBKDConfig`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testInvalidNumDims", test_invalid_num_dims),
        test("testInvalidNumIndexedDims", test_invalid_num_indexed_dims),
        test("testInvalidBytesPerDim", test_invalid_bytes_per_dim),
        test(
            "testInvalidMaxPointsPerLeafNode",
            test_invalid_max_points_per_leaf_node,
        ),
        test("testDeduplicate", test_deduplicate),
    ];
    class("org.apache.lucene.util.bkd.TestBKDConfig", methods)
}

const IAE: &str = "java.lang.IllegalArgumentException";

fn message_contains(t: &randomizedtesting::jdk::exception::Throwable, s: &str) -> bool {
    t.message().unwrap_or("").contains(s)
}

fn test_invalid_num_dims() -> Result<()> {
    let ex = expect_throws(IAE, || {
        BKDConfig::new(0, 0, 8, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)
    });
    Assert::assert_true(message_contains(
        &ex,
        &format!("numDims must be 1 .. {}", BKDConfig::MAX_DIMS),
    ));
    Ok(())
}

fn test_invalid_num_indexed_dims() -> Result<()> {
    {
        let ex = expect_throws(IAE, || {
            BKDConfig::new(1, 0, 8, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)
        });
        Assert::assert_true(message_contains(
            &ex,
            &format!("numIndexDims must be 1 .. {}", BKDConfig::MAX_INDEX_DIMS),
        ));
    }
    {
        let ex = expect_throws(IAE, || {
            BKDConfig::new(1, 2, 8, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)
        });
        Assert::assert_true(message_contains(&ex, "numIndexDims cannot exceed numDims"));
    }
    Ok(())
}

fn test_invalid_bytes_per_dim() -> Result<()> {
    let ex = expect_throws(IAE, || {
        BKDConfig::new(1, 1, 0, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)
    });
    Assert::assert_true(message_contains(&ex, "bytesPerDim must be > 0"));
    Ok(())
}

fn test_invalid_max_points_per_leaf_node() -> Result<()> {
    {
        let ex = expect_throws(IAE, || BKDConfig::new(1, 1, 8, -1));
        Assert::assert_true(message_contains(&ex, "maxPointsInLeafNode must be > 0"));
    }
    {
        let ex = expect_throws(IAE, || {
            BKDConfig::new(1, 1, 8, (ArrayUtil::MAX_ARRAY_LENGTH + 1) as i32)
        });
        Assert::assert_true(message_contains(
            &ex,
            "maxPointsInLeafNode must be <= ArrayUtil.MAX_ARRAY_LENGTH",
        ));
    }
    Ok(())
}

/// `testDeduplicate()`.
///
/// **Divergence from Lucene 10.5.0.** Java asserts object identity:
/// `BKDConfig.of` returns its cached default instance (`assertSame`) and a
/// new instance otherwise (`assertNotSame`). Rucene's `BKDConfig::of`
/// returns the configuration by value, so identity cannot be observed: the
/// port asserts that the deduplicated value equals the cached one, and that
/// two results of `of` are distinct values in memory.
fn test_deduplicate() -> Result<()> {
    {
        for dims in [1, 2] {
            for bytes_per_dim in [2, 4, 8, 16] {
                let config = BKDConfig::of(
                    dims,
                    dims,
                    bytes_per_dim,
                    BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
                )?;
                let deduped = BKDConfig::of(
                    dims,
                    dims,
                    bytes_per_dim,
                    BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
                )?;
                assert_equals(&deduped, &config);
            }
        }
    }
    {
        let config = BKDConfig::of(7, 4, 4, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)?;
        let deduped = BKDConfig::of(7, 4, 4, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)?;
        assert_equals(&deduped, &config);
    }
    {
        let config = BKDConfig::of(1, 1, 6, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)?;
        let deduped = BKDConfig::of(1, 1, 6, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE)?;
        Assert::assert_false(std::ptr::eq(&config, &deduped));
    }
    Ok(())
}
