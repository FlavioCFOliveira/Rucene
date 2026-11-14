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

//! `java.util.Date`, as the query parsers and `DateTools` use it: an instant
//! in milliseconds since the epoch.
//!
//! **Divergence.** An emulation of the JDK class, not a port of a Lucene
//! file: only the instant is kept (`Date.getTime()`), which is all the query
//! parsers read; the deprecated field accessors of `java.util.Date` are not
//! reproduced.

/// A `java.util.Date`: the milliseconds since 1970-01-01T00:00:00Z.
///
/// Equivalent to `java.util.Date`; two dates are equal, and order, by their
/// instants, as `Date.equals` and `Date.compareTo` compare them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    time: i64,
}

impl Date {
    /// `new Date(long date)`.
    pub fn new(date: i64) -> Self {
        Self { time: date }
    }

    /// `new Date()`: the current instant (`System.currentTimeMillis()`).
    pub fn now() -> Self {
        Self::new(current_time_millis())
    }

    /// `getTime()`.
    pub fn get_time(&self) -> i64 {
        self.time
    }

    /// `setTime(long time)`.
    pub fn set_time(&mut self, time: i64) {
        self.time = time;
    }

    /// `before(Date when)`.
    pub fn before(&self, when: &Date) -> bool {
        self.time < when.time
    }

    /// `after(Date when)`.
    pub fn after(&self, when: &Date) -> bool {
        self.time > when.time
    }
}

/// `System.currentTimeMillis()`.
pub fn current_time_millis() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}
