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

//! Port of `PassageFormatter.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::any::Any;
use std::fmt::{Debug, Display};

use rucene::error::Result;

use crate::uhighlight::passage::Passage;

/// The `Object` a passage formatter produces: any value with a string form
/// (`toString()`, which `UnifiedHighlighter.highlightFields` reads), that a
/// caller may downcast to its own type.
pub trait FormattedPassages: Any + Debug + Display + Send + Sync {
    /// This value as `Any`.
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Debug + Display + Send + Sync> FormattedPassages for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Creates a formatted snippet from the top passages.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.uhighlight.PassageFormatter`.
pub trait PassageFormatter: Debug + Send + Sync {
    /// Formats the top `passages` from `content` into a human-readable text
    /// snippet.
    ///
    /// Equivalent to `format(Passage[], String)`: `passages` are the top
    /// passages ranked by the scorer, sorted by start offset; `content` is
    /// the original text.
    ///
    /// # Errors
    ///
    /// What the formatter raises.
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>>;
}
