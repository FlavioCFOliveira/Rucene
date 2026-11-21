//! Port of `lucene/core/src/test/org/apache/lucene/util/Test2BPagedBytes.java`
//! (`@Monster("You must increase heap to > 2 G to run this")`).

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene::util::{BytesRef, PagedBytes};
use rucene_test_framework::store::mock_directory_wrapper::Throttling;
use rucene_test_framework::util::lucene_test_case::Monster;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test};

/// `Test2BPagedBytes`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.util.Test2BPagedBytes",
        vec![test("test", test_)],
        |builder| {
            builder.annotate(Monster {
                value: "You must increase heap to > 2 G to run this".to_string(),
            })
        },
    )
}

fn test_() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "test2BPagedBytes",
    ));
    if let Some(mock) = dir.as_mock() {
        mock.set_throttling(Throttling::Never);
    }
    let mut pb = PagedBytes::new(15);
    let directory = dir.as_directory();
    let mut data_output = directory.create_output("foo", &*DEFAULT_IO_CONTEXT)?;
    let mut net_bytes: i64 = 0;
    let seed = random().next_long();
    let mut last_fp: i64 = 0;
    let mut r2 = JdkRandom::new(seed);
    while (net_bytes as f64) < 1.1 * i32::MAX as f64 {
        let num_bytes = TestUtil::next_int(&r2, 1, 32768);
        let mut bytes = vec![0u8; num_bytes as usize];
        r2.next_bytes(&mut bytes);
        data_output.write_bytes(&bytes, 0, bytes.len())?;
        let fp = data_output.file_pointer();
        // Java `assert`.
        assert!(fp == last_fp + num_bytes as i64);
        last_fp = fp;
        net_bytes += num_bytes as i64;
    }
    data_output.close()?;
    let mut input = directory.open_input("foo", &*DEFAULT_IO_CONTEXT)?;
    let length = input.length();
    pb.copy_from_input(&mut *input, length)?;
    input.close()?;
    let reader = pb.freeze(true)?;

    r2 = JdkRandom::new(seed);
    net_bytes = 0;
    while (net_bytes as f64) < 1.1 * i32::MAX as f64 {
        let num_bytes = TestUtil::next_int(&r2, 1, 32768);
        let mut bytes = vec![0u8; num_bytes as usize];
        r2.next_bytes(&mut bytes);
        let expected = BytesRef::new(bytes);

        let actual = BytesRef::new(
            reader
                .fill_slice(net_bytes, num_bytes as usize)
                .into_owned(),
        );
        assert_equals(&expected, &actual);

        net_bytes += num_bytes as i64;
    }
    drop(directory);
    dir.close()
}
