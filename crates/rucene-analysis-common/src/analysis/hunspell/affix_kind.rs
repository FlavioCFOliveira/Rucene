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

//! The kind of an affix, ported from
//! `org.apache.lucene.analysis.hunspell.AffixKind`.

/// Whether an affix is a prefix or a suffix.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.AffixKind`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AffixKind {
    /// Java constant `PREFIX`.
    Prefix,
    /// Java constant `SUFFIX`.
    Suffix,
}

impl AffixKind {
    /// Equivalent to `name()`, which `toString()` returns.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prefix => "PREFIX",
            Self::Suffix => "SUFFIX",
        }
    }
}
