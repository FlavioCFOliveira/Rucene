// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! This crate ports, from ICU4J 77.1 (`com.ibm.icu:icu4j:77.1`, tag
//! `release-77-1`, Unicode License v3), the classes Apache Lucene 10.5.0
//! `lucene/analysis/icu` reaches, whole files (user decision D1 on rmp task
//! #640), together with every ICU data file those classes load statically,
//! embedded byte-identical (decision D2). The full ICU `LICENSE`, including
//! the notices of the break-iterator dictionaries, ships with the crate; each
//! source file keeps the ICU copyright line.
//!
//! The module tree mirrors the Java packages below `com.ibm.icu`:
//! `com.ibm.icu.impl` is the module [`r#impl`], `com.ibm.icu.text`
//! [`text`], `com.ibm.icu.util` [`util`], `com.ibm.icu.lang` [`lang`].
//! Each Java source file is one module, named in snake_case
//! (`ICUBinary.java` is [`r#impl::icu_binary`]); a nested Java type is
//! declared in the module of its source file.
//!
//! **Divergence.** Java's `String`, `CharSequence`, `StringBuilder`,
//! `StringBuffer` and `char[]` are sequences of UTF-16 units; the port keeps
//! that unit model, so that surrogates, offsets and lengths are those of
//! Java: a text parameter is `&[u16]` (Rust has no overloading, so the
//! `String`, `CharSequence` and `StringBuffer` overloads of one Java method
//! that behave alike become one function), a returned `String` is
//! `Vec<u16>`, and a `StringBuilder`, `StringBuffer` or `Appendable` that a
//! method appends to is `&mut Vec<u16>` (appending to it cannot fail, so the
//! `IOException` branches of `Appendable` callers have no counterpart). A
//! `StringBuffer` that two objects share in Java (`ReplaceableString`) is an
//! `Arc<Mutex<Vec<u16>>>`. Texts that only name things (resource paths,
//! exception messages, hexadecimal renderings) are Rust strings.
//!
//! **Divergence.** Java's exceptions surface as
//! [`rucene::error::LuceneError`] values in a `Result`, with one exception:
//! the `IndexOutOfBoundsException` family (`ArrayIndexOutOfBoundsException`,
//! `StringIndexOutOfBoundsException`), whether the JVM raises it on an
//! access outside an array, string or buffer or ICU throws it after its own
//! range check, is a Rust panic carrying the Java exception's text, as Rust
//! indexing panics; ICU's main code never catches it (the one catch, inside
//! `UTF16.newString`, is a local array growth the port does with an explicit
//! check). Methods whose only exceptions are of that family keep their
//! infallible Java signatures and document the panic. ICU's own exception
//! classes are typed values in
//! [`LuceneErrorKind::Throwable`](rucene::error::LuceneErrorKind::Throwable).
//!
//! Java `assert` statements are `debug_assert!`: they run in the debug
//! builds the tests use, as Java's run under `-ea`.

pub(crate) mod class_path_data;
pub mod jdk;

/// `com.ibm.icu.impl`.
pub mod r#impl {
    pub mod icu_currency_meta_info;
    /// `com.ibm.icu.impl.text`.
    pub mod text {
        pub mod rbnf_scanner_provider_impl;
    }
    pub mod assert;
    /// `com.ibm.icu.impl.breakiter`.
    pub mod breakiter {
        pub mod burmese_break_engine;
        pub mod bytes_dictionary_matcher;
        pub mod chars_dictionary_matcher;
        pub mod cjk_break_engine;
        pub mod dictionary_break_engine;
        pub mod dictionary_data;
        pub mod dictionary_matcher;
        pub mod khmer_break_engine;
        pub mod language_break_engine;
        pub mod lao_break_engine;
        pub mod lstm_break_engine;
        pub mod ml_break_engine;
        pub mod thai_break_engine;
        pub mod unhandled_break_engine;
    }
    pub mod cache_base;
    pub mod cache_value;
    pub mod character_iteration;
    pub mod character_iterator_wrapper;
    pub mod class_loader_util;
    pub mod cs_character_iterator;
    pub mod icu_binary;
    pub mod icu_config;
    pub mod icu_currency_display_info_provider;
    pub mod icu_data;
    pub mod icu_debug;
    pub mod icu_lang_data_tables;
    pub mod icu_locale_service;
    pub mod icu_notifier;
    pub mod icu_region_data_tables;
    pub mod icu_resource_bundle;
    pub mod icu_resource_bundle_impl;
    pub mod icu_resource_bundle_reader;
    pub mod icu_resource_table_access;
    pub mod icu_service;
    pub mod icurw_lock;
    pub mod illegal_icu_argument_exception;
    /// `com.ibm.icu.impl.locale`.
    pub mod locale {
        pub mod ascii_util;
        pub mod base_locale;
        pub mod extension;
        pub mod internal_locale_builder;
        pub mod key_type_data;
        pub mod language_tag;
        pub mod likely_subtags;
        pub mod locale_distance;
        pub mod locale_extensions;
        pub mod locale_object_cache;
        pub mod locale_syntax_exception;
        pub mod lsr;
        pub mod parse_status;
        pub mod string_token_iterator;
        pub mod unicode_locale_extension;
    }
    pub mod bmp_set;
    pub mod case_map_impl;
    pub mod character_properties_impl;
    pub mod currency_data;
    pub mod emoji_props;
    pub mod locale_display_names_impl;
    pub mod locale_fallback_data;
    pub mod locale_id_parser;
    pub mod locale_ids;
    pub mod locale_utility;
    pub mod norm2_all_modes;
    pub mod normalizer2_impl;
    pub mod pattern_props;
    pub mod rbbi_data_wrapper;
    pub mod replaceable_u_character_iterator;
    pub mod resource_bundle_wrapper;
    pub mod rule_character_iterator;
    pub mod simple_filtered_sentence_break_iterator;
    pub mod simple_formatter_impl;
    pub mod soft_cache;
    pub mod sorted_set_relation;
    pub mod standard_plural;
    pub mod string_range;
    pub mod trie2;
    pub mod trie2_16;
    pub mod trie2_32;
    pub mod trie2_writable;
    pub mod u_bidi_props;
    pub mod u_case_props;
    pub mod u_char_array_iterator;
    pub mod u_character_iterator_wrapper;
    pub mod u_character_name;
    pub mod u_character_name_choice;
    pub mod u_character_property;
    pub mod u_character_utility;
    pub mod u_property_aliases;
    pub mod u_resource;
    pub mod u_serialized_set;
    pub mod unicode_set_string_span;
    pub mod url_handler;
    pub mod utility;
    /// `com.ibm.icu.impl.coll`.
    pub mod coll {
        pub mod bocsu;
        pub mod collation;
        pub mod collation_builder;
        pub mod collation_compare;
        pub mod collation_data;
        pub mod collation_data_builder;
        pub mod collation_data_reader;
        pub mod collation_fast_latin;
        pub mod collation_fast_latin_builder;
        pub mod collation_fcd;
        pub mod collation_iterator;
        pub mod collation_keys;
        pub mod collation_loader;
        pub mod collation_root;
        pub mod collation_root_elements;
        pub mod collation_rule_parser;
        pub mod collation_settings;
        pub mod collation_tailoring;
        pub mod collation_weights;
        pub mod contractions_and_expansions;
        pub mod fcd_iter_collation_iterator;
        pub mod fcd_utf16_collation_iterator;
        pub mod iter_collation_iterator;
        pub mod shared_object;
        pub mod tailored_set;
        pub mod u_vector32;
        pub mod u_vector64;
        pub mod utf16_collation_iterator;
    }
    pub mod collection_set;
    pub mod formatted_string_builder;
    pub mod formatted_value_string_builder_impl;
    pub mod grego;
    pub mod icu_cache;
    pub mod pair;
    pub mod plural_rules_loader;
    pub mod simple_cache;
    pub mod static_unicode_sets;
    pub mod string_segment;
    pub mod text_trie_map;
    /// `com.ibm.icu.impl.number`.
    pub mod number {
        pub mod adopting_modifier_store;
        pub mod affix_pattern_provider;
        pub mod affix_utils;
        pub mod compact_data;
        pub mod constant_affix_modifier;
        pub mod constant_multi_field_modifier;
        pub mod currency_plural_info_affix_provider;
        pub mod currency_spacing_enabled_modifier;
        pub mod custom_symbol_currency;
        pub mod decimal_format_properties;
        pub mod decimal_quantity;
        pub mod decimal_quantity_abstract_bcd;
        pub mod decimal_quantity_dual_storage_bcd;
        pub mod grouper;
        pub mod localized_number_formatter_as_format;
        pub mod long_name_handler;
        pub mod long_name_multiplexer;
        pub mod macro_props;
        pub mod micro_props;
        pub mod micro_props_generator;
        pub mod mixed_unit_long_name_handler;
        pub mod modifier;
        pub mod modifier_store;
        pub mod multiplier_format_handler;
        pub mod multiplier_producer;
        pub mod mutable_pattern_modifier;
        pub mod padder;
        pub mod pattern_string_parser;
        pub mod pattern_string_utils;
        pub mod properties;
        pub mod properties_affix_pattern_provider;
        pub mod rounding_utils;
        pub mod simple_modifier;
        pub mod unit_conversion_handler;
        pub mod usage_prefs_handler;
        /// `com.ibm.icu.impl.number.parse`.
        pub mod parse {
            pub mod affix_matcher;
            pub mod affix_pattern_matcher;
            pub mod affix_token_matcher_factory;
            pub mod code_point_matcher;
            pub mod combined_currency_matcher;
            pub mod decimal_matcher;
            pub mod ignorables_matcher;
            pub mod infinity_matcher;
            pub mod minus_sign_matcher;
            pub mod multiplier_parse_handler;
            pub mod nan_matcher;
            pub mod number_parse_matcher;
            pub mod number_parser_impl;
            pub mod padding_matcher;
            pub mod parsed_number;
            pub mod parsing_utils;
            pub mod percent_matcher;
            pub mod permille_matcher;
            pub mod plus_sign_matcher;
            pub mod require_affix_validator;
            pub mod require_currency_validator;
            pub mod require_decimal_separator_validator;
            pub mod require_number_validator;
            pub mod scientific_matcher;
            pub mod series_matcher;
            pub mod symbol_matcher;
            pub mod validation_matcher;
        }
        /// `com.ibm.icu.impl.number.range`.
        pub mod range {
            pub mod prefix_infix_suffix_length_helper;
            pub mod range_macro_props;
            pub mod standard_plural_ranges;
        }
    }
    /// `com.ibm.icu.impl.units`.
    pub mod units {
        pub mod complex_units_converter;
        pub mod conversion_rates;
        pub mod measure_unit_impl;
        pub mod single_unit_impl;
        pub mod unit_preferences;
        pub mod units_converter;
        pub mod units_data;
        pub mod units_router;
    }
}

/// `com.ibm.icu.lang`.
pub mod lang {
    pub mod char_sequences;
    pub mod character_properties;
    pub mod u_character;
    pub mod u_character_enums;
    pub mod u_character_name_iterator;
    pub mod u_property;
    pub mod u_script;
}

/// `com.ibm.icu.text`.
pub mod text {
    pub(crate) mod any_transliterator;
    pub mod break_iterator;
    pub(crate) mod break_iterator_factory;
    pub(crate) mod break_transliterator;
    pub mod canonical_iterator;
    pub(crate) mod case_fold_transliterator;
    pub mod case_map;
    pub mod collation_element_iterator;
    pub mod collation_key;
    pub mod collator;
    pub(crate) mod collator_service_shim;
    pub mod compact_decimal_format;
    pub(crate) mod compound_transliterator;
    pub mod constrained_field_position;
    pub mod currency_display_names;
    pub mod currency_meta_info;
    pub mod currency_plural_info;
    pub mod decimal_format;
    pub mod decimal_format_symbols;
    pub mod display_context;
    pub mod display_options;
    pub mod edits;
    pub(crate) mod escape_transliterator;
    pub mod filtered_break_iterator_builder;
    pub mod filtered_normalizer2;
    pub mod formatted_value;
    pub(crate) mod function_replacer;
    pub mod list_formatter;
    pub mod locale_display_names;
    pub(crate) mod lowercase_transliterator;
    pub mod message_pattern;
    pub(crate) mod name_unicode_transliterator;
    pub(crate) mod nf_rule;
    pub(crate) mod nf_rule_set;
    pub(crate) mod nf_substitution;
    pub(crate) mod normalization_transliterator;
    pub mod normalizer;
    pub mod normalizer2;
    pub(crate) mod null_transliterator;
    pub mod number_format;
    pub(crate) mod number_format_service_shim;
    pub mod numbering_system;
    pub mod plural_format;
    pub mod plural_rules;
    pub mod plural_rules_serial_proxy;
    pub(crate) mod quantifier;
    pub mod raw_collation_key;
    pub(crate) mod rbbi_node;
    pub(crate) mod rbbi_rule_builder;
    pub(crate) mod rbbi_rule_parse_table;
    pub(crate) mod rbbi_rule_scanner;
    pub(crate) mod rbbi_set_builder;
    pub(crate) mod rbbi_symbol_table;
    pub(crate) mod rbbi_table_builder;
    pub(crate) mod rbnf_chinese_post_processor;
    pub mod rbnf_lenient_scanner;
    pub mod rbnf_lenient_scanner_provider;
    pub(crate) mod rbnf_post_processor;
    pub(crate) mod remove_transliterator;
    pub mod replaceable;
    pub(crate) mod replaceable_context_iterator;
    pub mod replaceable_string;
    pub mod rule_based_break_iterator;
    pub mod rule_based_collator;
    pub mod rule_based_number_format;
    pub mod rule_based_transliterator;
    pub mod simple_formatter;
    pub(crate) mod source_target_utility;
    pub(crate) mod string_matcher;
    pub(crate) mod string_replacer;
    pub mod string_transform;
    pub mod symbol_table;
    pub(crate) mod titlecase_transliterator;
    pub mod transform;
    pub(crate) mod transliteration_rule;
    pub(crate) mod transliteration_rule_set;
    pub mod transliterator;
    pub(crate) mod transliterator_id_parser;
    pub(crate) mod transliterator_parser;
    pub(crate) mod transliterator_registry;
    pub mod u_character_iterator;
    pub mod u_field_position;
    pub mod u_format;
    pub mod u_forward_character_iterator;
    pub(crate) mod unescape_transliterator;
    pub mod unicode_filter;
    pub mod unicode_matcher;
    pub(crate) mod unicode_name_transliterator;
    pub(crate) mod unicode_replacer;
    pub mod unicode_set;
    pub mod unicode_set_iterator;
    pub(crate) mod uppercase_transliterator;
    pub mod utf16;
}

/// `com.ibm.icu.math`.
pub mod math {
    pub mod big_decimal;
    pub mod math_context;
}

/// `com.ibm.icu.number`.
pub mod number {
    pub mod compact_notation;
    pub mod currency_precision;
    pub mod formatted_number;
    pub mod formatted_number_range;
    pub mod fraction_precision;
    pub mod integer_width;
    pub mod localized_number_formatter;
    pub mod localized_number_range_formatter;
    pub mod notation;
    pub mod number_formatter;
    pub mod number_formatter_impl;
    pub mod number_formatter_settings;
    pub mod number_property_mapper;
    pub mod number_range_formatter;
    pub mod number_range_formatter_impl;
    pub mod number_range_formatter_settings;
    pub mod number_skeleton_impl;
    pub mod precision;
    pub mod scale;
    pub mod scientific_notation;
    pub mod simple_notation;
    pub mod skeleton_syntax_exception;
    pub mod unlocalized_number_formatter;
    pub mod unlocalized_number_range_formatter;
}

/// `com.ibm.icu.util`.
pub mod util {
    pub mod byte_array_wrapper;
    pub mod bytes_trie;
    pub mod case_insensitive_string;
    pub mod chars_trie;
    pub mod chars_trie_builder;
    pub mod code_point_map;
    pub mod code_point_trie;
    pub mod currency;
    pub mod currency_amount;
    pub(crate) mod currency_service_shim;
    pub mod freezable;
    pub mod icu_clone_not_supported_exception;
    pub mod icu_exception;
    pub mod icu_input_too_long_exception;
    pub mod icu_unchecked_io_exception;
    pub mod illformed_locale_exception;
    pub mod locale_data;
    pub mod locale_matcher;
    pub mod locale_priority_list;
    pub mod measure;
    pub mod measure_unit;
    pub mod mutable_code_point_trie;
    pub mod no_unit;
    pub mod output;
    pub mod output_int;
    pub mod range_value_iterator;
    pub mod region;
    pub mod string_trie_builder;
    pub mod time_unit;
    pub mod u_locale;
    pub mod u_resource_bundle;
    pub mod u_resource_bundle_iterator;
    pub mod u_resource_type_mismatch_exception;
    pub mod value_iterator;
    pub mod version_info;
}
