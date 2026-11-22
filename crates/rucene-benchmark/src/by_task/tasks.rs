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

//! Skeletons of the Java package `org.apache.lucene.benchmark.byTask.tasks`.

pub mod add_doc_task;
pub mod add_faceted_doc_task;
pub mod add_indexes_task;
pub mod analyzer_factory_task;
pub mod clear_stats_task;
pub mod close_index_task;
pub mod close_reader_task;
pub mod close_taxonomy_index_task;
pub mod close_taxonomy_reader_task;
pub mod commit_index_task;
pub mod commit_taxonomy_index_task;
pub mod consume_content_source_task;
pub mod create_index_task;
pub mod create_taxonomy_index_task;
pub mod flush_index_task;
pub mod force_merge_task;
pub mod near_realtime_reader_task;
pub mod new_analyzer_task;
pub mod new_collation_analyzer_task;
pub mod new_locale_task;
pub mod new_round_task;
pub mod open_index_task;
pub mod open_reader_task;
pub mod open_taxonomy_index_task;
pub mod open_taxonomy_reader_task;
pub mod perf_task;
pub mod print_reader_task;
pub mod read_task;
pub mod read_tokens_task;
pub mod reopen_reader_task;
pub mod rep_all_task;
pub mod rep_select_by_pref_task;
pub mod rep_sum_by_name_round_task;
pub mod rep_sum_by_name_task;
pub mod rep_sum_by_pref_round_task;
pub mod rep_sum_by_pref_task;
pub mod report_task;
pub mod reset_inputs_task;
pub mod reset_system_erase_task;
pub mod reset_system_soft_task;
pub mod rollback_index_task;
pub mod search_task;
pub mod search_trav_ret_highlight_task;
pub mod search_trav_ret_load_field_selector_task;
pub mod search_trav_ret_task;
pub mod search_trav_task;
pub mod search_with_collector_task;
pub mod search_with_sort_task;
pub mod set_prop_task;
pub mod task_sequence;
pub mod update_doc_task;
pub mod wait_task;
pub mod warm_task;
pub mod write_enwiki_line_doc_task;
pub mod write_line_doc_task;
