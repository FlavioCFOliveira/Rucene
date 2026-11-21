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

//! The Java package `org.apache.lucene.analysis.hunspell`: a Hunspell
//! dictionary model, stemmer, spell checker and suggester.

pub mod affix_condition;
pub mod affix_kind;
pub mod affixed_word;
pub mod check_compound_pattern;
pub mod compound_rule;
pub mod conv_table;
pub mod dict_entries;
pub mod dict_entry;
pub mod dictionary;
pub mod entry_suggestion;
pub mod flag_enumerator;
pub mod flyweight_entry;
pub mod fragment_checker;
pub mod generating_suggester;
#[allow(clippy::module_inception)]
pub mod hunspell;
pub mod hunspell_stem_filter;
pub mod hunspell_stem_filter_factory;
pub mod iso8859_14_decoder;
pub mod modifying_suggester;
pub mod ngram_fragment_checker;
pub mod rep_entry;
pub mod root;
pub mod sorting_strategy;
pub mod stemmer;
pub mod suggester;
pub mod suggestible_entry_cache;
pub mod suggestion;
pub mod suggestion_timeout_exception;
pub mod timeout_policy;
pub mod trigram_automaton;
pub mod word_case;
pub mod word_context;
pub mod word_form_generator;
pub mod word_storage;
