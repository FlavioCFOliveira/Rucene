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

use crate::jdk::time_zone::gregorian_calendar_hour_of_day;
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
/// as every `LogMergePolicy` subclass is in this port. Java's
/// `GregorianCalendar(tz, Locale.ROOT)`, set once to a random instant, is
/// reduced to the one field `size` reads, its `HOUR_OF_DAY`, computed in `tz`
/// by [`gregorian_calendar_hour_of_day`] from the JDK 25 rules of the zone.
pub struct AlcoholicMergePolicy;

impl AlcoholicMergePolicy {
    /// `new AlcoholicMergePolicy(tz, random)`: the `LogMergePolicy` whose size
    /// function is this class's override.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(tz: TimeZone, random: JdkRandom) -> LogMergePolicy {
        let (size, max_merge_size) = Self::size_and_max_merge_size(tz, random);
        let mut policy = LogMergePolicy::new(Arc::new(size));
        policy.set_max_merge_size(max_merge_size);
        policy
    }

    /// The constructor's draws, in Java's order: the calendar's instant, then
    /// `maxMergeSize`.
    fn size_and_max_merge_size(tz: TimeZone, random: JdkRandom) -> (AlcoholicSize, i64) {
        // `calendar = new GregorianCalendar(tz, Locale.ROOT);
        // calendar.setTimeInMillis(TestUtil.nextLong(random, 0, Long.MAX_VALUE))`
        let millis = TestUtil::next_long(&random, 0, i64::MAX);
        let hour_of_day = gregorian_calendar_hour_of_day(tz.zone_info(), millis);
        let max_merge_size = i64::from(TestUtil::next_int(&random, 1024 * 1024, i32::MAX));
        (
            AlcoholicSize {
                random,
                hour_of_day,
            },
            max_merge_size,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `alcoholic_hours.tsv`, written by the oracle's `TimeZoneOracle`: per
    /// seed, `LuceneTestCase.randomTimeZone(new Random(seed))`, then `new
    /// AlcoholicMergePolicy(tz, new Random(seed))`'s `HOUR_OF_DAY` and
    /// `maxMergeSize`.
    #[test]
    fn constructor_matches_lucene() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/alcoholic_hours.tsv"
        );
        let text = std::fs::read_to_string(path).expect("read alcoholic_hours.tsv");
        let mut rows = 0;
        for line in text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let r: Vec<&str> = line.split('\t').collect();
            let seed: i64 = r[0].parse().expect("seed");
            let tz = crate::util::lucene_test_case::LuceneTestCase::random_time_zone(
                &JdkRandom::new(seed),
            );
            assert_eq!(tz.get_id(), r[1], "randomTimeZone, seed {seed}");
            let (size, max_merge_size) =
                AlcoholicMergePolicy::size_and_max_merge_size(tz, JdkRandom::new(seed));
            assert_eq!(
                size.hour_of_day.to_string(),
                r[2],
                "HOUR_OF_DAY, seed {seed}"
            );
            assert_eq!(
                max_merge_size.to_string(),
                r[3],
                "maxMergeSize, seed {seed}"
            );
            rows += 1;
        }
        assert!(rows >= 20, "{rows} rows");
    }
}
