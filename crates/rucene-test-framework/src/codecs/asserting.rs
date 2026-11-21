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

//! Port of `org.apache.lucene.tests.codecs.asserting`: codec for testing
//! that asserts various contracts of the codec apis.

pub mod asserting_codec;
pub mod asserting_doc_values_format;
pub mod asserting_knn_vectors_format;
pub mod asserting_live_docs_format;
pub mod asserting_norms_format;
pub mod asserting_points_format;
pub mod asserting_postings_format;
pub mod asserting_stored_fields_format;
pub mod asserting_term_vectors_format;

pub use asserting_codec::{AssertingCodec, Format};
pub use asserting_doc_values_format::AssertingDocValuesFormat;
pub use asserting_knn_vectors_format::{AssertingKnnVectorsFormat, AssertingKnnVectorsReader};
pub use asserting_live_docs_format::AssertingLiveDocsFormat;
pub use asserting_norms_format::AssertingNormsFormat;
pub use asserting_points_format::AssertingPointsFormat;
pub use asserting_postings_format::AssertingPostingsFormat;
pub use asserting_stored_fields_format::AssertingStoredFieldsFormat;
pub use asserting_term_vectors_format::AssertingTermVectorsFormat;
