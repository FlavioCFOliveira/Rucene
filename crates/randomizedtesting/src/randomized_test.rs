//! Port of `com.carrotsearch.randomizedtesting.RandomizedTest`
//! (randomizedtesting 2.8.1, `RandomizedTest.java`).

use std::fmt::Display;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::generators::{BiasedNumbers, RandomBytes, RandomNumbers, RandomPicks, RandomStrings};
use crate::jdk::exception::{self, illegal_argument, Throwable};
use crate::jdk::io::Closeable;
use crate::jdk::lang;
use crate::jdk::locale::{self, Locale, TimeZone};
use crate::jdk::math;
use crate::jdk::random::{JdkRandom, Random};
use crate::jdk::reflect::{modifier, Class, Constructor, Field, Value};
use crate::jdk::runtime;
use crate::jdk::system;
use crate::jdk::thread::Thread;
use crate::lifecycle_scope::LifecycleScope;
use crate::randomized_context::RandomizedContext;
use crate::temp_path_resource::TempPathResource;

/// `org.junit.AssumptionViolatedException`, the class of the assumption
/// failures [`RandomizedTest::assume_true`] and its siblings throw.
pub const ASSUMPTION_VIOLATED_EXCEPTION: &str = "org.junit.AssumptionViolatedException";

/// `DEFAULT_MULTIPLIER`.
const DEFAULT_MULTIPLIER: f64 = 1.0;

/// The global temporary directory and the lock of
/// `synchronized (RandomizedTest.class)`.
static GLOBAL_TEMP_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `tempSubFileNameCount`.
static TEMP_SUB_FILE_NAME_COUNT: AtomicI32 = AtomicI32::new(0);

/// Common scaffolding for randomized tests: `RandomizedTest`'s static
/// helpers, every one drawing from the calling thread's
/// [`RandomizedContext`].
///
/// Java's overloads take distinct names here (`random_int` /
/// `random_int_max`, `between` / `between_long`, ...); each method names
/// its Java signature.
///
/// **Divergences.**
/// * The protected `Charset` constants (`UTF8`, `UTF16`, `ISO8859_1`,
///   `US_ASCII`, `UTF32`) have no Rust counterpart: Rust strings are UTF-8.
/// * `$(Object...)` and `$$(Object[]...)` return their varargs array; Rust
///   has no varargs and an array or `vec!` literal is the same expression.
/// * An assumption failure panics with
///   `org.junit.AssumptionViolatedException: <message>`, which the harness
///   reports as an assumption (ignored) result.
pub struct RandomizedTest;

impl RandomizedTest {
    /// The declaration of `com.carrotsearch.randomizedtesting.RandomizedTest`
    /// for suites that extend it (`Class::builder(..).extends(..)`): its
    /// public constructor and its static fields, which the runner's rules
    /// read (`StaticFieldsInvariantRule` walks superclasses). `@RunWith`
    /// needs no declaration: every class runs under the port's
    /// `RandomizedRunner`.
    ///
    /// `tempSubFileNameCount` is an `AtomicInteger`, an object with one
    /// `int` field; its value is declared as a boxed `Integer`, the same
    /// layout for the memory estimator.
    pub fn class() -> Arc<Class> {
        static CLASS: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
            let charset = |name: &str, java: &'static str| {
                Field::new_static(name, "java.nio.charset.Charset", move || {
                    Value::object(java, java)
                })
                .modifiers(modifier::PROTECTED | modifier::FINAL)
            };
            Class::builder("com.carrotsearch.randomizedtesting.RandomizedTest")
                .modifiers(modifier::PUBLIC)
                .constructor(Constructor::no_args(|| ()))
                .field(
                    Field::new_static("SYSPROP_MULTIPLIER", "java.lang.String", || {
                        Value::Str(RandomizedTest::SYSPROP_MULTIPLIER.into())
                    })
                    .modifiers(modifier::PUBLIC | modifier::FINAL),
                )
                .field(charset("UTF8", "UTF-8"))
                .field(charset("UTF16", "UTF-16"))
                .field(charset("ISO8859_1", "ISO-8859-1"))
                .field(charset("US_ASCII", "US-ASCII"))
                .field(charset("UTF32", "UTF-32"))
                .field(
                    Field::new_static("DEFAULT_MULTIPLIER", "double", || {
                        Value::Double(DEFAULT_MULTIPLIER)
                    })
                    .modifiers(modifier::PRIVATE | modifier::FINAL),
                )
                .field(
                    Field::new_static("globalTempDir", "java.nio.file.Path", || {
                        match GLOBAL_TEMP_DIR
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone()
                        {
                            Some(p) => {
                                let shown = p.display().to_string();
                                Value::object(p, shown)
                            }
                            None => Value::Null,
                        }
                    })
                    .modifiers(modifier::PRIVATE),
                )
                .field(
                    Field::new_static(
                        "tempSubFileNameCount",
                        "java.util.concurrent.atomic.AtomicInteger",
                        || Value::Int(TEMP_SUB_FILE_NAME_COUNT.load(Ordering::Relaxed)),
                    )
                    .modifiers(modifier::PRIVATE),
                )
                .build()
        });
        Arc::clone(&CLASS)
    }

    /// `SYSPROP_MULTIPLIER`: the global multiplier property (a double).
    pub const SYSPROP_MULTIPLIER: &'static str = "randomized.multiplier";

    /// `getContext()`: `RandomizedContext.current()`.
    pub fn get_context() -> Arc<RandomizedContext> {
        RandomizedContext::current()
    }

    /// `isNightly()`: whether the `@Nightly` test group is enabled.
    pub fn is_nightly() -> bool {
        RandomizedTest::get_context().is_nightly()
    }

    /// `getRandom()`: the calling thread's `Random`.
    pub fn get_random() -> Arc<dyn Random> {
        RandomizedTest::get_context().get_random()
    }

    /// `randomBoolean()`.
    pub fn random_boolean() -> bool {
        RandomizedTest::get_random().next_boolean()
    }

    /// `randomByte()`: `(byte) nextInt()`.
    pub fn random_byte() -> i8 {
        RandomizedTest::get_random().next_int() as i8
    }

    /// `randomShort()`: `(short) nextInt()`.
    pub fn random_short() -> i16 {
        RandomizedTest::get_random().next_int() as i16
    }

    /// `randomInt()`.
    pub fn random_int() -> i32 {
        RandomizedTest::get_random().next_int()
    }

    /// `randomFloat()`.
    pub fn random_float() -> f32 {
        RandomizedTest::get_random().next_float()
    }

    /// `randomDouble()`.
    pub fn random_double() -> f64 {
        RandomizedTest::get_random().next_double()
    }

    /// `randomLong()`.
    pub fn random_long() -> i64 {
        RandomizedTest::get_random().next_long()
    }

    /// `randomGaussian()`: `Random.nextGaussian()`.
    pub fn random_gaussian() -> f64 {
        RandomizedTest::get_random().next_gaussian()
    }

    /// `biasedFloatBetween(min, max)`: `BiasedNumbers.randomFloatBetween`.
    pub fn biased_float_between(min: f32, max: f32) -> f32 {
        BiasedNumbers::random_float_between(&RandomizedTest::get_random(), min, max)
    }

    /// `biasedDoubleBetween(min, max)`: `BiasedNumbers.randomDoubleBetween`.
    pub fn biased_double_between(min: f64, max: f64) -> f64 {
        BiasedNumbers::random_double_between(&RandomizedTest::get_random(), min, max)
    }

    /// `randomBytesOfLength(int length)`: from a `java.util.Random` seeded
    /// by the context's `nextLong()`.
    pub fn random_bytes_of_length(length: i32) -> Vec<u8> {
        let r = JdkRandom::new(RandomizedTest::get_random().next_long());
        RandomBytes::random_bytes_of_length(&r, length)
    }

    /// `randomBytesOfLength(int minLength, int maxLength)`.
    pub fn random_bytes_of_length_between(min_length: i32, max_length: i32) -> Vec<u8> {
        let r = JdkRandom::new(RandomizedTest::get_random().next_long());
        RandomBytes::random_bytes_of_length_between(&r, min_length, max_length)
    }

    /// `randomInt(int max)` (deprecated): `[0, max]`.
    #[deprecated(note = "randomizedtesting deprecates randomInt(int)")]
    pub fn random_int_max(max: i32) -> i32 {
        RandomNumbers::random_int_between(&RandomizedTest::get_random(), 0, max)
    }

    /// `randomLong(long max)` (deprecated): `[0, max]`.
    #[deprecated(note = "randomizedtesting deprecates randomLong(long)")]
    pub fn random_long_max(max: i64) -> i64 {
        RandomNumbers::random_long_between(&RandomizedTest::get_random(), 0, max)
    }

    /// `randomIntBetween(min, max)`: `[min, max]`.
    pub fn random_int_between(min: i32, max: i32) -> i32 {
        RandomNumbers::random_int_between(&RandomizedTest::get_random(), min, max)
    }

    /// `between(int, int)`: `randomIntBetween`.
    pub fn between(min: i32, max: i32) -> i32 {
        RandomizedTest::random_int_between(min, max)
    }

    /// `randomLongBetween(min, max)`: `[min, max]`.
    pub fn random_long_between(min: i64, max: i64) -> i64 {
        RandomNumbers::random_long_between(&RandomizedTest::get_random(), min, max)
    }

    /// `between(long, long)`: `randomLongBetween`.
    pub fn between_long(min: i64, max: i64) -> i64 {
        RandomizedTest::random_long_between(min, max)
    }

    /// `atLeast(int min)`: a value `>= min`, scaled by `isNightly()` and the
    /// multiplier (`scaledRandomIntBetween(min, Integer.MAX_VALUE)`).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `min < 0`.
    pub fn at_least(min: i32) -> i32 {
        if min < 0 {
            illegal_argument(format!("atLeast requires non-negative argument: {min}"));
        }
        RandomizedTest::scaled_random_int_between(min, i32::MAX)
    }

    /// `atMost(int max)`: `scaledRandomIntBetween(0, max)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `max < 0`.
    pub fn at_most(max: i32) -> i32 {
        if max < 0 {
            illegal_argument(format!("atMost requires non-negative argument: {max}"));
        }
        RandomizedTest::scaled_random_int_between(0, max)
    }

    /// `rarely()`: `true` in about 10% of calls (`randomInt(100) >= 90`).
    pub fn rarely() -> bool {
        #[allow(deprecated)]
        let v = RandomizedTest::random_int_max(100);
        v >= 90
    }

    /// `frequently()`: `!rarely()`.
    pub fn frequently() -> bool {
        !RandomizedTest::rarely()
    }

    /// `randomFrom(T[] array)` and the primitive-array overloads.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for an empty array.
    pub fn random_from<T>(array: &[T]) -> &T {
        RandomPicks::random_from(&RandomizedTest::get_random(), array)
    }

    /// `randomFrom(List<T> list)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for an empty list.
    pub fn random_from_list<T>(list: &[T]) -> &T {
        RandomPicks::random_from_list(&RandomizedTest::get_random(), list)
    }

    /// `multiplier()`: the `randomized.multiplier` property, `1.0` by
    /// default.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` outside a context; `IllegalArgumentException`
    /// for a malformed property.
    pub fn multiplier() -> f64 {
        RandomizedTest::check_context();
        RandomizedTest::system_property_as_double(
            RandomizedTest::SYSPROP_MULTIPLIER,
            DEFAULT_MULTIPLIER,
        )
    }

    /// `iterations(min, max)`: `scaledRandomIntBetween(min, max)`.
    pub fn iterations(min: i32, max: i32) -> i32 {
        RandomizedTest::scaled_random_int_between(min, max)
    }

    /// `scaledRandomIntBetween(min, max)`: a value in `[min, max]`, closer to
    /// `min` on normal runs and to `max` on nightly runs, moved by the
    /// multiplier.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `min < 0` or `min > max`.
    pub fn scaled_random_int_between(min: i32, max: i32) -> i32 {
        if min < 0 {
            illegal_argument(format!("min must be >= 0: {min}"));
        }
        if min > max {
            illegal_argument(format!("max must be >= min: {min}, {max}"));
        }
        let point = math::min(1.0, RandomizedTest::random_gaussian().abs() * 0.3)
            * RandomizedTest::multiplier();
        let range = f64::from(max) - f64::from(min);
        // `(int) Math.round(...)`: the long's low 32 bits.
        let scaled = math::round(math::min(point * range, range)) as i32;
        if RandomizedTest::is_nightly() {
            max.wrapping_sub(scaled)
        } else {
            min.wrapping_add(scaled)
        }
    }

    /// `globalTempDir()`: a temporary directory under `java.io.tmpdir`,
    /// created once and removed at shutdown.
    ///
    /// **Divergence.** Java names it `tests-yyyyMMddHHmmss-SSS` in
    /// the JVM's default time zone; the port formats UTC (the standard
    /// library has no time zone database). The removal runs from
    /// [`runtime::run_shutdown_hooks`].
    ///
    /// # Errors
    ///
    /// `IOException`s with Java's messages when the temporary folder is not
    /// accessible or no unique name could be created.
    pub fn global_temp_dir() -> io::Result<PathBuf> {
        RandomizedTest::check_context();
        let mut guard = GLOBAL_TEMP_DIR.lock().unwrap_or_else(|e| e.into_inner());
        RandomizedTest::global_temp_dir_locked(&mut guard)
    }

    fn global_temp_dir_locked(slot: &mut Option<PathBuf>) -> io::Result<PathBuf> {
        if let Some(dir) = slot.as_ref() {
            return Ok(dir.clone());
        }
        let Some(temp_dir_path) = system::get_property("java.io.tmpdir") else {
            return Err(io::Error::other("No property java.io.tmpdir?"));
        };
        let temp_dir = PathBuf::from(temp_dir_path);
        let writable = std::fs::metadata(&temp_dir)
            .map(|m| m.is_dir() && !m.permissions().readonly())
            .unwrap_or(false);
        if !writable {
            return Err(io::Error::other(format!(
                "Temporary folder not accessible: {}",
                std::path::absolute(&temp_dir)
                    .unwrap_or(temp_dir.clone())
                    .display()
            )));
        }
        let dir = RandomizedTest::create_temp(&temp_dir, true, timestamp_name)?;
        *slot = Some(dir.clone());
        let hook_dir = dir.clone();
        runtime::add_shutdown_hook(move || {
            if let Err(e) = RandomizedTest::rm_dir(&hook_dir) {
                eprintln!(
                    "Could not delete global temporary folder: {}. Cause: ",
                    std::path::absolute(&hook_dir)
                        .unwrap_or(hook_dir.clone())
                        .display()
                );
                eprintln!("{e}");
            }
        });
        Ok(dir)
    }

    /// `createTemp(parent, dir, nameSupplier)`: up to eleven attempts, a
    /// random pause of up to 250 ms between them.
    fn create_temp(parent: &Path, dir: bool, name: impl Fn() -> String) -> io::Result<PathBuf> {
        let mut retries = 10;
        loop {
            let candidate = parent.join(name());
            let created = if dir {
                std::fs::create_dir(&candidate)
            } else {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                    .map(|_| ())
            };
            match created {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    retries -= 1;
                    if retries < 0 {
                        return Err(io::Error::other(format!(
                            "Could not create a unique temporary folder under: {}",
                            parent.display()
                        )));
                    }
                    let pause = JdkRandom::new_unseeded().next_int_bound(250);
                    std::thread::sleep(Duration::from_millis(pause as u64));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// `newTempDir()`: a temporary directory removed after the test
    /// (`LifecycleScope.TEST`).
    ///
    /// # Errors
    ///
    /// As [`global_temp_dir`](Self::global_temp_dir), or when creating the
    /// directory fails.
    pub fn new_temp_dir() -> io::Result<PathBuf> {
        RandomizedTest::new_temp_dir_in(LifecycleScope::Test)
    }

    /// `newTempDir(LifecycleScope)`: a temporary directory under the global
    /// one, removed after `scope`.
    ///
    /// # Errors
    ///
    /// As [`new_temp_dir`](Self::new_temp_dir).
    pub fn new_temp_dir_in(scope: LifecycleScope) -> io::Result<PathBuf> {
        RandomizedTest::new_temp_path(scope, true)
    }

    /// `newTempFile()`: a temporary file removed after the test.
    ///
    /// # Errors
    ///
    /// As [`new_temp_dir`](Self::new_temp_dir).
    pub fn new_temp_file() -> io::Result<PathBuf> {
        RandomizedTest::new_temp_file_in(LifecycleScope::Test)
    }

    /// `newTempFile(LifecycleScope)`: a temporary file, created on disk but
    /// not opened, removed after `scope`.
    ///
    /// # Errors
    ///
    /// As [`new_temp_dir`](Self::new_temp_dir).
    pub fn new_temp_file_in(scope: LifecycleScope) -> io::Result<PathBuf> {
        RandomizedTest::new_temp_path(scope, false)
    }

    fn new_temp_path(scope: LifecycleScope, dir: bool) -> io::Result<PathBuf> {
        RandomizedTest::check_context();
        let mut guard = GLOBAL_TEMP_DIR.lock().unwrap_or_else(|e| e.into_inner());
        let global = RandomizedTest::global_temp_dir_locked(&mut guard)?;
        let path = RandomizedTest::create_temp(&global, dir, RandomizedTest::next_temp_name)?;
        RandomizedTest::get_context().close_at_end(Arc::new(TempPathResource::new(&path)), scope);
        Ok(path)
    }

    /// `closeAfterTest(resource)`: closes `resource` after the test.
    pub fn close_after_test<T: Closeable + 'static>(resource: Arc<T>) -> Arc<T> {
        RandomizedTest::get_context().close_at_end(resource, LifecycleScope::Test)
    }

    /// `closeAfterSuite(resource)`: closes `resource` after the suite.
    pub fn close_after_suite<T: Closeable + 'static>(resource: Arc<T>) -> Arc<T> {
        RandomizedTest::get_context().close_at_end(resource, LifecycleScope::Suite)
    }

    /// `nextTempName()` (protected): `%04d has-space` of a process-wide
    /// counter.
    pub fn next_temp_name() -> String {
        format!(
            "{:04} has-space",
            TEMP_SUB_FILE_NAME_COUNT.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// `rmDir(Path)`: removes a folder recursively, files before their
    /// directory; symbolic links are removed, not followed.
    ///
    /// # Errors
    ///
    /// `Not a folder: <path>` when `path` is not a directory;
    /// `Could not remove directory: <path>` wrapping the first failure.
    pub fn rm_dir(path: &Path) -> io::Result<()> {
        if !path.is_dir() {
            return Err(io::Error::other(format!(
                "Not a folder: {}",
                path.display()
            )));
        }
        fn walk(p: &Path) -> io::Result<()> {
            let meta = std::fs::symlink_metadata(p)?;
            if meta.is_dir() {
                for entry in std::fs::read_dir(p)? {
                    walk(&entry?.path())?;
                }
                std::fs::remove_dir(p)
            } else {
                std::fs::remove_file(p)
            }
        }
        walk(path).map_err(|e| {
            io::Error::new(
                e.kind(),
                CausedIoError {
                    message: format!("Could not remove directory: {}", path.display()),
                    cause: e,
                },
            )
        })
    }

    /// `newServerSocket(LifecycleScope)`: a server socket on an ephemeral
    /// port of the wildcard address, closed after `scope` unless closed
    /// earlier.
    ///
    /// # Errors
    ///
    /// The bind failure.
    pub fn new_server_socket(scope: LifecycleScope) -> io::Result<Arc<ServerSocket>> {
        let socket = Arc::new(ServerSocket::bind_any()?);
        RandomizedTest::get_context().close_at_end(Arc::clone(&socket), scope);
        Ok(socket)
    }

    /// `randomLocale()`: one of the JVM's available locales, sorted by
    /// `toString()` (see [`crate::jdk::locale`]).
    pub fn random_locale() -> Locale {
        *RandomizedTest::random_from(locale::available_locales_sorted())
    }

    /// `randomTimeZone()`: one of the JVM's available time zone IDs, sorted
    /// (see [`crate::jdk::locale`]).
    pub fn random_time_zone() -> TimeZone {
        *RandomizedTest::random_from(locale::available_time_zones_sorted())
    }

    /// `randomAsciiOfLengthBetween` (deprecated).
    #[deprecated(note = "use random_ascii_letters_of_length_between")]
    pub fn random_ascii_of_length_between(min_code_units: i32, max_code_units: i32) -> String {
        RandomizedTest::random_ascii_letters_of_length_between(min_code_units, max_code_units)
    }

    /// `randomAsciiOfLength` (deprecated).
    #[deprecated(note = "use random_ascii_letters_of_length")]
    pub fn random_ascii_of_length(code_units: i32) -> String {
        RandomizedTest::random_ascii_letters_of_length(code_units)
    }

    /// `randomAsciiLettersOfLengthBetween`.
    pub fn random_ascii_letters_of_length_between(min_letters: i32, max_letters: i32) -> String {
        RandomStrings::random_ascii_letters_of_length_between(
            &RandomizedTest::get_random(),
            min_letters,
            max_letters,
        )
    }

    /// `randomAsciiLettersOfLength`.
    pub fn random_ascii_letters_of_length(code_units: i32) -> String {
        RandomStrings::random_ascii_letters_of_length(&RandomizedTest::get_random(), code_units)
    }

    /// `randomAsciiAlphanumOfLengthBetween`.
    pub fn random_ascii_alphanum_of_length_between(
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        RandomStrings::random_ascii_alphanum_of_length_between(
            &RandomizedTest::get_random(),
            min_code_units,
            max_code_units,
        )
    }

    /// `randomAsciiAlphanumOfLength`.
    pub fn random_ascii_alphanum_of_length(code_units: i32) -> String {
        RandomStrings::random_ascii_alphanum_of_length(&RandomizedTest::get_random(), code_units)
    }

    /// `randomUnicodeOfLengthBetween`.
    pub fn random_unicode_of_length_between(min_code_units: i32, max_code_units: i32) -> String {
        RandomStrings::random_unicode_of_length_between(
            &RandomizedTest::get_random(),
            min_code_units,
            max_code_units,
        )
    }

    /// `randomUnicodeOfLength`.
    pub fn random_unicode_of_length(code_units: i32) -> String {
        RandomStrings::random_unicode_of_length(&RandomizedTest::get_random(), code_units)
    }

    /// `randomUnicodeOfCodepointLengthBetween`.
    pub fn random_unicode_of_codepoint_length_between(
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        RandomStrings::random_unicode_of_codepoint_length_between(
            &RandomizedTest::get_random(),
            min_code_points,
            max_code_points,
        )
    }

    /// `randomUnicodeOfCodepointLength`.
    pub fn random_unicode_of_codepoint_length(code_points: i32) -> String {
        RandomStrings::random_unicode_of_codepoint_length(
            &RandomizedTest::get_random(),
            code_points,
        )
    }

    /// `randomRealisticUnicodeOfLengthBetween`.
    pub fn random_realistic_unicode_of_length_between(
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        RandomStrings::random_realistic_unicode_of_length_between(
            &RandomizedTest::get_random(),
            min_code_units,
            max_code_units,
        )
    }

    /// `randomRealisticUnicodeOfLength`.
    pub fn random_realistic_unicode_of_length(code_units: i32) -> String {
        RandomStrings::random_realistic_unicode_of_length(&RandomizedTest::get_random(), code_units)
    }

    /// `randomRealisticUnicodeOfCodepointLengthBetween`.
    pub fn random_realistic_unicode_of_codepoint_length_between(
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        RandomStrings::random_realistic_unicode_of_codepoint_length_between(
            &RandomizedTest::get_random(),
            min_code_points,
            max_code_points,
        )
    }

    /// `randomRealisticUnicodeOfCodepointLength`.
    pub fn random_realistic_unicode_of_codepoint_length(code_points: i32) -> String {
        RandomStrings::random_realistic_unicode_of_codepoint_length(
            &RandomizedTest::get_random(),
            code_points,
        )
    }

    /// `sleep(long millis)`: `Thread.sleep`, an `InterruptedException`
    /// rethrown unchecked (`Rethrow.rethrow`).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`timeout value is negative`) when
    /// `millis < 0`, as `Thread.sleep` throws; `InterruptedException`
    /// (`sleep interrupted`) when the thread is (or becomes) interrupted.
    pub fn sleep(millis: i64) {
        if let Err(e) = Thread::try_sleep(millis) {
            e.throw()
        }
    }

    /// `assumeTrue(boolean)`: `Assume.assumeTrue`.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`got: <false>, expected: is <true>`)
    /// when `condition` is false.
    pub fn assume_true(condition: bool) {
        if !condition {
            assumption_violated("got: <false>, expected: is <true>");
        }
    }

    /// `assumeFalse(boolean)`: `assumeTrue(!condition)`.
    ///
    /// # Panics
    ///
    /// As [`assume_true`](Self::assume_true).
    pub fn assume_false(condition: bool) {
        RandomizedTest::assume_true(!condition)
    }

    /// `assumeNotNull(Object...)`: `Assume.assumeNotNull`; each object is
    /// its `toString()`, `None` for `null`. `objects` itself is `None` for a
    /// `null` array.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` when the array or an item is null, with
    /// Hamcrest's description as JUnit 4.13.1 builds it.
    pub fn assume_not_null(objects: Option<&[Option<&dyn Display>]>) {
        let Some(objects) = objects else {
            assumption_violated("got: null, expected: not null")
        };
        if objects.iter().any(Option::is_none) {
            let items: Vec<String> = objects
                .iter()
                .map(|o| o.map_or_else(|| "null".to_string(), |v| v.to_string()))
                .collect();
            assumption_violated(&format!(
                "got: <[{}]>, expected: every item is not null",
                items.join(", ")
            ));
        }
    }

    /// `assumeTrue(String message, boolean condition)`.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`message`) when `condition` is false.
    pub fn assume_true_msg(message: &str, condition: bool) {
        if !condition {
            assumption_violated(message);
        }
    }

    /// `assumeFalse(String message, boolean condition)`.
    ///
    /// # Panics
    ///
    /// As [`assume_true_msg`](Self::assume_true_msg).
    pub fn assume_false_msg(message: &str, condition: bool) {
        RandomizedTest::assume_true_msg(message, !condition)
    }

    /// `assumeNoException(String msg, Throwable t)`: fails the assumption
    /// with `msg` when `t` is present, chaining `t` as the cause (`new
    /// AssumptionViolatedException(msg, t)`).
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`msg`, caused by `t`).
    pub fn assume_no_exception_msg(msg: &str, t: Option<&Throwable>) {
        if let Some(t) = t {
            Throwable::with_cause(
                ASSUMPTION_VIOLATED_EXCEPTION,
                Some(msg.to_string()),
                t.clone(),
            )
            .throw()
        }
    }

    /// `assumeNoException(Throwable t)`: `Assume.assumeNoException`, whose
    /// `AssumptionViolatedException(t, nullValue())` takes `t` as its cause.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`got: <t>, expected: null`, caused by
    /// `t`).
    pub fn assume_no_exception(t: Option<&Throwable>) {
        if let Some(t) = t {
            Throwable::with_cause(
                ASSUMPTION_VIOLATED_EXCEPTION,
                Some(format!("got: <{t}>, expected: null")),
                t.clone(),
            )
            .throw()
        }
    }

    /// The property's trimmed value, `None` when unset or blank.
    fn property(property_name: &str) -> Option<(String, String)> {
        let v = system::get_property(property_name)?;
        let trimmed = lang::trim(&v).to_string();
        (!trimmed.is_empty()).then_some((v, trimmed))
    }

    /// `systemPropertyAsDouble(name, default)`: `Double.parseDouble` of the
    /// trimmed property, or `default` when unset or blank.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Double value expected for property
    /// <name>: <value>`).
    pub fn system_property_as_double(property_name: &str, default_value: f64) -> f64 {
        match RandomizedTest::property(property_name) {
            None => default_value,
            Some((v, t)) => lang::parse_double(&t).unwrap_or_else(|| {
                illegal_argument(format!(
                    "Double value expected for property {property_name}: {v}"
                ))
            }),
        }
    }

    /// `systemPropertyAsFloat(name, default)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Float value expected for property
    /// <name>: <value>`).
    pub fn system_property_as_float(property_name: &str, default_value: f32) -> f32 {
        match RandomizedTest::property(property_name) {
            None => default_value,
            Some((v, t)) => lang::parse_float(&t).unwrap_or_else(|| {
                illegal_argument(format!(
                    "Float value expected for property {property_name}: {v}"
                ))
            }),
        }
    }

    /// `systemPropertyAsInt(name, default)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Integer value expected for property
    /// <name>: <value>`).
    pub fn system_property_as_int(property_name: &str, default_value: i32) -> i32 {
        match RandomizedTest::property(property_name) {
            None => default_value,
            Some((v, t)) => lang::parse_int(&t).unwrap_or_else(|| {
                illegal_argument(format!(
                    "Integer value expected for property {property_name}: {v}"
                ))
            }),
        }
    }

    /// `systemPropertyAsLong(String, int)`: `Long.parseLong` of the property,
    /// returned as a `float` with an `int` default, exactly as
    /// randomizedtesting 2.8.1 declares it.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Long value expected for property
    /// <name>: <value>`).
    pub fn system_property_as_long(property_name: &str, default_value: i32) -> f32 {
        match RandomizedTest::property(property_name) {
            None => default_value as f32,
            Some((v, t)) => match lang::parse_long(&t) {
                // Java's long-to-float widening rounds to nearest, as `as`.
                Some(l) => l as f32,
                None => illegal_argument(format!(
                    "Long value expected for property {property_name}: {v}"
                )),
            },
        }
    }

    /// `systemPropertyAsBoolean(name, default)`: `true` for `true`, `on`,
    /// `yes`, `enabled` and `false` for `false`, `off`, `no`, `disabled`.
    /// Java's lookup is case-sensitive, although its javadoc says otherwise.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Boolean value expected for property
    /// <name> (true/false, on/off, enabled/disabled, yes/no): <value>`).
    pub fn system_property_as_boolean(property_name: &str, default_value: bool) -> bool {
        match RandomizedTest::try_system_property_as_boolean(property_name, default_value) {
            Ok(b) => b,
            Err(message) => illegal_argument(message),
        }
    }

    /// [`system_property_as_boolean`](Self::system_property_as_boolean),
    /// with the `IllegalArgumentException` message as `Err`.
    pub(crate) fn try_system_property_as_boolean(
        property_name: &str,
        default_value: bool,
    ) -> Result<bool, String> {
        match RandomizedTest::property(property_name) {
            None => Ok(default_value),
            Some((_, t)) => match t.as_str() {
                "true" | "on" | "yes" | "enabled" => Ok(true),
                "false" | "off" | "no" | "disabled" => Ok(false),
                _ => Err(format!(
                    "Boolean value expected for property {property_name} (true/false, on/off, \
                     enabled/disabled, yes/no): {t}"
                )),
            },
        }
    }

    /// `checkContext()`: requires a context.
    fn check_context() {
        RandomizedContext::current();
    }
}

/// `new IOException(message, cause)`: Java's message, the cause as the
/// error's source.
#[derive(Debug)]
struct CausedIoError {
    message: String,
    cause: io::Error,
}

impl Display for CausedIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CausedIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// `throw new AssumptionViolatedException(message)`.
fn assumption_violated(message: &str) -> ! {
    exception::throw(ASSUMPTION_VIOLATED_EXCEPTION, message)
}

/// `tests-yyyyMMddHHmmss-SSS` of the current UTC time.
fn timestamp_name() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let millis = now.subsec_millis();
    let secs = now.as_secs() as i64;
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "tests-{y:04}{m:02}{d:02}{:02}{:02}{:02}-{millis:03}",
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    )
}

/// The proleptic Gregorian date of a count of days since 1970-01-01
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A `java.net.ServerSocket` as `RandomizedTest.newServerSocket` hands it
/// out: closable by its owner and by the context, whichever is first.
pub struct ServerSocket {
    listener: Mutex<Option<TcpListener>>,
    local: SocketAddr,
}

impl ServerSocket {
    /// `new ServerSocket(0)`: the wildcard address (IPv6 when available,
    /// else IPv4), an ephemeral port.
    fn bind_any() -> io::Result<ServerSocket> {
        let listener = TcpListener::bind("[::]:0").or_else(|_| TcpListener::bind("0.0.0.0:0"))?;
        let local = listener.local_addr()?;
        Ok(ServerSocket {
            listener: Mutex::new(Some(listener)),
            local,
        })
    }

    /// `getLocalPort()`.
    pub fn local_port(&self) -> u16 {
        self.local.port()
    }

    /// `isClosed()`.
    pub fn is_closed(&self) -> bool {
        self.listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
    }

    /// The open listener, for `accept` and friends; `None` once closed.
    pub fn with_listener<T>(&self, f: impl FnOnce(&TcpListener) -> T) -> Option<T> {
        self.listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(f)
    }
}

impl Closeable for ServerSocket {
    /// `ServerSocket.close()`; closing a closed socket does nothing, as the
    /// closeable `newServerSocket` registers checks `isClosed()` first.
    fn close(&self) -> io::Result<()> {
        self.listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        Ok(())
    }
}
