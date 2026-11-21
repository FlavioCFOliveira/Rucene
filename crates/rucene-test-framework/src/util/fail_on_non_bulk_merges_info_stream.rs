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

//! Port of `org.apache.lucene.tests.util.FailOnNonBulkMergesInfoStream`.

use rucene::util::InfoStream;

/// Hackidy-Häck-Hack to cause a test to fail on non-bulk merges.
///
/// Port of `FailOnNonBulkMergesInfoStream`: every component is enabled, and
/// a message that mentions `non-bulk merges` fails Java's `assert` (an
/// `AssertionError` under `-ea`, a panic with that throwable here when
/// `debug_assertions` are on).
// TODO: we should probably be a wrapper so verbose still works...
#[derive(Debug, Clone, Copy, Default)]
pub struct FailOnNonBulkMergesInfoStream;

impl InfoStream for FailOnNonBulkMergesInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.FailOnNonBulkMergesInfoStream"
    }

    fn message(&self, _component: &str, message: &str) {
        java_assert_unchecked!(!message.contains("non-bulk merges"));
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use randomizedtesting::jdk::exception::{panic_message, QuietPanics};
    use rucene::util::InfoStream;

    use super::FailOnNonBulkMergesInfoStream;

    #[test]
    fn non_bulk_merge_message_fails_the_assertion() {
        let s = FailOnNonBulkMergesInfoStream;
        assert!(s.is_enabled("SM"));
        s.message("SM", "merge store matchedCount=2 vs 2");
        if cfg!(debug_assertions) {
            let _quiet = QuietPanics::new();
            let e = catch_unwind(|| s.message("SM", "3 non-bulk merges")).unwrap_err();
            assert_eq!(panic_message(&*e), "java.lang.AssertionError");
        }
    }
}
