/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.util.TestRuleTemporaryFilesCleanup`, in
//! full under the sprint's cycle rule: `LuceneTestCase.createTempDir()`,
//! `createTempDir(String)` and `createTempFile` are served by it (task #348
//! installs it in `LuceneTestCase`'s class rules).

use std::io;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::system;
use randomizedtesting::rules::TestRuleAdapter;
use randomizedtesting::RandomizedContext;
use rucene::store::java_nio_file::{
    default_file_system, java_class_of, java_message_of, same_file_system, FileSystem,
    FileSystemProvider, Files, Path,
};
use rucene::util::IOUtils;

use crate::mockfile::{
    DisableFsyncFS, ExtrasFS, HandleLimitFS, LeakFS, MaxOpenHandles, ShuffleFS, VerboseFS,
    WindowsFS,
};
use crate::util::lucene_test_case::{
    SuppressFileSystems, SuppressTempFileChecks, LEAVE_TEMPORARY, SUPPRESS_FSYNC, VERBOSE,
};
use crate::util::test_rule_mark_failure::TestRuleMarkFailure;
use crate::util::test_rule_setup_and_restore_class_env::ThreadNameFixingPrintStreamInfoStream;

/// `TEMP_NAME_RETRY_THRESHOLD`: retry to create a temporary file name this
/// many times.
const TEMP_NAME_RETRY_THRESHOLD: i32 = 9999;

/// `cleanupQueue`: the temporary resources to remove after the suite
/// completes (static, as in Java).
static CLEANUP_QUEUE: Mutex<Vec<Path>> = Mutex::new(Vec::new());

#[derive(Default)]
struct State {
    /// `javaTempDir`: the writeable temporary base folder.
    java_temp_dir: Option<Path>,
    /// `tempDirBase`: the per-test-class temporary folder.
    temp_dir_base: Option<Path>,
    /// `fileSystem`: the per-test filesystem.
    file_system: Option<Arc<dyn FileSystem>>,
}

/// Port of the package-private `org.apache.lucene.tests.util.TestRuleTemporaryFilesCleanup`:
/// checks and cleans up temporary files.
pub struct TestRuleTemporaryFilesCleanup {
    state: Mutex<State>,
    /// `failureMarker`: the suite failure marker.
    failure_marker: Arc<TestRuleMarkFailure>,
}

/// An `IOException` thrown out of a rule hook.
fn throw_io(error: &io::Error) -> ! {
    Throwable::new(
        java_class_of(error).to_string(),
        java_message_of(error).unwrap_or_default(),
    )
    .throw()
}

/// `String.format(Locale.ENGLISH, "%03d", attempt)`.
fn three_digits(attempt: i32) -> String {
    format!("{attempt:03}")
}

impl TestRuleTemporaryFilesCleanup {
    /// `new TestRuleTemporaryFilesCleanup(failureMarker)`.
    pub fn new(failure_marker: Arc<TestRuleMarkFailure>) -> TestRuleTemporaryFilesCleanup {
        TestRuleTemporaryFilesCleanup {
            state: Mutex::new(State::default()),
            failure_marker,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `registerToRemoveAfterSuite(f)`: queues `f` for removal after the
    /// suite, unless `LEAVE_TEMPORARY`.
    pub fn register_to_remove_after_suite(&self, f: &Path) {
        if *LEAVE_TEMPORARY {
            system::err_println(&format!(
                "INFO: Will leave temporary file: {}",
                f.to_absolute_path()
            ));
            return;
        }
        CLEANUP_QUEUE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(f.clone());
    }

    /// `allowed(avoid, clazz)`: not suppressed by `*` or by its simple name.
    fn allowed(avoid: &[String], simple_name: &str) -> bool {
        !(avoid.iter().any(|a| a == "*") || avoid.iter().any(|a| a == simple_name))
    }

    /// `initializeFileSystem()`: the default filesystem wrapped, by the suite
    /// randomness, in the mock filesystems the suite does not suppress.
    fn initialize_file_system() -> Arc<dyn FileSystem> {
        let ctx = RandomizedContext::current();
        let target_class = ctx.get_target_class_object();
        let present = |ty| {
            target_class
                .as_ref()
                .is_some_and(|c| c.is_annotation_present(ty))
        };
        let mut avoid: Vec<String> = Vec::new();
        if let Some(a) = target_class
            .as_ref()
            .and_then(|c| c.get_annotation::<SuppressFileSystems>())
        {
            avoid.extend(a.value.iter().cloned());
        }
        let mut fs = default_file_system();
        if *VERBOSE && Self::allowed(&avoid, "VerboseFS") {
            let info_stream = Arc::new(ThreadNameFixingPrintStreamInfoStream::new_system_out());
            fs = FileSystemProvider::get_file_system(&*VerboseFS::new(fs, info_stream));
        }

        let random = ctx.get_random();

        // speed up tests by omitting actual fsync calls to the hardware most of the time.
        // (The random draw happens before `allowed`, as in Java's nested ifs.)
        if (present(&SUPPRESS_FSYNC) || random.next_int_bound(100) > 0)
            && Self::allowed(&avoid, "DisableFsyncFS")
        {
            fs = FileSystemProvider::get_file_system(&*DisableFsyncFS::new(fs));
        }

        // impacts test reproducibility across platforms.
        if random.next_int_bound(100) > 0 && Self::allowed(&avoid, "ShuffleFS") {
            fs = FileSystemProvider::get_file_system(&*ShuffleFS::new(fs, random.next_long()));
        }

        // otherwise, wrap with mockfilesystems for additional checks. some
        // of these have side effects (e.g. concurrency) so it doesn't always happen.
        if random.next_int_bound(10) > 0 {
            if Self::allowed(&avoid, "LeakFS") {
                fs = FileSystemProvider::get_file_system(&*LeakFS::new(fs));
            }
            if Self::allowed(&avoid, "HandleLimitFS") {
                let limit = target_class
                    .as_ref()
                    .and_then(|c| c.get_annotation::<MaxOpenHandles>())
                    .map_or(MaxOpenHandles::MAX_OPEN_FILES, |a| a.limit);
                fs = FileSystemProvider::get_file_system(&*HandleLimitFS::new(fs, limit));
            }
            // windows is currently slow
            if random.next_int_bound(10) == 0 && Self::allowed(&avoid, "WindowsFS") {
                fs = FileSystemProvider::get_file_system(&*WindowsFS::new(fs));
            }
            if Self::allowed(&avoid, "ExtrasFS") {
                let active = random.next_int_bound(4) == 0;
                let create_directory = random.next_boolean();
                fs = FileSystemProvider::get_file_system(&*ExtrasFS::new(
                    fs,
                    active,
                    create_directory,
                ));
            }
        }
        if *VERBOSE {
            system::out_println(&format!(
                "filesystem: {}",
                fs.provider().provider_to_string()
            ));
        }
        // `fs.provider().getFileSystem(URI.create("file:///"))`.
        fs.provider().get_file_system()
    }

    /// `initializeJavaTempDir()`: `tempDir` (or `java.io.tmpdir`) as a path of
    /// the suite filesystem, created, and resolved to its real path.
    fn initialize_java_temp_dir(file_system: &Arc<dyn FileSystem>) -> io::Result<Path> {
        let dir = system::get_property("tempDir")
            .or_else(|| system::get_property("java.io.tmpdir"))
            .unwrap_or_default();
        let java_temp_dir = Arc::clone(file_system).get_path(&dir, &[]);
        Files::create_directories(&java_temp_dir)?;
        randomizedtesting::jdk::exception::java_assert(
            Files::is_directory(&java_temp_dir, &[]) && Files::is_writable(&java_temp_dir),
            String::new,
        );
        java_temp_dir.to_real_path(&[])
    }

    /// `getPerTestClassTempDir()`: the suite's temporary folder,
    /// `<javaTempDir>/<class name, "org.apache.lucene." as "lucene.">_<seed>-NNN`,
    /// created on first use and queued for removal.
    ///
    /// # Panics
    ///
    /// `RuntimeException` after 9999 failed attempts, and
    /// `NullPointerException` outside the rule.
    pub fn get_per_test_class_temp_dir(&self) -> Path {
        if let Some(base) = self.lock().temp_dir_base.clone() {
            return base;
        }
        let ctx = RandomizedContext::current();
        let class_name = ctx.get_target_class().to_string();
        let prefix = match class_name.strip_prefix("org.apache.lucene.") {
            Some(rest) => format!("lucene.{rest}"),
            None => class_name,
        };
        let Some(java_temp_dir) = self.lock().java_temp_dir.clone() else {
            randomizedtesting::jdk::exception::throw_without_message(
                randomizedtesting::jdk::exception::NULL_POINTER_EXCEPTION,
            )
        };
        let mut attempt = 0;
        let f = loop {
            if attempt >= TEMP_NAME_RETRY_THRESHOLD {
                randomizedtesting::jdk::exception::throw(
                    randomizedtesting::jdk::exception::RUNTIME_EXCEPTION,
                    format!(
                        "Failed to get a temporary name too many times, check your temp directory and consider manually cleaning it: {}",
                        java_temp_dir.to_absolute_path()
                    ),
                )
            }
            attempt += 1;
            let f = java_temp_dir.resolve_str(&format!(
                "{prefix}_{}-{}",
                ctx.get_runner_seed_as_string(),
                three_digits(attempt)
            ));
            if Files::create_directory(&f).is_ok() {
                break f;
            }
        };
        self.lock().temp_dir_base = Some(f.clone());
        self.register_to_remove_after_suite(&f);
        f
    }

    /// `createTempDir(prefix)`: a new directory `<prefix>-NNN` under the
    /// suite's temporary folder, queued for removal.
    ///
    /// # Panics
    ///
    /// `RuntimeException` after 9999 failed attempts.
    pub fn create_temp_dir(&self, prefix: &str) -> Path {
        let base = self.get_per_test_class_temp_dir();
        let mut attempt = 0;
        let f = loop {
            if attempt >= TEMP_NAME_RETRY_THRESHOLD {
                randomizedtesting::jdk::exception::throw(
                    randomizedtesting::jdk::exception::RUNTIME_EXCEPTION,
                    format!(
                        "Failed to get a temporary name too many times, check your temp directory and consider manually cleaning it: {}",
                        base.to_absolute_path()
                    ),
                )
            }
            attempt += 1;
            let f = base.resolve_str(&format!("{prefix}-{}", three_digits(attempt)));
            if Files::create_directory(&f).is_ok() {
                break f;
            }
        };
        self.register_to_remove_after_suite(&f);
        f
    }

    /// `createTempFile(prefix, suffix)`: a new file `<prefix>-NNN<suffix>`
    /// under the suite's temporary folder, queued for removal.
    ///
    /// # Errors
    ///
    /// Declared `throws IOException`, as Java; the creation failures are
    /// retried.
    ///
    /// # Panics
    ///
    /// `RuntimeException` after 9999 failed attempts.
    pub fn create_temp_file(&self, prefix: &str, suffix: &str) -> io::Result<Path> {
        let base = self.get_per_test_class_temp_dir();
        let mut attempt = 0;
        let f = loop {
            if attempt >= TEMP_NAME_RETRY_THRESHOLD {
                randomizedtesting::jdk::exception::throw(
                    randomizedtesting::jdk::exception::RUNTIME_EXCEPTION,
                    format!(
                        "Failed to get a temporary name too many times, check your temp directory and consider manually cleaning it: {}",
                        base.to_absolute_path()
                    ),
                )
            }
            attempt += 1;
            let f = base.resolve_str(&format!("{prefix}-{}{suffix}", three_digits(attempt)));
            if Files::create_file(&f).is_ok() {
                break f;
            }
        };
        self.register_to_remove_after_suite(&f);
        Ok(f)
    }

    /// The suite filesystem `before()` installed, `None` outside the rule.
    pub fn file_system(&self) -> Option<Arc<dyn FileSystem>> {
        self.lock().file_system.clone()
    }
}

impl TestRuleAdapter for TestRuleTemporaryFilesCleanup {
    /// `before()`: the suite filesystem, then the Java temporary folder on it.
    fn before(&self) {
        randomizedtesting::jdk::exception::java_assert_no_message(
            self.lock().temp_dir_base.is_none(),
        );
        let file_system = Self::initialize_file_system();
        self.lock().file_system = Some(Arc::clone(&file_system));
        match Self::initialize_java_temp_dir(&file_system) {
            Ok(dir) => self.lock().java_temp_dir = Some(dir),
            Err(error) => throw_io(&error),
        }
    }

    /// `afterAlways(errors)`: drains the cleanup queue in reverse order; when
    /// the suite succeeded, removes everything (an `IOException` fails the
    /// suite unless `@SuppressTempFileChecks`) and closes the suite
    /// filesystem; otherwise reports where the files were left.
    fn after_always(&self, errors: &mut Vec<Throwable>) {
        let (everything, temp_dir_base_path) = {
            let mut queue = CLEANUP_QUEUE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let base = self
                .lock()
                .temp_dir_base
                .take()
                .map(|b| b.to_absolute_path().to_string());
            queue.reverse();
            (std::mem::take(&mut *queue), base)
        };
        if self.failure_marker.was_successful() {
            if let Err(e) = IOUtils::rm(everything) {
                let suite_class = RandomizedContext::current().get_target_class_object();
                if let Some(a) = suite_class
                    .as_ref()
                    .and_then(|c| c.get_annotation::<SuppressTempFileChecks>())
                {
                    let message = match &e {
                        rucene::error::LuceneError {
                            kind: rucene::error::LuceneErrorKind::Io(io),
                            ..
                        } => java_message_of(io).unwrap_or_default(),
                        other => other.to_string(),
                    };
                    system::err_println(&format!(
                        "WARNING: Leftover undeleted temporary files (bugUrl: {}): {message}",
                        a.bug_url
                    ));
                    return;
                }
                let thrown = match &e {
                    rucene::error::LuceneError {
                        kind: rucene::error::LuceneErrorKind::Io(io),
                        ..
                    } => Throwable::new(
                        java_class_of(io).to_string(),
                        java_message_of(io).unwrap_or_default(),
                    ),
                    other => Throwable::new("java.io.IOException", other.to_string()),
                };
                errors.push(thrown);
                return;
            }
            let fs = self.lock().file_system.clone();
            if let Some(fs) = fs {
                if !same_file_system(&fs, &default_file_system()) {
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fs.close())) {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => errors.push(Throwable::new(
                            java_class_of(&e).to_string(),
                            java_message_of(&e).unwrap_or_default(),
                        )),
                        Err(payload) => errors.push(Throwable::from_panic(payload)),
                    }
                }
            }
        } else if let Some(path) = temp_dir_base_path {
            system::err_println(&format!("NOTE: leaving temporary files on disk at: {path}"));
        }
    }
}
