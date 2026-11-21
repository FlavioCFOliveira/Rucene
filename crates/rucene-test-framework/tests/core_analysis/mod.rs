//! The helper classes and the ported test classes of the
//! `org.apache.lucene.analysis` core test package that `core_analysis_tests`
//! runs, and `org.apache.lucene.index.TestFieldReuse`, which extends
//! `BaseTokenStreamTestCase` as they do.

pub mod fake_factories;
pub mod test_abstract_analysis_factory;
pub mod test_analysis_spi_loader;
pub mod test_analyzer_wrapper;
pub mod test_automaton_to_token_stream;
pub mod test_caching_token_filter;
pub mod test_char_array_map;
pub mod test_char_array_set;
pub mod test_character_utils;
pub mod test_delegating_analyzer_wrapper;
pub mod test_field_reuse;
pub mod test_graph_token_filter;
pub mod test_graph_tokenizers;
pub mod test_stop_filter;
