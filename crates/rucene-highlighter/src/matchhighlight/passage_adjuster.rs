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

//! Port of `PassageAdjuster.java`, of the Java package
//! `org.apache.lucene.search.matchhighlight`.

use std::fmt::Debug;

use rucene::error::Result;

use crate::matchhighlight::offset_range::OffsetRange;
use crate::matchhighlight::passage::Passage;

/// Adjusts the range of one or more passages over a given value. An
/// adjuster could shift passage boundary to the next or previous word
/// delimiter or white space, for example.
///
/// Equivalent to `org.apache.lucene.search.matchhighlight.PassageAdjuster`.
/// The adjuster keeps the current value behind interior mutability, as the
/// selector that holds it is shared.
pub trait PassageAdjuster: Debug + Send + Sync {
    /// Equivalent to `currentValue(CharSequence)`.
    fn current_value(&self, value: &str);

    /// Equivalent to `adjust(Passage)`.
    ///
    /// # Errors
    ///
    /// What the adjuster raises.
    fn adjust(&self, p: &Passage) -> Result<OffsetRange>;
}