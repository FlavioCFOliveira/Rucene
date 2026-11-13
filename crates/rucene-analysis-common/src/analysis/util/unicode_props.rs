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

//! The Unicode properties the module's `CharTokenizer`s use, ported from
//! `org.apache.lucene.analysis.util.UnicodeProps`.

use std::sync::LazyLock;

use rucene::util::{Bits, SparseFixedBitSet};

/// This file contains unicode properties used by various `CharTokenizer`s.
/// The data was generated using ICU4J v77.1.0.0, unicode version: 16.0.0.0.
///
/// Equivalent to `org.apache.lucene.analysis.util.UnicodeProps`, a class of
/// static fields with a private constructor: the type has no value. The
/// table is Lucene's generated one (`GenerateUnicodeProps.groovy`), ported
/// as it is.
#[derive(Debug)]
pub enum UnicodeProps {}

impl UnicodeProps {
    /// Unicode version that was used to generate this file: `16.0.0.0`.
    ///
    /// Equivalent to `UnicodeProps.UNICODE_VERSION`.
    pub const UNICODE_VERSION: &'static str = "16.0.0.0";

    /// Bitset with Unicode WHITESPACE code points.
    ///
    /// Equivalent to `UnicodeProps.WHITESPACE`.
    pub fn whitespace() -> &'static dyn Bits {
        &*WHITESPACE
    }
}

/// `UnicodeProps.WHITESPACE`, built once, as Java's static initializer does.
static WHITESPACE: LazyLock<CodePointBits> = LazyLock::new(|| {
    create_bits(&[
        0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x0020, 0x0085, 0x00A0, 0x1680, 0x2000, 0x2001,
        0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200A, 0x2028, 0x2029,
        0x202F, 0x205F, 0x3000,
    ])
});

/// The anonymous `Bits` `createBits` returns: a bit set over the code points
/// up to the largest one given, whose length is every code point,
/// `0x10FFFF + 1`.
#[derive(Debug)]
struct CodePointBits {
    len: usize,
    bitset: SparseFixedBitSet,
}

impl Bits for CodePointBits {
    fn get(&self, index: usize) -> bool {
        index < self.len && self.bitset.get(index)
    }

    fn length(&self) -> usize {
        0x10FFFF + 1
    }
}

/// `createBits(int...)`.
fn create_bits(codepoints: &[usize]) -> CodePointBits {
    let len = codepoints[codepoints.len() - 1] + 1;
    let mut bitset =
        SparseFixedBitSet::new(len).expect("INVARIANT: the largest code point gives a length >= 1");
    for &i in codepoints {
        bitset.set(i);
    }
    CodePointBits { len, bitset }
}
