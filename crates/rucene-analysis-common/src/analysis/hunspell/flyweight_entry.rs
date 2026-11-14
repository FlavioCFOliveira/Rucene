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

//! The mutable entry the dictionary is enumerated with, ported from
//! `org.apache.lucene.analysis.hunspell.FlyweightEntry`.

use super::dictionary::Dictionary;

/// Java's `CharSequence`, as the package reads one: a length and the unit at
/// an index.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub trait CharSequence {
    /// Equivalent to `length()`.
    fn length(&self) -> usize;

    /// Equivalent to `charAt(int)`.
    fn char_at(&self, index: usize) -> u16;
}

impl CharSequence for [u16] {
    fn length(&self) -> usize {
        self.len()
    }

    fn char_at(&self, index: usize) -> u16 {
        self[index]
    }
}

impl CharSequence for Vec<u16> {
    fn length(&self) -> usize {
        self.len()
    }

    fn char_at(&self, index: usize) -> u16 {
        self[index]
    }
}

/// What [`FlyweightEntry::lower_case_root`] returns: the root's units, or a
/// view folding each unit of the root with `Dictionary.caseFold` as it is
/// read (the anonymous `CharSequence` of `WordStorage.MyFlyweightEntry`).
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum LowerCaseRoot<'a> {
    /// A root already lower-cased.
    Plain(&'a [u16]),
    /// A root folded unit by unit, with the dictionary's alternate casing
    /// flag.
    Folded {
        /// The root.
        chars: &'a [u16],
        /// `Dictionary.alternateCasing`.
        alternate_casing: bool,
    },
}

impl CharSequence for LowerCaseRoot<'_> {
    fn length(&self) -> usize {
        match self {
            Self::Plain(chars) | Self::Folded { chars, .. } => chars.len(),
        }
    }

    fn char_at(&self, index: usize) -> u16 {
        match self {
            Self::Plain(chars) => chars[index],
            Self::Folded {
                chars,
                alternate_casing,
            } => Dictionary::case_fold_with(*alternate_casing, chars[index]),
        }
    }
}

/// A mutable entry object used when enumerating the dictionary internally.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.FlyweightEntry`. The
/// root and the forms are returned as the active parts of Java's `CharsRef`
/// and `IntsRef`, which the enumeration reuses.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub trait FlyweightEntry {
    /// Equivalent to `hasTitleCase()`.
    fn has_title_case(&self) -> bool;

    /// Equivalent to `root()`.
    fn root(&self) -> &[u16];

    /// Equivalent to `lowerCaseRoot()`.
    fn lower_case_root(&self) -> LowerCaseRoot<'_>;

    /// Equivalent to `forms()`.
    fn forms(&mut self) -> &[i32];
}
