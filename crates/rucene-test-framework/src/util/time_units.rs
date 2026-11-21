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

//! Port of `org.apache.lucene.tests.util.TimeUnits`: time unit constants for
//! use in annotations.

/// Time unit constants for use in annotations.
///
/// Port of `TimeUnits` (`final`, no instances).
pub struct TimeUnits(());

impl TimeUnits {
    /// 1 second in milliseconds.
    pub const SECOND: i32 = 1000;
    /// 1 minute in milliseconds.
    pub const MINUTE: i32 = 60 * Self::SECOND;
    /// 1 hour in milliseconds.
    pub const HOUR: i32 = 60 * Self::MINUTE;
}

#[cfg(test)]
mod tests {
    use super::TimeUnits;

    #[test]
    fn time_units_are_lucenes() {
        assert_eq!(TimeUnits::SECOND, 1000);
        assert_eq!(TimeUnits::MINUTE, 60_000);
        assert_eq!(TimeUnits::HOUR, 3_600_000);
    }
}
