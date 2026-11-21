//! Shared helpers of the core-test targets that port Lucene 10.5.0's
//! `lucene/core/src/test` classes (`core_store_tests`, `core_root_tests`,
//! `core_internal_*_tests`, `core_analysis_*_tests`): the test-method
//! builders, the suite runner, and the few JDK classes those tests use that
//! the framework does not provide.

#![allow(dead_code)]

use std::sync::{Arc, Condvar, Mutex};

use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

/// One suite of a test binary at a time: the runner's state is process-wide.
pub static SUITES: Mutex<()> = Mutex::new(());

/// A `public void testX() throws Exception` method whose body is `body`; an
/// error it returns is thrown as the Java exception it stands for.
pub fn t(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body_any(move || body().unwrap_or_else(|e| rethrow_java(e)))
}

/// A `public void testX()` method over the instance state `T`.
pub fn ti<T: std::any::Any + Send>(name: &str, body: fn(&mut T) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut T| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// The builder of a `LuceneTestCase` subclass named `name` with a no-args
/// constructor and no instance state.
pub fn lucene_class(name: &str) -> ClassBuilder {
    Class::builder(name)
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
}

/// Runs the suite `class` builds (on the seeds `RUCENE_SUITE_SEEDS`
/// selects), one suite of the binary at a time.
pub fn run(class: fn() -> Arc<Class>) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    crate::suite_seeds::run_suite(class);
}

/// `java.util.concurrent.CountDownLatch`.
pub struct CountDownLatch {
    count: Mutex<i64>,
    zero: Condvar,
}

impl CountDownLatch {
    /// `new CountDownLatch(count)`.
    pub fn new(count: i64) -> CountDownLatch {
        CountDownLatch {
            count: Mutex::new(count),
            zero: Condvar::new(),
        }
    }

    /// `countDown()`.
    pub fn count_down(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
            if *count == 0 {
                self.zero.notify_all();
            }
        }
    }

    /// `await()`.
    pub fn await_(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count > 0 {
            count = self.zero.wait(count).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// `java.util.zip.CRC32`: the CRC-32 (IEEE 802.3, reflected polynomial
/// `0xEDB88320`) `java.util.zip.Checksum` the JDK implements.
#[derive(Clone, Default)]
pub struct Crc32 {
    crc: u32,
}

const fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

static CRC32_TABLE: [u32; 256] = crc32_table();

impl Crc32 {
    /// `new CRC32()`.
    pub fn new() -> Crc32 {
        Crc32 { crc: 0 }
    }

    /// `update(int b)`: the low eight bits of `b`.
    pub fn update(&mut self, b: i32) {
        self.update_bytes(&[b as u8], 0, 1);
    }

    /// `update(byte[] b, int off, int len)`.
    pub fn update_bytes(&mut self, b: &[u8], off: usize, len: usize) {
        let mut c = !self.crc;
        for &byte in &b[off..off + len] {
            c = CRC32_TABLE[((c ^ byte as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.crc = !c;
    }

    /// `update(byte[] b)`.
    pub fn update_all(&mut self, b: &[u8]) {
        self.update_bytes(b, 0, b.len());
    }

    /// `getValue()`.
    pub fn get_value(&self) -> i64 {
        self.crc as i64
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.crc = 0;
    }
}

/// Java's `byte[]` as the signed values `assertArrayEquals` prints.
pub fn jbytes(b: &[u8]) -> Vec<i8> {
    b.iter().map(|&x| x as i8).collect()
}

/// `assertArrayEquals(byte[] expecteds, byte[] actuals)`.
pub fn assert_bytes_equals(expected: &[u8], actual: &[u8]) {
    rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals(
        &jbytes(expected),
        &jbytes(actual),
    )
}

/// `assertArrayEquals(String message, byte[] expecteds, byte[] actuals)`.
pub fn assert_bytes_equals_msg(message: &str, expected: &[u8], actual: &[u8]) {
    rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals_msg(
        Some(message),
        &jbytes(expected),
        &jbytes(actual),
    )
}
