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

//! Port of `org.apache.lucene.tests.index.AlcoholicMergePolicy`: a
//! `LogMergePolicy` whose segment sizes are multiplied by a random "drunk
//! factor" outside working hours, so that merges are unpredictable.

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::Result;
use rucene::index::merge_policy::{LogMergePolicy, LogMergeSize, MergeContext};
use rucene::index::segment_info::SegmentCommitInfo;

use crate::util::lucene_test_case::TimeZone;
use crate::util::TestUtil;

/// Port of the private enum `AlcoholicMergePolicy.Drink`: the drunk factors in
/// declaration order (`Drink.values()`).
const DRINK_FACTORS: [i64; 5] = [
    15, // Beer
    17, // Wine
    21, // Champagne
    22, // WhiteRussian
    30, // SingleMalt
];

/// The `size` override of [`AlcoholicMergePolicy`], with the policy's
/// `random` and `calendar` fields.
#[derive(Debug)]
struct AlcoholicSize {
    random: JdkRandom,
    /// `calendar.get(Calendar.HOUR_OF_DAY)`: the calendar is set once, in the
    /// constructor, and never changes.
    hour_of_day: i32,
}

impl LogMergeSize for AlcoholicSize {
    /// `size(info, mergeContext)` ("@BlackMagic(level=Voodoo)"): before 6, after
    /// 20 or, one time in 23, "it's 5 o'clock somewhere", the size of a random
    /// drink's factor times the segment size; otherwise the segment size.
    fn size(
        &self,
        _policy: &LogMergePolicy,
        info: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<i64> {
        let hour_of_day = self.hour_of_day;
        if !(6..=20).contains(&hour_of_day)
            // it's 5 o'clock somewhere
            || self.random.next_int_bound(23) == 5
        {
            // pick a random drink during the day
            let factor =
                DRINK_FACTORS[self.random.next_int_bound(DRINK_FACTORS.len() as i32) as usize];
            return Ok(factor.wrapping_mul(info.size_in_bytes_uncached()?));
        }
        info.size_in_bytes_uncached()
    }

    fn simple_name(&self) -> &str {
        "AlcoholicMergePolicy"
    }
}

/// Port of `org.apache.lucene.tests.index.AlcoholicMergePolicy` (`extends
/// LogMergePolicy`): "Merge policy for testing, it is like an alcoholic. It
/// drinks (merges) at night, and randomly decides what to drink. During the
/// daytime it sleeps."
///
/// The policy is a [`LogMergePolicy`] whose size function is the override,
/// as every `LogMergePolicy` subclass is in this port.
///
/// **Divergence from Lucene 10.5.0 (pending user approval).** Java's
/// constructor sets a `GregorianCalendar(tz, Locale.ROOT)` to a random instant
/// and `size` reads its `HOUR_OF_DAY` in the class environment's time zone.
/// This port has no time-zone rules (the test framework's `TimeZone` carries
/// only the ID), so the hour is the hour of that instant in UTC. The hour only
/// decides when the drunk factor applies, so it changes which merges the
/// policy picks, never what an index holds.
pub struct AlcoholicMergePolicy;

impl AlcoholicMergePolicy {
    /// `new AlcoholicMergePolicy(tz, random)`: the `LogMergePolicy` whose size
    /// function is this class's override.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(tz: TimeZone, random: JdkRandom) -> LogMergePolicy {
        let _ = tz;
        // `calendar.setTimeInMillis(TestUtil.nextLong(random, 0, Long.MAX_VALUE))`
        let millis = TestUtil::next_long(&random, 0, i64::MAX);
        let hour_of_day = (millis / 3_600_000).rem_euclid(24) as i32;
        let max_merge_size = i64::from(TestUtil::next_int(&random, 1024 * 1024, i32::MAX));
        let mut policy = LogMergePolicy::new(Arc::new(AlcoholicSize {
            random,
            hour_of_day,
        }));
        policy.set_max_merge_size(max_merge_size);
        policy
    }
}
