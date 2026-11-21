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

//! Port of `org.apache.lucene.codecs.bitvectors`: a simple bit-vector
//! format that supports hamming distance and storing vectors in an HNSW
//! graph.
//!
//! | Rucene | Apache Lucene 10.5.0 |
//! | --- | --- |
//! | [`HnswBitVectorsFormat`] | `HnswBitVectorsFormat` |
//! | [`FlatBitVectorsScorer`] | `FlatBitVectorsScorer` |

pub mod flat_bit_vectors_scorer;
pub mod hnsw_bit_vectors_format;

pub use flat_bit_vectors_scorer::FlatBitVectorsScorer;
pub use hnsw_bit_vectors_format::{HnswBitVectorsFormat, NAME};
