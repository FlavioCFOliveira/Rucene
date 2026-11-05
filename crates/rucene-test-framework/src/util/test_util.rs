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

//! Port of `org.apache.lucene.tests.util.TestUtil`: general utility methods
//! for Lucene unit tests.
//!
//! # Strings
//!
//! Java's generators build `String`s of UTF-16 code units and count lengths
//! in code units (`String.length()`, `StringBuilder.setLength`). Every
//! generator of this class produces well-formed UTF-16 (surrogates only in
//! pairs), so the port returns a Rust [`String`] holding the same code points,
//! and counts every length the way Java does, in UTF-16 code units.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::lang::double_to_string;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::analysis::factories::AbstractAnalysisFactory;
use rucene::codecs::per_field::{PerFieldDocValuesFormat, PerFieldPostingsFormat};
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, Document, Field, FieldType, IntField, KeywordField,
    NumericDocValuesField, NumericValue, SortedDocValuesField, Store,
};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::{self, CheckIndex, Level, Status};
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_file_names;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{LogMergePolicy, TieredMergePolicy};
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, MergeScheduler};
use rucene::index::{CodecReader, DocValuesType, IndexableField, LeafReader};
use rucene::index::{
    IndexReader, MultiTerms, PostingsEnum, TermsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{
    FieldDoc, ScoreDoc, SortValue, TopDocs, TopFieldDocs, TotalHits, TotalHitsRelation,
};
use rucene::store::java_nio_file::{FileSystem, FileSystemProvider, Path};
use rucene::store::FilterDirectory;
use rucene::store::{
    ByteBuffersDirectory, Directory, LockFactory, NoLockFactory, DEFAULT_IO_CONTEXT,
};
use rucene::util::attribute::AttributeImpl;
use rucene::util::automaton::case_folding;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::BytesRef;
use rucene_codecs::blocktreeords::BlockTreeOrdsPostingsFormat;

use crate::codecs::asserting::AssertingCodec;
use crate::codecs::blockterms::LuceneFixedGap;
use crate::hamcrest::{
    array_with_size, equal_to, greater_than_or_equal_to, instance_of, less_than_or_equal_to, not,
    HamcrestValue, JavaObject, MatcherAssert,
};
use crate::jdk::executor_service::ExecutorService;
use crate::mockfile::{FilterFileSystem, VirusCheckingFS, WindowsFS};
use rucene::codecs::lucene99::hnsw_vectors_format::{
    DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN, DEFAULT_NUM_MERGE_WORKER,
};
use rucene::codecs::{
    Codec, DocValuesFormat, KnnVectorsFormat, Lucene104Codec, Lucene104PostingsFormat,
    Lucene90DocValuesFormat, Lucene99HnswVectorsFormat, PostingsFormat,
};

/// Port of `org.apache.lucene.tests.util.TestUtil`.
pub struct TestUtil;

/// `ops`: the operators of [`TestUtil::random_regexpish_string_with`], whose
/// `{0,5}` and `{1,5}` are Java's `"{0," + maxRecursionBound + "}"` and
/// `"{1," + maxRecursionBound + "}"` with `maxRecursionBound = 5`, the bounded
/// replacements for `'*'` and `'+'` (LUCENE-4111).
const OPS: [&str; 10] = [".", "?", "{0,5}", "{1,5}", "(", ")", "-", "[", "]", "|"];

/// `HTML_CHAR_ENTITIES`.
const HTML_CHAR_ENTITIES: [&str; 259] = [
    "AElig", "Aacute", "Acirc", "Agrave", "Alpha", "AMP", "Aring", "Atilde", "Auml", "Beta",
    "COPY", "Ccedil", "Chi", "Dagger", "Delta", "ETH", "Eacute", "Ecirc", "Egrave", "Epsilon",
    "Eta", "Euml", "Gamma", "GT", "Iacute", "Icirc", "Igrave", "Iota", "Iuml", "Kappa", "Lambda",
    "LT", "Mu", "Ntilde", "Nu", "OElig", "Oacute", "Ocirc", "Ograve", "Omega", "Omicron", "Oslash",
    "Otilde", "Ouml", "Phi", "Pi", "Prime", "Psi", "QUOT", "REG", "Rho", "Scaron", "Sigma",
    "THORN", "Tau", "Theta", "Uacute", "Ucirc", "Ugrave", "Upsilon", "Uuml", "Xi", "Yacute",
    "Yuml", "Zeta", "aacute", "acirc", "acute", "aelig", "agrave", "alefsym", "alpha", "amp",
    "and", "ang", "apos", "aring", "asymp", "atilde", "auml", "bdquo", "beta", "brvbar", "bull",
    "cap", "ccedil", "cedil", "cent", "chi", "circ", "clubs", "cong", "copy", "crarr", "cup",
    "curren", "dArr", "dagger", "darr", "deg", "delta", "diams", "divide", "eacute", "ecirc",
    "egrave", "empty", "emsp", "ensp", "epsilon", "equiv", "eta", "eth", "euml", "euro", "exist",
    "fnof", "forall", "frac12", "frac14", "frac34", "frasl", "gamma", "ge", "gt", "hArr", "harr",
    "hearts", "hellip", "iacute", "icirc", "iexcl", "igrave", "image", "infin", "int", "iota",
    "iquest", "isin", "iuml", "kappa", "lArr", "lambda", "lang", "laquo", "larr", "lceil", "ldquo",
    "le", "lfloor", "lowast", "loz", "lrm", "lsaquo", "lsquo", "lt", "macr", "mdash", "micro",
    "middot", "minus", "mu", "nabla", "nbsp", "ndash", "ne", "ni", "not", "notin", "nsub",
    "ntilde", "nu", "oacute", "ocirc", "oelig", "ograve", "oline", "omega", "omicron", "oplus",
    "or", "ordf", "ordm", "oslash", "otilde", "otimes", "ouml", "para", "part", "permil", "perp",
    "phi", "pi", "piv", "plusmn", "pound", "prime", "prod", "prop", "psi", "quot", "rArr", "radic",
    "rang", "raquo", "rarr", "rceil", "rdquo", "real", "reg", "rfloor", "rho", "rlm", "rsaquo",
    "rsquo", "sbquo", "scaron", "sdot", "sect", "shy", "sigma", "sigmaf", "sim", "spades", "sub",
    "sube", "sum", "sup", "sup1", "sup2", "sup3", "supe", "szlig", "tau", "there4", "theta",
    "thetasym", "thinsp", "thorn", "tilde", "times", "trade", "uArr", "uacute", "uarr", "ucirc",
    "ugrave", "uml", "upsih", "upsilon", "uuml", "weierp", "xi", "yacute", "yen", "yuml", "zeta",
    "zwj", "zwnj",
];

/// `blockStarts`.
#[rustfmt::skip]
const BLOCK_STARTS: [i32; 194] = [
    0x0000, 0x0080, 0x0100, 0x0180, 0x0250, 0x02B0, 0x0300, 0x0370, 0x0400, 0x0500, 0x0530,
    0x0590, 0x0600, 0x0700, 0x0750, 0x0780, 0x07C0, 0x0800, 0x0900, 0x0980, 0x0A00, 0x0A80,
    0x0B00, 0x0B80, 0x0C00, 0x0C80, 0x0D00, 0x0D80, 0x0E00, 0x0E80, 0x0F00, 0x1000, 0x10A0,
    0x1100, 0x1200, 0x1380, 0x13A0, 0x1400, 0x1680, 0x16A0, 0x1700, 0x1720, 0x1740, 0x1760,
    0x1780, 0x1800, 0x18B0, 0x1900, 0x1950, 0x1980, 0x19E0, 0x1A00, 0x1A20, 0x1B00, 0x1B80,
    0x1C00, 0x1C50, 0x1CD0, 0x1D00, 0x1D80, 0x1DC0, 0x1E00, 0x1F00, 0x2000, 0x2070, 0x20A0,
    0x20D0, 0x2100, 0x2150, 0x2190, 0x2200, 0x2300, 0x2400, 0x2440, 0x2460, 0x2500, 0x2580,
    0x25A0, 0x2600, 0x2700, 0x27C0, 0x27F0, 0x2800, 0x2900, 0x2980, 0x2A00, 0x2B00, 0x2C00,
    0x2C60, 0x2C80, 0x2D00, 0x2D30, 0x2D80, 0x2DE0, 0x2E00, 0x2E80, 0x2F00, 0x2FF0, 0x3000,
    0x3040, 0x30A0, 0x3100, 0x3130, 0x3190, 0x31A0, 0x31C0, 0x31F0, 0x3200, 0x3300, 0x3400,
    0x4DC0, 0x4E00, 0xA000, 0xA490, 0xA4D0, 0xA500, 0xA640, 0xA6A0, 0xA700, 0xA720, 0xA800,
    0xA830, 0xA840, 0xA880, 0xA8E0, 0xA900, 0xA930, 0xA960, 0xA980, 0xAA00, 0xAA60, 0xAA80,
    0xABC0, 0xAC00, 0xD7B0, 0xE000, 0xF900, 0xFB00, 0xFB50, 0xFE00, 0xFE10, 0xFE20, 0xFE30,
    0xFE50, 0xFE70, 0xFF00, 0xFFF0, 0x10000, 0x10080, 0x10100, 0x10140, 0x10190, 0x101D0,
    0x10280, 0x102A0, 0x10300, 0x10330, 0x10380, 0x103A0, 0x10400, 0x10450, 0x10480, 0x10800,
    0x10840, 0x10900, 0x10920, 0x10A00, 0x10A60, 0x10B00, 0x10B40, 0x10B60, 0x10C00, 0x10E60,
    0x11080, 0x12000, 0x12400, 0x13000, 0x1D000, 0x1D100, 0x1D200, 0x1D300, 0x1D360, 0x1D400,
    0x1F000, 0x1F030, 0x1F100, 0x1F200, 0x20000, 0x2A700, 0x2F800, 0xE0000, 0xE0100, 0xF0000,
    0x100000,
];

/// `blockEnds`.
#[rustfmt::skip]
const BLOCK_ENDS: [i32; 194] = [
    0x007F, 0x00FF, 0x017F, 0x024F, 0x02AF, 0x02FF, 0x036F, 0x03FF, 0x04FF, 0x052F, 0x058F,
    0x05FF, 0x06FF, 0x074F, 0x077F, 0x07BF, 0x07FF, 0x083F, 0x097F, 0x09FF, 0x0A7F, 0x0AFF,
    0x0B7F, 0x0BFF, 0x0C7F, 0x0CFF, 0x0D7F, 0x0DFF, 0x0E7F, 0x0EFF, 0x0FFF, 0x109F, 0x10FF,
    0x11FF, 0x137F, 0x139F, 0x13FF, 0x167F, 0x169F, 0x16FF, 0x171F, 0x173F, 0x175F, 0x177F,
    0x17FF, 0x18AF, 0x18FF, 0x194F, 0x197F, 0x19DF, 0x19FF, 0x1A1F, 0x1AAF, 0x1B7F, 0x1BBF,
    0x1C4F, 0x1C7F, 0x1CFF, 0x1D7F, 0x1DBF, 0x1DFF, 0x1EFF, 0x1FFF, 0x206F, 0x209F, 0x20CF,
    0x20FF, 0x214F, 0x218F, 0x21FF, 0x22FF, 0x23FF, 0x243F, 0x245F, 0x24FF, 0x257F, 0x259F,
    0x25FF, 0x26FF, 0x27BF, 0x27EF, 0x27FF, 0x28FF, 0x297F, 0x29FF, 0x2AFF, 0x2BFF, 0x2C5F,
    0x2C7F, 0x2CFF, 0x2D2F, 0x2D7F, 0x2DDF, 0x2DFF, 0x2E7F, 0x2EFF, 0x2FDF, 0x2FFF, 0x303F,
    0x309F, 0x30FF, 0x312F, 0x318F, 0x319F, 0x31BF, 0x31EF, 0x31FF, 0x32FF, 0x33FF, 0x4DBF,
    0x4DFF, 0x9FFF, 0xA48F, 0xA4CF, 0xA4FF, 0xA63F, 0xA69F, 0xA6FF, 0xA71F, 0xA7FF, 0xA82F,
    0xA83F, 0xA87F, 0xA8DF, 0xA8FF, 0xA92F, 0xA95F, 0xA97F, 0xA9DF, 0xAA5F, 0xAA7F, 0xAADF,
    0xABFF, 0xD7AF, 0xD7FF, 0xF8FF, 0xFAFF, 0xFB4F, 0xFDFF, 0xFE0F, 0xFE1F, 0xFE2F, 0xFE4F,
    0xFE6F, 0xFEFF, 0xFFEF, 0xFFFF, 0x1007F, 0x100FF, 0x1013F, 0x1018F, 0x101CF, 0x101FF,
    0x1029F, 0x102DF, 0x1032F, 0x1034F, 0x1039F, 0x103DF, 0x1044F, 0x1047F, 0x104AF, 0x1083F,
    0x1085F, 0x1091F, 0x1093F, 0x10A5F, 0x10A7F, 0x10B3F, 0x10B5F, 0x10B7F, 0x10C4F, 0x10E7F,
    0x110CF, 0x123FF, 0x1247F, 0x1342F, 0x1D0FF, 0x1D1FF, 0x1D24F, 0x1D35F, 0x1D37F, 0x1D7FF,
    0x1F02F, 0x1F09F, 0x1F1FF, 0x1F2FF, 0x2A6DF, 0x2B73F, 0x2FA1F, 0xE007F, 0xE01EF, 0xFFFFF,
    0x10FFFF,
];

impl TestUtil {
    /// `getDefaultCodec()`: the actual default codec (e.g.
    /// `LuceneMNCodec`) for this version of Lucene, `new Lucene104Codec()`.
    pub fn get_default_codec() -> Arc<dyn Codec> {
        Arc::new(Lucene104Codec::new())
    }

    /// `getDefaultPostingsFormat()`: the actual default postings format
    /// (e.g. `LuceneMNPostingsFormat`), `new Lucene104PostingsFormat()`.
    pub fn get_default_postings_format() -> Arc<dyn PostingsFormat> {
        Arc::new(Lucene104PostingsFormat::new())
    }

    /// `getDefaultPostingsFormat(minItemsPerBlock, maxItemsPerBlock)`:
    /// `new Lucene104PostingsFormat(minItemsPerBlock, maxItemsPerBlock)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for block sizes the block-tree writer
    /// refuses, as Java's constructor throws.
    pub fn get_default_postings_format_with(
        min_items_per_block: i32,
        max_items_per_block: i32,
    ) -> Arc<dyn PostingsFormat> {
        Arc::new(Lucene104PostingsFormat::with_block_sizes(
            min_items_per_block,
            max_items_per_block,
        ))
    }

    /// `getDefaultDocValuesFormat()`: the actual default doc-values format,
    /// `new Lucene90DocValuesFormat()`.
    pub fn get_default_doc_values_format() -> Arc<dyn DocValuesFormat> {
        Arc::new(Lucene90DocValuesFormat::new())
    }

    /// `getDefaultKnnVectorsFormat()`: `new
    /// Lucene99HnswVectorsFormat(DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH, 0)`,
    /// whose `tinySegmentsThreshold` of 0 builds a graph for every segment.
    pub fn get_default_knn_vectors_format() -> Arc<dyn KnnVectorsFormat> {
        Arc::new(Lucene99HnswVectorsFormat::with_merge_exec_and_threshold(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            0,
        ))
    }

    /// `nextInt(r, start, end)`: a number in `[start, end]`,
    /// `RandomNumbers.randomIntBetween(r, start, end)`.
    ///
    /// # Panics
    ///
    /// As `RandomNumbers.randomIntBetween`.
    pub fn next_int<R: Random + ?Sized>(r: &R, start: i32, end: i32) -> i32 {
        RandomNumbers::random_int_between(r, start, end)
    }

    /// `floatUlpEquals(x, y, maxUlps)`: whether `x` and `y` are within
    /// `maxUlps` units in the last place, over their raw bits.
    pub fn float_ulp_equals(x: f32, y: f32, max_ulps: i16) -> bool {
        let x_int = x.to_bits() as i32;
        let y_int = y.to_bits() as i32;
        let max_ulps = i32::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            // Numbers have opposite signs, take care of overflow. Remove the
            // sign bit to obtain the absolute ULP above zero.
            let delta_plus = x_int & i32::MAX;
            let delta_minus = y_int & i32::MAX;
            // If either value is NaN, the exponent bits are set to (255 << 23)
            // and the distance above 0.0 is always above a short ULP error.
            // Avoid possible overflow from adding the deltas by splitting the
            // comparison.
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        // Numbers have same sign, there is no risk of overflow.
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }

    /// `doubleUlpEquals(x, y, maxUlps)`.
    pub fn double_ulp_equals(x: f64, y: f64, max_ulps: i32) -> bool {
        let x_int = x.to_bits() as i64;
        let y_int = y.to_bits() as i64;
        let max_ulps = i64::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            let delta_plus = x_int & i64::MAX;
            let delta_minus = y_int & i64::MAX;
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }

    /// `nextLong(r, start, end)`: a number in `[start, end]`, both
    /// inclusive.
    ///
    /// A range of at most `Integer.MAX_VALUE` values draws `r.nextInt(range)`;
    /// a larger one multiplies the range by `r.nextDouble()` exactly (Java's
    /// `BigDecimal` product) and truncates, which the port computes exactly in
    /// 128-bit integers: the range is below 2^65 and the double's significand
    /// below 2^53.
    ///
    /// # Panics
    ///
    /// `AssertionError` (`start=…,end=…`) under assertions when `end < start`.
    pub fn next_long<R: Random + ?Sized>(r: &R, start: i64, end: i64) -> i64 {
        java_assert_unchecked!(end >= start, "start={start},end={end}");
        let range = i128::from(end) + 1 - i128::from(start);
        if range <= i128::from(i32::MAX) {
            start + i64::from(r.next_int_bound(range as i32))
        } else {
            // probably not evenly distributed when range is large, but OK for tests
            let augend = Self::truncated_product(range as u128, r.next_double());
            let result = (i128::from(start) + augend as i128) as i64;
            java_assert_unchecked!(result >= start);
            java_assert_unchecked!(result <= end);
            result
        }
    }

    /// `new BigDecimal(range).multiply(new BigDecimal(d)).toBigInteger()` for
    /// `0 <= d < 1` and `range < 2^65`: the exact product, truncated.
    fn truncated_product(range: u128, d: f64) -> u128 {
        let bits = d.to_bits();
        let biased_exponent = ((bits >> 52) & 0x7ff) as i32;
        let fraction = u128::from(bits & ((1u64 << 52) - 1));
        // d = significand * 2^exponent, with exponent < 0 for d < 1.
        let (significand, exponent) = if biased_exponent == 0 {
            (fraction, -1074)
        } else {
            (fraction | (1u128 << 52), biased_exponent - 1075)
        };
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            (range * significand) >> shift
        }
    }

    /// `nextBigInteger(random, maxBytes)`: a randomish big integer with `1 ..
    /// maxBytes` storage, `new BigInteger(buffer)` over `nextInt(random, 1,
    /// maxBytes)` random bytes.
    ///
    /// **Divergence.** Rust has no `java.math.BigInteger`; as
    /// `NumericUtils::big_int_to_sortable_bytes` does in core, the value is
    /// returned in the form `BigInteger.toByteArray()` returns: its minimal
    /// big-endian two's-complement bytes.
    pub fn next_big_integer<R: Random + ?Sized>(random: &R, max_bytes: i32) -> Vec<u8> {
        let length = Self::next_int(random, 1, max_bytes);
        let mut buffer = vec![0u8; length as usize];
        random.next_bytes(&mut buffer);
        // BigInteger(byte[]) drops the redundant sign bytes, which
        // toByteArray() never writes back.
        let mut start = 0;
        while start + 1 < buffer.len() {
            let (b, next) = (buffer[start], buffer[start + 1]);
            if (b == 0 && next & 0x80 == 0) || (b == 0xff && next & 0x80 != 0) {
                start += 1;
            } else {
                break;
            }
        }
        buffer.split_off(start)
    }

    /// `randomSimpleString(r, maxLength)`: `randomSimpleString(r, 0,
    /// maxLength)`.
    pub fn random_simple_string_max<R: Random + ?Sized>(r: &R, max_length: i32) -> String {
        Self::random_simple_string_between(r, 0, max_length)
    }

    /// `randomSimpleString(r, minLength, maxLength)`: `nextInt(r, minLength,
    /// maxLength)` characters in `'a'..='z'`.
    pub fn random_simple_string_between<R: Random + ?Sized>(
        r: &R,
        min_length: i32,
        max_length: i32,
    ) -> String {
        let end = Self::next_int(r, min_length, max_length);
        if end == 0 {
            // allow 0 length
            return String::new();
        }
        (0..end)
            .map(|_| char::from(Self::next_int(r, i32::from(b'a'), i32::from(b'z')) as u8))
            .collect()
    }

    /// `randomSimpleStringRange(r, minChar, maxChar, maxLength)`: `nextInt(r,
    /// 0, maxLength)` code units in `minChar..=maxChar`.
    ///
    /// # Panics
    ///
    /// When the range holds a surrogate code unit, which a Rust [`String`]
    /// cannot hold (Java would return a malformed `String`).
    pub fn random_simple_string_range<R: Random + ?Sized>(
        r: &R,
        min_char: u16,
        max_char: u16,
        max_length: i32,
    ) -> String {
        let end = Self::next_int(r, 0, max_length);
        if end == 0 {
            // allow 0 length
            return String::new();
        }
        let buffer: Vec<u16> = (0..end)
            .map(|_| Self::next_int(r, i32::from(min_char), i32::from(max_char)) as u16)
            .collect();
        String::from_utf16(&buffer).expect("a surrogate code unit cannot be held by a Rust String")
    }

    /// `randomSimpleString(r)`: `randomSimpleString(r, 0, 10)`.
    pub fn random_simple_string<R: Random + ?Sized>(r: &R) -> String {
        Self::random_simple_string_between(r, 0, 10)
    }

    /// `randomUnicodeString(r)`: random string, including full unicode range,
    /// `randomUnicodeString(r, 20)`.
    pub fn random_unicode_string<R: Random + ?Sized>(r: &R) -> String {
        Self::random_unicode_string_max(r, 20)
    }

    /// `randomUnicodeString(r, maxLength)`: a random string of up to
    /// `maxLength` UTF-16 code units.
    pub fn random_unicode_string_max<R: Random + ?Sized>(r: &R, max_length: i32) -> String {
        let end = Self::next_int(r, 0, max_length);
        if end == 0 {
            // allow 0 length
            return String::new();
        }
        let mut buffer = vec![0u16; end as usize];
        Self::random_fixed_length_unicode_string(r, &mut buffer, 0, end as usize);
        utf16_to_string(&buffer)
    }

    /// `randomFixedLengthUnicodeString(random, chars, offset, length)`: fills
    /// `chars[offset..offset + length]` with a valid random unicode code unit
    /// sequence.
    ///
    /// Java makes a surrogate pair only while `i < length - 1`, comparing the
    /// absolute index with the *length*; the port keeps that comparison.
    ///
    /// # Panics
    ///
    /// Where Java throws `ArrayIndexOutOfBoundsException`: a surrogate pair
    /// started at the last index of `chars`.
    pub fn random_fixed_length_unicode_string<R: Random + ?Sized>(
        random: &R,
        chars: &mut [u16],
        offset: usize,
        length: usize,
    ) {
        let mut i = offset;
        let end = offset + length;
        while i < end {
            let t = random.next_int_bound(5);
            if t == 0 && (i as i64) < length as i64 - 1 {
                // Make a surrogate pair
                // High surrogate
                chars[i] = Self::next_int(random, 0xd800, 0xdbff) as u16;
                i += 1;
                // Low surrogate
                chars[i] = Self::next_int(random, 0xdc00, 0xdfff) as u16;
                i += 1;
            } else if t <= 1 {
                chars[i] = random.next_int_bound(0x80) as u16;
                i += 1;
            } else if t == 2 {
                chars[i] = Self::next_int(random, 0x80, 0x7ff) as u16;
                i += 1;
            } else if t == 3 {
                chars[i] = Self::next_int(random, 0x800, 0xd7ff) as u16;
                i += 1;
            } else if t == 4 {
                chars[i] = Self::next_int(random, 0xe000, 0xffff) as u16;
                i += 1;
            }
        }
    }

    /// `randomRegexpishString(r)`: `randomRegexpishString(r, 20)`.
    pub fn random_regexpish_string<R: Random + ?Sized>(r: &R) -> String {
        Self::random_regexpish_string_with(r, 20)
    }

    /// `randomRegexpishString(r, maxLength)`: a string that's "regexpish"
    /// (contains lots of operators typically found in regular expressions);
    /// `maxLength` is a hint that a few characters may exceed.
    pub fn random_regexpish_string_with<R: Random + ?Sized>(r: &R, max_length: i32) -> String {
        let mut regexp = String::with_capacity(max_length.max(0) as usize);
        let mut i = Self::next_int(r, 0, max_length);
        while i > 0 {
            if r.next_boolean() {
                regexp.push(char::from(RandomNumbers::random_int_between(
                    r,
                    i32::from(b'a'),
                    i32::from(b'z'),
                ) as u8));
            } else {
                regexp.push_str(RandomPicks::random_from_list(r, &OPS));
            }
            i -= 1;
        }
        regexp
    }

    /// `randomHtmlishString(random, numElements)`.
    pub fn random_htmlish_string<R: Random + ?Sized>(random: &R, num_elements: i32) -> String {
        let end = Self::next_int(random, 0, num_elements);
        if end == 0 {
            // allow 0 length
            return String::new();
        }
        // `"…".substring(nextInt(random, 0, n))` of an n-character string.
        let substring = |s: &'static str, from: i32| &s[from as usize..];
        let mut sb = String::new();
        for _ in 0..end {
            let val = random.next_int_bound(25);
            match val {
                0 => sb.push_str("<p>"),
                1 => {
                    sb.push('<');
                    sb.push_str(substring("    ", Self::next_int(random, 0, 4)));
                    sb.push_str(&Self::random_simple_string(random));
                    let mut j = 0;
                    while j < Self::next_int(random, 0, 10) {
                        sb.push(' ');
                        sb.push_str(&Self::random_simple_string(random));
                        sb.push_str(substring(" ", Self::next_int(random, 0, 1)));
                        sb.push('=');
                        sb.push_str(substring(" ", Self::next_int(random, 0, 1)));
                        sb.push_str(substring("\"", Self::next_int(random, 0, 1)));
                        sb.push_str(&Self::random_simple_string(random));
                        sb.push_str(substring("\"", Self::next_int(random, 0, 1)));
                        j += 1;
                    }
                    sb.push_str(substring("    ", Self::next_int(random, 0, 4)));
                    sb.push_str(substring("/", Self::next_int(random, 0, 1)));
                    sb.push_str(substring(">", Self::next_int(random, 0, 1)));
                }
                2 => {
                    sb.push_str("</");
                    sb.push_str(substring("    ", Self::next_int(random, 0, 4)));
                    sb.push_str(&Self::random_simple_string(random));
                    sb.push_str(substring("    ", Self::next_int(random, 0, 4)));
                    sb.push_str(substring(">", Self::next_int(random, 0, 1)));
                }
                3 => sb.push('>'),
                4 => sb.push_str("</p>"),
                5 => sb.push_str("<!--"),
                6 => sb.push_str("<!--#"),
                7 => sb.push_str("<script><!-- f('"),
                8 => sb.push_str("</script>"),
                9 => sb.push_str("<?"),
                10 => sb.push_str("?>"),
                11 => sb.push('"'),
                12 => sb.push_str("\\\""),
                13 => sb.push('\''),
                14 => sb.push_str("\\'"),
                15 => sb.push_str("-->"),
                16 => {
                    sb.push('&');
                    match Self::next_int(random, 0, 2) {
                        0 => sb.push_str(&Self::random_simple_string(random)),
                        1 => sb.push_str(
                            HTML_CHAR_ENTITIES
                                [random.next_int_bound(HTML_CHAR_ENTITIES.len() as i32) as usize],
                        ),
                        _ => {}
                    }
                    sb.push_str(substring(";", Self::next_int(random, 0, 1)));
                }
                17 => {
                    sb.push_str("&#");
                    if Self::next_int(random, 0, 1) == 0 {
                        sb.push_str(&Self::next_int(random, 0, i32::MAX - 1).to_string());
                        sb.push_str(substring(";", Self::next_int(random, 0, 1)));
                    }
                }
                18 => {
                    sb.push_str("&#x");
                    if Self::next_int(random, 0, 1) == 0 {
                        // Integer.toString(i, 16) of a non-negative int.
                        sb.push_str(&format!("{:x}", Self::next_int(random, 0, i32::MAX - 1)));
                        sb.push_str(substring(";", Self::next_int(random, 0, 1)));
                    }
                }
                19 => sb.push(';'),
                20 => sb.push_str(&Self::next_int(random, 0, i32::MAX - 1).to_string()),
                21 => sb.push('\n'),
                22 => sb.push_str(substring("          ", Self::next_int(random, 0, 10))),
                23 => {
                    sb.push('<');
                    if Self::next_int(random, 0, 3) == 0 {
                        sb.push_str(substring("          ", Self::next_int(random, 1, 10)));
                    }
                    if Self::next_int(random, 0, 1) == 0 {
                        sb.push('/');
                        if Self::next_int(random, 0, 3) == 0 {
                            sb.push_str(substring("          ", Self::next_int(random, 1, 10)));
                        }
                    }
                    match Self::next_int(random, 0, 3) {
                        0 => sb.push_str(&Self::randomly_recase_code_points(random, "script")),
                        1 => sb.push_str(&Self::randomly_recase_code_points(random, "style")),
                        2 => sb.push_str(&Self::randomly_recase_code_points(random, "br")),
                        // default: append nothing
                        _ => {}
                    }
                    sb.push_str(substring(">", Self::next_int(random, 0, 1)));
                }
                _ => sb.push_str(&Self::random_simple_string(random)),
            }
        }
        sb
    }

    /// `randomlyRecaseCodePoints(random, str)`: randomly upcases
    /// (`Character.toUpperCase(int)`, the simple mapping), downcases or
    /// leaves intact each code point of `str`.
    pub fn randomly_recase_code_points<R: Random + ?Sized>(random: &R, str: &str) -> String {
        let mut builder = String::with_capacity(str.len());
        for c in str.chars() {
            let code_point = c as i32;
            let mapped = match Self::next_int(random, 0, 2) {
                0 => case_folding::to_upper_case(code_point),
                1 => case_folding::to_lower_case(code_point),
                // leave intact
                _ => code_point,
            };
            builder.push(
                char::from_u32(mapped as u32)
                    .expect("INVARIANT: a simple case mapping maps a scalar value to one"),
            );
        }
        builder
    }

    /// `randomRealisticUnicodeString(r)`: between 0 and 20 code points, all
    /// within the same unicode block.
    pub fn random_realistic_unicode_string<R: Random + ?Sized>(r: &R) -> String {
        Self::random_realistic_unicode_string_max(r, 20)
    }

    /// `randomRealisticUnicodeString(r, maxLength)`: up to `maxLength` code
    /// points, all within the same unicode block.
    pub fn random_realistic_unicode_string_max<R: Random + ?Sized>(
        r: &R,
        max_length: i32,
    ) -> String {
        Self::random_realistic_unicode_string_between(r, 0, max_length)
    }

    /// `randomRealisticUnicodeString(r, minLength, maxLength)`: between
    /// `minLength` and `maxLength` code points, all within the same unicode
    /// block.
    pub fn random_realistic_unicode_string_between<R: Random + ?Sized>(
        r: &R,
        min_length: i32,
        max_length: i32,
    ) -> String {
        let end = Self::next_int(r, min_length, max_length);
        let block = r.next_int_bound(BLOCK_STARTS.len() as i32) as usize;
        (0..end)
            .map(|_| {
                let code_point = Self::next_int(r, BLOCK_STARTS[block], BLOCK_ENDS[block]);
                char::from_u32(code_point as u32)
                    .expect("INVARIANT: no unicode block of the table holds a surrogate")
            })
            .collect()
    }

    /// `randomFixedByteLengthUnicodeString(r, length)`: a random string whose
    /// UTF-8 encoding is `length` bytes.
    pub fn random_fixed_byte_length_unicode_string<R: Random + ?Sized>(
        r: &R,
        length: i32,
    ) -> String {
        let mut buffer = vec![0u16; length as usize * 3];
        let mut bytes = length;
        let mut i = 0;
        while i < buffer.len() && bytes != 0 {
            let t = if bytes >= 4 {
                r.next_int_bound(5)
            } else if bytes == 3 {
                r.next_int_bound(4)
            } else if bytes == 2 {
                r.next_int_bound(2)
            } else {
                0
            };
            if t == 0 {
                buffer[i] = r.next_int_bound(0x80) as u16;
                bytes -= 1;
            } else if t == 1 {
                buffer[i] = Self::next_int(r, 0x80, 0x7ff) as u16;
                bytes -= 2;
            } else if t == 2 {
                buffer[i] = Self::next_int(r, 0x800, 0xd7ff) as u16;
                bytes -= 3;
            } else if t == 3 {
                buffer[i] = Self::next_int(r, 0xe000, 0xffff) as u16;
                bytes -= 3;
            } else if t == 4 {
                // Make a surrogate pair
                // High surrogate
                buffer[i] = Self::next_int(r, 0xd800, 0xdbff) as u16;
                i += 1;
                // Low surrogate
                buffer[i] = Self::next_int(r, 0xdc00, 0xdfff) as u16;
                bytes -= 4;
            }
            i += 1;
        }
        utf16_to_string(&buffer[..i])
    }

    /// `randomBinaryTerm(r)`: `randomBinaryTerm(r, r.nextInt(15))`.
    pub fn random_binary_term<R: Random + ?Sized>(r: &R) -> BytesRef {
        let length = r.next_int_bound(15);
        Self::random_binary_term_with(r, length)
    }

    /// `randomBinaryTerm(r, length)`: `new BytesRef(length)` whose whole
    /// array `r.nextBytes` fills.
    pub fn random_binary_term_with<R: Random + ?Sized>(r: &R, length: i32) -> BytesRef {
        let mut bytes = vec![0u8; length as usize];
        r.next_bytes(&mut bytes);
        BytesRef::new(bytes)
    }

    /// `STRING_CODEPOINT_COMPARATOR`: compares two strings code point by code
    /// point, returning Java's `-1`, `1`, or the difference of the code
    /// point counts.
    pub fn string_codepoint_compare(a: &str, b: &str) -> i32 {
        let a_code_points: Vec<char> = a.chars().collect();
        let b_code_points: Vec<char> = b.chars().collect();
        for (x, y) in a_code_points.iter().zip(&b_code_points) {
            if x < y {
                return -1;
            } else if x > y {
                return 1;
            }
        }
        a_code_points.len() as i32 - b_code_points.len() as i32
    }

    /// `stringToCharSequence(string, random)`: `bytesToCharSequence(new
    /// BytesRef(string), random)`.
    pub fn string_to_char_sequence<R: Random + ?Sized>(string: &str, random: &R) -> String {
        Self::bytes_to_char_sequence(&BytesRef::from_char_sequence(string), random)
    }

    /// `bytesToCharSequence(ref, random)`: the UTF-8 `ref` decoded, as a
    /// `CharsRef` (`random.nextInt(5) == 4`), a `CharBuffer` (`3`) or a
    /// `String`.
    ///
    /// **Divergence.** The three Java `CharSequence` classes hold the same
    /// characters and Rust has no `CharSequence` hierarchy: the port draws
    /// the same `nextInt(5)` and returns the characters as a [`String`].
    ///
    /// # Panics
    ///
    /// When `ref` is not UTF-8 (Java decodes it leniently).
    pub fn bytes_to_char_sequence<R: Random + ?Sized>(r#ref: &BytesRef, random: &R) -> String {
        random.next_int_bound(5);
        r#ref
            .utf8_to_string()
            .expect("bytesToCharSequence of a BytesRef that is not UTF-8")
    }

    /// `randomPattern(random)`: a valid (compiling) regular expression with
    /// random stuff inside: the first `randomRegexpishString(random)` that
    /// `Pattern.compile` accepts and whose `replaceAll("_")` over
    /// `"AB𠀀C"` is valid UTF-16 (LUCENE-4078).
    ///
    /// **Divergence.** Rust has no `java.util.regex.Pattern`. The port
    /// compiles each candidate as core's
    /// `AbstractAnalysisFactory::get_pattern` compiles a Java pattern (its
    /// declared dialect limits apply) and returns the accepted pattern's
    /// text, Java's `Pattern.pattern()`, for `get_pattern` to compile.
    ///
    /// Java's check runs `Matcher.find` over UTF-16 indexes: a pattern of
    /// ASCII characters starts a match attempt at every `char` index,
    /// including index 3 between the two surrogates, and an empty match
    /// advances the next search by one `char`. Any match that starts at index
    /// 3 splits the pair (an empty one inserts `_` between the surrogates,
    /// another replaces the low one alone), so the replacement is invalid
    /// exactly when the `find` sequence yields a match starting at 3. The port
    /// replays that sequence ([`java_find_splits_pair`]); a Rust string cannot
    /// hold the lone `\uDC00` an attempt at index 3 sees, so that attempt runs
    /// on `"\u{E000}C"`: of the constructs `randomRegexpishString` writes
    /// (letters, `.`, `?`, `{m,n}`, groups, `|`, and classes of ASCII
    /// characters, never negated), only `.` matches either character, as it
    /// matches the lone surrogate in Java.
    pub fn random_pattern<R: Random + ?Sized>(random: &R) -> String {
        loop {
            let pattern = Self::random_regexpish_string(random);
            if compile_java_pattern(&pattern).is_some() && !java_find_splits_pair(&pattern) {
                return pattern;
            }
            // Loop trying until we hit something that compiles.
        }
    }

    /// `randomAnalysisString(random, maxLength, simple)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` of `random.nextInt(0)` when `maxLength` is
    /// 0, as Java.
    pub fn random_analysis_string<R: Random + ?Sized>(
        random: &R,
        max_length: i32,
        simple: bool,
    ) -> String {
        java_assert_unchecked!(max_length >= 0);

        // sometimes just a purely random string
        if random.next_int_bound(31) == 0 {
            let word_length = random.next_int_bound(max_length);
            return Self::random_sub_string(random, word_length, simple);
        }

        // otherwise, try to make it more realistic with 'words' since most tests use MockTokenizer
        // first decide how big the string will really be: 0..n
        let max_length = random.next_int_bound(max_length);
        let avg_word_length = Self::next_int(random, 3, 8);
        let mut sb: Vec<u16> = Vec::new();
        while (sb.len() as i32) < max_length {
            if !sb.is_empty() {
                sb.push(u16::from(b' '));
            }
            let mut word_length = -1;
            while word_length < 0 {
                word_length = (random.next_gaussian() * 3.0 + f64::from(avg_word_length)) as i32;
            }
            let word_length = word_length.min(max_length - sb.len() as i32);
            sb.extend(Self::random_sub_string(random, word_length, simple).encode_utf16());
        }
        utf16_to_string(&sb)
    }

    /// `randomSubString(random, wordLength, simple)`: a string of
    /// `wordLength` UTF-16 code units (one fewer when the cut would split a
    /// surrogate pair).
    pub fn random_sub_string<R: Random + ?Sized>(
        random: &R,
        word_length: i32,
        simple: bool,
    ) -> String {
        if word_length == 0 {
            return String::new();
        }

        let evilness = Self::next_int(random, 0, 20);

        let mut sb: Vec<u16> = Vec::new();
        while (sb.len() as i32) < word_length {
            let part = if simple {
                if random.next_boolean() {
                    Self::random_simple_string_max(random, word_length)
                } else {
                    Self::random_htmlish_string(random, word_length)
                }
            } else if evilness < 10 {
                Self::random_simple_string_max(random, word_length)
            } else if evilness < 15 {
                // we should always get wordLength back!
                java_assert_unchecked!(sb.is_empty());
                Self::random_realistic_unicode_string_between(random, word_length, word_length)
            } else if evilness == 16 {
                Self::random_htmlish_string(random, word_length)
            } else if evilness == 17 {
                // gives a lot of punctuation
                Self::random_regexpish_string_with(random, word_length)
            } else {
                Self::random_unicode_string_max(random, word_length)
            };
            sb.extend(part.encode_utf16());
        }
        if sb.len() as i32 > word_length {
            sb.truncate(word_length as usize);
            if (0xd800..=0xdbff).contains(&sb[word_length as usize - 1]) {
                sb.truncate(word_length as usize - 1);
            }
        }
        let s = utf16_to_string(&sb);

        if random.next_int_bound(17) == 0 {
            // mix up case
            let mixed_up = Self::randomly_recase_code_points(random, &s);
            java_assert_unchecked!(mixed_up.encode_utf16().count() == s.encode_utf16().count());
            mixed_up
        } else {
            s
        }
    }

    /// `bytesRefToString(br)`: for debugging, `br.utf8ToString() + " " + br`,
    /// or `br.toString()` (hex) when the bytes do not decode; `"(null)"` for
    /// `None`.
    ///
    /// Java's `utf8ToString` (`UnicodeUtil.UTF8toUTF16`) decodes leniently and
    /// fails only on the `assert`s of a continuation or `0xf8..` lead byte;
    /// the port decodes the same way and falls back to hex on those bytes.
    /// **Divergence.** A lenient decoding that yields an unpaired surrogate
    /// is not a Rust string: its code unit becomes U+FFFD.
    pub fn bytes_ref_to_string(br: Option<&BytesRef>) -> String {
        match br {
            None => "(null)".to_string(),
            Some(br) => match lenient_utf8_to_utf16(br.slice()) {
                Some(units) => format!(
                    "{} {}",
                    String::from_utf16_lossy(&units),
                    br.to_hex_string()
                ),
                // If BytesRef isn't actually UTF8, or it's e.g. a
                // prefix of UTF8 that ends mid-unicode-char, we
                // fall back to hex:
                None => br.to_hex_string(),
            },
        }
    }

    /// `checkIndex(dir)`: `checkIndex(dir, MIN_LEVEL_FOR_SLOW_CHECKS)`.
    ///
    /// # Errors
    ///
    /// As [`check_index_with`](Self::check_index_with).
    pub fn check_index(dir: Arc<dyn Directory>) -> Result<Status> {
        Self::check_index_with_level(dir, Level::MIN_LEVEL_FOR_SLOW_CHECKS)
    }

    /// `checkIndex(dir, level)`: `checkIndex(dir, level, false, true,
    /// null)`.
    ///
    /// # Errors
    ///
    /// As [`check_index_with`](Self::check_index_with).
    pub fn check_index_with_level(dir: Arc<dyn Directory>, level: i32) -> Result<Status> {
        Self::check_index_with(dir, level, false, true, None)
    }

    /// `checkIndex(dir, level, failFast, concurrent, output)`: runs
    /// `CheckIndex` over `dir` (with a bogus `NoLockFactory` lock), with
    /// `RandomizedTest.randomIntBetween(2, 5)` threads when `concurrent`, its
    /// messages into `output`.
    ///
    /// Each message is one line of `output`, as Java's `PrintStream` holds
    /// it; core's `CheckIndex` reports whole messages (its declared
    /// divergence), so a label and its result Java prints with one `print`
    /// and one `println` are two messages there.
    ///
    /// # Errors
    ///
    /// `RuntimeException` (`CheckIndex failed`), after printing the output,
    /// when the index is not clean; the checker's own errors.
    pub fn check_index_with(
        dir: Arc<dyn Directory>,
        level: i32,
        fail_fast: bool,
        concurrent: bool,
        output: Option<&mut Vec<u8>>,
    ) -> Result<Status> {
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(1024)));
        // TODO: actually use the dir's locking, unless test uses a special method?
        // some tests e.g. exception tests become much more complicated if they have to close the writer
        let lock = NoLockFactory::instance().obtain_lock(&*dir, "bogus")?;
        let mut checker = CheckIndex::with_write_lock(dir, Some(lock));
        checker.set_level(level)?;
        checker.set_fail_fast(fail_fast);
        checker
            .set_info_stream_verbose(Some(Arc::new(PrintStreamLines(Arc::clone(&buffer)))), false);
        if concurrent {
            checker.set_thread_count(RandomizedTest::random_int_between(2, 5))?;
        } else {
            checker.set_thread_count(1)?;
        }
        let index_status = checker.check_index();
        checker.close()?;
        let index_status = index_status?;
        let text = buffer.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(output) = output {
            output.extend_from_slice(&text);
        }

        if !index_status.clean {
            println!("CheckIndex failed");
            println!("{}", String::from_utf8_lossy(&text));
            Err(LuceneError::Unchecked {
                class: "java.lang.RuntimeException",
                message: Some("CheckIndex failed".to_string()),
                cause: None,
            })
        } else {
            if *crate::util::lucene_test_case::INFOSTREAM {
                println!("{}", String::from_utf8_lossy(&text));
            }
            Ok(index_status)
        }
    }

    /// `alwaysPostingsFormat(format)`: a codec that can read any of the
    /// default codecs and formats, but always writes in `format`: an
    /// `AssertingCodec` whose `getPostingsFormatForField` returns `format`.
    pub fn always_postings_format(format: Arc<dyn PostingsFormat>) -> Arc<dyn Codec> {
        if *crate::util::lucene_test_case::VERBOSE {
            println!("forcing postings format to:{}", format.name());
        }
        Arc::new(AssertingCodec::with_formats_for_field(
            Some(Arc::new(move |_field: &str| Arc::clone(&format))),
            None,
            None,
        ))
    }

    /// `alwaysDocValuesFormat(format)`: an `AssertingCodec` whose
    /// `getDocValuesFormatForField` returns `format`.
    pub fn always_doc_values_format(format: Arc<dyn DocValuesFormat>) -> Arc<dyn Codec> {
        if *crate::util::lucene_test_case::VERBOSE {
            println!("TestUtil: forcing docvalues format to:{}", format.name());
        }
        Arc::new(AssertingCodec::with_formats_for_field(
            None,
            Some(Arc::new(move |_field: &str| Arc::clone(&format))),
            None,
        ))
    }

    /// `alwaysKnnVectorsFormat(format)`: an `AssertingCodec` whose
    /// `getKnnVectorsFormatForField` returns `format`.
    pub fn always_knn_vectors_format(format: Arc<dyn KnnVectorsFormat>) -> Arc<dyn Codec> {
        if *crate::util::lucene_test_case::VERBOSE {
            println!("TestUtil: forcing knn vectors format to:{}", format.name());
        }
        Arc::new(AssertingCodec::with_formats_for_field(
            None,
            None,
            Some(Arc::new(move |_field: &str| Arc::clone(&format))),
        ))
    }

    /// `getPostingsFormatWithOrds(r)`: a random postings format that supports
    /// term ordinals, `LuceneFixedGap` or `BlockTreeOrdsPostingsFormat`.
    pub fn get_postings_format_with_ords<R: Random + ?Sized>(r: &R) -> Arc<dyn PostingsFormat> {
        match r.next_int_bound(2) {
            0 => Arc::new(LuceneFixedGap::new()),
            1 => Arc::new(BlockTreeOrdsPostingsFormat::new()),
            // TODO: these don't actually support ords!
            // case 2: return new FSTOrdPostingsFormat();
            _ => unreachable!("Random.nextInt(2) is 0 or 1"),
        }
    }

    /// `anyFilesExceptWriteLock(dir)`: whether `dir` holds a file other than
    /// `write.lock`.
    ///
    /// # Errors
    ///
    /// The errors of `dir.listAll()`.
    pub fn any_files_except_write_lock(dir: &dyn Directory) -> Result<bool> {
        let files = dir.list_all()?;
        Ok(files.len() > 1 || (files.len() == 1 && files[0] != "write.lock"))
    }

    /// `ramCopyOf(dir)`: a `ByteBuffersDirectory` copy of the `segments*`
    /// and codec files (`IndexFileNames.CODEC_FILE_PATTERN`) of `dir`.
    ///
    /// # Errors
    ///
    /// The errors of listing and copying the files.
    pub fn ram_copy_of(dir: &dyn Directory) -> Result<Box<dyn Directory>> {
        let ram = ByteBuffersDirectory::new();
        for file in dir.list_all()? {
            if file.starts_with(index_file_names::SEGMENTS) || matches_codec_file_pattern(&file) {
                ram.copy_from(dir, &file, &file, &*DEFAULT_IO_CONTEXT)?;
            }
        }
        Ok(Box::new(ram))
    }

    /// `docs(random, r, field, term, reuse, flags)`: the postings of `term`
    /// in `field` of the reader, with random features ([`docs_of`]); `None`
    /// if the field or the term doesn't exist.
    ///
    /// # Errors
    ///
    /// The reader's errors.
    ///
    /// [`docs_of`]: Self::docs_of
    pub fn docs<R: Random + ?Sized>(
        random: &R,
        r: &Arc<dyn IndexReader>,
        field: &str,
        term: &BytesRef,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        let Some(terms) = MultiTerms::get_terms(r, field)? else {
            return Ok(None);
        };
        let mut terms_enum = terms.iterator()?;
        if !terms_enum.seek_exact(term)? {
            return Ok(None);
        }
        Self::docs_of(random, &mut *terms_enum, reuse, flags).map(Some)
    }

    /// `docs(random, termsEnum, reuse, flags)`: a `PostingsEnum` with random
    /// features available: half of the time positions, offsets, payloads or
    /// all of them (without `reuse`), else `flags` with `FREQS` half of the
    /// time.
    ///
    /// # Errors
    ///
    /// The terms enum's errors.
    pub fn docs_of<R: Random + ?Sized>(
        random: &R,
        terms_enum: &mut dyn TermsEnum,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let mut flags = flags;
        if random.next_boolean() {
            if random.next_boolean() {
                let pos_flags = match random.next_int_bound(4) {
                    0 => POSTINGS_ENUM_POSITIONS,
                    1 => POSTINGS_ENUM_OFFSETS,
                    2 => POSTINGS_ENUM_PAYLOADS,
                    _ => POSTINGS_ENUM_ALL,
                };
                return terms_enum.postings(None, pos_flags);
            }
            flags |= POSTINGS_ENUM_FREQS;
        }
        terms_enum.postings(reuse, flags)
    }

    /// `checkIterator(iterator, expectedSize, allowNull)`: the iterator
    /// returns `expected_size` elements, then none.
    ///
    /// Java also checks that `remove()` throws `UnsupportedOperationException`
    /// and, unless `allowNull`, that no element is `null`; a Rust
    /// [`Iterator`] has no `remove` and no `null` element, so those checks
    /// hold by type. The `NoSuchElementException` of `next()` past the end is
    /// `None`, which a second call must return again.
    ///
    /// # Panics
    ///
    /// The `AssertionError` of a broken iterator.
    pub fn check_iterator<I: Iterator>(mut iterator: I, expected_size: i64, _allow_null: bool) {
        for _ in 0..expected_size {
            assert!(iterator.next().is_some());
        }
        assert!(iterator.next().is_none());
        assert!(
            iterator.next().is_none(),
            "broken iterator (allows next() when hasNext==false)"
        );
    }

    /// `checkReadOnly(coll)`: the collection's size is the number of elements
    /// its iterator returns. Java also checks that `remove`, `add` and
    /// `addAll` throw `UnsupportedOperationException`; a collection behind a
    /// shared reference cannot be modified, which holds by type.
    ///
    /// # Panics
    ///
    /// `AssertionError` (`broken collection, reported size is …`).
    pub fn check_read_only<'c, T: 'c, C>(coll: &'c C, reported_size: usize)
    where
        &'c C: IntoIterator<Item = &'c T>,
    {
        let size = coll.into_iter().count();
        assert_eq!(
            size, reported_size,
            "broken collection, reported size is {reported_size} but iterator has {size} elements"
        );
        Self::check_iterator(coll.into_iter(), size as i64, false);
    }

    /// `unzip(in, destDir)`: extracts the zip `input` into the clean
    /// directory `dest_dir` (Java closes the stream; the port takes the
    /// bytes).
    ///
    /// The entries are read as `ZipInputStream` reads them, from their local
    /// headers in order: `STORED` or `DEFLATED` data, sized by the header or,
    /// with a data descriptor, by the end of the deflate stream.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing the files; `ZipException` for an entry
    /// `ZipInputStream` refuses.
    pub fn unzip(input: &[u8], dest_dir: &std::path::Path) -> std::io::Result<()> {
        let mut pos = 0;
        let u16_at = |p: usize| -> std::io::Result<usize> {
            input
                .get(p..p + 2)
                .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
        };
        let u32_at = |p: usize| -> std::io::Result<usize> {
            input
                .get(p..p + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))
        };
        // Local file headers (PK\x03\x04); anything else ends the entries.
        while input.get(pos..pos + 4) == Some(&b"PK\x03\x04"[..]) {
            let flags = u16_at(pos + 6)?;
            let method = u16_at(pos + 8)?;
            let compressed_size = u32_at(pos + 18)?;
            let name_len = u16_at(pos + 26)?;
            let extra_len = u16_at(pos + 28)?;
            let name_bytes = input
                .get(pos + 30..pos + 30 + name_len)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
            let name = String::from_utf8_lossy(name_bytes).into_owned();
            let data_start = pos + 30 + name_len + extra_len;
            let has_descriptor = flags & 8 != 0;
            let (data, used) = match method {
                0 => {
                    if has_descriptor {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "only DEFLATED entries can have EXT descriptor",
                        ));
                    }
                    let data = input
                        .get(data_start..data_start + compressed_size)
                        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::UnexpectedEof))?;
                    (data.to_vec(), compressed_size)
                }
                8 => crate::jdk::gzip::inflate_raw(&input[data_start.min(input.len())..])?,
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid compression method {method}"),
                    ))
                }
            };
            let mut next = data_start
                + if has_descriptor {
                    used
                } else {
                    compressed_size
                };
            if has_descriptor {
                // Optional signature, then CRC-32 and the two sizes.
                if input.get(next..next + 4) == Some(&b"PK\x07\x08"[..]) {
                    next += 4;
                }
                next += 12;
            }
            let target_file = dest_dir.join(&name);
            // be on the safe side: do not rely on that directories are always extracted
            // before their children (although this makes sense, but is it guaranteed?)
            if let Some(parent) = target_file.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !name.ends_with('/') {
                std::fs::write(&target_file, &data)?;
            }
            pos = next;
        }
        Ok(())
    }

    /// `syncConcurrentMerges(writer)`: `syncConcurrentMerges` of the
    /// writer's live merge scheduler.
    pub fn sync_concurrent_merges_of_writer(writer: &IndexWriter) {
        Self::sync_concurrent_merges(&*writer.get_config().merge_scheduler());
    }

    /// `syncConcurrentMerges(ms)`: waits for the running merges of `ms` when
    /// it is a `ConcurrentMergeScheduler` (`ConcurrentMergeScheduler.sync()`).
    pub fn sync_concurrent_merges(ms: &dyn MergeScheduler) {
        if let Some(cms) = ms
            .as_any()
            .and_then(|any| any.downcast_ref::<ConcurrentMergeScheduler>())
        {
            cms.sync();
        }
    }

    /// `checkReader(reader)`: [`check_leaf_reader`](Self::check_leaf_reader)
    /// at `MIN_LEVEL_FOR_SLOW_CHECKS` on every leaf of `reader`.
    ///
    /// # Errors
    ///
    /// As [`check_leaf_reader`](Self::check_leaf_reader).
    pub fn check_reader(reader: &Arc<dyn IndexReader>) -> Result<()> {
        for context in Arc::clone(reader).leaves()? {
            Self::check_leaf_reader(context.leaf_reader(), Level::MIN_LEVEL_FOR_SLOW_CHECKS)?;
        }
        Ok(())
    }

    /// `checkReader(reader, level)`: runs `CheckIndex`'s tests (live docs,
    /// field infos, norms, postings, stored fields, term vectors, doc values,
    /// points) with `failFast` on the reader — itself when it is a
    /// `CodecReader` (after `checkIntegrity()`), else
    /// `SlowCodecReaderWrapper.wrap(reader)` — then checks the reader API
    /// (`checkReaderSanity`) and that `getFieldInfos()` returns the same
    /// instance twice.
    ///
    /// **Divergence.** [`LeafReader::get_field_infos`] returns an owned
    /// snapshot, so Java's identity check `reader.getFieldInfos() !=
    /// reader.getFieldInfos()` compares what identifies the instance's
    /// content: the two snapshots must hold the same `FieldInfo` objects
    /// (pointer-equal), which a reader that rebuilds its field infos per
    /// call does not. Its message names the reader by its `toString()`
    /// where Java prints `getClass()`, which a trait object cannot name.
    ///
    /// # Errors
    ///
    /// The first error a `CheckIndex` test raises (`failFast`); a
    /// `RuntimeException` when the reader API returns values a field does
    /// not have, or different field infos.
    pub fn check_leaf_reader(reader: Arc<dyn LeafReader>, level: i32) -> Result<()> {
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(1024)));
        let info_stream = PrintStreamLines(Arc::clone(&buffer));

        let codec_reader: Arc<dyn CodecReader> = match Arc::clone(&reader).as_codec_reader() {
            Some(codec_reader) => {
                reader.check_integrity()?;
                codec_reader
            }
            None => SlowCodecReaderWrapper::wrap(Arc::clone(&reader))?,
        };
        let cr = &*codec_reader;
        let stream = Some(&info_stream as &dyn rucene::util::InfoStream);
        check_index::test_live_docs(cr, stream, true)?;
        check_index::test_field_infos(cr, stream, true)?;
        check_index::test_field_norms(cr, stream, true)?;
        check_index::test_postings(cr, stream, false, level, true)?;
        check_index::test_stored_fields(cr, stream, true)?;
        check_index::test_term_vectors(cr, stream, false, level, true)?;
        check_index::test_doc_values(cr, stream, true)?;
        check_index::test_points(cr, stream, true)?;

        // some checks really against the reader API
        Self::check_reader_sanity(&*reader)?;

        if *crate::util::lucene_test_case::INFOSTREAM {
            let text = buffer.lock().unwrap_or_else(|e| e.into_inner()).clone();
            println!("{}", String::from_utf8_lossy(&text));
        }

        // FieldInfos should be cached at the reader and always return the same instance
        let first = reader.get_field_infos();
        let second = reader.get_field_infos();
        let same = first.len() == second.len()
            && first
                .iter()
                .zip(second.iter())
                .all(|(a, b)| std::ptr::eq(a, b));
        if !same {
            return Err(runtime_exception(format!(
                "getFieldInfos() returned different instances for class: {}",
                reader_class_name(&*reader)
            )));
        }
        Ok(())
    }

    /// `checkReaderSanity(reader)`: the reader returns no norms for a field
    /// that omits them, no doc values for a field without them, and only the
    /// doc values of the field's type.
    fn check_reader_sanity(reader: &dyn LeafReader) -> Result<()> {
        for info in reader.get_field_infos().iter() {
            let name = info.name.as_str();
            // reader shouldn't return normValues if the field does not have them
            if !info.has_norms() && reader.get_norm_values(name)?.is_some() {
                return Err(runtime_exception(format!(
                    "field: {name} should omit norms but has them!"
                )));
            }

            // reader shouldn't return docValues if the field does not have them
            // reader shouldn't return multiple docvalues types for the same field.
            let binary = || -> Result<bool> { Ok(reader.get_binary_doc_values(name)?.is_some()) };
            let numeric = || -> Result<bool> { Ok(reader.get_numeric_doc_values(name)?.is_some()) };
            let sorted = || -> Result<bool> { Ok(reader.get_sorted_doc_values(name)?.is_some()) };
            let sorted_set =
                || -> Result<bool> { Ok(reader.get_sorted_set_doc_values(name)?.is_some()) };
            let sorted_numeric =
                || -> Result<bool> { Ok(reader.get_sorted_numeric_doc_values(name)?.is_some()) };
            // Java evaluates each `||` chain left to right and stops at the
            // first non-null accessor; so does each chain here.
            let multiple = match info.get_doc_values_type() {
                DocValuesType::NONE => {
                    if binary()? || numeric()? || sorted()? || sorted_set()? {
                        return Err(runtime_exception(format!(
                            "field: {name} has docvalues but should omit them!"
                        )));
                    }
                    false
                }
                DocValuesType::SORTED => {
                    binary()? || numeric()? || sorted_numeric()? || sorted_set()?
                }
                DocValuesType::SORTED_NUMERIC => {
                    binary()? || numeric()? || sorted_set()? || sorted()?
                }
                DocValuesType::SORTED_SET => {
                    binary()? || numeric()? || sorted_numeric()? || sorted()?
                }
                DocValuesType::BINARY => {
                    numeric()? || sorted()? || sorted_numeric()? || sorted_set()?
                }
                DocValuesType::NUMERIC => {
                    binary()? || sorted()? || sorted_numeric()? || sorted_set()?
                }
            };
            if multiple {
                return Err(runtime_exception(format!(
                    "{name} returns multiple docvalues types!"
                )));
            }
        }
        Ok(())
    }

    /// `getPostingsFormat(field)`: [`get_postings_format_of`] the default
    /// codec (`Codec.getDefault()`).
    ///
    /// # Panics
    ///
    /// When no default codec is registered, where Java's `Codec.getDefault()`
    /// fails.
    ///
    /// [`get_postings_format_of`]: Self::get_postings_format_of
    pub fn get_postings_format(field: &str) -> String {
        Self::get_postings_format_of(&*default_codec(), field)
    }

    /// `getPostingsFormat(codec, field)`: the name of the postings format
    /// `codec` writes `field` with — its `PerFieldPostingsFormat`'s
    /// `getPostingsFormatForField(field)` when it has one, else its postings
    /// format.
    pub fn get_postings_format_of(codec: &dyn Codec, field: &str) -> String {
        let p = codec.postings_format();
        match p
            .as_any()
            .and_then(|any| any.downcast_ref::<PerFieldPostingsFormat>())
        {
            Some(per_field) => per_field
                .get_postings_format_for_field(field)
                .name()
                .to_string(),
            None => p.name().to_string(),
        }
    }

    /// `getDocValuesFormat(field)`: [`get_doc_values_format_of`] the default
    /// codec (`Codec.getDefault()`).
    ///
    /// # Panics
    ///
    /// When no default codec is registered, where Java's `Codec.getDefault()`
    /// fails.
    ///
    /// [`get_doc_values_format_of`]: Self::get_doc_values_format_of
    pub fn get_doc_values_format(field: &str) -> String {
        Self::get_doc_values_format_of(&*default_codec(), field)
    }

    /// `getDocValuesFormat(codec, field)`: the name of the doc-values format
    /// `codec` writes `field` with — its `PerFieldDocValuesFormat`'s
    /// `getDocValuesFormatForField(field)` when it has one, else its
    /// doc-values format.
    pub fn get_doc_values_format_of(codec: &dyn Codec, field: &str) -> String {
        let f = codec.doc_values_format();
        match f
            .as_any()
            .and_then(|any| any.downcast_ref::<PerFieldDocValuesFormat>())
        {
            Some(per_field) => per_field
                .get_doc_values_format_for_field(field)
                .name()
                .to_string(),
            None => f.name().to_string(),
        }
    }

    /// `fieldSupportsHugeBinaryDocValues(field)`: `false` for the
    /// `Lucene40` and `Lucene42` doc-values formats of the default codec.
    ///
    /// # Panics
    ///
    /// As [`get_doc_values_format`](Self::get_doc_values_format).
    // TODO: remove this, push this test to Lucene40/Lucene42 codec tests
    pub fn field_supports_huge_binary_doc_values(field: &str) -> bool {
        let dv_format = Self::get_doc_values_format(field);
        !(dv_format == "Lucene40" || dv_format == "Lucene42")
    }

    /// `addIndexesSlowly(writer, readers...)`: adds every leaf of `readers`
    /// to `writer` as a `SlowCodecReaderWrapper` (`addIndexes(CodecReader...)`).
    ///
    /// # Errors
    ///
    /// The errors of wrapping the leaves and of `addIndexes`.
    pub fn add_indexes_slowly(
        writer: &IndexWriter,
        readers: &[Arc<dyn DirectoryReader>],
    ) -> Result<()> {
        let mut leaves: Vec<Arc<dyn CodecReader>> = Vec::new();
        for reader in readers {
            for context in Arc::clone(reader).leaves()? {
                leaves.push(SlowCodecReaderWrapper::wrap(context.leaf_reader())?);
            }
        }
        writer.add_indexes_codec_readers(&leaves)?;
        Ok(())
    }

    /// `reduceOpenFiles(w)`: configures the writer's live merge policy and
    /// scheduler to keep the number of open files lowish — `noCFSRatio` 1.0,
    /// a merge factor (`LogMergePolicy`) or `maxMergeAtOnce` and
    /// `segmentsPerTier` (`TieredMergePolicy`) of at most 5, and 3 merges on
    /// 2 threads for a `ConcurrentMergeScheduler`.
    ///
    /// # Errors
    ///
    /// The error of a setter: none for Lucene's policies; core's
    /// `MergePolicy::set_no_cfs_ratio` default (a policy without
    /// compound-file settings) fails with `UnsupportedOperation`.
    pub fn reduce_open_files(w: &IndexWriter) -> Result<()> {
        // keep number of open files lowish
        let mp = w.get_config().merge_policy();
        mp.set_no_cfs_ratio(1.0)?;
        if let Some(any) = mp.as_any() {
            if let Some(lmp) = any.downcast_ref::<LogMergePolicy>() {
                lmp.set_merge_factor(5.min(lmp.get_merge_factor()))?;
            } else if let Some(tmp) = any.downcast_ref::<TieredMergePolicy>() {
                tmp.set_max_merge_at_once(5.min(tmp.get_max_merge_at_once()))?;
                tmp.set_segments_per_tier(5.0f64.min(tmp.get_segments_per_tier()))?;
            }
        }
        let ms = w.get_config().merge_scheduler();
        if let Some(cms) = ms
            .as_any()
            .and_then(|any| any.downcast_ref::<ConcurrentMergeScheduler>())
        {
            // wtf... shouldn't it be even lower since it's 1 by default?!?!
            cms.set_max_merges_and_threads(3, 2)?;
        }
        Ok(())
    }

    /// `assertAttributeReflection(att, reflectedValues)`: `att.reflectWith`
    /// reports exactly `reflected_values`, keyed `"AttributeClass#key"`.
    ///
    /// **Divergence.** Core's
    /// [`AttributeReflector`](rucene::util::attribute::AttributeReflector) reports a source-level
    /// type name and a `Debug` value where Java reports the attribute
    /// interface's `Class` and an `Object`; the map compared here is keyed by
    /// that name, `#` and the key, with each value's `Debug` rendering, and
    /// is printed in `java.util.HashMap`'s order.
    ///
    /// # Panics
    ///
    /// `AssertionError` (`Reflection does not produce same map expected:<…>
    /// but was:<…>`).
    pub fn assert_attribute_reflection(
        att: &dyn AttributeImpl,
        reflected_values: &HashMap<String, String>,
    ) {
        let mut map: HashMap<String, String> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        att.reflect_with(&mut |_att_type: std::any::TypeId,
                               att_class: &'static str,
                               key: &str,
                               value: &dyn std::fmt::Debug| {
            let k = format!("{att_class}#{key}");
            if map.insert(k.clone(), format!("{value:?}")).is_none() {
                order.push(k);
            }
        });
        if &map != reflected_values {
            let render = |m: &HashMap<String, String>, keys: &mut dyn Iterator<Item = String>| {
                let mut java = JavaHashMap::new();
                for k in keys {
                    let v = m[&k].clone();
                    java.insert(k, v);
                }
                let entries: Vec<String> = java.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{{{}}}", entries.join(", "))
            };
            let mut expected_keys: Vec<String> = reflected_values.keys().cloned().collect();
            expected_keys.sort();
            Assert::assert_equals_msg(
                Some("Reflection does not produce same map"),
                render(reflected_values, &mut expected_keys.into_iter()),
                render(&map, &mut order.into_iter()),
            );
        }
    }

    /// `assertConsistent(expected, actual)`: the two results have the same
    /// top hits (docs, scores, sort values) and consistent hit counts.
    ///
    /// Java's `TopDocs` holds `ScoreDoc`s or `FieldDoc`s; the port's
    /// [`TopDocs`] holds `ScoreDoc`s and [`TopFieldDocs`] `FieldDoc`s, so
    /// either is a [`TopDocsView`].
    ///
    /// # Panics
    ///
    /// The `AssertionError` (or JUnit's `ArrayComparisonFailure`) of the
    /// first inconsistency, with Java's message.
    pub fn assert_consistent<'a, 'b>(
        expected: impl Into<TopDocsView<'a>>,
        actual: impl Into<TopDocsView<'b>>,
    ) {
        let expected: TopDocsView<'_> = expected.into();
        let actual: TopDocsView<'_> = actual.into();
        let (expected_hits, actual_hits) = (expected.total_hits(), actual.total_hits());
        Assert::assert_equals_msg(
            Some("wrong total hits"),
            expected_hits.value() == 0,
            actual_hits.value() == 0,
        );
        if expected_hits.relation() == TotalHitsRelation::EQUAL_TO {
            if actual_hits.relation() == TotalHitsRelation::EQUAL_TO {
                MatcherAssert::assert_that_reason(
                    "wrong total hits",
                    &actual_hits.value(),
                    &equal_to(expected_hits.value()),
                );
            } else {
                MatcherAssert::assert_that_reason(
                    "wrong total hits",
                    &actual_hits.value(),
                    &less_than_or_equal_to(expected_hits.value()),
                );
            }
        } else if actual_hits.relation() == TotalHitsRelation::EQUAL_TO {
            MatcherAssert::assert_that_reason(
                "wrong total hits",
                &actual_hits.value(),
                &greater_than_or_equal_to(expected_hits.value()),
            );
        }
        let expected_docs = expected.score_docs();
        let actual_docs = actual.score_docs();
        MatcherAssert::assert_that_reason(
            "wrong hit count",
            &actual_docs[..],
            &array_with_size(expected_docs.len() as i32),
        );
        for (expected_sd, actual_sd) in expected_docs.iter().zip(actual_docs.iter()) {
            MatcherAssert::assert_that_reason(
                "wrong hit docID",
                &actual_sd.score_doc().doc,
                &equal_to(expected_sd.score_doc().doc),
            );
            assert_double_equals(
                "wrong hit score",
                f64::from(expected_sd.score_doc().score),
                f64::from(actual_sd.score_doc().score),
                0.0,
            );
            if let Hit::Field(expected_fd) = expected_sd {
                MatcherAssert::assert_that(actual_sd, &instance_of(FIELD_DOC_CLASS));
                let Hit::Field(actual_fd) = actual_sd else {
                    unreachable!("INVARIANT: instanceOf(FieldDoc) passed");
                };
                assert_array_equals(
                    "wrong sort field values",
                    expected_fd.fields.as_deref(),
                    actual_fd.fields.as_deref(),
                );
            } else {
                MatcherAssert::assert_that(actual_sd, &not(instance_of(FIELD_DOC_CLASS)));
            }
        }
    }

    /// `cloneDocument(doc1)`: a document of new fields with the names, values
    /// and types of `doc1`'s: a `KeywordField` or `IntField` as one, a
    /// numeric, binary or sorted doc-values field as one, a point field as a
    /// `BinaryPoint` of its packed bytes and type, any other field as a
    /// `Field` of its string value and type. "this is likely buggy, and
    /// cannot clone fields with tokenStreamValues, etc. Use at your own
    /// risk!!"
    ///
    /// **Divergence.** Java first casts each field to `Field`, a
    /// `ClassCastException` for an `IndexableField` that is not one; the port
    /// reads every value through [`IndexableField`], whose implementations
    /// are the ports of `Field`'s subclasses.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`unknown Type: …`) for a field whose doc
    /// values are sorted-set or sorted-numeric (and is not a `KeywordField`
    /// or `IntField`); `IllegalArgumentException` (`value must not be null`,
    /// `bytes must not be null`) for a field without the string or binary
    /// value its clone takes, `NullPointerException` without the number or
    /// packed bytes; the errors of the field constructors.
    pub fn clone_document(doc1: &Document) -> Result<Document> {
        let mut doc2 = Document::new();
        for f in doc1.get_fields() {
            let f: &dyn IndexableField = f.as_ref();
            let field_type = f.field_type();
            let dv_type = field_type.doc_values_type();
            let dim_count = field_type.point_dimension_count();
            let stored = |f: &dyn IndexableField| {
                if f.field_type().stored() {
                    Store::YES
                } else {
                    Store::NO
                }
            };
            let any = f.as_any();
            let field2: Box<dyn IndexableField> = if any.is_some_and(|a| a.is::<KeywordField>()) {
                // `new KeywordField(name, (String) null, store)` fails in
                // `Field(String, CharSequence, IndexableFieldType)`.
                let value = f.string_value().ok_or_else(value_must_not_be_null)?;
                Box::new(KeywordField::new(f.name(), value, stored(f))?)
            } else if any.is_some_and(|a| a.is::<IntField>()) {
                let value = number_int_value(f.numeric_value().ok_or_else(null_pointer)?);
                Box::new(IntField::new(f.name(), value, stored(f)))
            } else if dv_type != DocValuesType::NONE {
                match dv_type {
                    DocValuesType::NUMERIC => {
                        let value = number_long_value(f.numeric_value().ok_or_else(null_pointer)?);
                        Box::new(NumericDocValuesField::new(f.name(), value))
                    }
                    // `Field(String, BytesRef, IndexableFieldType)` refuses a
                    // null value.
                    DocValuesType::BINARY => Box::new(BinaryDocValuesField::new(
                        f.name(),
                        f.binary_value().ok_or_else(bytes_must_not_be_null)?,
                    )),
                    DocValuesType::SORTED => Box::new(SortedDocValuesField::new(
                        f.name(),
                        f.binary_value().ok_or_else(bytes_must_not_be_null)?,
                    )),
                    other => {
                        return Err(LuceneError::illegal_state(format!(
                            "unknown Type: {other:?}"
                        )))
                    }
                }
            } else if dim_count != 0 {
                let br = f.binary_value().ok_or_else(null_pointer)?;
                let bytes = BytesRef::new(br.bytes[br.offset..br.offset + br.length].to_vec());
                Box::new(BinaryPoint::new_with_packed(
                    f.name(),
                    bytes,
                    FieldType::new_from(field_type),
                )?)
            } else {
                let value = f.string_value().ok_or_else(value_must_not_be_null)?;
                Box::new(Field::new(
                    f.name(),
                    value,
                    FieldType::new_from(field_type),
                )?)
            };
            doc2.add(field2);
        }

        Ok(doc2)
    }

    /// `shutdownExecutorService(ex)`: shuts `ex` down and waits up to one
    /// second for it; an interruption is reported on the standard error
    /// stream.
    pub fn shutdown_executor_service(ex: Option<&dyn ExecutorService>) {
        if let Some(ex) = ex {
            ex.shutdown();
            if let Err(e) = ex.await_termination(Duration::from_secs(1)) {
                // Just report it on the syserr.
                eprintln!("Could not properly close executor service.");
                eprintln!("{e}");
            }
        }
    }

    /// `hasWindowsFS(dir)`: whether the filesystem of the `FSDirectory` under
    /// `dir`'s `FilterDirectory` wrappers is (or wraps) a `WindowsFS`.
    ///
    /// # Errors
    ///
    /// `AlreadyClosedException` for a closed `FSDirectory`
    /// (`FSDirectory.getDirectory()`).
    pub fn has_windows_fs(dir: &dyn Directory) -> Result<bool> {
        let dir = FilterDirectory::unwrap(dir);
        match dir.as_fs_directory() {
            Some(fs_dir) => Ok(Self::has_windows_fs_path(fs_dir.get_directory()?)),
            None => Ok(false),
        }
    }

    /// `hasWindowsFS(path)`: whether `path`'s filesystem is (or wraps) a
    /// `WindowsFS`.
    pub fn has_windows_fs_path(path: &Path) -> bool {
        find_provider::<WindowsFS>(path.get_file_system()).is_some()
    }

    /// `hasVirusChecker(dir)`: whether the filesystem of the `FSDirectory`
    /// under `dir`'s `FilterDirectory` wrappers is (or wraps) a
    /// `VirusCheckingFS`.
    ///
    /// # Errors
    ///
    /// `AlreadyClosedException` for a closed `FSDirectory`.
    pub fn has_virus_checker(dir: &dyn Directory) -> Result<bool> {
        let dir = FilterDirectory::unwrap(dir);
        match dir.as_fs_directory() {
            Some(fs_dir) => Ok(Self::has_virus_checker_path(fs_dir.get_directory()?)),
            None => Ok(false),
        }
    }

    /// `hasVirusChecker(path)`: whether `path`'s filesystem is (or wraps) a
    /// `VirusCheckingFS`.
    pub fn has_virus_checker_path(path: &Path) -> bool {
        find_provider::<VirusCheckingFS>(path.get_file_system()).is_some()
    }

    /// `disableVirusChecker(in)`: disables the `VirusCheckingFS` under the
    /// `FSDirectory` of `in`; whether it was in use and enabled.
    ///
    /// # Errors
    ///
    /// `AlreadyClosedException` for a closed `FSDirectory`.
    pub fn disable_virus_checker(r#in: &dyn Directory) -> Result<bool> {
        let dir = FilterDirectory::unwrap(r#in);
        if let Some(fs_dir) = dir.as_fs_directory() {
            if let Some(vfs) =
                find_provider::<VirusCheckingFS>(fs_dir.get_directory()?.get_file_system())
            {
                let vfs = vfs
                    .as_any()
                    .downcast_ref::<VirusCheckingFS>()
                    .expect("INVARIANT: find_provider matched a VirusCheckingFS");
                let is_enabled = vfs.is_enabled();
                vfs.disable();
                return Ok(is_enabled);
            }
        }
        Ok(false)
    }

    /// `enableVirusChecker(in)`: enables the `VirusCheckingFS` under the
    /// `FSDirectory` of `in`, if any.
    ///
    /// # Errors
    ///
    /// `AlreadyClosedException` for a closed `FSDirectory`.
    pub fn enable_virus_checker(r#in: &dyn Directory) -> Result<()> {
        let dir = FilterDirectory::unwrap(r#in);
        if let Some(fs_dir) = dir.as_fs_directory() {
            if let Some(vfs) =
                find_provider::<VirusCheckingFS>(fs_dir.get_directory()?.get_file_system())
            {
                vfs.as_any()
                    .downcast_ref::<VirusCheckingFS>()
                    .expect("INVARIANT: find_provider matched a VirusCheckingFS")
                    .enable();
            }
        }
        Ok(())
    }
}

/// A `java.lang.RuntimeException` with `message`.
fn runtime_exception(message: String) -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.RuntimeException",
        message: Some(message),
        cause: None,
    }
}

/// The `NullPointerException` Java raises dereferencing a `null` value.
fn null_pointer() -> LuceneError {
    LuceneError::Unchecked {
        class: "java.lang.NullPointerException",
        message: None,
        cause: None,
    }
}

/// `Field(String, CharSequence, IndexableFieldType)`'s refusal of a `null`
/// value.
fn value_must_not_be_null() -> LuceneError {
    LuceneError::IllegalArgument("value must not be null".to_string())
}

/// `Field(String, BytesRef, IndexableFieldType)`'s refusal of a `null` value.
fn bytes_must_not_be_null() -> LuceneError {
    LuceneError::IllegalArgument("bytes must not be null".to_string())
}

/// `Number.intValue()`: the Java narrowing conversion of the number to an
/// `int` (`(int)` of a `long` keeps the low 32 bits; of a `float` or `double`
/// it rounds toward zero, saturates, and maps NaN to 0, as Rust's `as` does).
fn number_int_value(n: NumericValue) -> i32 {
    match n {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => v as i32,
        NumericValue::Float(v) => v as i32,
        NumericValue::Double(v) => v as i32,
    }
}

/// `Number.longValue()`: the Java conversion of the number to a `long`.
fn number_long_value(n: NumericValue) -> i64 {
    match n {
        NumericValue::Int(v) => i64::from(v),
        NumericValue::Long(v) => v,
        NumericValue::Float(v) => v as i64,
        NumericValue::Double(v) => v as i64,
    }
}

/// `Codec.getDefault()`.
fn default_codec() -> Arc<dyn Codec> {
    rucene::codecs::get_default().unwrap_or_else(|| {
        panic!(
            "default codec '{}' is not registered in the global codec registry",
            rucene::codecs::DEFAULT_CODEC_NAME
        )
    })
}

/// What stands for `reader.getClass()` in a message: the reader's
/// `toString()`, since a Rust trait object cannot name its concrete type.
fn reader_class_name(reader: &dyn LeafReader) -> String {
    reader.to_string()
}

/// The provider of the first `FilterFileSystem` in the chain from `fs` whose
/// provider is a `T` (`while (fs instanceof FilterFileSystem ffs) { if
/// (ffs.provider() instanceof T) ...; fs = ffs.getDelegate(); }`).
fn find_provider<T: 'static>(fs: &Arc<dyn FileSystem>) -> Option<Arc<dyn FileSystemProvider>> {
    let mut fs = Arc::clone(fs);
    loop {
        let delegate = {
            let ffs = fs.as_any().downcast_ref::<FilterFileSystem>()?;
            let provider = ffs.provider();
            if provider.as_any().is::<T>() {
                return Some(provider);
            }
            Arc::clone(ffs.get_delegate())
        };
        fs = delegate;
    }
}

/// The class name `assertConsistent`'s `instanceOf` matchers name.
const FIELD_DOC_CLASS: &str = "org.apache.lucene.search.FieldDoc";

/// The class name of a `ScoreDoc` that is not a `FieldDoc`.
const SCORE_DOC_CLASS: &str = "org.apache.lucene.search.ScoreDoc";

/// The results [`TestUtil::assert_consistent`] compares: Java's `TopDocs`,
/// whose hits are `ScoreDoc`s ([`TopDocs`]) or `FieldDoc`s
/// ([`TopFieldDocs`]).
#[derive(Debug, Clone, Copy)]
pub enum TopDocsView<'a> {
    /// A `TopDocs` of `ScoreDoc`s.
    Scores(&'a TopDocs),
    /// A `TopFieldDocs`, whose hits are `FieldDoc`s.
    Fields(&'a TopFieldDocs),
}

impl<'a> From<&'a TopDocs> for TopDocsView<'a> {
    fn from(top_docs: &'a TopDocs) -> Self {
        TopDocsView::Scores(top_docs)
    }
}

impl<'a> From<&'a TopFieldDocs> for TopDocsView<'a> {
    fn from(top_docs: &'a TopFieldDocs) -> Self {
        TopDocsView::Fields(top_docs)
    }
}

impl<'a> TopDocsView<'a> {
    /// `totalHits`.
    fn total_hits(&self) -> TotalHits {
        match self {
            TopDocsView::Scores(t) => t.total_hits,
            TopDocsView::Fields(t) => t.total_hits,
        }
    }

    /// `scoreDocs`.
    fn score_docs(&self) -> Vec<Hit<'a>> {
        match self {
            TopDocsView::Scores(t) => t.score_docs.iter().map(Hit::Score).collect(),
            TopDocsView::Fields(t) => t.score_docs.iter().map(Hit::Field).collect(),
        }
    }
}

/// One hit of a [`TopDocsView`]: a `ScoreDoc` or a `FieldDoc`.
#[derive(Debug, Clone, Copy)]
enum Hit<'a> {
    Score(&'a ScoreDoc),
    Field(&'a FieldDoc),
}

impl Hit<'_> {
    /// The `ScoreDoc` part of the hit.
    fn score_doc(&self) -> &ScoreDoc {
        match self {
            Hit::Score(sd) => sd,
            Hit::Field(fd) => &fd.score_doc,
        }
    }
}

impl HamcrestValue for Hit<'_> {
    /// `<toString()>`.
    fn describe_value(&self) -> String {
        match self {
            Hit::Score(sd) => format!("<{sd}>"),
            Hit::Field(fd) => format!("<{fd}>"),
        }
    }
}

impl JavaObject for Hit<'_> {
    fn java_class_name(&self) -> String {
        match self {
            Hit::Score(_) => SCORE_DOC_CLASS.to_string(),
            Hit::Field(_) => FIELD_DOC_CLASS.to_string(),
        }
    }

    /// A `FieldDoc` is a `ScoreDoc`.
    fn is_instance_of(&self, class_name: &str) -> bool {
        match self {
            Hit::Score(_) => class_name == SCORE_DOC_CLASS,
            Hit::Field(_) => class_name == FIELD_DOC_CLASS || class_name == SCORE_DOC_CLASS,
        }
    }
}

/// JUnit's `Assert.assertEquals(String message, double expected, double
/// actual, double delta)`: equal when `Double.compare` is 0 or the difference
/// is within `delta`, else `failNotEquals` with the boxed `Double`s.
fn assert_double_equals(message: &str, expected: f64, actual: f64, delta: f64) {
    // `Double.compare(expected, actual) == 0`: the same bits, or two NaNs.
    let same = expected.to_bits() == actual.to_bits() || (expected.is_nan() && actual.is_nan());
    if same || (expected - actual).abs() <= delta {
        return;
    }
    fail_not_equals(
        message,
        &double_to_string(expected),
        "java.lang.Double",
        &double_to_string(actual),
        "java.lang.Double",
    );
}

/// JUnit's `Assert.failNotEquals(message, expected, actual)`: `message `,
/// then `expected:<x> but was:<y>`, or, when the two render alike, each with
/// its class (`expected: java.lang.Long<5> but was: java.lang.Integer<5>`).
fn fail_not_equals(
    message: &str,
    expected: &str,
    expected_class: &str,
    actual: &str,
    actual_class: &str,
) -> ! {
    Assert::fail(&format!(
        "{}{}",
        if message.is_empty() {
            String::new()
        } else {
            format!("{message} ")
        },
        not_equals_message(expected, expected_class, actual, actual_class)
    ))
}

/// The part of JUnit's `Assert.format` after the message.
fn not_equals_message(
    expected: &str,
    expected_class: &str,
    actual: &str,
    actual_class: &str,
) -> String {
    if expected == actual {
        format!("expected: {expected_class}<{expected}> but was: {actual_class}<{actual}>")
    } else {
        format!("expected:<{expected}> but was:<{actual}>")
    }
}

/// The class of a sort value, as `getClass().getName()` names the object
/// Java holds (`null` for `null`).
fn sort_value_class(value: &SortValue) -> &'static str {
    match value {
        SortValue::Null => "null",
        SortValue::Int(_) => "java.lang.Integer",
        SortValue::Long(_) => "java.lang.Long",
        SortValue::Float(_) => "java.lang.Float",
        SortValue::Double(_) => "java.lang.Double",
        SortValue::Bytes(_) => "org.apache.lucene.util.BytesRef",
    }
}

/// JUnit 4.13.1's `Assert.assertArrayEquals(String message, Object[]
/// expecteds, Object[] actuals)` over sort values (`ComparisonCriteria
/// .arrayEquals` with `ExactComparisonCriteria`): `expected array was null`
/// or `actual array was null` (an `AssertionError`), else an
/// `ArrayComparisonFailure` naming the first differing index (a missing
/// element is `end of array`), after `array lengths differed, …` when the
/// lengths differ.
fn assert_array_equals(
    message: &str,
    expecteds: Option<&[SortValue]>,
    actuals: Option<&[SortValue]>,
) {
    if expecteds == actuals {
        return;
    }
    let mut header = format!("{message}: ");
    let Some(expecteds) = expecteds else {
        Assert::fail(&format!("{header}expected array was null"))
    };
    let Some(actuals) = actuals else {
        Assert::fail(&format!("{header}actual array was null"))
    };
    if actuals.len() != expecteds.len() {
        header.push_str(&format!(
            "array lengths differed, expected.length={} actual.length={}",
            expecteds.len(),
            actuals.len()
        ));
    }
    // `ArrayComparisonFailure.getMessage()`: the header, then `"; "` unless
    // the header is empty or ends with a space, the index, `"; "` and the
    // cause's message (`Assert.assertEquals(Object, Object)` of the two
    // elements, `end of array` past the end of the shorter one).
    let fail_at = |index: usize, expected: Option<&SortValue>, actual: Option<&SortValue>| -> ! {
        let render =
            |v: Option<&SortValue>| v.map_or("end of array".to_string(), |v| v.to_string());
        let cause = not_equals_message(
            &render(expected),
            expected.map_or("org.junit.internal.ComparisonCriteria$1", sort_value_class),
            &render(actual),
            actual.map_or("org.junit.internal.ComparisonCriteria$1", sort_value_class),
        );
        let separator = if header.is_empty() || header.ends_with(' ') {
            ""
        } else {
            "; "
        };
        Throwable::new(
            "org.junit.internal.ArrayComparisonFailure",
            format!("{header}{separator}arrays first differed at element [{index}]; {cause}"),
        )
        .throw()
    };
    let prefix_length = actuals.len().min(expecteds.len());
    for i in 0..prefix_length {
        if expecteds[i] != actuals[i] {
            fail_at(i, Some(&expecteds[i]), Some(&actuals[i]));
        }
    }
    if actuals.len() != expecteds.len() {
        fail_at(
            prefix_length,
            expecteds.get(prefix_length),
            actuals.get(prefix_length),
        );
    }
}

/// The [`String`] of well-formed UTF-16 `units`.
fn utf16_to_string(units: &[u16]) -> String {
    String::from_utf16(units).expect("INVARIANT: the generators write surrogates in pairs only")
}

/// `UnicodeUtil.UTF8toUTF16` as `BytesRef.utf8ToString` runs it under `-ea`:
/// `None` where an `assert` fails (a lead byte in `0x80..0xc0` or `0xf8..`).
///
/// # Panics
///
/// Where Java throws `ArrayIndexOutOfBoundsException`: a sequence cut by the
/// end of the bytes.
fn lenient_utf8_to_utf16(utf8: &[u8]) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(utf8.len());
    let mut offset = 0;
    while offset < utf8.len() {
        let b = u32::from(utf8[offset]);
        offset += 1;
        if b < 0xc0 {
            if b >= 0x80 {
                return None;
            }
            out.push(b as u16);
        } else if b < 0xe0 {
            out.push((((b & 0x1f) << 6) + (u32::from(utf8[offset]) & 0x3f)) as u16);
            offset += 1;
        } else if b < 0xf0 {
            out.push(
                (((b & 0xf) << 12)
                    + ((u32::from(utf8[offset]) & 0x3f) << 6)
                    + (u32::from(utf8[offset + 1]) & 0x3f)) as u16,
            );
            offset += 2;
        } else {
            if b >= 0xf8 {
                return None;
            }
            let ch = ((b & 0x7) << 18)
                + ((u32::from(utf8[offset]) & 0x3f) << 12)
                + ((u32::from(utf8[offset + 1]) & 0x3f) << 6)
                + (u32::from(utf8[offset + 2]) & 0x3f);
            offset += 3;
            // `UNI_MAX_BMP` is 0xFFFF, compared exclusively, as Java.
            let ch = ch as i32;
            if ch < 0xffff {
                out.push(ch as u16);
            } else {
                let ch_half = ch - 0x0010000;
                out.push(((ch_half >> 10) + 0xD800) as u16);
                out.push(((ch_half & 0x3ff) + 0xDC00) as u16);
            }
        }
    }
    Some(out)
}

/// Whether `java.util.regex.Pattern.compile(pattern)` accepts `pattern`, as
/// core's `AbstractAnalysisFactory::get_pattern` translates and compiles it;
/// the compiled pattern when it does.
fn compile_java_pattern(pattern: &str) -> Option<impl Fn(&str) -> Option<usize>> {
    let mut args = std::collections::HashMap::new();
    args.insert("pattern".to_string(), pattern.to_string());
    let regex = AbstractAnalysisFactory::get_pattern(&mut args, "pattern", "TestUtil").ok()?;
    // The byte length of the leftmost match, if any, of a pattern anchored
    // with `^`.
    Some(move |text: &str| regex.find(text).ok().flatten().map(|m| m.end() - m.start()))
}

/// Whether `Pattern.compile(pattern).matcher("AB𠀀C").replaceAll(..)`
/// finds a match that starts at UTF-16 index 3, between the surrogates (see
/// [`TestUtil::random_pattern`]), for a `pattern` that compiles.
fn java_find_splits_pair(pattern: &str) -> bool {
    let anchored = compile_java_pattern(&format!("^(?:{pattern})"))
        .expect("INVARIANT: a compiling pattern compiles inside an anchored group");
    // The text from each UTF-16 index of "AB𠀀C" (5 units), with
    // the lone low surrogate at index 3 as U+E000.
    let suffixes = [
        "AB\u{20000}C",
        "B\u{20000}C",
        "\u{20000}C",
        "\u{E000}C",
        "C",
        "",
    ];
    // The UTF-16 length of the prefix of a suffix that a match covers.
    let units = |index: usize, bytes: usize| suffixes[index][..bytes].encode_utf16().count();
    let mut from = 0;
    while from <= 5 {
        // Matcher.search(from): the first index with a match.
        let found = (from..=5).find_map(|i| anchored(suffixes[i]).map(|len| (i, units(i, len))));
        match found {
            None => return false,
            Some((3, _)) => return true,
            // Matcher.find(): the next search starts at the end of the
            // match, one char further after an empty match.
            Some((start, 0)) => from = start + 1,
            Some((start, len)) => from = start + len,
        }
    }
    false
}

/// `IndexFileNames.CODEC_FILE_PATTERN.matcher(file).matches()`, for
/// `_[a-z0-9]+(_.*)?\..*`: after `_` and a run of `[a-z0-9]` (which `_` and
/// `.` end, so no backtracking into it can help), either `.` or `_` and a
/// later `.`, with no line terminator (which Java's `.` does not match).
fn matches_codec_file_pattern(file: &str) -> bool {
    let Some(rest) = file.strip_prefix('_') else {
        return false;
    };
    let run = rest
        .bytes()
        .take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        .count();
    if run == 0 {
        return false;
    }
    let rest = &rest[run..];
    let terminator = |c: char| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    if rest.contains(terminator) {
        return false;
    }
    rest.starts_with('.') || (rest.starts_with('_') && rest[1..].contains('.'))
}

/// A `PrintStream` over a byte buffer, as `CheckIndex`'s info stream: each
/// message is one line.
struct PrintStreamLines(Arc<Mutex<Vec<u8>>>);

impl rucene::util::InfoStream for PrintStreamLines {
    fn message(&self, _component: &str, message: &str) {
        let mut buffer = self.0.lock().unwrap_or_else(|e| e.into_inner());
        buffer.extend_from_slice(message.as_bytes());
        buffer.push(b'\n');
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {}

    /// The class Java's `CheckIndex` reports to: a `java.io.PrintStream`.
    fn java_class_name(&self) -> &'static str {
        "java.io.PrintStream"
    }
}
