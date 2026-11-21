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

//! Port of `org.apache.lucene.codecs.uniformsplit.IndexDictionary`.

use rucene::error::Result;
use rucene::store::DataOutput;
use rucene::util::BytesRef;

use super::block_encoder::BlockEncoder;

/// Immutable stateless index dictionary kept in RAM.
///
/// Implementations must be immutable.
///
/// Use [`Builder`] to build the [`IndexDictionary`].
///
/// Create a stateful [`Browser`] to seek a term in this [`IndexDictionary`]
/// and get its corresponding block file pointer to the terms block file.
///
/// There is a single implementation of this interface,
/// [`FSTDictionary`](super::FSTDictionary). However this interface allows you
/// to plug easily a new kind of index dictionary to experiment and improve
/// the existing one.
///
/// Port of the interface `org.apache.lucene.codecs.uniformsplit.IndexDictionary`
/// (`@lucene.experimental`).
pub trait IndexDictionary: Send + Sync {
    /// Writes this dictionary to the provided output.
    ///
    /// `block_encoder` is the [`BlockEncoder`] for specific encoding of this
    /// index dictionary, or `None` if none.
    ///
    /// # Errors
    ///
    /// The I/O errors of the output and of the encoder.
    fn write(
        &self,
        output: &mut dyn DataOutput,
        block_encoder: Option<&dyn BlockEncoder>,
    ) -> Result<()>;

    /// Creates a new [`Browser`].
    ///
    /// # Errors
    ///
    /// The I/O errors of creating it.
    fn browser(&self) -> Result<Box<dyn Browser>>;
}

/// Builds an immutable [`IndexDictionary`].
///
/// Port of the interface `IndexDictionary.Builder`.
pub trait Builder {
    /// Adds a [block key - block file pointer] entry to the dictionary.
    ///
    /// The Uniform Split technique adds block keys in the dictionary. See
    /// [`BlockReader`](super::BlockReader) and [`TermBytes`](super::TermBytes)
    /// for more info about block key and minimal distinguishing prefix (MDP).
    ///
    /// All block keys are added in strictly increasing order of the block
    /// file pointers, this allows long encoding optimizations such as with
    /// `PositiveIntOutputs` for `FST`.
    ///
    /// `block_key` is the minimal distinguishing prefix (MDP) of the first
    /// term of a block; `block_file_pointer` the non-negative file pointer to
    /// the start of the block in the block file.
    ///
    /// # Errors
    ///
    /// The errors of the dictionary construction.
    fn add(&mut self, block_key: &BytesRef, block_file_pointer: i64) -> Result<()>;

    /// Builds the immutable [`IndexDictionary`] for the added entries.
    ///
    /// # Errors
    ///
    /// The errors of the dictionary construction.
    fn build(&mut self) -> Result<Box<dyn IndexDictionary>>;
}

/// Stateful [`Browser`] to seek a term in this [`IndexDictionary`] and get
/// its corresponding block file pointer in the block file.
///
/// Port of the interface `IndexDictionary.Browser`.
pub trait Browser {
    /// Seeks the given term in the [`IndexDictionary`] and returns its
    /// corresponding block file pointer.
    ///
    /// Returns the block file pointer corresponding to the term if it matches
    /// exactly a block key in the dictionary. Otherwise the floor block key,
    /// which is the greatest block key present in the dictionary that is
    /// alphabetically preceding the searched term. Otherwise `-1` if there is
    /// no floor block key because the searched term precedes alphabetically
    /// the first block key of the dictionary.
    ///
    /// # Errors
    ///
    /// The I/O errors of reading the dictionary.
    fn seek_block(&mut self, term: &BytesRef) -> Result<i64>;
}

/// Supplier for a new stateful [`Browser`] created on the immutable
/// [`IndexDictionary`].
///
/// The immutable [`IndexDictionary`] is lazy loaded thread safely. This lazy
/// loading allows us to load it only when `TermsEnum.seekCeil` or
/// `TermsEnum.seekExact` are called (it is not loaded for a direct all-terms
/// enumeration).
///
/// Port of the interface `IndexDictionary.BrowserSupplier` (an
/// `IOSupplier<Browser>`).
pub trait BrowserSupplier: Send + Sync {
    /// Port of `IOSupplier.get()`.
    ///
    /// # Errors
    ///
    /// The I/O errors of loading the dictionary.
    fn get(&self) -> Result<Box<dyn Browser>>;
}
