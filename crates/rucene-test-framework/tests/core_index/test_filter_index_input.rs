//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFilterIndexInput.java`.
//!
//! * `testOverrides` reflects on `FilterIndexInput.class.getMethods()`;
//!   Rucene has no run-time reflection, so the methods are declared lists:
//!   the `IndexInput`/`DataInput` methods Rucene's `impl DataInput for
//!   FilterIndexInput` and `impl IndexInput for FilterIndexInput`
//!   (`src/store.rs`) define, under their Java names, and the abstract
//!   methods of `IndexInput.java` and `DataInput.java`. Rucene's required
//!   `to_java_string` (Java's inherited `toString()`) is not listed: a Rust
//!   trait cannot inherit `Object.toString()`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene::store::{Directory, FSDirectory, FilterIndexInput, IndexInput, DEFAULT_IO_CONTEXT};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, random};
use crate::test_index_input::{
    check_random_reads, check_reads, check_seeks_and_skips, instance_test, random_test_bytes,
    IndexInputInstance, InterceptingIndexInput, READ_TEST_BYTES,
};

/// `TestFilterIndexInput`, which extends `TestIndexInput`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestFilterIndexInput")
        .extends(&crate::test_index_input::class_def())
        .constructor(Constructor::no_args(|| IndexInputInstance {
            get_index_input,
        }))
        .method(instance_test("testRawFilterIndexInputRead", |_| {
            test_raw_filter_index_input_read()
        }))
        .method(instance_test("testOverrides", |_| test_overrides()).annotate(Test::new()))
        .method(instance_test("testUnwrap", |_| test_unwrap()))
        .build()
}

/// `getIndexInput(len)` (`@Override`).
fn get_index_input(len: i64) -> Box<dyn IndexInput> {
    Box::new(FilterIndexInput::new(
        "wrapped foo",
        Box::new(InterceptingIndexInput::new("foo", len)),
    ))
}

/// Port of `TestFilterIndexInput.testRawFilterIndexInputRead`.
fn test_raw_filter_index_input_read() -> Result<()> {
    for _ in 0..10 {
        let random = random();
        let dir = LuceneTestCase::new_directory();
        let d = dir.as_directory();
        let mut os = d.create_output("foo", &*LuceneTestCase::new_io_context(&*random))?;
        os.write_bytes(READ_TEST_BYTES, 0, READ_TEST_BYTES.len())?;
        os.close()?;
        let mut is = FilterIndexInput::new(
            "wrapped foo",
            d.open_input("foo", &*LuceneTestCase::new_io_context(&*random))?,
        );
        check_reads(&mut is, "java.io.IOException")?;
        check_seeks_and_skips(&mut is, &*random)?;
        is.close()?;

        let bytes = random_test_bytes();
        let mut os = d.create_output("bar", &*LuceneTestCase::new_io_context(&*random))?;
        os.write_bytes(&bytes, 0, bytes.len())?;
        os.close()?;
        let mut is = FilterIndexInput::new(
            "wrapped bar",
            d.open_input("bar", &*LuceneTestCase::new_io_context(&*random))?,
        );
        check_random_reads(&mut is)?;
        check_seeks_and_skips(&mut is, &*random)?;
        is.close()?;
        drop(d);
        dir.close()?;
    }
    Ok(())
}

/// The public methods `FilterIndexInput` declares, as `(name, parameter
/// types)`: the members of Rucene's `impl DataInput for FilterIndexInput` and
/// `impl IndexInput for FilterIndexInput`, under their Java names, plus
/// `getDelegate()` and the static `unwrap(IndexInput)`.
const FILTER_INDEX_INPUT_METHODS: &[(&str, &[&str])] = &[
    ("unwrap", &["org.apache.lucene.store.IndexInput"]),
    ("getDelegate", &[]),
    ("readByte", &[]),
    ("readBytes", &["byte[]", "int", "int"]),
    ("readBytes", &["byte[]", "int", "int", "boolean"]),
    ("skipBytes", &["long"]),
    ("close", &[]),
    ("getFilePointer", &[]),
    ("length", &[]),
    ("seek", &["long"]),
    ("slice", &["java.lang.String", "long", "long"]),
    ("clone", &[]),
    ("prefetch", &["long", "long"]),
    ("isLoaded", &[]),
    ("randomAccessSlice", &["long", "long"]),
];

/// `IndexInput.class.getMethod(name, types)`: the public methods of
/// `IndexInput` and `DataInput` (`IndexInput.java`, `DataInput.java`), with
/// whether each is abstract.
const INDEX_INPUT_METHODS: &[(&str, &[&str], bool)] = &[
    ("readByte", &[], true),
    ("readBytes", &["byte[]", "int", "int"], true),
    ("readBytes", &["byte[]", "int", "int", "boolean"], false),
    ("readShort", &[], false),
    ("readInt", &[], false),
    ("readGroupVInt", &["int[]", "int"], false),
    ("readVInt", &[], false),
    ("readZInt", &[], false),
    ("readLong", &[], false),
    ("readLongs", &["long[]", "int", "int"], false),
    ("readInts", &["int[]", "int", "int"], false),
    ("readFloats", &["float[]", "int", "int"], false),
    ("readVLong", &[], false),
    ("readZLong", &[], false),
    ("readString", &[], false),
    ("readMapOfStrings", &[], false),
    ("readSetOfStrings", &[], false),
    ("close", &[], true),
    ("getFilePointer", &[], true),
    ("seek", &["long"], true),
    ("skipBytes", &["long"], false),
    ("length", &[], true),
    ("toString", &[], false),
    ("clone", &[], false),
    ("slice", &["java.lang.String", "long", "long"], true),
    (
        "slice",
        &[
            "java.lang.String",
            "long",
            "long",
            "org.apache.lucene.store.IOContext",
        ],
        false,
    ),
    ("randomAccessSlice", &["long", "long"], false),
    ("prefetch", &["long", "long"], false),
    (
        "updateIOContext",
        &["org.apache.lucene.store.IOContext"],
        false,
    ),
    ("isLoaded", &[], false),
];

/// Port of `TestFilterIndexInput.testOverrides`.
fn test_overrides() -> Result<()> {
    for (name, parameter_types) in FILTER_INDEX_INPUT_METHODS {
        if name.contains("clone") {
            // special case
            continue;
        }
        // `m.getDeclaringClass() == FilterIndexInput.class`: every listed
        // method is declared by `FilterIndexInput`.
        // verify that only abstract methods are overridden
        match INDEX_INPUT_METHODS
            .iter()
            .find(|(n, p, _)| n == name && p == parameter_types)
        {
            Some((_, _, is_abstract)) => Assert::assert_true_msg(
                &format!("Non-abstract method {name} is overridden"),
                *is_abstract,
            ),
            // `assertTrue(e instanceof NoSuchMethodException)`.
            None => Assert::assert_true(true),
        }
    }
    Ok(())
}

/// Port of `TestFilterIndexInput.testUnwrap`.
fn test_unwrap() -> Result<()> {
    let dir: Box<dyn Directory> = Box::new(FSDirectory::open(LuceneTestCase::create_temp_dir())?);
    let mut ignored = dir.create_output("test", &*DEFAULT_IO_CONTEXT)?;
    let index_input = dir.open_input("test", &*DEFAULT_IO_CONTEXT)?;
    let index_input_addr = &*index_input as *const dyn IndexInput as *const ();
    let mut filter_index_input = FilterIndexInput::new("wrapper of test", index_input);
    assert_equals(
        &index_input_addr,
        &(filter_index_input.get_delegate() as *const dyn IndexInput as *const ()),
    );
    assert_equals(
        &index_input_addr,
        &(FilterIndexInput::unwrap(&filter_index_input) as *const dyn IndexInput as *const ()),
    );
    ignored.close()?;
    filter_index_input.close()?;
    dir.close()
}
