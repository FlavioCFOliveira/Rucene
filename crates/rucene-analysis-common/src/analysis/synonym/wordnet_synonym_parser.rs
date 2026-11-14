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

//! Parser for wordnet prolog format, ported from
//! `org.apache.lucene.analysis.synonym.WordnetSynonymParser`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, CharReader};
use rucene::error::{LuceneError, Result};
use rucene::util::chars_ref::CharsRef;
use rucene::util::refs::CharsRefBuilder;

use super::solr_synonym_parser::parse_lines;
use super::synonym_map::{Builder, Parser};
use crate::jdk::io::LineNumberReader;

/// `String.substring(int, int)`, failing as Java's
/// `StringIndexOutOfBoundsException` does.
fn substring(s: &[u16], begin: usize, end: usize) -> Result<&[u16]> {
    if begin > end || end > s.len() {
        return Err(LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin {begin}, end {end}, length {}", s.len()),
        ));
    }
    Ok(&s[begin..end])
}

// TODO: allow you to specify syntactic categories (e.g. just nouns, etc)
/// Parser for wordnet prolog format
///
/// See <https://wordnet.princeton.edu/documentation/prologdb5wn> for a
/// description of the format.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.WordnetSynonymParser`
/// (`@lucene.experimental`), which extends `SynonymMap.Parser`. Lines are
/// read as UTF-16 units, Java's strings.
#[derive(Debug)]
pub struct WordnetSynonymParser {
    builder: Builder,
    analyzer: Arc<dyn Analyzer>,
    expand: bool,
}

impl WordnetSynonymParser {
    /// Equivalent to `WordnetSynonymParser(boolean, boolean, Analyzer)`.
    pub fn new(dedup: bool, expand: bool, analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            builder: Builder::with_dedup(dedup),
            analyzer,
            expand,
        }
    }

    /// The body of `parse(Reader)`'s `try` block.
    fn parse_synsets(&mut self, br: &mut LineNumberReader<&mut dyn CharReader>) -> Result<()> {
        let mut last_syn_set_id: Vec<u16> = Vec::new();
        // `ArrayUtil.grow(synset, synsetSize + 1)` over `new CharsRef[8]`:
        // the slots past `synset_size` are never read.
        let mut synset: Vec<CharsRef> = Vec::with_capacity(8);
        let mut synset_size = 0;

        while let Some(line) = br.read_line()? {
            let syn_set_id = substring(&line, 2, 11)?.to_vec();

            if syn_set_id != last_syn_set_id {
                self.add_internal(&synset, synset_size)?;
                synset_size = 0;
            }

            let synonym = self.parse_synonym(&line, &mut CharsRefBuilder::new())?;
            if synset_size < synset.len() {
                synset[synset_size] = synonym;
            } else {
                synset.push(synonym);
            }
            synset_size += 1;
            last_syn_set_id = syn_set_id;
        }

        // final synset in the file
        self.add_internal(&synset, synset_size)
    }

    /// Equivalent to `parseSynonym(String, CharsRefBuilder)`: the text
    /// between the first and the last `'` of the line, with `''` read as
    /// `'`, analyzed.
    fn parse_synonym(&self, line: &[u16], reuse: &mut CharsRefBuilder) -> Result<CharsRef> {
        let quote = u16::from(b'\'');
        let start = line
            .iter()
            .position(|&u| u == quote)
            .map_or(0, |index| index + 1);
        let end = line.iter().rposition(|&u| u == quote);
        // `line.lastIndexOf('\'')` is -1 without a quote, which `substring`
        // rejects as `begin 0, end -1`.
        let Some(end) = end else {
            return Err(LuceneError::unchecked(
                "java.lang.StringIndexOutOfBoundsException",
                format!("begin {start}, end -1, length {}", line.len()),
            ));
        };

        let quoted = substring(line, start, end)?;
        // `.replace("''", "'")`
        let mut text: Vec<u16> = Vec::with_capacity(quoted.len());
        let mut i = 0;
        while i < quoted.len() {
            if quoted[i] == quote && i + 1 < quoted.len() && quoted[i + 1] == quote {
                text.push(quote);
                i += 2;
            } else {
                text.push(quoted[i]);
                i += 1;
            }
        }
        self.analyze(&text, reuse)
    }

    /// Equivalent to `addInternal(CharsRef[], int)`.
    fn add_internal(&mut self, synset: &[CharsRef], size: usize) -> Result<()> {
        if size <= 1 {
            return Ok(()); // nothing to do
        }

        if self.expand {
            for i in 0..size {
                for j in 0..size {
                    if i != j {
                        self.builder.add(&synset[i], &synset[j], true)?;
                    }
                }
            }
        } else {
            for item in &synset[..size] {
                self.builder.add(item, &synset[0], false)?;
            }
        }
        Ok(())
    }
}

impl Parser for WordnetSynonymParser {
    fn builder(&self) -> &Builder {
        &self.builder
    }

    fn builder_mut(&mut self) -> &mut Builder {
        &mut self.builder
    }

    fn analyzer(&self) -> &dyn Analyzer {
        self.analyzer.as_ref()
    }

    /// Equivalent to `WordnetSynonymParser.parse(Reader)`: the lines of a
    /// `LineNumberReader` over `input`, which is closed at the end; a
    /// synset is the run of lines whose units 2 to 10 (the synset id) are
    /// equal, and an `IllegalArgumentException` becomes
    /// `java.text.ParseException("Invalid synonym rule at line <n>")` with
    /// that cause.
    fn parse(&mut self, input: &mut dyn CharReader) -> Result<()> {
        parse_lines(input, |br| self.parse_synsets(br))
    }
}
