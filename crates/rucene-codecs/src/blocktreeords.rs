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

//! Port of `org.apache.lucene.codecs.blocktreeords`: same postings format as
//! Lucene50, except the terms dictionary also supports ords, i.e. returning
//! which ord the enum is seeked to, and seeking by ord.
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`BlockTreeOrdsPostingsFormat`] | `BlockTreeOrdsPostingsFormat` |
//! | [`FSTOrdsOutputs`], [`Output`] | `FSTOrdsOutputs`, `FSTOrdsOutputs.Output` |
//! | [`OrdsBlockTreeTermsReader`] | `OrdsBlockTreeTermsReader` |
//! | [`OrdsBlockTreeTermsWriter`] | `OrdsBlockTreeTermsWriter` |
//! | [`OrdsFieldReader`] | `OrdsFieldReader` |
//! | [`OrdsIntersectTermsEnum`] | `OrdsIntersectTermsEnum` |
//! | `ords_intersect_terms_enum_frame` | `OrdsIntersectTermsEnumFrame` |
//! | [`OrdsSegmentTermsEnum`] | `OrdsSegmentTermsEnum` |
//! | `ords_segment_terms_enum_frame` | `OrdsSegmentTermsEnumFrame` |

pub mod block_tree_ords_postings_format;
pub mod fst_ords_outputs;
pub mod ords_block_tree_terms_reader;
pub mod ords_block_tree_terms_writer;
pub mod ords_field_reader;
pub mod ords_intersect_terms_enum;
mod ords_intersect_terms_enum_frame;
pub mod ords_segment_terms_enum;
mod ords_segment_terms_enum_frame;

pub use block_tree_ords_postings_format::BlockTreeOrdsPostingsFormat;
pub use fst_ords_outputs::{FSTOrdsOutputs, Output};
pub use ords_block_tree_terms_reader::OrdsBlockTreeTermsReader;
pub use ords_block_tree_terms_writer::OrdsBlockTreeTermsWriter;
pub use ords_field_reader::OrdsFieldReader;
pub use ords_intersect_terms_enum::OrdsIntersectTermsEnum;
pub use ords_segment_terms_enum::OrdsSegmentTermsEnum;

/// Java's `assert cond;` (`assert cond : message;`) under `-ea`: an
/// `AssertionError`, raised in builds with `debug_assertions` (see
/// [`LuceneErrorKind::AssertionError`]).
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::AssertionError(String::new())));
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(rucene::error::LuceneErrorKind::AssertionError(format!($($msg)+))));
        }
    };
}
pub(crate) use java_assert;

pub(crate) use crate::java_util::{array_len, out_of_bounds_i32, OwnedArrayInput, TermBuilder};
