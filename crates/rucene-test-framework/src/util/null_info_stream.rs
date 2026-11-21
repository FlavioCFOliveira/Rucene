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

//! Port of `org.apache.lucene.tests.util.NullInfoStream`.

use rucene::util::InfoStream;

/// Prints nothing. Just to make sure tests pass w/ and without enabled
/// InfoStream without actually making noise.
///
/// Port of `NullInfoStream` (`@lucene.experimental`). Java's `assert
/// component != null` and `assert message != null` have no counterpart: a
/// Rust `&str` cannot be `null`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullInfoStream;

impl InfoStream for NullInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.util.NullInfoStream"
    }

    fn message(&self, _component: &str, _message: &str) {}

    /// To actually enable logging, we just ignore on `message()`.
    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}
}

#[cfg(test)]
mod tests {
    use rucene::util::InfoStream;

    use super::NullInfoStream;

    #[test]
    fn null_info_stream_is_enabled_and_silent() {
        let s = NullInfoStream;
        assert!(s.is_enabled("IW"));
        s.message("IW", "anything");
        s.close();
        assert_eq!(
            s.java_class_name(),
            "org.apache.lucene.tests.util.NullInfoStream"
        );
    }
}
