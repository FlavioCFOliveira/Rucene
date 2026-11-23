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
    pub mod icu_data;
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
        pub mod locale_extensions;
        pub mod locale_object_cache;
        pub mod locale_syntax_exception;
        pub mod lsr;
        pub mod parse_status;
        pub mod string_token_iterator;
        pub mod unicode_locale_extension;
    }
    pub mod locale_fallback_data;
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
}
