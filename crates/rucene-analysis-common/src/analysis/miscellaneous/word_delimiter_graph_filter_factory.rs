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
//! Factory for `WordDelimiterGraphFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilterFactory`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::{CharArraySet, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::word_delimiter_graph_filter::{
    new_word_delimiter_graph_filter_with, CATENATE_ALL, CATENATE_NUMBERS, CATENATE_WORDS,
    GENERATE_NUMBER_PARTS, GENERATE_WORD_PARTS, IGNORE_KEYWORDS, PRESERVE_ORIGINAL,
    SPLIT_ON_CASE_CHANGE, SPLIT_ON_NUMERICS, STEM_ENGLISH_POSSESSIVE,
};
use super::word_delimiter_iterator::{
    get_type, ALPHA, ALPHANUM, DEFAULT_WORD_DELIM_TABLE, DIGIT, LOWER, SUBWORD_DELIM, UPPER,
};
use crate::spi::check_unknown_parameters;

/// source => type
///
/// Equivalent to the private `WordDelimiterGraphFilterFactory.TYPE_PATTERN`,
/// `Pattern.compile("(.*)\\s*=>\\s*(.*)\\s*$")`.
static TYPE_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(r"(.*)\s*=>\s*(.*)\s*$", 0).expect("INVARIANT: the pattern is valid")
});

/// `String.trim()`: the characters up to U+0020 removed from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Parses a list of MappingCharFilter style rules into a custom byte[] type
/// table.
///
/// Equivalent to the private `parseTypes(List<String>)` that
/// `WordDelimiterGraphFilterFactory` and `WordDelimiterFilterFactory` each
/// declare, identically; the port keeps one copy for both.
///
/// # Errors
///
/// `IllegalArgumentException` with Java's messages for a rule that does not
/// match, that maps more than one character, or that names an unknown type;
/// `NoSuchElementException` when there are no rules (Java's
/// `typeMap.lastKey()`); what [`parse_string`] raises.
pub(super) fn parse_types(rules: &[String]) -> Result<Arc<[u8]>> {
    // Java's `SortedMap<Character, Byte>`.
    let mut type_map: BTreeMap<u16, u8> = BTreeMap::new();
    for rule in rules {
        let Some(groups) = TYPE_PATTERN.find_groups(rule) else {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid Mapping Rule : [{rule}]"
            )));
        };
        let group = |i: usize| groups[i].clone().unwrap_or_default();
        let lhs = parse_string(java_trim(&group(1)))?;
        let rhs = parse_type(java_trim(&group(2)));
        if lhs.len() != 1 {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid Mapping Rule : [{rule}]. Only a single character is allowed."
            )));
        }
        let Some(rhs) = rhs else {
            return Err(LuceneError::illegal_argument(format!(
                "Invalid Mapping Rule : [{rule}]. Illegal type."
            )));
        };
        type_map.insert(lhs[0], rhs);
    }

    // ensure the table is always at least as big as DEFAULT_WORD_DELIM_TABLE for performance
    let Some((&last_key, _)) = type_map.last_key_value() else {
        return Err(LuceneError::unchecked(
            "java.util.NoSuchElementException",
            String::new(),
        ));
    };
    let mut types = vec![0u8; (usize::from(last_key) + 1).max(DEFAULT_WORD_DELIM_TABLE.len())];
    for (i, slot) in types.iter_mut().enumerate() {
        *slot = get_type(i as i32);
    }
    for (key, value) in type_map {
        types[usize::from(key)] = value;
    }
    Ok(Arc::from(types))
}

/// Equivalent to the private `parseType(String)` of both word delimiter
/// factories: `None` is Java's `null`.
fn parse_type(s: &str) -> Option<u8> {
    match s {
        "LOWER" => Some(LOWER),
        "UPPER" => Some(UPPER),
        "ALPHA" => Some(ALPHA),
        "DIGIT" => Some(DIGIT),
        "ALPHANUM" => Some(ALPHANUM),
        "SUBWORD_DELIM" => Some(SUBWORD_DELIM),
        _ => None,
    }
}

/// The length of Java's `char[] out = new char[256]`, the buffer
/// `parseString` writes into.
const OUT_LENGTH: usize = 256;

/// Equivalent to the private `parseString(String)` of both word delimiter
/// factories: the escapes `\\`, `\n`, `\t`, `\r`, `\b`, `\f` and `\uXXXX`
/// are replaced; the result is UTF-16 units, Java's `String`.
///
/// # Errors
///
/// `IllegalArgumentException` "Invalid escaped char in [s]" for a trailing
/// `\` or a short `\u` escape, the `NumberFormatException` of
/// `Integer.parseInt(hex, 16)` for a malformed one, and the
/// `ArrayIndexOutOfBoundsException` Java's 256-char buffer throws for a
/// longer result.
pub(super) fn parse_string(s: &str) -> Result<Vec<u16>> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut read_pos = 0;
    let len = units.len();
    let mut out: Vec<u16> = Vec::new();
    let invalid = || LuceneError::illegal_argument(format!("Invalid escaped char in [{s}]"));
    while read_pos < len {
        let mut c = units[read_pos];
        read_pos += 1;
        if c == u16::from(b'\\') {
            if read_pos >= len {
                return Err(invalid());
            }
            c = units[read_pos];
            read_pos += 1;
            match c {
                0x5C /* '\\' */ => c = u16::from(b'\\'),
                0x6E /* 'n' */ => c = u16::from(b'\n'),
                0x74 /* 't' */ => c = u16::from(b'\t'),
                0x72 /* 'r' */ => c = u16::from(b'\r'),
                0x62 /* 'b' */ => c = 0x08,
                0x66 /* 'f' */ => c = 0x0C,
                0x75 /* 'u' */ => {
                    if read_pos + 3 >= len {
                        return Err(invalid());
                    }
                    let hex = String::from_utf16_lossy(&units[read_pos..read_pos + 4]);
                    let value = rucene::util::extra::java_parse_long_radix(&hex, 16)
                        .and_then(|v| i32::try_from(v).ok())
                        .ok_or_else(|| LuceneError::number_format_for_input_string(&hex, 16))?;
                    c = value as u16;
                    read_pos += 4;
                }
                _ => {}
            }
        }
        if out.len() == OUT_LENGTH {
            return Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {OUT_LENGTH} out of bounds for length {OUT_LENGTH}"),
            ));
        }
        out.push(c);
    }
    Ok(out)
}

/// Factory for `WordDelimiterGraphFilter`.
///
/// ```text
/// <fieldType name="text_wd" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.WordDelimiterGraphFilterFactory" protected="protectedword.txt"
///             preserveOriginal="0" splitOnNumerics="1" splitOnCaseChange="1"
///             catenateWords="0" catenateNumbers="0" catenateAll="0"
///             generateWordParts="1" generateNumberParts="1" stemEnglishPossessive="1"
///             types="wdfftypes.txt" ignoreKeywords="0" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilterFactory`
/// (since 6.5.0), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct WordDelimiterGraphFilterFactory {
    base: AbstractAnalysisFactory,
    word_files: Option<String>,
    types: Option<String>,
    flags: i32,
    type_table: Option<Arc<[u8]>>,
    protected_words: Option<CharArraySet>,
    adjust_offsets: bool,
}

impl WordDelimiterGraphFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `WordDelimiterGraphFilterFactory.NAME`.
    pub const NAME: &'static str = "wordDelimiterGraph";

    /// Equivalent to `WordDelimiterGraphFilterFactory.PROTECTED_TOKENS`.
    pub const PROTECTED_TOKENS: &'static str = "protected";

    /// Equivalent to `WordDelimiterGraphFilterFactory.TYPES`.
    pub const TYPES: &'static str = "types";

    /// Equivalent to `WordDelimiterGraphFilterFactory.OFFSETS`.
    pub const OFFSETS: &'static str = "adjustOffsets";

    /// Creates a new WordDelimiterGraphFilterFactory.
    ///
    /// Equivalent to `WordDelimiterGraphFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what an
    /// unparsable flag raises, and `IllegalArgumentException` for a leftover
    /// argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut flags = 0;
        let mut flag = |name: &str, default_val: i32, bit: i32| -> Result<()> {
            if AbstractAnalysisFactory::get_int(args, name, default_val)? != 0 {
                flags |= bit;
            }
            Ok(())
        };
        flag("generateWordParts", 1, GENERATE_WORD_PARTS)?;
        flag("generateNumberParts", 1, GENERATE_NUMBER_PARTS)?;
        flag("catenateWords", 0, CATENATE_WORDS)?;
        flag("catenateNumbers", 0, CATENATE_NUMBERS)?;
        flag("catenateAll", 0, CATENATE_ALL)?;
        flag("splitOnCaseChange", 1, SPLIT_ON_CASE_CHANGE)?;
        flag("splitOnNumerics", 1, SPLIT_ON_NUMERICS)?;
        flag("preserveOriginal", 0, PRESERVE_ORIGINAL)?;
        flag("stemEnglishPossessive", 1, STEM_ENGLISH_POSSESSIVE)?;
        flag("ignoreKeywords", 0, IGNORE_KEYWORDS)?;
        let word_files = AbstractAnalysisFactory::get(args, Self::PROTECTED_TOKENS);
        let types = AbstractAnalysisFactory::get(args, Self::TYPES);
        let adjust_offsets = AbstractAnalysisFactory::get_boolean(args, Self::OFFSETS, true);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            word_files,
            types,
            flags,
            type_table: None,
            protected_words: None,
            adjust_offsets,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `WordDelimiterGraphFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `WordDelimiterGraphFilterFactory.class`, with its `NAME`
    /// field and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl ResourceLoaderAware for WordDelimiterGraphFilterFactory {
    /// Equivalent to `WordDelimiterGraphFilterFactory.inform(ResourceLoader)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(word_files) = &self.word_files {
            self.protected_words =
                AbstractAnalysisFactory::get_word_set(loader, Some(word_files), false)?;
        }
        if let Some(types) = &self.types {
            let files = AbstractAnalysisFactory::split_file_names(Some(types));
            let mut wlist = Vec::new();
            for file in &files {
                let lines = AbstractAnalysisFactory::get_lines(loader, file.trim())?;
                wlist.extend(lines);
            }
            self.type_table = Some(parse_types(&wlist)?);
        }
        Ok(())
    }
}

impl TokenFilterFactory for WordDelimiterGraphFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilterFactory"
    }

    /// Equivalent to `WordDelimiterGraphFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let type_table = self
            .type_table
            .clone()
            .unwrap_or_else(|| Arc::clone(&DEFAULT_WORD_DELIM_TABLE));
        Ok(Box::new(new_word_delimiter_graph_filter_with(
            input,
            self.adjust_offsets,
            type_table,
            self.flags,
            self.protected_words.clone(),
        )?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
