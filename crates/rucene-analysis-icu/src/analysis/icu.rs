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

//! Analysis components based on ICU.
//!
//! Equivalent to the package `org.apache.lucene.analysis.icu`
//! (`package-info.java`).

pub mod icu_collation_attribute_factory;
pub mod icu_collation_doc_values_field;
pub mod icu_collation_key_analyzer;
pub mod icu_folding_filter;
pub mod icu_folding_filter_factory;
pub mod icu_normalizer2_char_filter;
pub mod icu_normalizer2_char_filter_factory;
pub mod icu_normalizer2_filter;
pub mod icu_normalizer2_filter_factory;
pub mod icu_transform_filter;
pub mod icu_transform_filter_factory;
pub mod segmentation;
pub mod tokenattributes;
