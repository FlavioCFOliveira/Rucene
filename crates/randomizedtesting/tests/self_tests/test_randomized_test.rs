//! Port of `com.carrotsearch.randomizedtesting.TestRandomizedTest`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the `RandomizedTest`
//! helpers.

use std::sync::Arc;

use randomizedtesting::jdk::exception::{Throwable, ILLEGAL_ARGUMENT_EXCEPTION, THROWABLE};
use randomizedtesting::jdk::io::Closeable;
use randomizedtesting::jdk::locale;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::{LifecycleScope, RandomizedTest as R, ASSUMPTION_VIOLATED_EXCEPTION};

use crate::support::{
    assert_equals, assert_false, assert_true, assert_true_msg, cls, fail, fail_msg, test,
    try_catch, unit, RANDOMIZED_TEST,
};

fn io<T>(r: std::io::Result<T>) -> T {
    r.unwrap_or_else(|e| Throwable::new("java.io.IOException", e.to_string()).throw())
}

/// `str.codePointCount(0, str.length())`.
fn count_codepoints(s: &str) -> usize {
    s.chars().count()
}

/// `expect IllegalArgumentException`.
fn expect_iae(f: impl FnOnce()) {
    match try_catch(f) {
        Err(e) if e.is_instance(ILLEGAL_ARGUMENT_EXCEPTION) => {} // expected.
        Err(e) => e.throw(),
        Ok(()) => {}
    }
}

#[cfg(unix)]
fn is_executable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn is_executable(_m: &std::fs::Metadata) -> bool {
    true
}

/// `TestRandomizedTest extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomizedTest")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testRandomIntBetween", || {
            let mut array = [false; 10];
            for _ in 0..10000 {
                array[R::random_int_between(0, array.len() as i32 - 1) as usize] = true;
            }
            for b in array {
                assert_true(b);
            }
        }))
        .method(test("testAtLeast", || {
            assert_equals(R::at_least(i32::MAX), i32::MAX);
            let v = R::random_int_between(0, i32::MAX);
            for _ in 0..10000 {
                assert_true(R::at_least(v) >= v);
            }
        }))
        .method(test("testAtMost", || {
            assert_equals(R::at_most(0), 0);
            let v = R::random_int_between(0, i32::MAX);
            for _ in 0..10000 {
                assert_true(R::at_most(v) <= v);
            }
        }))
        .method(test("testRandomIntBetweenBoundaryCases", || {
            for _ in 0..10000 {
                let j = R::random_int_between(0, i32::MAX);
                assert_true(j >= 0);
                // This must fall in range, but nonetheless
                R::random_int_between(i32::MIN, i32::MAX);
            }
        }))
        .method(test("testRandomFromArray", || {
            expect_iae(|| {
                let empty: [Option<i32>; 0] = [];
                R::random_from(&empty);
                fail();
            });
            let mut ints: Vec<Option<i32>> = (0..10).map(Some).collect();
            for _ in 0..10000 {
                if let Some(j) = *R::random_from(&ints) {
                    ints[j as usize] = None;
                }
            }
            for i in &ints {
                assert_true(i.is_none());
            }
        }))
        .method(test("testRandomFromList", || {
            expect_iae(|| {
                let empty: Vec<Option<i32>> = Vec::new();
                R::random_from_list(&empty);
                fail();
            });
            let mut ints: Vec<Option<i32>> = (0..10).map(Some).collect();
            for _ in 0..10000 {
                if let Some(j) = *R::random_from_list(&ints) {
                    ints[j as usize] = None;
                }
            }
            for i in &ints {
                assert_true(i.is_none());
            }
        }))
        .method(test("testNewTempDir", || {
            for _ in 0..10 {
                let dir = io(R::new_temp_dir());
                let m = io(std::fs::metadata(&dir));
                assert_true(m.is_dir());
                assert_true(!m.permissions().readonly());
                let mut entries = io(std::fs::read_dir(&dir));
                assert_true(is_executable(&m));
                assert_false(entries.next().is_some());
            }
        }))
        .method(test("testNewTempFile", || {
            for _ in 0..10 {
                let file = io(R::new_temp_file());
                let m = io(std::fs::metadata(&file));
                assert_true(m.is_file());
                assert_true(!m.permissions().readonly());
                assert_true(std::fs::File::open(&file).is_ok());
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                assert_true(name.contains(' '));
                drop(io(std::fs::File::create(&file)));
            }
        }))
        .method(test("testRandomLocale", || {
            let _ = R::random_locale();
        }))
        .method(test("testRandomTimeZone", || {
            for id in locale::available_time_zones_sorted() {
                if id.id().is_empty() {
                    fail_msg(&format!("getTimeZone null: {}", id.id()));
                }
            }
            let _ = R::random_time_zone();
        }))
        .method(test("testRandomAsciiOfLength", || {
            assert_true(R::random_ascii_letters_of_length(0).is_empty());
            for _ in 0..1000 {
                let max_length = R::random_int_between(0, 20);
                let s = R::random_ascii_letters_of_length(max_length);
                assert_true(s.chars().all(|c| c.is_ascii_alphabetic()));
                assert_true(s.encode_utf16().count() as i32 <= max_length);
            }
        }))
        .method(test("testRandomAlphanumOfLength", || {
            assert_true(R::random_ascii_alphanum_of_length(0).is_empty());
            for _ in 0..1000 {
                let max_length = R::random_int_between(0, 20);
                let s = R::random_ascii_alphanum_of_length(max_length);
                assert_true(s.chars().all(|c| c.is_ascii_alphanumeric()));
                assert_true(s.encode_utf16().count() as i32 <= max_length);
            }
        }))
        .method(test("testRandomUnicodeOfLength", || {
            for _ in 0..1000 {
                let max_length = R::random_int_between(0, 20);
                let s = R::random_unicode_of_length(max_length);
                let len = s.encode_utf16().count() as i32;
                assert_true_msg(&format!("{len} {max_length}"), len <= max_length);
            }
        }))
        .method(test("testRandomRealisticUnicodeOfLength", || {
            assert_true(R::random_realistic_unicode_of_length(0).is_empty());
            assert_true(R::random_realistic_unicode_of_codepoint_length(0).is_empty());
            for _ in 0..1000 {
                let min_length = R::random_int_between(0, 20);
                let max_length = min_length + R::random_int_between(0, 20);
                let s = R::random_realistic_unicode_of_codepoint_length(max_length);
                assert_true(count_codepoints(&s) as i32 <= max_length);
                let s =
                    R::random_realistic_unicode_of_codepoint_length_between(min_length, max_length);
                let c = count_codepoints(&s) as i32;
                assert_true(c >= min_length);
                assert_true(c <= max_length);
            }
        }))
        .method(test("testAssumeTrue", || {
            let message = R::random_unicode_of_length(10);
            match try_catch(|| R::assume_true_msg(&message, false)) {
                Err(e) if e.is_instance(ASSUMPTION_VIOLATED_EXCEPTION) => {
                    assert_true(e.message().unwrap_or("").contains(&message))
                }
                Err(e) => e.throw(),
                Ok(()) => {}
            }
        }))
        .method(test("testAssumeNoException", || {
            let message = R::random_unicode_of_length(10);
            let t = Throwable::with_message(THROWABLE, None);
            match try_catch(|| R::assume_no_exception_msg(&message, Some(&t))) {
                Err(e) if e.is_instance(ASSUMPTION_VIOLATED_EXCEPTION) => {
                    assert_true(e.message().unwrap_or("").contains(&message));
                    // assertSame(t, e.getCause()): the chained throwable.
                    assert_true(e.cause().is_some_and(|c| c.to_string() == t.to_string()));
                }
                Err(e) => e.throw(),
                Ok(()) => {}
            }
        }))
        .method(test("testIterations", || {
            assert_equals(0, R::iterations(0, 0));
            assert_equals(i32::MAX, R::iterations(i32::MAX, i32::MAX));
            let mut i = 0;
            while i < R::iterations(1, 1000) {
                let j = R::iterations(0, 100);
                assert_true((0..=100).contains(&j));
                i += 1;
            }
        }))
        .method(test("testNewServerSocket", || {
            let socket = io(R::new_server_socket(LifecycleScope::Test));
            io(socket.close());
        }))
        .method(test("testRarely", || {
            let calls = 100_000;
            let rarely = (0..calls).filter(|_| R::rarely()).count();
            let rf = rarely as f64 / calls as f64 * 100.0;
            assert_true_msg(
                &format!(
                    "rarely should be > 5% & < 15%: {}",
                    randomizedtesting::jdk::lang::double_to_string(rf)
                ),
                rf > 5.0 && rf < 15.0,
            );
        }))
        .build()]
}
