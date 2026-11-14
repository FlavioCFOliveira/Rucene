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

//! The type declared in `PatternConsumer.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

use super::hyphen::Hyphen;

/// An element of a hyphenated word: Java's `ArrayList<Object>` of
/// alternating `String` and [`Hyphen`] instances.
///
/// **Divergence.** Java types the list `ArrayList<Object>` and tells the two
/// kinds apart with `instanceof`; Rust has no `Object`, so the list holds
/// this enum of the only two kinds `PatternParser` puts in it.
///
/// The Java `String`s of this package are held as their UTF-16 units
/// (`Vec<u16>`, `&[u16]`), the `char`s the Java code indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyphenatedWordPart {
    /// A `String` element, as its UTF-16 units.
    String(Vec<u16>),
    /// A [`Hyphen`] element.
    Hyphen(Hyphen),
}

/// This interface is used to connect the XML pattern file parser to the
/// hyphenation tree.
///
/// This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.PatternConsumer`.
pub trait PatternConsumer {
    /// Add a character class. A character class defines characters that are
    /// considered equivalent for the purpose of hyphenation (e.g. "aA"). It
    /// usually means to ignore case.
    ///
    /// Equivalent to `PatternConsumer.addClass(String)`.
    fn add_class(&mut self, chargroup: &[u16]);

    /// Add a hyphenation exception. An exception replaces the result obtained
    /// by the algorithm for cases for which this fails or the user wants to
    /// provide his own hyphenation. A hyphenatedword is a vector of
    /// alternating String's and [`Hyphen`] instances
    ///
    /// Equivalent to `PatternConsumer.addException(String, ArrayList<Object>)`.
    fn add_exception(&mut self, word: &[u16], hyphenatedword: Vec<HyphenatedWordPart>);

    /// Add hyphenation patterns.
    ///
    /// `pattern` is the pattern, `values` the interletter values expressed as
    /// a string of digit characters.
    ///
    /// Equivalent to `PatternConsumer.addPattern(String, String)`.
    fn add_pattern(&mut self, pattern: &[u16], values: &[u16]);
}
