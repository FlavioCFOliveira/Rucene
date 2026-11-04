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

//! Port of `org.apache.lucene.tests.codecs.cranky`: a codec for testing that
//! throws random `IOException`s.
//!
//! Every format draws `random.nextInt(n) == 0` (`n` is 100, 1000 or 10000)
//! before or after the calls Java makes fail, from the one
//! `java.util.Random` the [`CrankyCodec`] was given, so the sequence of
//! failures for a seed is Java's.

pub mod cranky_codec;
pub mod cranky_compound_format;
pub mod cranky_doc_values_format;
pub mod cranky_field_infos_format;
pub mod cranky_live_docs_format;
pub mod cranky_norms_format;
pub mod cranky_points_format;
pub mod cranky_postings_format;
pub mod cranky_segment_info_format;
pub mod cranky_stored_fields_format;
pub mod cranky_term_vectors_format;

pub use cranky_codec::CrankyCodec;

use randomizedtesting::jdk::random::Random;
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::io_exception;

/// `if (random.nextInt(bound) == 0) throw new IOException(message);`
pub(crate) fn maybe_fail(random: &dyn Random, bound: i32, message: &str) -> Result<()> {
    if random.next_int_bound(bound) == 0 {
        return Err(LuceneError::Io(io_exception(message)));
    }
    Ok(())
}
