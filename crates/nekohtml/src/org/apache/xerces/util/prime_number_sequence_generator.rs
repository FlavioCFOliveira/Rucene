/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `PrimeNumberSequenceGenerator.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::time::{SystemTime, UNIX_EPOCH};

use rucene::util::java_random::JavaRandom;

/// Utility class for filling a list of prime numbers.
///
/// Equivalent to the package-private
/// `org.apache.xerces.util.PrimeNumberSequenceGenerator`. Java's `new
/// Random()` is seeded from the clock; so is the port's.
#[derive(Debug)]
pub(crate) struct PrimeNumberSequenceGenerator;

const PRIMES: [i32; 128] = [
    3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
    101, 103, 107, 109, 113, 127, 131, 137, 139, 149, 151, 157, 163, 167, 173, 179, 181, 191, 193,
    197, 199, 211, 223, 227, 229, 233, 239, 241, 251, 257, 263, 269, 271, 277, 281, 283, 293, 307,
    311, 313, 317, 331, 337, 347, 349, 353, 359, 367, 373, 379, 383, 389, 397, 401, 409, 419, 421,
    431, 433, 439, 443, 449, 457, 461, 463, 467, 479, 487, 491, 499, 503, 509, 521, 523, 541, 547,
    557, 563, 569, 571, 577, 587, 593, 599, 601, 607, 613, 617, 619, 631, 641, 643, 647, 653, 659,
    661, 673, 677, 683, 691, 701, 709, 719, 727,
];

impl PrimeNumberSequenceGenerator {
    /// Equivalent to `generateSequence(int[])`.
    pub(crate) fn generate_sequence(array_to_fill: &mut [i32]) {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        let mut r = JavaRandom::new(seed);
        for slot in array_to_fill.iter_mut() {
            *slot = PRIMES[r.next_int_bound(PRIMES.len() as i32) as usize];
        }
    }
}
