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

//! Tokenizer that breaks text into words with the Unicode Text Segmentation
//! algorithm.
//!
//! Equivalent to the package `org.apache.lucene.analysis.icu.segmentation`
//! (`package-info.java`).

pub mod break_iterator_wrapper;
pub mod char_array_iterator;
pub mod composite_break_iterator;
pub mod default_icu_tokenizer_config;
pub mod icu_tokenizer;
pub mod icu_tokenizer_config;
pub mod icu_tokenizer_factory;
pub mod script_iterator;
