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
    pub mod icu_lang_data_tables;
    pub mod icu_region_data_tables;
    pub mod icu_debug;
    pub mod icu_locale_service;
    pub mod icu_notifier;
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
    pub mod locale_fallback_data;
    pub mod currency_data;
    pub mod locale_display_names_impl;
    pub mod simple_formatter_impl;
    pub mod standard_plural;
    pub mod locale_id_parser;
    pub mod locale_ids;
    pub mod locale_utility;
    pub mod normalizer2_impl;
    pub mod pattern_props;
    pub mod rbbi_data_wrapper;
    pub mod replaceable_u_character_iterator;
    pub mod resource_bundle_wrapper;
    pub mod simple_filtered_sentence_break_iterator;
    pub mod soft_cache;
    pub mod trie2;
    pub mod trie2_16;
    pub mod trie2_32;
    pub mod u_char_array_iterator;
    pub mod u_character_iterator_wrapper;
    pub mod u_property_aliases;
    pub mod u_resource;
    pub mod url_handler;
    pub mod utility;
    pub mod bmp_set;
    pub mod case_map_impl;
    pub mod character_properties_impl;
    pub mod emoji_props;
    pub mod norm2_all_modes;
    pub mod rule_character_iterator;
    pub mod sorted_set_relation;
    pub mod string_range;
    pub mod u_bidi_props;
    pub mod u_case_props;
    pub mod u_character_name;
    pub mod u_character_name_choice;
    pub mod u_character_property;
    pub mod u_character_utility;
    pub mod u_serialized_set;
    pub mod unicode_set_string_span;
    pub mod trie2_writable;
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
}

/// `com.ibm.icu.lang`.
pub mod lang {
    pub mod u_character;
    pub mod u_property;
    pub mod u_script;
    pub mod char_sequences;
    pub mod character_properties;
    pub mod u_character_enums;
    pub mod u_character_name_iterator;
}

/// `com.ibm.icu.text`.
pub mod text {
    pub mod break_iterator;
    pub(crate) mod break_iterator_factory;
    pub mod edits;
    pub mod filtered_break_iterator_builder;
    pub(crate) mod rbbi_node;
    pub(crate) mod rbbi_rule_builder;
    pub(crate) mod rbbi_rule_parse_table;
    pub(crate) mod rbbi_rule_scanner;
    pub(crate) mod rbbi_set_builder;
    pub(crate) mod rbbi_symbol_table;
    pub(crate) mod rbbi_table_builder;
    pub mod replaceable;
    pub mod replaceable_string;
    pub mod rule_based_break_iterator;
    pub mod string_transform;
    pub mod transform;
    pub mod u_character_iterator;
    pub mod u_forward_character_iterator;
    pub mod utf16;
    pub mod filtered_normalizer2;
    pub mod normalizer;
    pub mod normalizer2;
    pub mod symbol_table;
    pub mod unicode_filter;
    pub mod unicode_matcher;
    pub mod unicode_set;
    pub mod unicode_set_iterator;
    pub(crate) mod any_transliterator;
    pub(crate) mod break_transliterator;
    pub(crate) mod case_fold_transliterator;
    pub(crate) mod compound_transliterator;
    pub(crate) mod escape_transliterator;
    pub(crate) mod function_replacer;
    pub(crate) mod lowercase_transliterator;
    pub(crate) mod name_unicode_transliterator;
    pub(crate) mod normalization_transliterator;
    pub(crate) mod null_transliterator;
    pub(crate) mod quantifier;
    pub(crate) mod remove_transliterator;
    pub(crate) mod replaceable_context_iterator;
    pub mod rule_based_transliterator;
    pub(crate) mod source_target_utility;
    pub(crate) mod string_matcher;
    pub(crate) mod string_replacer;
    pub(crate) mod titlecase_transliterator;
    pub(crate) mod transliteration_rule;
    pub(crate) mod transliteration_rule_set;
    pub mod transliterator;
    pub(crate) mod transliterator_id_parser;
    pub(crate) mod transliterator_parser;
    pub(crate) mod transliterator_registry;
    pub(crate) mod unescape_transliterator;
    pub(crate) mod unicode_name_transliterator;
    pub(crate) mod unicode_replacer;
    pub(crate) mod uppercase_transliterator;
    pub mod canonical_iterator;
    pub mod case_map;
    pub mod currency_display_names;
    pub mod display_context;
    pub mod locale_display_names;
    pub mod collation_element_iterator;
    pub mod collation_key;
    pub mod collator;
    pub(crate) mod collator_service_shim;
    pub mod raw_collation_key;
    pub mod rule_based_collator;
}

/// `com.ibm.icu.util`.
pub mod util {
    pub mod case_insensitive_string;
    pub mod byte_array_wrapper;
    pub mod bytes_trie;
    pub mod chars_trie;
    pub mod chars_trie_builder;
    pub mod code_point_map;
    pub mod code_point_trie;
    pub mod icu_clone_not_supported_exception;
    pub mod icu_exception;
    pub mod icu_unchecked_io_exception;
    pub mod illformed_locale_exception;
    pub mod locale_data;
    pub mod locale_matcher;
    pub mod locale_priority_list;
    pub mod mutable_code_point_trie;
    pub mod output;
    pub mod output_int;
    pub mod region;
    pub mod string_trie_builder;
    pub mod u_locale;
    pub mod u_resource_bundle;
    pub mod u_resource_bundle_iterator;
    pub mod u_resource_type_mismatch_exception;
    pub mod version_info;
    pub mod freezable;
    pub mod range_value_iterator;
    pub mod value_iterator;
    pub mod icu_input_too_long_exception;
}
