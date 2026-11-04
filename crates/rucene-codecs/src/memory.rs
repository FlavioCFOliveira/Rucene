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

//! Port of `org.apache.lucene.codecs.memory`: term dictionaries and postings
//! formats held in RAM.
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`FSTPostingsFormat`] | `FSTPostingsFormat` |
//! | [`FSTTermsReader`] | `FSTTermsReader` |
//! | [`FSTTermsWriter`] | `FSTTermsWriter` |
//! | [`FSTTermOutputs`], [`TermData`] | `FSTTermOutputs`, `FSTTermOutputs.TermData` |

pub mod fst_postings_format;
pub mod fst_term_outputs;
pub mod fst_terms_reader;
pub mod fst_terms_writer;

pub use fst_postings_format::FSTPostingsFormat;
pub use fst_term_outputs::{FSTTermOutputs, TermData};
pub use fst_terms_reader::FSTTermsReader;
pub use fst_terms_writer::FSTTermsWriter;
