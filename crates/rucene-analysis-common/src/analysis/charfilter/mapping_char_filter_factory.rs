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

//! Factory for `MappingCharFilter`, ported from
//! `org.apache.lucene.analysis.charfilter.MappingCharFilterFactory`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, SPINameField,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::CharReader;
use rucene::error::{LuceneError, Result};
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::mapping_char_filter::MappingCharFilter;
use super::normalize_char_map::{Builder, NormalizeCharMap};
use crate::spi::check_unknown_parameters;

/// "source" => "target"
///
/// Equivalent to the private `MappingCharFilterFactory.p`.
static P: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile("\"(.*)\"\\s*=>\\s*\"(.*)\"\\s*$", 0)
        .expect("INVARIANT: the pattern is valid")
});

/// The length of `MappingCharFilterFactory.out`, `new char[256]`.
const OUT_LENGTH: usize = 256;

/// `String.trim()`: the characters up to U+0020 removed from both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Factory for `MappingCharFilter`.
///
/// ```text
/// <fieldType name="text_map" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <charFilter class="solr.MappingCharFilterFactory" mapping="mapping.txt"/>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.MappingCharFilterFactory`
/// (since Solr 1.4), a `ResourceLoaderAware` factory.
#[derive(Debug)]
pub struct MappingCharFilterFactory {
    base: AbstractAnalysisFactory,
    /// Java's protected `normMap`, `null` as `None`.
    norm_map: Option<Arc<NormalizeCharMap>>,
    mapping: Option<String>,
    /// Java's package-private `char[] out = new char[256]`, the scratch of
    /// [`parse_string`](Self::parse_string).
    out: Vec<u16>,
}

impl MappingCharFilterFactory {
    /// SPI name
    ///
    /// Equivalent to `MappingCharFilterFactory.NAME`.
    pub const NAME: &'static str = "mapping";

    /// Creates a new MappingCharFilterFactory
    ///
    /// Equivalent to `MappingCharFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mapping = AbstractAnalysisFactory::get(args, "mapping");
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            norm_map: None,
            mapping,
            out: vec![0; OUT_LENGTH],
        })
    }

    /// Default ctor for compatibility with SPI
    ///
    /// Equivalent to `MappingCharFilterFactory()`.
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

    /// Equivalent to the protected field `MappingCharFilterFactory.normMap`.
    pub fn norm_map(&self) -> Option<&Arc<NormalizeCharMap>> {
        self.norm_map.as_ref()
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.CharFilterFactory`
    /// names it: `MappingCharFilterFactory.class`, with its `NAME` field and
    /// its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn CharFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.charfilter.MappingCharFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn CharFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }

    /// Equivalent to the protected `MappingCharFilterFactory.parseRules(List<String>,
    /// NormalizeCharMap.Builder)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `Invalid Mapping Rule : [<rule>], file =
    /// <mapping>`, for a rule the pattern does not find, and what
    /// [`parse_string`](Self::parse_string) and [`Builder::add_utf16`] return.
    pub fn parse_rules(&mut self, rules: &[String], builder: &mut Builder) -> Result<()> {
        for rule in rules {
            let Some(groups) = P.find_groups(rule) else {
                return Err(LuceneError::illegal_argument(format!(
                    "Invalid Mapping Rule : [{rule}], file = {}",
                    self.mapping.as_deref().unwrap_or("null")
                )));
            };
            let group = |i: usize| groups[i].clone().unwrap_or_default();
            let source = self.parse_string(&group(1))?;
            let target = self.parse_string(&group(2))?;
            builder.add_utf16(&source, &target)?;
        }
        Ok(())
    }

    /// Equivalent to the protected `MappingCharFilterFactory.parseString(String)`:
    /// the UTF-16 code units of the Java string it returns, which may hold a
    /// lone surrogate.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `Invalid escaped char in [<s>]`, for a
    /// trailing `\` or a `\u` without four units after it; Java's
    /// `NumberFormatException` for a `\u` escape that is not hexadecimal; and
    /// Java's `ArrayIndexOutOfBoundsException` for a string longer than the
    /// 256 units of the `out` buffer.
    pub fn parse_string(&mut self, s: &str) -> Result<Vec<u16>> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut read_pos = 0;
        let len = units.len();
        let mut write_pos = 0;
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
                    0x22 /* '"' */ => c = u16::from(b'"'),
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
            if write_pos == self.out.len() {
                return Err(LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!(
                        "Index {write_pos} out of bounds for length {}",
                        self.out.len()
                    ),
                ));
            }
            self.out[write_pos] = c;
            write_pos += 1;
        }
        Ok(self.out[..write_pos].to_vec())
    }
}

impl ResourceLoaderAware for MappingCharFilterFactory {
    /// Equivalent to `MappingCharFilterFactory.inform(ResourceLoader)`: the
    /// rules of every file `mapping` names; a map that accepts nothing (an
    /// empty file) becomes no map at all.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if let Some(mapping) = self.mapping.clone() {
            let files = AbstractAnalysisFactory::split_file_names(Some(&mapping));
            let mut wlist: Vec<String> = Vec::new();
            for file in &files {
                let lines = AbstractAnalysisFactory::get_lines(loader, java_trim(file))?;
                wlist.extend(lines);
            }
            let mut builder = Builder::new();
            self.parse_rules(&wlist, &mut builder)?;
            let norm_map = builder.build()?;
            if norm_map.map.is_none() {
                // if the inner FST is null, it means it accepts nothing (e.g.
                // the file is empty) so just set the whole map to null
                self.norm_map = None;
            } else {
                self.norm_map = Some(Arc::new(norm_map));
            }
        }
        Ok(())
    }
}

impl CharFilterFactory for MappingCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.charfilter.MappingCharFilterFactory"
    }

    /// Equivalent to `MappingCharFilterFactory.create(Reader)`.
    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        // if the map is null, it means there's actually no mappings... just
        // return the original stream as there is nothing to do here.
        match &self.norm_map {
            None => Ok(input),
            Some(norm_map) => Ok(Box::new(MappingCharFilter::new(
                Arc::clone(norm_map),
                input,
            ))),
        }
    }

    /// Equivalent to `MappingCharFilterFactory.normalize(Reader)`,
    /// `create(input)`.
    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        self.create(input)
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
