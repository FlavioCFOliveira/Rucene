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

//! Skeletons of the Java package `org.apache.lucene.benchmark.byTask.feeds`.

pub mod abstract_query_maker;
pub mod content_items_source;
pub mod content_source;
pub mod demo_html_parser;
pub mod dir_content_source;
pub mod doc_data;
pub mod doc_maker;
pub mod enwiki_content_source;
pub mod enwiki_query_maker;
pub mod facet_source;
pub mod file_based_query_maker;
pub mod geonames_line_parser;
pub mod html_parser;
pub mod line_doc_source;
pub mod long_to_english_content_source;
pub mod long_to_english_query_maker;
pub mod no_more_data_exception;
pub mod query_maker;
pub mod random_facet_source;
pub mod reuters_content_source;
pub mod reuters_query_maker;
pub mod simple_query_maker;
pub mod simple_sloppy_phrase_query_maker;
pub mod single_doc_source;
pub mod sortable_single_doc_source;
pub mod spatial_doc_maker;
pub mod spatial_file_query_maker;
pub mod trec_content_source;
pub mod trec_doc_parser;
pub mod trec_fbis_parser;
pub mod trec_fr94_parser;
pub mod trec_ft_parser;
pub mod trec_gov2_parser;
pub mod trec_la_times_parser;
pub mod trec_parser_by_path;
