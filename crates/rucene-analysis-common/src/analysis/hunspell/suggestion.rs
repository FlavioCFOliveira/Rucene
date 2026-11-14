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

//! A suggestion with its presentable forms, ported from
//! `org.apache.lucene.analysis.hunspell.Suggestion`.

use std::fmt;

use rucene::error::Result;

use super::hunspell::Hunspell;
use super::word_case::WordCase;
use crate::jdk::character::{is_upper_case, to_upper_case_root, to_upper_case_unit};
use crate::jdk::string::{contains, to_rust_string, units};

/// A suggested word (`raw`) and the forms it is presented in (`result`).
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Suggestion`: equal when
/// both the raw word and the results are.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Suggestion {
    pub(crate) raw: Vec<u16>,
    pub(crate) result: Vec<Vec<u16>>,
}

impl Suggestion {
    /// Equivalent to `Suggestion(String, String, WordCase, Hunspell)`.
    ///
    /// # Errors
    ///
    /// What spelling the adjusted candidate raises.
    pub(crate) fn new(
        raw: Vec<u16>,
        misspelled: &[u16],
        original_case: WordCase,
        speller: &Hunspell,
    ) -> Result<Self> {
        let mut result = Vec::new();
        let adjusted = adjust_suggestion_case(&raw, misspelled, original_case);
        let accepted = contains(&adjusted, &units(" ")) || speller.spell_units(&adjusted)?;
        result.push(clean_output(
            speller,
            if accepted { adjusted } else { raw.clone() },
        )?);
        if original_case == WordCase::Upper
            && speller.dictionary.check_sharp_s
            && raw.contains(&0x00DF)
        {
            result.push(clean_output(speller, raw.clone())?);
        }
        Ok(Self { raw, result })
    }
}

/// Equivalent to `adjustSuggestionCase(String, String, WordCase)`.
fn adjust_suggestion_case(
    candidate: &[u16],
    misspelled: &[u16],
    original_case: WordCase,
) -> Vec<u16> {
    if original_case == WordCase::Upper {
        return to_upper_case_root(candidate);
    }
    if is_upper_case(i32::from(misspelled[0])) {
        let mut adjusted = Vec::with_capacity(candidate.len());
        adjusted.push(to_upper_case_unit(candidate[0]));
        adjusted.extend_from_slice(&candidate[1..]);
        return adjusted;
    }
    candidate.to_vec()
}

/// Equivalent to `cleanOutput(Hunspell, String)`.
fn clean_output(speller: &Hunspell, s: Vec<u16>) -> Result<Vec<u16>> {
    let Some(oconv) = &speller.dictionary.oconv else {
        return Ok(s);
    };
    let mut sb = s;
    oconv.apply_mappings(&mut sb)?;
    Ok(sb)
}

impl fmt::Display for Suggestion {
    /// Equivalent to `toString()`, the raw word.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_rust_string(&self.raw))
    }
}
