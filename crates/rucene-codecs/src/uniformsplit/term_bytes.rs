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

//! Port of `org.apache.lucene.codecs.uniformsplit.TermBytes`.

use rucene::error::Result;
use rucene::util::string_helper::StringHelper;
use rucene::util::{Accountable, BytesRef, RamUsageEstimator};

use super::ram_usage_util;

/// Term of a block line.
///
/// Contains the term bytes and the minimal distinguishing prefix (MDP) length
/// of this term.
///
/// The MDP is the minimal prefix that distinguishes a term from its immediate
/// previous term (terms are alphabetically sorted).
///
/// The incremental encoding suffix is the suffix starting at the last byte of
/// the MDP (inclusive).
///
/// Example: For the block
///
/// ```text
/// client
/// color
/// company
/// companies
/// ```
///
/// "color" - MDP is "co" - incremental encoding suffix is "olor".
/// "company" - MDP is "com" - incremental encoding suffix is "mpany".
/// "companies" - MDP is "compani" - incremental encoding suffix is "ies".
///
/// Port of `org.apache.lucene.codecs.uniformsplit.TermBytes`
/// (`@lucene.experimental`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TermBytes {
    pub(crate) mdp_length: i32,
    pub(crate) term: BytesRef,
}

impl TermBytes {
    /// Port of `TermBytes(int, BytesRef)`.
    pub fn new(mdp_length: i32, term: BytesRef) -> Self {
        let mut t = Self::default();
        t.reset(mdp_length, term);
        t
    }

    /// Port of `TermBytes.reset(int, BytesRef)`.
    pub fn reset(&mut self, mdp_length: i32, term: BytesRef) -> &mut Self {
        debug_assert!(
            term.length > 0 && mdp_length > 0 || term.length == 0 && mdp_length == 0,
            "Inconsistent mdpLength={mdp_length}, term.length={}",
            term.length
        );
        debug_assert!(
            term.length == 0 || mdp_length as usize <= term.length,
            "Too large mdpLength={mdp_length}, term.length={}",
            term.length
        );
        debug_assert!(term.offset == 0);
        self.mdp_length = mdp_length;
        self.term = term;
        self
    }

    /// This term MDP length.
    pub fn get_mdp_length(&self) -> i32 {
        self.mdp_length
    }

    /// This term bytes.
    pub fn get_term(&self) -> &BytesRef {
        &self.term
    }

    /// The offset of this term incremental encoding suffix.
    pub fn get_suffix_offset(&self) -> i32 {
        suffix_offset(self.mdp_length)
    }

    /// The length of this term incremental encoding suffix.
    pub fn get_suffix_length(&self) -> i32 {
        self.term.length as i32 - self.get_suffix_offset()
    }

    /// Computes the length of the minimal distinguishing prefix (MDP) between
    /// a current term and its previous term (terms are alphabetically
    /// sorted).
    ///
    /// Example: If previous="car" and current="cartridge", then MDP length is
    /// 4. It is the length of the minimal prefix distinguishing "cartridge"
    /// from "car", that is, the length of "cart".
    ///
    /// Port of `TermBytes.computeMdpLength(BytesRef, BytesRef)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` of `StringHelper.sortKeyLength` for terms
    /// out of order.
    pub fn compute_mdp_length(
        previous_term: Option<&BytesRef>,
        current_term: &BytesRef,
    ) -> Result<i32> {
        let mdp_length = match previous_term {
            None => 1,
            Some(previous) => StringHelper::sort_key_length(previous, current_term)?,
        };
        Ok(mdp_length.min(current_term.length as i32))
    }
}

/// `Math.max(mdpLength - 1, 0)`, `getSuffixOffset()` of a term whose MDP
/// length is `mdp_length`.
pub(crate) fn suffix_offset(mdp_length: i32) -> i32 {
    (mdp_length - 1).max(0)
}

impl Accountable for TermBytes {
    /// Port of `TermBytes.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<TermBytes>()
            + ram_usage_util::ram_bytes_used_bytes_ref(&self.term)
    }
}

/// Port of `org.apache.lucene.codecs.uniformsplit.TestTermBytes` (Lucene's
/// `lucene/codecs` tests, 10.5.0), on Lucene's own vocabularies.
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn to_map(src: &[[&str; 2]]) -> HashMap<String, String> {
        assert!(!src.is_empty(), "insert at least one row");
        src.iter()
            .map(|[k, v]| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sorted_keys(vocab: &HashMap<String, String>) -> Vec<BytesRef> {
        let mut keys: Vec<&String> = vocab.keys().collect();
        keys.sort();
        keys.into_iter()
            .map(|k| BytesRef::new(k.as_bytes().to_vec()))
            .collect()
    }

    fn compress_prefixes(vocab: &[BytesRef]) -> Vec<TermBytes> {
        let mut term_bytes = Vec::with_capacity(vocab.len());
        let mut last: Option<&BytesRef> = None;
        for current in vocab {
            let mdp = TermBytes::compute_mdp_length(last, current).unwrap();
            term_bytes.push(TermBytes::new(mdp, current.clone()));
            last = Some(current);
        }
        term_bytes
    }

    fn suffix_bytes(t: &TermBytes) -> &[u8] {
        let offset = t.get_suffix_offset() as usize;
        &t.get_term().bytes[offset..offset + t.get_suffix_length() as usize]
    }

    fn validate_map_list(expected: &HashMap<String, String>, src: &[BytesRef], result: &[String]) {
        for (i, s) in src.iter().enumerate() {
            let key = String::from_utf8(s.slice().to_vec()).unwrap();
            assert_eq!(expected[&key], result[i], "Error in line {i}");
        }
    }

    fn validate_incremental_decoding(vocab: &HashMap<String, String>) {
        let src = sorted_keys(vocab);
        let output = compress_prefixes(&src);
        let mut previous: Vec<u8> = Vec::new();
        for (i, t) in output.iter().enumerate() {
            let offset = t.get_suffix_offset() as usize;
            previous.truncate(offset);
            previous.resize(offset, 0);
            previous.extend_from_slice(suffix_bytes(t));
            assert_eq!(previous, src[i].slice(), "Error in line {i}");
        }
    }

    fn validate_expected_suffix(vocab: &[[&str; 2]]) {
        let vocab = to_map(vocab);
        let src = sorted_keys(&vocab);
        let output = compress_prefixes(&src);
        let result: Vec<String> = output
            .iter()
            .map(|e| {
                format!(
                    "{}{}",
                    e.get_suffix_offset(),
                    String::from_utf8(suffix_bytes(e).to_vec()).unwrap()
                )
            })
            .collect();
        validate_map_list(&vocab, &src, &result);
        validate_incremental_decoding(&vocab);
    }

    fn validate_expected_mdp(vocab: &[[&str; 2]]) {
        let vocab = to_map(vocab);
        let src = sorted_keys(&vocab);
        let output = compress_prefixes(&src);
        let result: Vec<String> = output
            .iter()
            .map(|e| {
                String::from_utf8(e.get_term().bytes[..e.get_mdp_length() as usize].to_vec())
                    .unwrap()
            })
            .collect();
        validate_map_list(&vocab, &src, &result);
        validate_incremental_decoding(&vocab);
    }

    #[test]
    fn test_mdp_a() {
        validate_expected_mdp(&[
            ["aa", "a"],
            ["abbreviator", "ab"],
            ["abidingly", "abi"],
            ["aboiteaus", "abo"],
            ["abranchiates", "abr"],
            ["absentminded", "abs"],
        ]);
    }

    #[test]
    fn test_incremental_a() {
        validate_expected_suffix(&[
            ["aa", "0aa"],
            ["abbreviator", "1bbreviator"],
            ["abidingly", "2idingly"],
            ["aboiteaus", "2oiteaus"],
            ["abranchiates", "2ranchiates"],
            ["absentminded", "2sentminded"],
            ["rodriguez", "0rodriguez"],
            ["romero", "2mero"],
        ]);
    }

    #[test]
    fn test_mdp_mix2() {
        validate_expected_mdp(&[
            ["abaco", "a"],
            ["amigo", "am"],
            ["bloom", "b"],
            ["break", "br"],
            ["can", "c"],
            ["car", "car"],
            ["carmagedon", "carm"],
            ["danger", "d"],
            ["lala", "l"],
            ["literature", "li"],
            ["lucene", "lu"],
            ["nature", "n"],
            ["naval", "nav"],
            ["rico", "r"],
            ["weird", "w"],
            ["zoo", "z"],
        ]);
    }

    #[test]
    fn test_mdp() {
        validate_expected_mdp(&[
            ["abaco", "a"],
            ["amigo", "am"],
            ["arco", "ar"],
            ["bloom", "b"],
            ["frien", "f"],
            ["frienchies", "frienc"],
            ["friend", "friend"],
            ["friendalan", "frienda"],
            ["friende", "friende"],
        ]);
    }

    #[test]
    fn test_incremental() {
        validate_expected_suffix(&[
            ["abaco", "0abaco"],
            ["amigo", "1migo"],
            ["arco", "1rco"],
            ["bloom", "0bloom"],
            ["frien", "0frien"],
            ["frienchies", "5chies"],
            ["friend", "5d"],
            ["friendalan", "6alan"],
            ["friende", "6e"],
        ]);
    }

    #[test]
    fn test_incremental_simple() {
        validate_expected_suffix(&[
            ["abaco", "0abaco"],
            ["rodriguez", "0rodriguez"],
            ["roma", "2ma"],
            ["romero", "3ero"],
        ]);
    }

    #[test]
    fn test_mdp_simple() {
        validate_expected_mdp(&[["abaco", "a"], ["rodriguez", "r"], ["romero", "rom"]]);
    }

    #[test]
    fn test_mdp_mix() {
        validate_expected_mdp(&[
            ["aaab", "a"],
            ["arco", "ar"],
            ["busqueda", "b"],
            ["trabajo", "t"],
            ["zufix", "z"],
            ["zzfix", "zz"],
        ]);
    }
}
