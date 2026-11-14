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

//! The in-memory structure of a hunspell dictionary, ported from
//! `org.apache.lucene.analysis.hunspell.Dictionary`.
//!
//! A Java `String` or `char[]` of the package is held as its UTF-16 units
//! (`Vec<u16>`, `&[u16]`), so that every position, length and comparison
//! counts and compares units as Java does; the public methods take and
//! return Rust strings.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Debug;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::store::Directory;
use rucene::util::extra::ParseException;
use rucene::util::fst::{
    Arc as FSTArc, Builder as FSTCompilerBuilder, BytesReader, InputType, IntSequenceOutputs,
    Outputs, Util, END_LABEL, FST,
};
use rucene::util::{IntsRef, IntsRefBuilder};

use super::affix_condition::{self, AffixCondition, ALWAYS_TRUE_KEY};
use super::affix_kind::AffixKind;
use super::check_compound_pattern::CheckCompoundPattern;
use super::compound_rule::CompoundRule;
use super::conv_table::ConvTable;
use super::dict_entries::DictEntries;
use super::dict_entry::{is_phonetic_field, DictEntry};
use super::flag_enumerator::{FlagEnumerator, Lookup};
use super::iso8859_14_decoder::ISO8859_14Decoder;
use super::rep_entry::RepEntry;
use super::sorting_strategy::{EntryAccumulator, EntrySupplier, SortingStrategy};
use super::word_case::WordCase;
use super::word_storage::{Builder as WordStorageBuilder, WordStorage};
use crate::jdk::character::{
    char_count, code_point_at, is_letter, is_whitespace, to_lower_case_unit, to_upper_case_unit,
};
use crate::jdk::charset::{Charset, CharsetDecoder, StandardCharset};
use crate::jdk::io::{
    BufferedReader, CodingErrorAction, InputStreamReader, LineNumberReader, ReadChars,
};
use crate::jdk::string::{
    equals_str, index_of_unit, is_blank, last_index_of_unit, parse_int, split_unit,
    split_whitespace, starts_with_str, strip, to_rust_string, trim, units,
};

/// Derived from woorm/LibreOffice dictionaries. See
/// `TestAllDictionaries.testMaxPrologueNeeded`.
///
/// Equivalent to `Dictionary.MAX_PROLOGUE_SCAN_WINDOW`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const MAX_PROLOGUE_SCAN_WINDOW: usize = 30 * 1024;

/// Equivalent to `Dictionary.NOFLAGS`.
pub(crate) const NOFLAGS: [u16; 0] = [];

/// Equivalent to `Dictionary.FLAG_UNSET`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const FLAG_UNSET: u16 = 0;

/// Equivalent to `Dictionary.DEFAULT_FLAGS`.
const DEFAULT_FLAGS: u16 = 65510;

/// Called 'ONLYUPCASEFLAG' in Hunspell.
///
/// Equivalent to `Dictionary.HIDDEN_FLAG`.
pub(crate) const HIDDEN_FLAG: u16 = 65511;

/// Offset of the flag in `affix_data`: `Dictionary.AFFIX_FLAG`.
pub(crate) const AFFIX_FLAG: usize = 0;
/// Offset of the strip ord in `affix_data`: `Dictionary.AFFIX_STRIP_ORD`.
pub(crate) const AFFIX_STRIP_ORD: usize = 1;
/// Offset of the condition in `affix_data`: `Dictionary.AFFIX_CONDITION`.
const AFFIX_CONDITION: usize = 2;
/// Offset of the append flags in `affix_data`: `Dictionary.AFFIX_APPEND`.
pub(crate) const AFFIX_APPEND: usize = 3;

/// Flag separator after escaping: `Dictionary.FLAG_SEPARATOR`.
const FLAG_SEPARATOR: u16 = 0x1f;
/// Separator for boundary of entry (may be followed by morph data):
/// `Dictionary.MORPH_SEPARATOR`.
const MORPH_SEPARATOR: u16 = 0x1e;

/// Equivalent to `Dictionary.BOM_UTF8`.
const BOM_UTF8: [u8; 3] = [0xef, 0xbb, 0xbf];

/// Equivalent to `Dictionary.CHARSET_ALIASES`.
const CHARSET_ALIASES: [(&str, &str); 2] = [
    ("microsoft-cp1251", "windows-1251"),
    ("TIS620-2533", "TIS-620"),
];

/// `new ArrayIndexOutOfBoundsException("Index <index> out of bounds for
/// length <length>")`, as an array access throws it.
pub(crate) fn array_index_out_of_bounds(index: i64, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// The protected methods a subclass of `Dictionary` overrides to tune
/// loading.
///
/// Java's `Dictionary` is subclassed to override `hashFactor()` and the
/// `tolerate*` methods, which its constructor calls; a Rust type has no
/// subclass, so the overriding object is passed to
/// [`Dictionary::with_subclass`]. Each default is Java's.
pub trait DictionarySubclass: Send + Sync {
    /// The factor determining the size of the internal hash table used for
    /// storing the entries. The table size is `entry_count * hashFactor`.
    /// The default factor is 1.0. If there are too many hash collisions, the
    /// factor can be increased, resulting in faster access, but more memory
    /// usage.
    ///
    /// Equivalent to `hashFactor()`.
    fn hash_factor(&self) -> f64 {
        1.0
    }

    /// Whether incorrect PFX/SFX rule counts should be silently ignored.
    /// False by default: a `ParseException` will happen.
    ///
    /// Equivalent to `tolerateAffixRuleCountMismatches()`.
    fn tolerate_affix_rule_count_mismatches(&self) -> bool {
        false
    }

    /// Whether incorrect REP rule counts will be silently ignored. False by
    /// default: a `ParseException` will happen.
    ///
    /// Equivalent to `tolerateRepRuleCountMismatches()`.
    fn tolerate_rep_rule_count_mismatches(&self) -> bool {
        false
    }

    /// Whether duplicate ICONV/OCONV lines should be silently ignored. False
    /// by default: an `IllegalStateException` will happen.
    ///
    /// Equivalent to `tolerateDuplicateConversionMappings()`.
    fn tolerate_duplicate_conversion_mappings(&self) -> bool {
        false
    }
}

/// `Dictionary` itself, which overrides nothing.
struct NoSubclass;

impl DictionarySubclass for NoSubclass {}

/// In-memory structure for the dictionary (.dic) and affix (.aff) data of a
/// hunspell dictionary.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary`.
pub struct Dictionary {
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub decoder: CharsetDecoder,

    pub(crate) prefixes: Option<FST<IntSequenceOutputs>>,
    pub(crate) suffixes: Option<FST<IntSequenceOutputs>>,
    pub(crate) breaks: Breaks,

    /// All condition checks used by prefixes and suffixes. these are
    /// typically re-used across many affix stripping rules. so these are
    /// deduplicated, to save RAM.
    pub(crate) patterns: Vec<Option<Arc<dyn AffixCondition>>>,

    /// The entries in the .dic file, mapping to their set of flags.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub words: WordStorage,

    /// The list of unique flagsets (wordforms). theoretically huge, but
    /// practically small (for Polish this is 756), otherwise humans wouldn't
    /// be able to deal with it either.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub flag_lookup: Lookup,

    // the list of unique strip affixes.
    pub(crate) strip_data: Vec<u16>,
    pub(crate) strip_offsets: Vec<i32>,

    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub word_chars: Vec<u16>,

    // 4 chars per affix, each char representing an unsigned 2-byte integer
    pub(crate) affix_data: Vec<u16>,
    current_affix: usize,

    pub(crate) flag_parsing_strategy: Arc<dyn FlagParsingStrategy>,

    // AF entries
    aliases: Option<Vec<Vec<u16>>>,
    alias_count: usize,

    // AM entries
    morph_aliases: Option<Vec<Vec<u16>>>,
    morph_alias_count: usize,

    /// Empty data at 0.
    pub(crate) morph_data: Vec<Vec<u16>>,

    /// We set this during sorting, so we know to add an extra int (index in
    /// `morph_data`) to FST output.
    pub(crate) has_custom_morph_data: bool,

    pub(crate) ignore_case: bool,
    pub(crate) check_sharp_s: bool,
    pub(crate) complex_prefixes: bool,

    /// All flags used in affix continuation classes. If an outer affix's
    /// flag isn't here, there's no need to do 2-level affix stripping with
    /// it.
    second_stage_prefix_flags: Vec<u16>,
    second_stage_suffix_flags: Vec<u16>,

    pub(crate) circumfix: u16,
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub keepcase: u16,
    pub(crate) force_u_case: u16,
    pub(crate) needaffix: u16,
    pub(crate) forbiddenword: u16,
    pub(crate) onlyincompound: u16,
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub compound_begin: u16,
    pub(crate) compound_middle: u16,
    pub(crate) compound_end: u16,
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub compound_flag: u16,
    pub(crate) compound_permit: u16,
    pub(crate) compound_forbid: u16,
    pub(crate) check_compound_case: bool,
    pub(crate) check_compound_dup: bool,
    pub(crate) check_compound_rep: bool,
    pub(crate) check_compound_triple: bool,
    pub(crate) simplified_triple: bool,
    pub(crate) compound_min: i32,
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub compound_max: i32,
    /// Nullable.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub compound_rules: Option<Vec<CompoundRule>>,
    pub(crate) check_compound_patterns: Vec<CheckCompoundPattern>,

    // ignored characters (dictionary, affix, inputs)
    ignore: Option<Vec<u16>>,

    pub(crate) try_chars: Vec<u16>,
    pub(crate) neighbor_key_groups: Vec<Vec<u16>>,
    pub(crate) enable_split_suggestions: bool,
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub rep_table: Vec<RepEntry>,
    pub(crate) map_table: Vec<Vec<Vec<u16>>>,
    pub(crate) max_diff: i32,
    pub(crate) max_n_gram_suggestions: i32,
    pub(crate) only_max_diff: bool,
    pub(crate) no_suggest: u16,
    pub(crate) sub_standard: u16,
    pub(crate) iconv: Option<ConvTable>,
    pub(crate) oconv: Option<ConvTable>,

    // true if we can strip suffixes "down to nothing"
    pub(crate) full_strip: bool,

    // language declaration of the dictionary
    pub(crate) language: Option<Vec<u16>>,
    // true if case algorithms should use alternate (Turkish/Azeri) mapping
    alternate_casing: bool,

    /// The overrides of the protected methods.
    subclass: Arc<dyn DictionarySubclass>,
}

impl Debug for Dictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dictionary")
            .field("ignore_case", &self.ignore_case)
            .field("current_affix", &self.current_affix)
            .field("has_custom_morph_data", &self.has_custom_morph_data)
            .finish_non_exhaustive()
    }
}

/// The inputs a constructor reads: Java's `InputStream`s, which it does not
/// close.
pub type InputStream<'a> = &'a mut dyn Read;

impl Dictionary {
    /// Creates a new Dictionary containing the information read from the
    /// provided InputStreams to hunspell affix and dictionary files. You have
    /// to close the provided InputStreams yourself.
    ///
    /// Equivalent to `Dictionary(Directory, String, InputStream,
    /// InputStream)`: `temp_dir` is the directory to use for offline sorting
    /// and `temp_file_name_prefix` the prefix of its temporary file names.
    ///
    /// # Errors
    ///
    /// What reading the streams raises (`IOException`), and a
    /// `java.text.ParseException` ([`ParseException`]) if the content of the
    /// files does not meet expected formats, with the other exceptions the
    /// Java constructor throws.
    pub fn new(
        temp_dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        affix: InputStream<'_>,
        dictionary: InputStream<'_>,
    ) -> Result<Dictionary> {
        Self::with_dictionaries(
            temp_dir,
            temp_file_name_prefix,
            affix,
            vec![dictionary],
            false,
        )
    }

    /// As [`new`](Self::new), for several dictionary files.
    ///
    /// Equivalent to `Dictionary(Directory, String, InputStream,
    /// List<InputStream>, boolean)`, with
    /// [`SortingStrategy::offline`].
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_dictionaries(
        temp_dir: Arc<dyn Directory>,
        temp_file_name_prefix: &str,
        affix: InputStream<'_>,
        dictionaries: Vec<InputStream<'_>>,
        ignore_case: bool,
    ) -> Result<Dictionary> {
        Self::with_sorting_strategy(
            affix,
            dictionaries,
            ignore_case,
            SortingStrategy::offline(temp_dir, temp_file_name_prefix),
        )
    }

    /// Creates a new Dictionary containing the information read from the
    /// provided InputStreams to hunspell affix and dictionary files, loading
    /// the entries with `sorting_strategy`. You have to close the provided
    /// InputStreams yourself.
    ///
    /// Equivalent to `Dictionary(InputStream, List<InputStream>, boolean,
    /// SortingStrategy)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_sorting_strategy(
        affix: InputStream<'_>,
        dictionaries: Vec<InputStream<'_>>,
        ignore_case: bool,
        sorting_strategy: SortingStrategy,
    ) -> Result<Dictionary> {
        Self::with_subclass(
            affix,
            dictionaries,
            ignore_case,
            sorting_strategy,
            Arc::new(NoSubclass),
        )
    }

    /// As [`with_sorting_strategy`](Self::with_sorting_strategy), for a
    /// subclass of `Dictionary` that overrides the methods of `subclass`.
    ///
    /// Equivalent to `Dictionary(InputStream, List<InputStream>, boolean,
    /// SortingStrategy)` called by a subclass constructor.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_subclass(
        affix: InputStream<'_>,
        dictionaries: Vec<InputStream<'_>>,
        ignore_case: bool,
        sorting_strategy: SortingStrategy,
        subclass: Arc<dyn DictionarySubclass>,
    ) -> Result<Dictionary> {
        let mut dictionary = Dictionary {
            decoder: replacing_decoder(Charset::iso_8859_1())?,
            prefixes: None,
            suffixes: None,
            breaks: Breaks::default_breaks(),
            patterns: Vec::new(),
            words: WordStorage::empty(),
            flag_lookup: FlagEnumerator::new().finish(),
            strip_data: Vec::new(),
            strip_offsets: Vec::new(),
            word_chars: Vec::new(),
            affix_data: vec![0; 32],
            current_affix: 0,
            flag_parsing_strategy: Arc::new(SimpleFlagParsingStrategy),
            aliases: None,
            alias_count: 0,
            morph_aliases: None,
            morph_alias_count: 0,
            morph_data: vec![Vec::new()],
            has_custom_morph_data: false,
            ignore_case,
            check_sharp_s: false,
            complex_prefixes: false,
            second_stage_prefix_flags: Vec::new(),
            second_stage_suffix_flags: Vec::new(),
            circumfix: 0,
            keepcase: 0,
            force_u_case: 0,
            needaffix: 0,
            forbiddenword: 0,
            onlyincompound: 0,
            compound_begin: 0,
            compound_middle: 0,
            compound_end: 0,
            compound_flag: 0,
            compound_permit: 0,
            compound_forbid: 0,
            check_compound_case: false,
            check_compound_dup: false,
            check_compound_rep: false,
            check_compound_triple: false,
            simplified_triple: false,
            compound_min: 3,
            compound_max: i32::MAX,
            compound_rules: None,
            check_compound_patterns: Vec::new(),
            ignore: None,
            try_chars: Vec::new(),
            neighbor_key_groups: vec![units("qwertyuiop"), units("asdfghjkl"), units("zxcvbnm")],
            enable_split_suggestions: true,
            rep_table: Vec::new(),
            map_table: Vec::new(),
            max_diff: 5,
            max_n_gram_suggestions: 4,
            only_max_diff: false,
            no_suggest: 0,
            sub_standard: 0,
            iconv: None,
            oconv: None,
            full_strip: false,
            language: None,
            alternate_casing: false,
            subclass,
        };

        // `new BufferedInputStream(affix, MAX_PROLOGUE_SCAN_WINDOW)`, whose
        // `close()` does nothing. `maybeConsume` reads up to the length of
        // the BOM and pushes back what does not match it.
        // I assume we don't support other BOMs (utf16, etc.)? We trivially could,
        // by adding maybeConsume() with a proper bom... but I don't see hunspell repo to have
        // any such exotic examples.
        let mut head = Vec::with_capacity(BOM_UTF8.len());
        let stream_charset = if maybe_consume(affix, &BOM_UTF8, &mut head)? {
            head.clear();
            Charset::utf_8()
        } else {
            Charset::iso_8859_1()
        };
        let mut affix_stream = Cursor::new(head).chain(affix);

        // pass 1: look for encoding & flag. This is simple but works. We just prefetch
        // a large enough chunk of the input and scan through it. The buffered data will
        // be subsequently reused anyway so nothing is wasted.
        let mut prologue = Vec::new();
        (&mut affix_stream)
            .take((MAX_PROLOGUE_SCAN_WINDOW - 1) as u64)
            .read_to_end(&mut prologue)?;
        dictionary.read_config(Cursor::new(prologue.as_slice()), stream_charset)?;

        // pass 2: parse affixes
        let mut flag_enumerator = FlagEnumerator::new();
        let decoder = dictionary.decoder.clone();
        let affix_stream = Cursor::new(prologue).chain(affix_stream);
        dictionary.read_affix_file(affix_stream, decoder.clone(), &mut flag_enumerator)?;

        // read dictionary entries
        let mut acc = sorting_strategy.start()?;
        dictionary.merge_dictionaries(dictionaries, &decoder, acc.as_mut())?;
        let mut sorted = acc.finish_and_sort()?;
        let words = dictionary.read_sorted_dictionaries(&mut flag_enumerator, sorted.as_mut());
        // try-with-resources: `close()` runs, and its failure is suppressed by
        // the body's
        let closed = sorted.close();
        dictionary.words = match (words, closed) {
            (Ok(words), Ok(())) => words,
            (Ok(_), Err(e)) => return Err(e),
            (Err(mut e), Err(suppressed)) => {
                e.add_suppressed(suppressed);
                return Err(e);
            }
            (Err(e), Ok(())) => return Err(e),
        };
        dictionary.flag_lookup = flag_enumerator.finish();
        dictionary.aliases = None; // no longer needed
        dictionary.morph_aliases = None; // no longer needed
        Ok(dictionary)
    }

    /// Equivalent to `formStep()`.
    pub(crate) fn form_step(&self) -> usize {
        if self.has_custom_morph_data {
            2
        } else {
            1
        }
    }

    /// Looks up Hunspell word forms from the dictionary.
    ///
    /// Equivalent to `lookupWord(char[], int, int)`.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn lookup_word(&self, word: &[u16], offset: usize, length: usize) -> Option<IntsRef> {
        self.words.lookup_word(word, offset, length)
    }

    /// Only for testing.
    ///
    /// Equivalent to `lookupPrefix(char[])`.
    ///
    /// # Errors
    ///
    /// Java's `NullPointerException` when the dictionary has no prefix, and
    /// what reading the FST raises.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn lookup_prefix(&self, word: &[u16]) -> Result<Option<IntsRef>> {
        Self::lookup(self.prefixes.as_ref(), word)
    }

    /// Only for testing.
    ///
    /// Equivalent to `lookupSuffix(char[])`.
    ///
    /// # Errors
    ///
    /// As [`lookup_prefix`](Self::lookup_prefix).
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn lookup_suffix(&self, word: &[u16]) -> Result<Option<IntsRef>> {
        Self::lookup(self.suffixes.as_ref(), word)
    }

    /// Equivalent to `lookup(FST<IntsRef>, char[])`.
    fn lookup(fst: Option<&FST<IntSequenceOutputs>>, word: &[u16]) -> Result<Option<IntsRef>> {
        let Some(fst) = fst else {
            return Err(LuceneError::null_pointer());
        };
        let mut bytes_reader = fst.get_bytes_reader()?;
        let mut arc = FSTArc::default();
        fst.get_first_arc(&mut arc);
        // Accumulate output as we go
        let mut output = fst.outputs().no_output();
        let mut i = 0;
        while i < word.len() {
            let cp = code_point_at(word, i, word.len());
            match Self::next_arc(fst, &mut arc, bytes_reader.as_mut(), &output, cp)? {
                None => return Ok(None),
                Some(next) => output = next,
            }
            i += char_count(cp);
        }
        Self::next_arc(fst, &mut arc, bytes_reader.as_mut(), &output, END_LABEL)
    }

    /// Follows the arc labelled `ch` from `arc`, returning the output so far,
    /// or `None` when there is no such arc.
    ///
    /// Equivalent to `nextArc(FST, FST.Arc, FST.BytesReader, IntsRef, int)`.
    ///
    /// # Errors
    ///
    /// What reading the FST raises (Java wraps it in a `RuntimeException`).
    pub(crate) fn next_arc(
        fst: &FST<IntSequenceOutputs>,
        arc: &mut FSTArc<IntsRef>,
        reader: &mut dyn BytesReader,
        output: &IntsRef,
        ch: i32,
    ) -> Result<Option<IntsRef>> {
        if !fst.find_target_arc_in_place(ch, arc, reader)? {
            return Ok(None);
        }
        Ok(Some(fst.outputs().add(output, arc.output())))
    }

    /// Reads the affix file through the provided InputStream, building up
    /// the prefix and suffix maps.
    ///
    /// Equivalent to `readAffixFile(InputStream, CharsetDecoder,
    /// FlagEnumerator)`.
    fn read_affix_file<R: Read>(
        &mut self,
        affix_stream: R,
        decoder: CharsetDecoder,
        flags: &mut FlagEnumerator,
    ) -> Result<()> {
        let mut prefixes: BTreeMap<Vec<u16>, Vec<i32>> = BTreeMap::new();
        let mut suffixes: BTreeMap<Vec<u16>, Vec<i32>> = BTreeMap::new();
        let mut prefix_cont_flags: HashSet<u16> = HashSet::new();
        let mut suffix_cont_flags: HashSet<u16> = HashSet::new();
        let mut seen_patterns: HashMap<Vec<u16>, usize> = HashMap::new();

        // zero condition -> 0 ord
        seen_patterns.insert(units(ALWAYS_TRUE_KEY), 0);
        self.patterns.push(None);

        // zero strip -> 0 ord
        let mut seen_strips = LinkedStrips::default();
        seen_strips.put(Vec::new(), 0);

        let mut reader =
            LineNumberReader::new(InputStreamReader::with_decoder(affix_stream, decoder));
        while let Some(line) = reader.read_line()? {
            let mut line = line;
            // ignore any BOM marker on first line
            if reader.get_line_number() == 1 && line.first() == Some(&0xFEFF) {
                line = line[1..].to_vec();
            }
            let line = trim(&line).to_vec();
            if line.is_empty() {
                continue;
            }

            let first_word = split_whitespace(&line, false)
                .into_iter()
                .next()
                .unwrap_or_default();
            let is = |directive: &str| equals_str(&first_word, directive);
            if is("AF") {
                self.parse_alias(&line)?;
            } else if is("AM") {
                self.parse_morph_alias(&line)?;
            } else if is("PFX") {
                self.parse_affix(
                    &mut prefixes,
                    &mut prefix_cont_flags,
                    &line,
                    &mut reader,
                    AffixKind::Prefix,
                    &mut seen_patterns,
                    &mut seen_strips,
                    flags,
                )?;
            } else if is("SFX") {
                self.parse_affix(
                    &mut suffixes,
                    &mut suffix_cont_flags,
                    &line,
                    &mut reader,
                    AffixKind::Suffix,
                    &mut seen_patterns,
                    &mut seen_strips,
                    flags,
                )?;
            } else if equals_str(&line, "COMPLEXPREFIXES") {
                // 2-stage prefix+1-stage suffix instead of 2-stage suffix+1-stage prefix
                self.complex_prefixes = true;
            } else if is("CIRCUMFIX") {
                self.circumfix = self.parse_flag_argument(&reader, &line)?;
            } else if is("KEEPCASE") {
                self.keepcase = self.parse_flag_argument(&reader, &line)?;
            } else if is("FORCEUCASE") {
                self.force_u_case = self.parse_flag_argument(&reader, &line)?;
            } else if is("NEEDAFFIX") || is("PSEUDOROOT") {
                self.needaffix = self.parse_flag_argument(&reader, &line)?;
            } else if is("ONLYINCOMPOUND") {
                self.onlyincompound = self.parse_flag_argument(&reader, &line)?;
            } else if is("CHECKSHARPS") {
                self.check_sharp_s = true;
            } else if is("IGNORE") {
                let mut ignore = single_argument(&reader, &line)?;
                ignore.sort_unstable();
                self.ignore = Some(ignore);
            } else if is("ICONV") || is("OCONV") {
                let num = parse_num(&reader, &line)?;
                let res = self.parse_conversions(&mut reader, num)?;
                if starts_with_str(&line, "I") {
                    self.iconv = Some(res);
                } else {
                    self.oconv = Some(res);
                }
            } else if is("FULLSTRIP") {
                self.full_strip = true;
            } else if is("LANG") {
                self.language = Some(single_argument(&reader, &line)?);
                self.alternate_casing = self.has_language(&["tr", "az"]);
            } else if is("BREAK") {
                self.breaks = parse_breaks(&mut reader, &line)?;
            } else if is("WORDCHARS") {
                self.word_chars = first_argument(&reader, &line)?;
            } else if is("TRY") {
                self.try_chars = first_argument(&reader, &line)?;
            } else if is("REP") {
                if self.subclass.tolerate_rep_rule_count_mismatches() {
                    let parts = split_by_space_range(&reader, &line, 2, i32::MAX as usize)?;
                    // ignore REP N, as actual N may be incorrect
                    if parts.len() >= 3 {
                        self.rep_table.push(RepEntry::new(&parts[1], &parts[2]));
                    }
                } else {
                    let count = parse_num(&reader, &line)?;
                    for _ in 0..count {
                        let next = read_line_or_null_pointer(&mut reader)?;
                        let parts = split_by_space_range(&reader, &next, 3, i32::MAX as usize)?;
                        self.rep_table.push(RepEntry::new(&parts[1], &parts[2]));
                    }
                }
            } else if is("MAP") {
                let count = parse_num(&reader, &line)?;
                for _ in 0..count {
                    let next = read_line_or_null_pointer(&mut reader)?;
                    self.map_table.push(parse_map_entry(&reader, &next)?);
                }
            } else if is("KEY") {
                self.neighbor_key_groups =
                    split_unit(&single_argument(&reader, &line)?, u16::from(b'|'));
            } else if is("NOSPLITSUGS") {
                self.enable_split_suggestions = false;
            } else if is("MAXNGRAMSUGS") {
                self.max_n_gram_suggestions = parse_int(&single_argument(&reader, &line)?)?;
            } else if is("MAXDIFF") {
                let i = parse_int(&single_argument(&reader, &line)?)?;
                if !(0..=10).contains(&i) {
                    return Err(ParseException::error(
                        "MAXDIFF should be between 0 and 10",
                        reader.get_line_number(),
                    ));
                }
                self.max_diff = i;
            } else if is("ONLYMAXDIFF") {
                self.only_max_diff = true;
            } else if is("FORBIDDENWORD") {
                self.forbiddenword = self.parse_flag_argument(&reader, &line)?;
            } else if is("NOSUGGEST") {
                self.no_suggest = self.parse_flag_argument(&reader, &line)?;
            } else if is("SUBSTANDARD") {
                self.sub_standard = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDMIN") {
                self.compound_min = parse_num(&reader, &line)?.max(1);
            } else if is("COMPOUNDWORDMAX") {
                self.compound_max = parse_num(&reader, &line)?.max(1);
            } else if is("COMPOUNDRULE") {
                let num = parse_num(&reader, &line)?;
                self.compound_rules = Some(self.parse_compound_rules(&mut reader, num)?);
            } else if is("COMPOUNDFLAG") {
                self.compound_flag = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDBEGIN") {
                self.compound_begin = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDMIDDLE") {
                self.compound_middle = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDEND") {
                self.compound_end = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDPERMITFLAG") {
                self.compound_permit = self.parse_flag_argument(&reader, &line)?;
            } else if is("COMPOUNDFORBIDFLAG") {
                self.compound_forbid = self.parse_flag_argument(&reader, &line)?;
            } else if is("CHECKCOMPOUNDCASE") {
                self.check_compound_case = true;
            } else if is("CHECKCOMPOUNDDUP") {
                self.check_compound_dup = true;
            } else if is("CHECKCOMPOUNDREP") {
                self.check_compound_rep = true;
            } else if is("CHECKCOMPOUNDTRIPLE") {
                self.check_compound_triple = true;
            } else if is("SIMPLIFIEDTRIPLE") {
                self.simplified_triple = true;
            } else if is("CHECKCOMPOUNDPATTERN") {
                let count = parse_num(&reader, &line)?;
                for _ in 0..count {
                    let next = read_line_or_null_pointer(&mut reader)?;
                    self.check_compound_patterns.push(CheckCompoundPattern::new(
                        &next,
                        self.flag_parsing_strategy.as_ref(),
                    )?);
                }
            } else if is("SET") {
                let expected = self.decoder.charset();
                let actual = get_decoder(&single_argument(&reader, &line)?)?.charset();
                check_critical_directive_same("SET", &reader, expected == actual)?;
            } else if is("FLAG") {
                let strategy = get_flag_parsing_strategy(&line, self.decoder.charset())?;
                check_critical_directive_same(
                    "FLAG",
                    &reader,
                    self.flag_parsing_strategy.class_name() == strategy.class_name(),
                )?;
            }
        }

        self.prefixes = affix_fst(&prefixes)?;
        self.suffixes = affix_fst(&suffixes)?;
        self.second_stage_prefix_flags = to_sorted_char_array(&prefix_cont_flags);
        self.second_stage_suffix_flags = to_sorted_char_array(&suffix_cont_flags);

        let total_chars: usize = seen_strips.keys.iter().map(Vec::len).sum();
        self.strip_data = Vec::with_capacity(total_chars);
        self.strip_offsets = vec![0; seen_strips.keys.len() + 1];
        let mut current_offset = 0i32;
        let mut current_index = 0;
        for strip in &seen_strips.keys {
            self.strip_offsets[current_index] = current_offset;
            current_index += 1;
            self.strip_data.extend_from_slice(strip);
            current_offset += strip.len() as i32;
        }
        debug_assert_eq!(current_index, seen_strips.keys.len());
        self.strip_offsets[current_index] = current_offset;
        Ok(())
    }

    /// `flagParsingStrategy.parseFlag(singleArgument(reader, line))`.
    fn parse_flag_argument<S: ReadChars>(
        &self,
        reader: &LineNumberReader<S>,
        line: &[u16],
    ) -> Result<u16> {
        self.flag_parsing_strategy
            .parse_flag(&single_argument(reader, line)?)
    }

    /// Whether the dictionary's `LANG` names one of `lang_codes`.
    ///
    /// Equivalent to `hasLanguage(String...)`.
    pub(crate) fn has_language(&self, lang_codes: &[&str]) -> bool {
        let Some(language) = &self.language else {
            return false;
        };
        let lang_code = extract_language_code(language);
        lang_codes.iter().any(|code| equals_str(lang_code, code))
    }

    /// The dictionary entries for the given root, or `None` if there's none.
    /// `root` is a string to look up in the dictionary. No case conversion or
    /// affix removal is performed. To get the possible roots of any word, you
    /// may call [`Hunspell::get_roots`](super::hunspell::Hunspell::get_roots).
    ///
    /// Equivalent to `lookupEntries(String)`.
    pub fn lookup_entries(&self, root: &str) -> Option<Box<dyn DictEntries + '_>> {
        self.lookup_entries_units(&units(root))
            .map(|entries| Box::new(entries) as Box<dyn DictEntries + '_>)
    }

    /// [`lookup_entries`](Self::lookup_entries) of a root given as UTF-16
    /// units.
    pub(crate) fn lookup_entries_units(&self, root: &[u16]) -> Option<DictEntriesImpl<'_>> {
        let forms = self.lookup_word(root, 0, root.len())?;
        Some(DictEntriesImpl {
            dictionary: self,
            root: root.to_vec(),
            forms,
        })
    }

    /// Equivalent to `dictEntry(String, int, int)`.
    pub(crate) fn dict_entry(&self, root: &[u16], flag_id: i32, morph_data_id: i32) -> DictEntry {
        let flags = self
            .flag_parsing_strategy
            .print_flags(&self.flag_lookup.get_flags(flag_id));
        let morph = if morph_data_id == 0 {
            Vec::new()
        } else {
            self.morph_data[morph_data_id as usize].clone()
        };
        DictEntry::new(root.to_vec(), flags, morph)
    }

    /// Equivalent to `parseConversions(LineNumberReader, int)`.
    fn parse_conversions<S: ReadChars>(
        &self,
        reader: &mut LineNumberReader<S>,
        num: i32,
    ) -> Result<ConvTable> {
        let mut mappings: BTreeMap<Vec<u16>, Vec<u16>> = BTreeMap::new();
        for _ in 0..num {
            let next = read_line_or_null_pointer(reader)?;
            let parts = split_by_space_range(reader, &next, 3, 3)?;
            if mappings
                .insert(parts[1].clone(), parts[2].clone())
                .is_some()
                && !self.subclass.tolerate_duplicate_conversion_mappings()
            {
                return Err(LuceneError::illegal_state(format!(
                    "duplicate mapping specified for: {}",
                    to_rust_string(&parts[1])
                )));
            }
        }
        ConvTable::new(&mappings)
    }

    /// Parses the encoding and flag format specified in the provided
    /// InputStream.
    ///
    /// Equivalent to `readConfig(InputStream, Charset)`.
    fn read_config<R: Read>(&mut self, stream: R, stream_charset: Charset) -> Result<()> {
        let mut reader =
            LineNumberReader::new(InputStreamReader::with_charset(stream, stream_charset));
        let mut flag_line: Option<Vec<u16>> = None;
        let mut charset_found = false;
        let mut flag_found = false;
        while let Some(line) = reader.read_line()? {
            if is_blank(&line) {
                continue;
            }
            let first_word = split_whitespace(&line, false)
                .into_iter()
                .next()
                .unwrap_or_default();
            if equals_str(&first_word, "SET") {
                self.decoder = get_decoder(&single_argument(&reader, &line)?)?;
                charset_found = true;
            } else if equals_str(&first_word, "FLAG") {
                // Preserve the flag line for parsing later since we need the decoder's charset
                // and just in case they come out of order.
                flag_line = Some(line);
                flag_found = true;
            } else {
                continue;
            }
            if charset_found && flag_found {
                break;
            }
        }
        if flag_found {
            let flag_line = flag_line.expect("INVARIANT: set with flag_found");
            self.flag_parsing_strategy =
                get_flag_parsing_strategy(&flag_line, self.decoder.charset())?;
        }
        Ok(())
    }

    /// Equivalent to `parseCompoundRules(LineNumberReader, int)`.
    fn parse_compound_rules<S: ReadChars>(
        &self,
        reader: &mut LineNumberReader<S>,
        num: i32,
    ) -> Result<Vec<CompoundRule>> {
        if num < 0 {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                num.to_string(),
            ));
        }
        let mut compound_rules = Vec::with_capacity(num as usize);
        for _ in 0..num {
            let next = read_line_or_null_pointer(reader)?;
            compound_rules.push(CompoundRule::new(
                &single_argument(reader, &next)?,
                self.flag_parsing_strategy.as_ref(),
            )?);
        }
        Ok(compound_rules)
    }

    /// Parses a specific affix rule putting the result into the provided
    /// affix map.
    ///
    /// Equivalent to `parseAffix(TreeMap, CharHashSet, String,
    /// LineNumberReader, AffixKind, Map, Map, FlagEnumerator)`.
    #[allow(clippy::too_many_arguments)]
    fn parse_affix<S: ReadChars>(
        &mut self,
        affixes: &mut BTreeMap<Vec<u16>, Vec<i32>>,
        second_stage_flags: &mut HashSet<u16>,
        header: &[u16],
        reader: &mut LineNumberReader<S>,
        kind: AffixKind,
        seen_patterns: &mut HashMap<Vec<u16>, usize>,
        seen_strips: &mut LinkedStrips,
        flags: &mut FlagEnumerator,
    ) -> Result<()> {
        let mut sb: Vec<u16> = Vec::new();
        let args = split_whitespace(header, true);

        let cross_product = equals_str(java_index(&args, 2)?, "Y");

        let num_lines = match parse_int(java_index(&args, 3)?) {
            Ok(num_lines) => num_lines,
            Err(e) if e.is_number_format_exception() => {
                if self.subclass.tolerate_affix_rule_count_mismatches() {
                    return Ok(());
                }
                return Err(ParseException::error(
                    format!("Affix rule header expected; got {}", to_rust_string(header)),
                    reader.get_line_number(),
                ));
            }
            Err(e) => return Err(e),
        };
        let min_size = (self.current_affix as i64 * 4 + i64::from(num_lines) * 4).max(0) as usize;
        if self.affix_data.len() < min_size {
            self.affix_data.resize(min_size, 0);
        }

        for _ in 0..num_lines {
            if self.subclass.tolerate_affix_rule_count_mismatches() {
                // push back line if counts are wrong, with a limit: forgiveness is finite.
                reader.mark(256)?;
            }
            let Some(line) = reader.read_line()? else {
                if self.subclass.tolerate_affix_rule_count_mismatches() {
                    return Ok(()); // can happen due to rule count mismatches, EOF
                }
                return Err(ParseException::error(
                    format!("Premature end of rules for {}", to_rust_string(header)),
                    reader.get_line_number(),
                ));
            };

            // from the manpage: PFX flag stripping prefix [condition [morphological_fields...]]
            let rule_args = split_by_space_range(reader, &line, 4, i32::MAX as usize)?;

            if rule_args[1] != args[1] {
                if self.subclass.tolerate_affix_rule_count_mismatches() {
                    reader.reset()?; // can happen due to count mismatch: push back what we found
                    return Ok(());
                }
                return Err(ParseException::error(
                    format!(
                        "Affix rule mismatch. Header: {}; rule: {}",
                        to_rust_string(header),
                        to_rust_string(&line)
                    ),
                    reader.get_line_number(),
                ));
            }

            let flag = self.flag_parsing_strategy.parse_flag(&rule_args[1])?;
            let strip = if equals_str(&rule_args[2], "0") {
                Vec::new()
            } else {
                rule_args[2].clone()
            };
            let mut affix_arg = rule_args[3].clone();
            let mut append_flags: Option<Vec<u16>> = None;

            // first: parse continuation classes out of affix
            if let Some(flag_sep) = last_index_of_unit(&affix_arg, u16::from(b'/')) {
                let mut flag_part = affix_arg[flag_sep + 1..].to_vec();
                affix_arg.truncate(flag_sep);

                if self.alias_count > 0 {
                    flag_part = self.get_alias_value(parse_int(&flag_part)?)?;
                }

                let parsed = self.flag_parsing_strategy.parse_flags(&flag_part)?;
                second_stage_flags.extend(parsed.iter().copied());
                append_flags = Some(parsed);
            }
            // zero affix -> empty string
            if equals_str(&affix_arg, "0") {
                affix_arg.clear();
            }

            let condition = if rule_args.len() > 4 {
                rule_args[4].clone()
            } else {
                units(".")
            };
            let key = affix_condition::unique_key(kind, &strip, &condition);

            // deduplicate patterns
            let pattern_index = match seen_patterns.get(&key) {
                Some(&index) => index,
                None => {
                    let index = self.patterns.len();
                    if index > i16::MAX as usize {
                        return Err(LuceneError::unsupported_operation(
                            "Too many patterns, please report this to dev@lucene.apache.org",
                        ));
                    }
                    seen_patterns.insert(key, index);
                    self.patterns.push(Some(affix_condition::compile(
                        kind, &strip, &condition, &line,
                    )?));
                    index
                }
            };

            let strip_ord = match seen_strips.get(&strip) {
                Some(ord) => ord,
                None => {
                    let ord = seen_strips.keys.len();
                    seen_strips.put(strip.clone(), ord);
                    if ord > usize::from(u16::MAX) {
                        return Err(LuceneError::unsupported_operation(
                            "Too many unique strips, please report this to dev@lucene.apache.org",
                        ));
                    }
                    ord
                }
            };

            let mut append_flags = append_flags.unwrap_or_default();

            let append_flags_ord = flags.add(&mut append_flags)?;
            let append_flags_ord = if append_flags_ord < 0 {
                // already exists in our hash
                (-append_flags_ord) - 1
            } else if append_flags_ord > i32::from(i16::MAX) {
                // this limit is probably flexible, but it's a good sanity check too
                return Err(LuceneError::unsupported_operation(
                    "Too many unique append flags, please report this to dev@lucene.apache.org",
                ));
            } else {
                append_flags_ord
            };

            let data_start = self.current_affix * 4;
            self.affix_data[data_start + AFFIX_FLAG] = flag;
            self.affix_data[data_start + AFFIX_STRIP_ORD] = strip_ord as u16;
            // encode crossProduct into patternIndex
            let pattern_ord = (pattern_index << 1) | usize::from(cross_product);
            self.affix_data[data_start + AFFIX_CONDITION] = pattern_ord as u16;
            self.affix_data[data_start + AFFIX_APPEND] = append_flags_ord as u16;

            if self.needs_input_cleaning(&affix_arg) {
                self.clean_input(&affix_arg, &mut sb)?;
                affix_arg = sb.clone();
            }

            if kind == AffixKind::Suffix {
                affix_arg = string_builder_reverse(&affix_arg);
            }

            affixes
                .entry(affix_arg)
                .or_default()
                .push(self.current_affix as i32);
            self.current_affix += 1;
        }
        Ok(())
    }

    /// Equivalent to `affixData(int, int)`.
    pub(crate) fn affix_data(&self, affix_index: i32, offset: usize) -> u16 {
        self.affix_data[affix_index as usize * 4 + offset]
    }

    /// Equivalent to `isCrossProduct(int)`.
    pub(crate) fn is_cross_product(&self, affix: i32) -> bool {
        (self.affix_data(affix, AFFIX_CONDITION) & 1) == 1
    }

    /// Equivalent to `getAffixCondition(int)`.
    pub(crate) fn get_affix_condition(&self, affix: i32) -> usize {
        usize::from(self.affix_data(affix, AFFIX_CONDITION) >> 1)
    }

    /// Equivalent to `unescapeEntry(String)`.
    fn unescape_entry(entry: &[u16]) -> Vec<u16> {
        let mut sb = Vec::with_capacity(entry.len() + 1);
        let end = morph_boundary(entry);
        let mut i = 0;
        while i < end {
            let ch = entry[i];
            if ch == u16::from(b'\\') && i + 1 < entry.len() {
                sb.push(entry[i + 1]);
                i += 1;
            } else if ch == u16::from(b'/') && i > 0 {
                sb.push(FLAG_SEPARATOR);
            } else if !should_skip_escaped_char(ch) {
                sb.push(ch);
            }
            i += 1;
        }
        sb.push(MORPH_SEPARATOR);
        if end < entry.len() {
            for &c in &entry[end..] {
                if !should_skip_escaped_char(c) {
                    sb.push(c);
                }
            }
        }
        sb
    }

    /// Equivalent to `mergeDictionaries(List<InputStream>, CharsetDecoder,
    /// EntryAccumulator)`.
    fn merge_dictionaries(
        &mut self,
        dictionaries: Vec<InputStream<'_>>,
        decoder: &CharsetDecoder,
        acc: &mut dyn EntryAccumulator,
    ) -> Result<()> {
        let mut sb: Vec<u16> = Vec::new();
        for dictionary in dictionaries {
            let mut lines =
                BufferedReader::new(InputStreamReader::with_decoder(dictionary, decoder.clone()));
            lines.read_line()?; // first line is number of entries (approximately, sometimes)

            while let Some(line) = lines.read_line()? {
                // wild and unpredictable code comment rules
                if line.is_empty() || line[0] == u16::from(b'#') || line[0] == u16::from(b'\t') {
                    continue;
                }
                let line = Self::unescape_entry(&line);
                // if we haven't seen any custom morphological data, try to parse one
                if !self.has_custom_morph_data {
                    if let Some(morph_start) = index_of_unit(&line, MORPH_SEPARATOR, 0) {
                        let data = &line[morph_start + 1..];
                        self.has_custom_morph_data = self
                            .split_morph_data(data)?
                            .iter()
                            .any(|s| !is_phonetic_field(s));
                    }
                }

                self.write_normalized_word_entry(&mut sb, &line, acc)?;
            }
        }
        Ok(())
    }

    /// Equivalent to `writeNormalizedWordEntry(StringBuilder, String,
    /// EntryAccumulator)`.
    fn write_normalized_word_entry(
        &self,
        reuse: &mut Vec<u16>,
        line: &[u16],
        acc: &mut dyn EntryAccumulator,
    ) -> Result<()> {
        let flag_sep = index_of_unit(line, FLAG_SEPARATOR, 0);
        let morph_sep = index_of_unit(line, MORPH_SEPARATOR, 0)
            .expect("INVARIANT: unescapeEntry appends the morph separator");
        let sep = flag_sep.unwrap_or(morph_sep);
        if sep == 0 {
            return Ok(());
        }

        let before_sep = &line[..sep];
        let written: Vec<u16> = if self.needs_input_cleaning(before_sep) {
            self.clean_input(before_sep, reuse)?;
            reuse.extend_from_slice(&line[sep..]);
            reuse.clone()
        } else {
            line.to_vec()
        };

        let sep = written.len() - (line.len() - sep);
        acc.add_entry(&written)?;

        let word_case = WordCase::case_of_length(&written, sep);
        if word_case == WordCase::Mixed
            || word_case == WordCase::Upper && flag_sep.is_some_and(|flag_sep| flag_sep > 0)
        {
            self.add_hidden_capitalized_word(reuse, acc, &written[..sep], &written[sep..])?;
        }
        Ok(())
    }

    /// Equivalent to `addHiddenCapitalizedWord(StringBuilder,
    /// EntryAccumulator, String, String)`.
    fn add_hidden_capitalized_word(
        &self,
        reuse: &mut Vec<u16>,
        acc: &mut dyn EntryAccumulator,
        word: &[u16],
        after_sep: &[u16],
    ) -> Result<()> {
        reuse.clear();
        reuse.push(to_upper_case_unit(word[0]));
        for &c in &word[1..] {
            reuse.push(self.case_fold(c));
        }
        reuse.push(FLAG_SEPARATOR);
        reuse.push(HIDDEN_FLAG);
        let skip = usize::from(after_sep[0] == FLAG_SEPARATOR);
        reuse.extend_from_slice(&after_sep[skip..]);
        acc.add_entry(reuse)
    }

    /// Equivalent to `toLowerCase(String)`: each unit folded with
    /// [`case_fold`](Self::case_fold).
    pub(crate) fn to_lower_case(&self, word: &[u16]) -> Vec<u16> {
        word.iter().map(|&c| self.case_fold(c)).collect()
    }

    /// Equivalent to `toTitleCase(String)`.
    pub(crate) fn to_title_case(&self, word: &[u16]) -> Vec<u16> {
        let mut chars = Vec::with_capacity(word.len());
        chars.push(to_upper_case_unit(word[0]));
        for &c in &word[1..] {
            chars.push(self.case_fold(c));
        }
        chars
    }

    /// Equivalent to `readSortedDictionaries(FlagEnumerator, EntrySupplier)`.
    fn read_sorted_dictionaries(
        &mut self,
        flags: &mut FlagEnumerator,
        sorted: &mut dyn EntrySupplier,
    ) -> Result<WordStorage> {
        let mut morph_indices: HashMap<Vec<u16>, i32> = HashMap::new();
        let non_suggestible = self.all_non_suggestible_flags();
        let hash_factor = self.subclass.hash_factor();
        let has_custom_morph_data = self.has_custom_morph_data;
        let alternate_casing = self.alternate_casing;

        // TODO: the flags themselves can be double-chars (long) or also numeric
        // either way the trick is to encode them as char... but they must be parsed differently
        let word_count = sorted.word_count();
        let mut builder = WordStorageBuilder::new(
            word_count,
            hash_factor,
            has_custom_morph_data,
            flags,
            non_suggestible,
        )?;
        while let Some(line) = sorted.next()? {
            let (entry, word_form, end): (Vec<u16>, Vec<u16>, usize) =
                match index_of_unit(&line, FLAG_SEPARATOR, 0) {
                    None => {
                        let end = index_of_unit(&line, MORPH_SEPARATOR, 0)
                            .ok_or_else(LuceneError::null_pointer)?;
                        (line[..end].to_vec(), NOFLAGS.to_vec(), end)
                    }
                    Some(flag_sep) => {
                        let end = index_of_unit(&line, MORPH_SEPARATOR, 0)
                            .ok_or_else(LuceneError::null_pointer)?;
                        let hidden = line[flag_sep + 1] == HIDDEN_FLAG;
                        let mut flag_part =
                            strip(&line[flag_sep + if hidden { 2 } else { 1 }..end]).to_vec();
                        if self.alias_count > 0 && !flag_part.is_empty() {
                            flag_part = self.get_alias_value(parse_int(&flag_part)?)?;
                        }

                        let mut word_form = self.flag_parsing_strategy.parse_flags(&flag_part)?;
                        if hidden {
                            word_form.push(HIDDEN_FLAG);
                        }
                        (line[..flag_sep].to_vec(), word_form, end)
                    }
                };

            if entry.is_empty() {
                continue;
            }

            let mut morph_data_id = 0;
            if end + 1 < line.len() {
                let mut morph_fields = self.read_morph_fields(&entry, &line[end + 1..])?;
                if !morph_fields.is_empty() {
                    morph_fields.sort();
                    let joined = join_with_space(&morph_fields);
                    morph_data_id = self.add_morph_fields(&mut morph_indices, joined);
                }
            }

            builder.add(&entry, word_form, morph_data_id)?;
        }

        WordStorage::new(builder, alternate_casing)
    }

    /// The factor of the hash table's size.
    ///
    /// Equivalent to `hashFactor()`, as the subclass overrides it.
    pub fn hash_factor(&self) -> f64 {
        self.subclass.hash_factor()
    }

    /// Equivalent to `tolerateAffixRuleCountMismatches()`, as the subclass
    /// overrides it.
    pub fn tolerate_affix_rule_count_mismatches(&self) -> bool {
        self.subclass.tolerate_affix_rule_count_mismatches()
    }

    /// Equivalent to `tolerateRepRuleCountMismatches()`, as the subclass
    /// overrides it.
    pub fn tolerate_rep_rule_count_mismatches(&self) -> bool {
        self.subclass.tolerate_rep_rule_count_mismatches()
    }

    /// Equivalent to `tolerateDuplicateConversionMappings()`, as the
    /// subclass overrides it.
    pub fn tolerate_duplicate_conversion_mappings(&self) -> bool {
        self.subclass.tolerate_duplicate_conversion_mappings()
    }

    /// Equivalent to `allNonSuggestibleFlags()`.
    pub(crate) fn all_non_suggestible_flags(&self) -> Vec<u16> {
        let mut set: HashSet<u16> = HashSet::with_capacity(5);
        set.insert(HIDDEN_FLAG);
        for c in [
            self.no_suggest,
            self.forbiddenword,
            self.onlyincompound,
            self.sub_standard,
        ] {
            if c != FLAG_UNSET {
                set.insert(c);
            }
        }
        to_sorted_char_array(&set)
    }

    /// Equivalent to `readMorphFields(String, String)`.
    fn read_morph_fields(&mut self, word: &[u16], unparsed: &[u16]) -> Result<Vec<Vec<u16>>> {
        let mut morph_fields = Vec::new();
        for datum in self.split_morph_data(unparsed)? {
            if is_phonetic_field(&datum) {
                self.add_phonetic_rep_entries(word, &datum[3..]);
            } else {
                morph_fields.push(datum);
            }
        }
        Ok(morph_fields)
    }

    /// Equivalent to `addMorphFields(Map<String, Integer>, String)`.
    fn add_morph_fields(
        &mut self,
        indices: &mut HashMap<Vec<u16>, i32>,
        morph_fields: Vec<u16>,
    ) -> i32 {
        if let Some(&already_cached) = indices.get(&morph_fields) {
            return already_cached;
        }
        let index = self.morph_data.len() as i32;
        indices.insert(morph_fields.clone(), index);
        self.morph_data.push(morph_fields);
        index
    }

    /// Equivalent to `addPhoneticRepEntries(String, String)`.
    fn add_phonetic_rep_entries(&mut self, word: &[u16], ph: &[u16]) {
        // e.g. "pretty ph:prity ph:priti->pretti" to suggest both prity->pretty and pritier->prettiest
        let arrow = crate::jdk::string::index_of(ph, &units("->"), 0);
        let (mut pattern, mut replacement) = match arrow {
            Some(arrow) if arrow > 0 => (ph[..arrow].to_vec(), ph[arrow + 2..].to_vec()),
            _ => (ph.to_vec(), word.to_vec()),
        };

        // when the ph: field ends with *, strip last character of pattern and replacement
        // e.g., "pretty ph:prity*" results in "prit->prett" replacement instead of "prity->pretty",
        // to get both prity->pretty and pritiest->prettiest suggestions.
        if pattern.ends_with(&[u16::from(b'*')]) && pattern.len() > 2 && replacement.len() > 1 {
            pattern.truncate(pattern.len() - 2);
            replacement.truncate(replacement.len() - 1);
        }

        // capitalize lowercase pattern for capitalized words to support
        // good suggestions also for capitalized misspellings,
        // e.g. Wednesday ph:wendsay results in wendsay -> Wednesday and Wendsay -> Wednesday.
        if WordCase::case_of(word) == WordCase::Title
            && WordCase::case_of(&pattern) == WordCase::Lower
        {
            // add also lowercase word in the case of German or
            // Hungarian to support lowercase suggestions lowercased by
            // compound word generation or derivational suffixes
            // for example by adjectival suffix "-i" of geographical names in Hungarian:
            // Massachusetts ph:messzecsuzec
            // messzecsuzeci -> massachusettsi (adjective)
            // For lowercasing by conditional PFX rules, see e.g. germancompounding test
            if self.has_language(&["de", "hu"]) {
                let lower = self.to_lower_case(&replacement);
                self.rep_table.push(RepEntry::new(&pattern, &lower));
            }
            let title = self.to_title_case(&pattern);
            self.rep_table.push(RepEntry::new(&title, &replacement));
        }
        self.rep_table.push(RepEntry::new(&pattern, &replacement));
    }

    /// Equivalent to `isDotICaseChangeDisallowed(char[])`.
    pub(crate) fn is_dot_i_case_change_disallowed(&self, word: &[u16]) -> bool {
        word[0] == 0x0130 && !self.alternate_casing
    }

    /// Equivalent to `parseAlias(String)`.
    fn parse_alias(&mut self, line: &[u16]) -> Result<()> {
        let rule_args = split_whitespace(line, true);
        match &mut self.aliases {
            None => {
                // first line should be the aliases count
                let count = parse_int(java_index(&rule_args, 1)?)?;
                if count < 0 {
                    return Err(LuceneError::unchecked(
                        "java.lang.NegativeArraySizeException",
                        count.to_string(),
                    ));
                }
                self.aliases = Some(vec![Vec::new(); count as usize]);
            }
            Some(aliases) => {
                // an alias can map to no flags
                let alias_value = if rule_args.len() == 1 {
                    Vec::new()
                } else {
                    rule_args[1].clone()
                };
                if self.alias_count >= aliases.len() {
                    return Err(array_index_out_of_bounds(
                        self.alias_count as i64,
                        aliases.len(),
                    ));
                }
                aliases[self.alias_count] = alias_value;
                self.alias_count += 1;
            }
        }
        Ok(())
    }

    /// Equivalent to `getAliasValue(int)`.
    fn get_alias_value(&self, id: i32) -> Result<Vec<u16>> {
        let aliases = self
            .aliases
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?;
        let index = i64::from(id) - 1;
        if index < 0 || index >= aliases.len() as i64 {
            return Err(LuceneError::illegal_argument_with_cause(
                format!("Bad flag alias number:{id}"),
                Arc::new(array_index_out_of_bounds(index, aliases.len())),
            ));
        }
        Ok(aliases[index as usize].clone())
    }

    /// Equivalent to `parseMorphAlias(String)`.
    fn parse_morph_alias(&mut self, line: &[u16]) -> Result<()> {
        match &mut self.morph_aliases {
            None => {
                // first line should be the aliases count
                let count = parse_int(java_substring_from(line, 3)?)?;
                if count < 0 {
                    return Err(LuceneError::unchecked(
                        "java.lang.NegativeArraySizeException",
                        count.to_string(),
                    ));
                }
                self.morph_aliases = Some(vec![Vec::new(); count as usize]);
            }
            Some(morph_aliases) => {
                let arg = java_substring_from(line, 2)?.to_vec(); // leave the space
                if self.morph_alias_count >= morph_aliases.len() {
                    return Err(array_index_out_of_bounds(
                        self.morph_alias_count as i64,
                        morph_aliases.len(),
                    ));
                }
                morph_aliases[self.morph_alias_count] = arg;
                self.morph_alias_count += 1;
            }
        }
        Ok(())
    }

    /// Equivalent to `splitMorphData(String)`.
    fn split_morph_data(&self, morph_data: &[u16]) -> Result<Vec<Vec<u16>>> {
        let mut morph_data = morph_data.to_vec();
        // first see if it's an alias
        if self.morph_alias_count > 0 {
            if let Ok(alias) = parse_int(trim(&morph_data)) {
                let morph_aliases = self
                    .morph_aliases
                    .as_ref()
                    .ok_or_else(LuceneError::null_pointer)?;
                let index = i64::from(alias) - 1;
                if index < 0 || index >= morph_aliases.len() as i64 {
                    return Err(array_index_out_of_bounds(index, morph_aliases.len()));
                }
                morph_data = morph_aliases[index as usize].clone();
            }
        }
        if is_blank(&morph_data) {
            return Ok(Vec::new());
        }

        let mut result = Vec::new();
        let mut start = 0;
        for i in 0..=morph_data.len() {
            if i == morph_data.len() || is_whitespace(i32::from(morph_data[i])) {
                if i as i64 - start as i64 > 3
                    && is_letter(i32::from(morph_data[start]))
                    && is_letter(i32::from(morph_data[start + 1]))
                    && morph_data[start + 2] == u16::from(b':')
                {
                    result.push(morph_data[start..i].to_vec());
                }
                start = i + 1;
            }
        }
        Ok(result)
    }

    /// Whether one of the entries of `forms` has `flag`.
    ///
    /// Equivalent to `hasFlag(IntsRef, char)`.
    pub(crate) fn has_flag_forms(&self, forms: &IntsRef, flag: u16) -> bool {
        let form_step = self.form_step();
        (0..forms.length)
            .step_by(form_step)
            .any(|i| self.has_flag(forms.ints[forms.offset + i], flag))
    }

    /// Equivalent to `isFlagAppendedByAffix(int, char)`.
    pub(crate) fn is_flag_appended_by_affix(&self, affix_id: i32, flag: u16) -> bool {
        if affix_id < 0 || flag == FLAG_UNSET {
            return false;
        }
        let append_id = self.affix_data(affix_id, AFFIX_APPEND);
        self.has_flag(i32::from(append_id), flag)
    }

    /// Equivalent to `hasFlag(int, char)`.
    pub(crate) fn has_flag(&self, entry_id: i32, flag: u16) -> bool {
        self.flag_lookup.has_flag(entry_id, flag)
    }

    /// Equivalent to `mayNeedInputCleaning()`.
    ///
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub fn may_need_input_cleaning(&self) -> bool {
        self.ignore_case || self.ignore.is_some() || self.iconv.is_some()
    }

    /// Equivalent to `needsInputCleaning(CharSequence)`.
    pub(crate) fn needs_input_cleaning(&self, input: &[u16]) -> bool {
        if self.may_need_input_cleaning() {
            for &ch in input {
                if self
                    .ignore
                    .as_ref()
                    .is_some_and(|ignore| ignore.binary_search(&ch).is_ok())
                    || self.ignore_case && self.case_fold(ch) != ch
                    || self
                        .iconv
                        .as_ref()
                        .is_some_and(|iconv| iconv.might_replace_char(ch))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Writes into `reuse` the input without the ignored characters, folded
    /// when the dictionary ignores case, and converted by `ICONV`.
    ///
    /// Equivalent to `cleanInput(CharSequence, StringBuilder)`, which
    /// returns `reuse`.
    ///
    /// # Errors
    ///
    /// What applying the `ICONV` table raises.
    pub(crate) fn clean_input(&self, input: &[u16], reuse: &mut Vec<u16>) -> Result<()> {
        reuse.clear();

        for &ch in input {
            if self
                .ignore
                .as_ref()
                .is_some_and(|ignore| ignore.binary_search(&ch).is_ok())
            {
                continue;
            }

            let ch = if self.ignore_case && self.iconv.is_none() {
                // if we have no input conversion mappings, do this on-the-fly
                self.case_fold(ch)
            } else {
                ch
            };

            reuse.push(ch);
        }

        if let Some(iconv) = &self.iconv {
            iconv.apply_mappings(reuse)?;
            if self.ignore_case {
                for c in reuse.iter_mut() {
                    *c = self.case_fold(*c);
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `isSecondStagePrefix(char)`.
    pub(crate) fn is_second_stage_prefix(&self, flag: u16) -> bool {
        self.second_stage_prefix_flags.binary_search(&flag).is_ok()
    }

    /// Equivalent to `isSecondStageSuffix(char)`.
    pub(crate) fn is_second_stage_suffix(&self, flag: u16) -> bool {
        self.second_stage_suffix_flags.binary_search(&flag).is_ok()
    }

    /// Folds single character (according to LANG if present).
    ///
    /// Equivalent to `caseFold(char)`.
    pub(crate) fn case_fold(&self, c: u16) -> u16 {
        Self::case_fold_with(self.alternate_casing, c)
    }

    /// `caseFold(char)` of a dictionary whose `alternateCasing` is
    /// `alternate_casing`.
    pub(crate) fn case_fold_with(alternate_casing: bool, c: u16) -> u16 {
        if alternate_casing {
            if c == u16::from(b'I') {
                0x0131 // 'ı'
            } else if c == 0x0130 {
                // 'İ'
                u16::from(b'i')
            } else {
                to_lower_case_unit(c)
            }
        } else {
            to_lower_case_unit(c)
        }
    }

    /// Returns true if this dictionary was constructed with the `ignoreCase`
    /// option.
    ///
    /// Equivalent to `getIgnoreCase()`.
    pub fn get_ignore_case(&self) -> bool {
        self.ignore_case
    }

    /// Returns the default temporary directory pointed to by
    /// `java.io.tmpdir`. If not accessible or not available, an IOException
    /// is thrown.
    ///
    /// Equivalent to `getDefaultTempDir()`; `java.io.tmpdir` is the
    /// platform's temporary directory, [`std::env::temp_dir`].
    ///
    /// # Errors
    ///
    /// `IOException("Temporary path not present or writeable?: <path>")`.
    pub(crate) fn get_default_temp_dir() -> Result<PathBuf> {
        let tmp_path = std::env::temp_dir();
        let nio_path = rucene::store::java_nio_file::IntoNioPath::into_nio_path(tmp_path.as_path());
        if !rucene::store::java_nio_file::Files::is_writable(&nio_path) {
            return Err(crate::jdk::io::io_exception(
                "java.io.IOException",
                Some(format!(
                    "Temporary path not present or writeable?: {}",
                    std::path::absolute(&tmp_path)
                        .unwrap_or_else(|_| tmp_path.clone())
                        .display()
                )),
            ));
        }
        Ok(tmp_path)
    }
}

/// The homonym entries of a root.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$DictEntriesImpl`,
/// the local class of `Dictionary.lookupEntries(String)`.
#[derive(Debug)]
pub(crate) struct DictEntriesImpl<'a> {
    dictionary: &'a Dictionary,
    root: Vec<u16>,
    forms: IntsRef,
}

impl DictEntries for DictEntriesImpl<'_> {
    fn size(&self) -> usize {
        self.forms.length / self.dictionary.form_step()
    }

    fn get(&self, entry_index: usize) -> DictEntry {
        let step = self.dictionary.form_step();
        self.dictionary.dict_entry(
            &self.root,
            self.forms.ints[self.forms.offset + entry_index * step],
            if self.dictionary.has_custom_morph_data {
                self.forms.ints[self.forms.offset + entry_index * 2 + 1]
            } else {
                0
            },
        )
    }
}

/// The insertion-ordered strips of `readAffixFile` (Java's
/// `LinkedHashMap<String, Integer>`).
#[derive(Default)]
struct LinkedStrips {
    keys: Vec<Vec<u16>>,
    ords: HashMap<Vec<u16>, usize>,
}

impl LinkedStrips {
    fn get(&self, strip: &[u16]) -> Option<usize> {
        self.ords.get(strip).copied()
    }

    fn put(&mut self, strip: Vec<u16>, ord: usize) {
        self.ords.insert(strip.clone(), ord);
        self.keys.push(strip);
    }
}

/// Equivalent to `maybeConsume(BufferedInputStream, byte[])`: reads the
/// bytes of `bytes` one by one while they match; the bytes read are left in
/// `read` (Java resets the stream to before them when they do not all
/// match).
fn maybe_consume(stream: &mut dyn Read, bytes: &[u8], read: &mut Vec<u8>) -> Result<bool> {
    for &b in bytes {
        let mut next = [0u8; 1];
        let n = loop {
            match stream.read(&mut next) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        };
        if n == 0 {
            return Ok(false); // covers EOF (-1) as well.
        }
        read.push(next[0]);
        if next[0] != b {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Equivalent to `checkCriticalDirectiveSame(String, LineNumberReader,
/// Object, Object)`, with `same` the result of `expected.equals(actual)`.
fn check_critical_directive_same<S: ReadChars>(
    directive: &str,
    reader: &LineNumberReader<S>,
    same: bool,
) -> Result<()> {
    if !same {
        return Err(ParseException::error(
            format!(
                "{directive} directive should occur at most once, and in the first {MAX_PROLOGUE_SCAN_WINDOW} bytes of the *.aff file"
            ),
            reader.get_line_number(),
        ));
    }
    Ok(())
}

/// Equivalent to `parseMapEntry(LineNumberReader, String)`.
fn parse_map_entry<S: ReadChars>(
    reader: &LineNumberReader<S>,
    line: &[u16],
) -> Result<Vec<Vec<u16>>> {
    let unparsed = first_argument(reader, line)?;
    let mut map_entry = Vec::new();
    let mut j = 0;
    while j < unparsed.len() {
        if unparsed[j] == u16::from(b'(') {
            let Some(closing) = index_of_unit(&unparsed, u16::from(b')'), j) else {
                return Err(ParseException::error(
                    format!("Unclosed parenthesis: {}", to_rust_string(line)),
                    reader.get_line_number(),
                ));
            };
            map_entry.push(unparsed[j + 1..closing].to_vec());
            j = closing;
        } else {
            map_entry.push(vec![unparsed[j]]);
        }
        j += 1;
    }
    Ok(map_entry)
}

/// Equivalent to `extractLanguageCode(String)`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn extract_language_code(iso_code: &[u16]) -> &[u16] {
    match index_of_unit(iso_code, u16::from(b'_'), 0) {
        None => iso_code,
        Some(underscore) => &iso_code[..underscore],
    }
}

/// Equivalent to `parseNum(LineNumberReader, String)`.
fn parse_num<S: ReadChars>(reader: &LineNumberReader<S>, line: &[u16]) -> Result<i32> {
    parse_int(&split_by_space_range(reader, line, 2, i32::MAX as usize)?[1])
}

/// Equivalent to `singleArgument(LineNumberReader, String)`.
fn single_argument<S: ReadChars>(reader: &LineNumberReader<S>, line: &[u16]) -> Result<Vec<u16>> {
    Ok(split_by_space_range(reader, line, 2, 2)?.swap_remove(1))
}

/// Equivalent to `firstArgument(LineNumberReader, String)`.
fn first_argument<S: ReadChars>(reader: &LineNumberReader<S>, line: &[u16]) -> Result<Vec<u16>> {
    Ok(split_by_space_range(reader, line, 2, i32::MAX as usize)?.swap_remove(1))
}

/// Equivalent to `splitBySpace(LineNumberReader, String, int, int)`.
fn split_by_space_range<S: ReadChars>(
    reader: &LineNumberReader<S>,
    line: &[u16],
    min_parts: usize,
    max_parts: usize,
) -> Result<Vec<Vec<u16>>> {
    let parts = split_whitespace(line, true);
    if parts.len() < min_parts
        || parts.len() > max_parts && !starts_with_str(&parts[max_parts], "#")
    {
        return Err(ParseException::error(
            format!("Invalid syntax: {}", to_rust_string(line)),
            reader.get_line_number(),
        ));
    }
    Ok(parts)
}

/// `reader.readLine()` whose result Java dereferences at once: a
/// `NullPointerException` at the end of the stream.
fn read_line_or_null_pointer<S: ReadChars>(reader: &mut LineNumberReader<S>) -> Result<Vec<u16>> {
    reader.read_line()?.ok_or_else(LuceneError::null_pointer)
}

/// `args[index]`, or Java's `ArrayIndexOutOfBoundsException`.
fn java_index(args: &[Vec<u16>], index: usize) -> Result<&[u16]> {
    args.get(index)
        .map(Vec::as_slice)
        .ok_or_else(|| array_index_out_of_bounds(index as i64, args.len()))
}

/// `s.substring(begin)`, or Java's `StringIndexOutOfBoundsException`.
fn java_substring_from(s: &[u16], begin: usize) -> Result<&[u16]> {
    if begin > s.len() {
        return Err(LuceneError::from(
            rucene::error::LuceneErrorKind::StringIndexOutOfBounds(format!(
                "begin {begin}, end {}, length {}",
                s.len(),
                s.len()
            )),
        ));
    }
    Ok(&s[begin..])
}

/// `String.join(" ", fields)`.
fn join_with_space(fields: &[Vec<u16>]) -> Vec<u16> {
    let mut joined = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            joined.push(u16::from(b' '));
        }
        joined.extend_from_slice(field);
    }
    joined
}

/// Equivalent to `parseBreaks(LineNumberReader, String)`.
fn parse_breaks<S: ReadChars>(reader: &mut LineNumberReader<S>, line: &[u16]) -> Result<Breaks> {
    let mut starting = LinkedSet::default();
    let mut ending = LinkedSet::default();
    let mut middle = LinkedSet::default();
    let num = parse_num(reader, line)?;
    for _ in 0..num {
        let next = read_line_or_null_pointer(reader)?;
        let break_str = single_argument(reader, &next)?;
        if starts_with_str(&break_str, "^") {
            starting.add(break_str[1..].to_vec());
        } else if break_str.ends_with(&[u16::from(b'$')]) {
            ending.add(break_str[..break_str.len() - 1].to_vec());
        } else {
            middle.add(break_str);
        }
    }
    Ok(Breaks::new(starting.items, ending.items, middle.items))
}

/// An insertion-ordered set of strings (Java's `LinkedHashSet<String>`).
#[derive(Default)]
struct LinkedSet {
    items: Vec<Vec<u16>>,
    seen: HashSet<Vec<u16>>,
}

impl LinkedSet {
    fn add(&mut self, item: Vec<u16>) {
        if self.seen.insert(item.clone()) {
            self.items.push(item);
        }
    }
}

/// Equivalent to `affixFST(TreeMap<String, IntArrayList>)`: an FST of the
/// code points of each affix to its ids.
fn affix_fst(affixes: &BTreeMap<Vec<u16>, Vec<i32>>) -> Result<Option<FST<IntSequenceOutputs>>> {
    let outputs = IntSequenceOutputs::get_singleton();
    let mut fst_compiler = FSTCompilerBuilder::new(InputType::Byte4, outputs).build()?;
    let mut scratch = IntsRefBuilder::new();
    for (key, entries) in affixes {
        let input = Util::to_utf32_units(key, 0, key.len(), &mut scratch);
        let output = IntsRef::new(entries.clone());
        fst_compiler.add(&input, output)?;
    }
    let metadata = fst_compiler.compile()?;
    Ok(FST::from_fst_reader(
        metadata,
        fst_compiler.into_fst_reader()?,
    ))
}

/// `new StringBuilder(s).reverse()`: the units reversed, with each surrogate
/// pair kept in order.
fn string_builder_reverse(s: &[u16]) -> Vec<u16> {
    let mut value: Vec<u16> = s.iter().rev().copied().collect();
    let mut i = 0;
    while i + 1 < value.len() {
        let c2 = value[i];
        if crate::jdk::character::is_low_surrogate(c2) {
            let c1 = value[i + 1];
            if crate::jdk::character::is_high_surrogate(c1) {
                value[i] = c1;
                i += 1;
                value[i] = c2;
            }
        }
        i += 1;
    }
    value
}

/// The charsets whose decoders `getDecoder` returns.
///
/// Equivalent to `getDecoder(String)`: Lucene's `ISO8859_14Decoder` for
/// `ISO8859-14`, otherwise a decoder of `Charset.forName` (after
/// [`CHARSET_ALIASES`]) that replaces malformed input.
///
/// # Errors
///
/// What `Charset.forName` throws.
fn get_decoder(encoding: &[u16]) -> Result<CharsetDecoder> {
    if equals_str(encoding, "ISO8859-14") {
        return Ok(ISO8859_14Decoder::new_decoder());
    }
    let mut encoding = to_rust_string(encoding);
    if let Some((_, canon)) = CHARSET_ALIASES.iter().find(|(alias, _)| *alias == encoding) {
        encoding = (*canon).to_string();
    }
    replacing_decoder(Charset::for_name(&encoding)?)
}

/// Equivalent to `replacingDecoder(Charset)`.
fn replacing_decoder(charset: Charset) -> Result<CharsetDecoder> {
    Ok(charset
        .new_decoder()?
        .on_malformed_input(CodingErrorAction::Replace))
}

/// Determines the appropriate [`FlagParsingStrategy`] based on the FLAG
/// definition line taken from the affix file.
///
/// Equivalent to `getFlagParsingStrategy(String, Charset)`.
///
/// # Errors
///
/// `IllegalArgumentException("Illegal FLAG specification: ...")` or
/// `IllegalArgumentException("Unknown flag type: ...")`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn get_flag_parsing_strategy(
    flag_line: &[u16],
    charset: Charset,
) -> Result<Arc<dyn FlagParsingStrategy>> {
    let parts = split_whitespace(flag_line, true);
    if parts.len() != 2 {
        return Err(LuceneError::illegal_argument(format!(
            "Illegal FLAG specification: {}",
            to_rust_string(flag_line)
        )));
    }
    let flag_type = &parts[1];

    if equals_str(flag_type, "num") {
        return Ok(Arc::new(NumFlagParsingStrategy));
    } else if equals_str(flag_type, "UTF-8") {
        if Charset::iso_8859_1() == charset {
            return Ok(Arc::new(DefaultAsUtf8FlagParsingStrategy));
        }
        return Ok(Arc::new(SimpleFlagParsingStrategy));
    } else if equals_str(flag_type, "long") {
        return Ok(Arc::new(DoubleASCIIFlagParsingStrategy));
    }

    Err(LuceneError::illegal_argument(format!(
        "Unknown flag type: {}",
        to_rust_string(flag_type)
    )))
}

/// Equivalent to `indexOfSpaceOrTab(String, int)`.
pub(crate) fn index_of_space_or_tab(text: &[u16], start: usize) -> Option<usize> {
    let pos1 = index_of_unit(text, u16::from(b'\t'), start);
    let pos2 = index_of_unit(text, u16::from(b' '), start);
    match (pos1, pos2) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Equivalent to `morphBoundary(String)`.
fn morph_boundary(line: &[u16]) -> usize {
    let Some(mut end) = index_of_space_or_tab(line, 0) else {
        return line.len();
    };
    while end < line.len() {
        if line[end] == u16::from(b'\t')
            || end > 0
                && end + 3 < line.len()
                && is_letter(i32::from(line[end + 1]))
                && is_letter(i32::from(line[end + 2]))
                && line[end + 3] == u16::from(b':')
        {
            break;
        }
        match index_of_space_or_tab(line, end + 1) {
            Some(next) => end = next,
            None => return line.len(),
        }
    }
    end
}

/// Equivalent to `shouldSkipEscapedChar(char)`: BINARY EXECUTABLES EMBEDDED
/// IN ZULU DICTIONARIES!!!!!!!
fn should_skip_escaped_char(ch: u16) -> bool {
    ch == FLAG_SEPARATOR || ch == MORPH_SEPARATOR
}

/// Equivalent to `toSortedCharArray(CharHashSet)`.
pub(crate) fn to_sorted_char_array(set: &HashSet<u16>) -> Vec<u16> {
    let mut chars: Vec<u16> = set.iter().copied().collect();
    chars.sort_unstable();
    chars
}

/// Abstraction of the process of parsing flags taken from the affix and dic
/// files.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$FlagParsingStrategy`,
/// an abstract class: [`parse_flags`](Self::parse_flags) and
/// [`print_flag`](Self::print_flag) are its abstract methods, the others its
/// concrete ones.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub trait FlagParsingStrategy: Debug + Send + Sync {
    /// Java's `getClass()`, compared by `readAffixFile`.
    fn class_name(&self) -> &'static str;

    /// Parses the given String into a single flag.
    ///
    /// Equivalent to `parseFlag(String)`: the first flag (we don't check the
    /// flag count, as Hunspell accepts longer sequences,
    /// <https://github.com/hunspell/hunspell/issues/707>; `checkFlags` is
    /// false).
    ///
    /// # Errors
    ///
    /// What [`parse_flags`](Self::parse_flags) raises, and Java's
    /// `ArrayIndexOutOfBoundsException` when it parses no flag.
    fn parse_flag(&self, raw_flag: &[u16]) -> Result<u16> {
        let flags = self.parse_flags(raw_flag)?;
        flags
            .first()
            .copied()
            .ok_or_else(|| array_index_out_of_bounds(0, 0))
    }

    /// Parses the given String into multiple flags.
    ///
    /// Equivalent to `parseFlags(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for flags the strategy rejects.
    fn parse_flags(&self, raw_flags: &[u16]) -> Result<Vec<u16>>;

    /// The original string representation of the given flag encoded by
    /// [`parse_flags`](Self::parse_flags).
    ///
    /// Equivalent to `printFlag(char)`.
    fn print_flag(&self, flag: u16) -> Vec<u16>;

    /// A presentable sorted concatenation of
    /// [`print_flag`](Self::print_flag) results.
    ///
    /// Equivalent to `printFlags(char[])`: the flags from `DEFAULT_FLAGS` up
    /// are skipped, and the printed flags are joined with `,` for
    /// `NumFlagParsingStrategy`, with nothing otherwise.
    fn print_flags(&self, encoded_flags: &[u16]) -> Vec<u16> {
        let mut printed: Vec<Vec<u16>> = encoded_flags
            .iter()
            .filter(|&&c| c < DEFAULT_FLAGS)
            .map(|&c| self.print_flag(c))
            .collect();
        printed.sort();
        let delimiter: &[u16] = if self.class_name() == NumFlagParsingStrategy::CLASS_NAME {
            &[0x2C]
        } else {
            &[]
        };
        let mut joined = Vec::new();
        for (i, flag) in printed.iter().enumerate() {
            if i > 0 {
                joined.extend_from_slice(delimiter);
            }
            joined.extend_from_slice(flag);
        }
        joined
    }

    /// Parse flags from a string resulting from
    /// [`print_flags`](Self::print_flags).
    ///
    /// Equivalent to `parseUtfFlags(String)`.
    ///
    /// # Errors
    ///
    /// As [`parse_flags`](Self::parse_flags).
    fn parse_utf_flags(&self, flags_in_utf: &[u16]) -> Result<Vec<u16>> {
        self.parse_flags(flags_in_utf)
    }
}

/// Simple implementation of [`FlagParsingStrategy`] that treats the chars in
/// each String as a individual flags. Can be used with both the ASCII and
/// UTF-8 flag types.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$SimpleFlagParsingStrategy`.
#[derive(Debug)]
pub(crate) struct SimpleFlagParsingStrategy;

impl FlagParsingStrategy for SimpleFlagParsingStrategy {
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.hunspell.Dictionary$SimpleFlagParsingStrategy"
    }

    fn parse_flags(&self, raw_flags: &[u16]) -> Result<Vec<u16>> {
        Ok(raw_flags.to_vec())
    }

    fn print_flag(&self, flag: u16) -> Vec<u16> {
        vec![flag]
    }
}

/// Used to read flags as UTF-8 even if the rest of the file is in the
/// default (8-bit) encoding.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$DefaultAsUtf8FlagParsingStrategy`.
#[derive(Debug)]
pub(crate) struct DefaultAsUtf8FlagParsingStrategy;

impl FlagParsingStrategy for DefaultAsUtf8FlagParsingStrategy {
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.hunspell.Dictionary$DefaultAsUtf8FlagParsingStrategy"
    }

    /// `new String(rawFlags.getBytes(DEFAULT_CHARSET),
    /// StandardCharsets.UTF_8).toCharArray()`.
    fn parse_flags(&self, raw_flags: &[u16]) -> Result<Vec<u16>> {
        let bytes = StandardCharset::Iso8859_1.encode(raw_flags);
        Ok(Charset::utf_8().new_decoder()?.decode_replacing(&bytes))
    }

    fn print_flag(&self, flag: u16) -> Vec<u16> {
        vec![flag]
    }

    fn parse_utf_flags(&self, flags_in_utf: &[u16]) -> Result<Vec<u16>> {
        Ok(flags_in_utf.to_vec())
    }
}

/// Implementation of [`FlagParsingStrategy`] that assumes each flag is
/// encoded in its numerical form. In the case of multiple flags, each number
/// is separated by a comma.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$NumFlagParsingStrategy`.
#[derive(Debug)]
pub(crate) struct NumFlagParsingStrategy;

impl NumFlagParsingStrategy {
    const CLASS_NAME: &'static str =
        "org.apache.lucene.analysis.hunspell.Dictionary$NumFlagParsingStrategy";
}

/// `Integer.parseInt(group, 0, group.length(), 10)` of a non-empty group of
/// ASCII digits: the value, or Java's `NumberFormatException("Error at index
/// <i> in: \"<group>\"")` at the digit where the value overflows.
fn parse_digit_group(group: &[u16]) -> Result<i32> {
    let limit = -i32::MAX;
    let multmin = limit / 10;
    let mut result = 0i32;
    for (i, &c) in group.iter().enumerate() {
        let digit = i32::from(c - u16::from(b'0'));
        let in_range = result > multmin || result == multmin && digit <= 10 * multmin - limit;
        if !in_range {
            return Err(LuceneError::number_format(format!(
                "Error at index {i} in: \"{}\"",
                to_rust_string(group)
            )));
        }
        result = 10 * result - digit;
    }
    Ok(-result)
}

impl FlagParsingStrategy for NumFlagParsingStrategy {
    fn class_name(&self) -> &'static str {
        Self::CLASS_NAME
    }

    fn parse_flags(&self, raw_flags: &[u16]) -> Result<Vec<u16>> {
        let mut result: Vec<u16> = Vec::new();
        let mut group: Vec<u16> = Vec::new();
        for i in 0..=raw_flags.len() {
            if i == raw_flags.len() || raw_flags[i] == u16::from(b',') {
                if !group.is_empty() {
                    // ignoring empty flags (this happens in danish, for example)
                    let flag = parse_digit_group(&group)?;
                    if flag >= i32::from(DEFAULT_FLAGS) {
                        // accept 0 due to https://github.com/hunspell/hunspell/issues/708
                        return Err(LuceneError::illegal_argument(format!(
                            "Num flags should be between 0 and {DEFAULT_FLAGS}, found {flag}"
                        )));
                    }
                    result.push(flag as u16);
                    group.clear();
                }
            } else if raw_flags[i] >= u16::from(b'0') && raw_flags[i] <= u16::from(b'9') {
                group.push(raw_flags[i]);
            }
        }
        Ok(result)
    }

    fn print_flag(&self, flag: u16) -> Vec<u16> {
        units(&flag.to_string())
    }
}

/// Implementation of [`FlagParsingStrategy`] that assumes each flag is
/// encoded as two ASCII characters whose codes must be combined into a
/// single character.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$DoubleASCIIFlagParsingStrategy`.
#[derive(Debug)]
pub(crate) struct DoubleASCIIFlagParsingStrategy;

impl FlagParsingStrategy for DoubleASCIIFlagParsingStrategy {
    fn class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.hunspell.Dictionary$DoubleASCIIFlagParsingStrategy"
    }

    fn parse_flags(&self, raw_flags: &[u16]) -> Result<Vec<u16>> {
        let mut flags = vec![0u16; raw_flags.len() / 2];
        for (i, flag) in flags.iter_mut().enumerate() {
            let f1 = raw_flags[i * 2];
            let f2 = raw_flags[i * 2 + 1];
            if f1 >= 256 || f2 >= 256 {
                return Err(LuceneError::illegal_argument(format!(
                    "Invalid flags (LONG flags must be double ASCII): {}",
                    to_rust_string(raw_flags)
                )));
            }
            *flag = (f1 << 8) | f2;
        }
        Ok(flags)
    }

    fn print_flag(&self, flag: u16) -> Vec<u16> {
        vec![(flag & 0xff00) >> 8, flag & 0xff]
    }
}

/// Possible word breaks according to BREAK directives.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Dictionary$Breaks`.
#[derive(Debug, Clone)]
pub(crate) struct Breaks {
    pub(crate) starting: Vec<Vec<u16>>,
    pub(crate) ending: Vec<Vec<u16>>,
    pub(crate) middle: Vec<Vec<u16>>,
}

impl Breaks {
    /// Equivalent to `Breaks(Collection, Collection, Collection)`.
    pub(crate) fn new(
        starting: Vec<Vec<u16>>,
        ending: Vec<Vec<u16>>,
        middle: Vec<Vec<u16>>,
    ) -> Self {
        Self {
            starting,
            ending,
            middle,
        }
    }

    /// Equivalent to `Breaks.DEFAULT`: `-` at the start, the end and the
    /// middle.
    pub(crate) fn default_breaks() -> Self {
        let minus = vec![units("-")];
        Self::new(minus.clone(), minus.clone(), minus)
    }

    /// Equivalent to `isNotEmpty()`.
    pub(crate) fn is_not_empty(&self) -> bool {
        !self.middle.is_empty() || !self.starting.is_empty() || !self.ending.is_empty()
    }
}
