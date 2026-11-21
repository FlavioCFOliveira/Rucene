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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms`: pluggable
//! term index / block terms dictionary implementations.
//!
//! Extension of [`uniformsplit`](super) with Shared Terms principle: Terms
//! are shared between all fields. It is particularly adapted to index a
//! massive number of fields because all the terms are stored in a single FST
//! dictionary.
//!
//! * Designed to be extensible
//! * Highly reduced on-heap memory usage when dealing with a massive number
//!   of fields.
//!
//! (`package-info.java`.)
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`FieldMetadataTermState`] | `FieldMetadataTermState` |
//! | [`STBlockLine`] | `STBlockLine` (and its `Serializer`) |
//! | [`STBlockReader`] | `STBlockReader` |
//! | [`STBlockWriter`] | `STBlockWriter` |
//! | [`STIntersectBlockReader`] | `STIntersectBlockReader` |
//! | [`STMergingBlockReader`] | `STMergingBlockReader` |
//! | `st_merging_terms_enum` | `STMergingTermsEnum` (package-private) |
//! | [`STUniformSplitPostingsFormat`] | `STUniformSplitPostingsFormat` |
//! | [`STUniformSplitTerms`] | `STUniformSplitTerms` |
//! | [`st_uniform_split_terms_reader`] | `STUniformSplitTermsReader` |
//! | [`STUniformSplitTermsWriter`] | `STUniformSplitTermsWriter` |
//! | [`UnionFieldMetadataBuilder`] | `UnionFieldMetadataBuilder` |

pub mod field_metadata_term_state;
pub mod st_block_line;
pub mod st_block_reader;
pub mod st_block_writer;
pub mod st_intersect_block_reader;
pub mod st_merging_block_reader;
mod st_merging_terms_enum;
pub mod st_uniform_split_postings_format;
pub mod st_uniform_split_terms;
pub mod st_uniform_split_terms_reader;
pub mod st_uniform_split_terms_writer;
pub mod union_field_metadata_builder;

pub use field_metadata_term_state::FieldMetadataTermState;
pub use st_block_line::STBlockLine;
pub use st_block_reader::STBlockReader;
pub use st_block_writer::STBlockWriter;
pub use st_intersect_block_reader::STIntersectBlockReader;
pub use st_merging_block_reader::STMergingBlockReader;
pub use st_uniform_split_postings_format::STUniformSplitPostingsFormat;
pub use st_uniform_split_terms::STUniformSplitTerms;
pub use st_uniform_split_terms_writer::STUniformSplitTermsWriter;
pub use union_field_metadata_builder::UnionFieldMetadataBuilder;
