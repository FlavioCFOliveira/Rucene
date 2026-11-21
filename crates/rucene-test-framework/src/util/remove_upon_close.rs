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

//! Port of `org.apache.lucene.tests.util.RemoveUponClose`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::{io_exception_with_cause, IntoNioPath};
use rucene::util::IOUtils;

use crate::util::TestRuleMarkFailure;

/// A closeable that attempts to remove a given file/folder.
///
/// Port of the package-private `RemoveUponClose` (`implements Closeable`).
///
/// **Divergence (declared).** Java records the creating thread's stack
/// (`Thread.currentThread().getStackTrace()`, one `\t<frame>\n` per frame)
/// for the error message; the port records the Rust backtrace
/// (`std::backtrace::Backtrace::force_capture()`), one `\t<line>\n` per
/// line: JVM frames do not exist here.
pub struct RemoveUponClose {
    path: PathBuf,
    failure_marker: Arc<TestRuleMarkFailure>,
    creation_stack: String,
}

impl RemoveUponClose {
    /// `new RemoveUponClose(Path path, TestRuleMarkFailure failureMarker)`.
    pub fn new(path: impl AsRef<Path>, failure_marker: Arc<TestRuleMarkFailure>) -> Self {
        let mut b = String::new();
        for e in std::backtrace::Backtrace::force_capture()
            .to_string()
            .lines()
        {
            b.push('\t');
            b.push_str(e);
            b.push('\n');
        }
        RemoveUponClose {
            path: path.as_ref().to_path_buf(),
            failure_marker,
            creation_stack: b,
        }
    }

    /// `close()`: only if there were no other test failures, removes the
    /// path (when it exists).
    ///
    /// # Errors
    ///
    /// `IOException("Could not remove temporary location '<absolute path>',
    /// created at stack trace:\n<stack>", cause)` when `IOUtils.rm` fails;
    /// its cause is `IOUtils.rm`'s exception (`RemoveUponClose.java:50-55`).
    pub fn close(&self) -> Result<()> {
        // only if there were no other test failures.
        if self.failure_marker.was_successful() && self.path.exists() {
            if let Err(e) = IOUtils::rm([self.path.as_path()]) {
                return Err(LuceneError::from(LuceneErrorKind::Io(
                    io_exception_with_cause(
                        format!(
                            "Could not remove temporary location '{}', created at stack trace:\n{}",
                            self.path.as_path().into_nio_path().to_absolute_path(),
                            self.creation_stack
                        ),
                        Some(e),
                    ),
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::RemoveUponClose;
    use crate::util::TestRuleMarkFailure;

    fn scratch(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rucene-remove-upon-close-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join("sub/f"), b"x").unwrap();
        p
    }

    #[test]
    fn removes_the_path_after_a_successful_test() {
        let p = scratch("ok");
        let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
        RemoveUponClose::new(&p, Arc::clone(&marker))
            .close()
            .unwrap();
        assert!(!p.exists());
        // A path that no longer exists is left alone.
        RemoveUponClose::new(&p, marker).close().unwrap();
    }

    #[test]
    fn keeps_the_path_after_a_failure() {
        let p = scratch("failed");
        let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
        marker.mark_failed();
        RemoveUponClose::new(&p, marker).close().unwrap();
        assert!(p.exists());
        std::fs::remove_dir_all(&p).unwrap();
    }

    /// `RemoveUponClose.java:50-55`: the `IOException` carries `IOUtils.rm`'s
    /// exception as its cause, and its message is only the location and the
    /// creation stack (no appended "Caused by").
    #[cfg(unix)]
    #[test]
    fn a_failed_removal_reports_the_rm_exception_as_its_cause() {
        use std::os::unix::fs::PermissionsExt;

        let p = scratch("locked");
        let sub = p.join("sub");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        // A process that may still write into a read-only directory (root)
        // cannot make `IOUtils.rm` fail this way.
        let privileged = std::fs::write(sub.join("probe"), b"x").is_ok();
        let marker = Arc::new(TestRuleMarkFailure::new(Vec::new()));
        let result = RemoveUponClose::new(&p, marker).close();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).ok();
        if privileged {
            result.unwrap();
            assert!(!p.exists());
            return;
        }
        let e = result.expect_err("rm must fail on a read-only directory");
        let rucene::error::LuceneErrorKind::Io(io) = e.kind() else {
            panic!("expected an IOException, got {e:?}");
        };
        let message = io.to_string();
        assert!(
            message.starts_with("Could not remove temporary location '"),
            "{message}"
        );
        assert!(
            message.contains("', created at stack trace:\n"),
            "{message}"
        );
        assert!(!message.contains("Caused by"), "{message}");
        let cause = rucene::store::java_nio_file::java_cause_of(io).expect("getCause()");
        assert!(
            cause
                .to_string()
                .contains("Could not remove the following files (in the order of attempts):"),
            "{cause}"
        );
        std::fs::remove_dir_all(&p).unwrap();
    }
}
