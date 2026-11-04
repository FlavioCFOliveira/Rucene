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

//! Port of `org.apache.lucene.codecs.simpletext`: the plain-text codec,
//! **FOR RECREATIONAL USE ONLY**.
//!
//! Every file the codec writes is human-readable text; each one ends with
//! the checksum line `SimpleTextUtil.writeChecksum` writes.
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`SimpleTextCodec`] | `SimpleTextCodec` |
//! | [`SimpleTextPostingsFormat`] | `SimpleTextPostingsFormat` |
//! | [`fields_reader::SimpleTextFieldsReader`] | `SimpleTextFieldsReader` |
//! | [`fields_writer::SimpleTextFieldsWriter`] | `SimpleTextFieldsWriter` |
//! | `skip_reader::SimpleTextSkipReader` | `SimpleTextSkipReader` |
//! | `skip_writer::SimpleTextSkipWriter` | `SimpleTextSkipWriter` |
//! | [`SimpleTextStoredFieldsFormat`] | `SimpleTextStoredFieldsFormat` |
//! | [`stored_fields_reader::SimpleTextStoredFieldsReader`] | `SimpleTextStoredFieldsReader` |
//! | [`stored_fields_writer::SimpleTextStoredFieldsWriter`] | `SimpleTextStoredFieldsWriter` |
//! | [`SimpleTextTermVectorsFormat`] | `SimpleTextTermVectorsFormat` |
//! | [`term_vectors_reader::SimpleTextTermVectorsReader`] | `SimpleTextTermVectorsReader` |
//! | [`term_vectors_writer::SimpleTextTermVectorsWriter`] | `SimpleTextTermVectorsWriter` |
//! | [`util`] | `SimpleTextUtil` |
//! | [`SimpleTextLiveDocsFormat`] | `SimpleTextLiveDocsFormat` |
//! | [`SimpleTextNormsFormat`] | `SimpleTextNormsFormat` |
//! | [`SimpleTextFieldInfosFormat`] | `SimpleTextFieldInfosFormat` |
//! | [`SimpleTextSegmentInfoFormat`] | `SimpleTextSegmentInfoFormat` |
//! | [`SimpleTextCompoundFormat`] | `SimpleTextCompoundFormat` |
//! | [`SimpleTextDocValuesFormat`] | `SimpleTextDocValuesFormat` |
//! | [`doc_values_reader::SimpleTextDocValuesReader`] | `SimpleTextDocValuesReader` |
//! | [`doc_values_writer::SimpleTextDocValuesWriter`] | `SimpleTextDocValuesWriter` |
//! | [`SimpleTextPointsFormat`] | `SimpleTextPointsFormat` |
//! | [`points_reader::SimpleTextPointsReader`] | `SimpleTextPointsReader` |
//! | [`points_writer::SimpleTextPointsWriter`] | `SimpleTextPointsWriter` |
//! | [`bkd_reader::SimpleTextBKDReader`] | `SimpleTextBKDReader` |
//! | [`bkd_writer::SimpleTextBKDWriter`] | `SimpleTextBKDWriter` |
//! | [`SimpleTextKnnVectorsFormat`] | `SimpleTextKnnVectorsFormat` |
//! | [`knn_vectors_reader::SimpleTextKnnVectorsReader`] | `SimpleTextKnnVectorsReader` |
//! | [`knn_vectors_writer::SimpleTextKnnVectorsWriter`] | `SimpleTextKnnVectorsWriter` |

//! # Exceptions
//!
//! The unchecked exceptions the Java readers throw that `LuceneError` has no
//! dedicated variant for — `IndexOutOfBoundsException`,
//! `ArrayIndexOutOfBoundsException`, `NullPointerException`,
//! `NegativeArraySizeException` and a plain `RuntimeException`, including the
//! `new RuntimeException(ioe)` of the doc-values value functions — are
//! [`LuceneError::Unchecked`](rucene::error::LuceneError::Unchecked), with
//! Java's class, message and cause; `crates/rucene-codecs/tests/simpletext_oracle.rs`
//! compares them with what Lucene 10.5.0 throws. Where a Java `assert` decides
//! the outcome, debug builds behave as a JVM with `-ea` and release builds as
//! one without.
//!
//! **Divergence.** `NumberFormatException` is reported as its superclass
//! `IllegalArgumentException`, with its message, as everywhere in the port.

pub mod bkd_reader;
pub mod bkd_writer;
pub mod codec;
pub mod compound_format;
pub mod doc_values_format;
pub mod doc_values_reader;
pub mod doc_values_writer;
pub mod field_infos_format;
pub mod fields_reader;
pub mod fields_writer;
pub mod knn_vectors_format;
pub mod knn_vectors_reader;
pub mod knn_vectors_writer;
pub mod live_docs_format;
pub mod norms_format;
pub mod points_format;
pub mod points_reader;
pub mod points_writer;
pub mod postings_format;
pub mod segment_info_format;
pub(crate) mod skip_reader;
pub(crate) mod skip_writer;
pub mod stored_fields_format;
pub mod stored_fields_reader;
pub mod stored_fields_writer;
pub mod term_vectors_format;
pub mod term_vectors_reader;
pub mod term_vectors_writer;
pub mod util;

pub use codec::SimpleTextCodec;
pub use compound_format::SimpleTextCompoundFormat;
pub use doc_values_format::SimpleTextDocValuesFormat;
pub use field_infos_format::SimpleTextFieldInfosFormat;
pub use knn_vectors_format::SimpleTextKnnVectorsFormat;
pub use live_docs_format::SimpleTextLiveDocsFormat;
pub use norms_format::SimpleTextNormsFormat;
pub use points_format::SimpleTextPointsFormat;
pub use postings_format::SimpleTextPostingsFormat;
pub use segment_info_format::SimpleTextSegmentInfoFormat;
pub use stored_fields_format::SimpleTextStoredFieldsFormat;
pub use term_vectors_format::SimpleTextTermVectorsFormat;
