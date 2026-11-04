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

//! Port of `org.apache.lucene.codecs.uniformsplit.FSTDictionary`.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::store::{ByteArrayDataInput, ByteBuffersDataOutput, DataOutput, IndexInput};
use rucene::util::fst::{
    Builder as FSTCompilerBuilder, BytesRefFSTEnum, FSTCompiler, InputType, OffHeapFSTStore,
    PositiveIntOutputs, Util, FST,
};
use rucene::util::{BytesRef, IntsRefBuilder};

use super::block_decoder::BlockDecoder;
use super::block_encoder::BlockEncoder;
use super::index_dictionary::{self, IndexDictionary};
use super::AliasedOutput;

/// Immutable stateless `FST`-based index dictionary kept in memory.
///
/// Use [`Builder`] to build the [`IndexDictionary`].
///
/// Create a stateful [`Browser`] to seek a term in this [`IndexDictionary`]
/// and get its corresponding block file pointer to the terms block file.
///
/// Its greatest advantage is to be very compact in memory thanks to both the
/// compaction of the `FST` as a byte array, and the incremental encoding of
/// the leaves block pointer values, which are long integers in increasing
/// order, with `PositiveIntOutputs`. With a compact dictionary in memory we
/// can increase the number of blocks. This allows us to reduce the average
/// block size, which means faster scan inside a block.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.FSTDictionary`
/// (`@lucene.experimental`).
pub struct FSTDictionary {
    pub(crate) fst: Arc<FST<PositiveIntOutputs>>,
}

impl FSTDictionary {
    /// Port of `FSTDictionary(FST<Long>)`.
    pub fn new(fst: FST<PositiveIntOutputs>) -> Self {
        Self { fst: Arc::new(fst) }
    }

    /// Reads a [`FSTDictionary`] from the provided input.
    ///
    /// `block_decoder` is the [`BlockDecoder`] to use for specific decoding,
    /// or `None` if none.
    ///
    /// Port of `FSTDictionary.read(DataInput, BlockDecoder, boolean)`. Java's
    /// input is a `DataInput` that it casts to `IndexInput` to keep the FST
    /// off-heap; its only caller, `BrowserSupplier.get`, passes an
    /// `IndexInput`, which this port takes. The off-heap store reads a clone
    /// of the input, positioned where Java's store reads its own.
    ///
    /// # Errors
    ///
    /// The errors of the decoder and of reading the FST.
    pub fn read(
        input: &mut dyn IndexInput,
        block_decoder: Option<&dyn BlockDecoder>,
        is_fst_on_heap: bool,
    ) -> Result<FSTDictionary> {
        let fst_outputs = PositiveIntOutputs::get_singleton();
        let fst = match block_decoder {
            None => {
                let metadata = FST::read_metadata(input, fst_outputs)?;
                if is_fst_on_heap {
                    FST::new(metadata, input)?
                } else {
                    let store =
                        OffHeapFSTStore::new(input.clone_input()?, input.file_pointer(), &metadata);
                    FST::from_reader(metadata, Box::new(store))
                }
            }
            Some(block_decoder) => {
                let num_bytes = input.read_v_long()?;
                let decoded_bytes = block_decoder.decode(input, num_bytes)?;
                let mut fst_data_input =
                    ByteArrayDataInput::with_range(&decoded_bytes.bytes, 0, decoded_bytes.length);
                // OffHeapFSTStore.init() requires a DataInput which is an instance of IndexInput.
                // When the block is decoded we must load the FST on heap.
                let metadata = FST::read_metadata(&mut fst_data_input, fst_outputs)?;
                FST::new(metadata, &mut fst_data_input)?
            }
        };
        Ok(FSTDictionary::new(fst))
    }
}

impl IndexDictionary for FSTDictionary {
    /// Port of `FSTDictionary.write(DataOutput, BlockEncoder)`.
    fn write(
        &self,
        output: &mut dyn DataOutput,
        block_encoder: Option<&dyn BlockEncoder>,
    ) -> Result<()> {
        match block_encoder {
            None => {
                let cell: RefCell<&mut dyn DataOutput> = RefCell::new(output);
                self.fst
                    .save(&mut AliasedOutput(&cell), &mut AliasedOutput(&cell))
            }
            Some(block_encoder) => {
                let mut bytes_data_output = ByteBuffersDataOutput::new_resettable_instance();
                {
                    let cell: RefCell<&mut dyn DataOutput> = RefCell::new(&mut bytes_data_output);
                    self.fst
                        .save(&mut AliasedOutput(&cell), &mut AliasedOutput(&cell))?;
                }
                let encoded_bytes = block_encoder.encode(
                    &mut bytes_data_output.to_data_input()?,
                    bytes_data_output.size() as i64,
                )?;
                output.write_v_long(encoded_bytes.size())?;
                encoded_bytes.write_to(output)
            }
        }
    }

    fn browser(&self) -> Result<Box<dyn index_dictionary::Browser>> {
        Ok(Box::new(Browser {
            fst_enum: BytesRefFSTEnum::new_shared(Arc::clone(&self.fst))?,
        }))
    }
}

/// Stateful [`Browser`] to seek a term in this [`FSTDictionary`] and get its
/// corresponding block file pointer in the block file.
///
/// Port of the inner class `FSTDictionary.Browser`.
pub struct Browser {
    fst_enum: BytesRefFSTEnum<'static, PositiveIntOutputs>,
}

impl index_dictionary::Browser for Browser {
    fn seek_block(&mut self, term: &BytesRef) -> Result<i64> {
        let seek_floor = self.fst_enum.seek_floor(term)?;
        Ok(seek_floor.map_or(-1, |io| *io.output))
    }
}

/// Provides stateful [`Browser`] to seek in the [`FSTDictionary`].
///
/// Port of the static nested class `FSTDictionary.BrowserSupplier`
/// (`@lucene.experimental`). Java's double-checked `synchronized` lazy load
/// is a [`Mutex`] here.
pub struct BrowserSupplier {
    state: Mutex<SupplierState>,
    block_decoder: Option<Arc<dyn BlockDecoder>>,
    is_fst_on_heap: bool,
}

struct SupplierState {
    dictionary_input: Box<dyn IndexInput>,
    /// Lazy loaded immutable index dictionary FST. The FST is either kept
    /// off-heap, or hold in RAM on-heap.
    dictionary: Option<Arc<FSTDictionary>>,
}

impl BrowserSupplier {
    /// Port of `BrowserSupplier(IndexInput, long, BlockDecoder, boolean)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of cloning and seeking the input.
    pub fn new(
        dictionary_input: &dyn IndexInput,
        dictionary_start_fp: i64,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        is_fst_on_heap: bool,
    ) -> Result<Self> {
        let mut input = dictionary_input.clone_input()?;
        input.seek(dictionary_start_fp)?;
        Ok(Self {
            state: Mutex::new(SupplierState {
                dictionary_input: input,
                dictionary: None,
            }),
            block_decoder,
            is_fst_on_heap,
        })
    }
}

impl index_dictionary::BrowserSupplier for BrowserSupplier {
    fn get(&self) -> Result<Box<dyn index_dictionary::Browser>> {
        let dictionary = {
            let mut state = self.state.lock().map_err(|_| {
                LuceneError::illegal_state("the dictionary supplier's lock is poisoned")
            })?;
            match &state.dictionary {
                Some(dictionary) => Arc::clone(dictionary),
                None => {
                    let dictionary = Arc::new(FSTDictionary::read(
                        state.dictionary_input.as_mut(),
                        self.block_decoder.as_deref(),
                        self.is_fst_on_heap,
                    )?);
                    state.dictionary = Some(Arc::clone(&dictionary));
                    dictionary
                }
            }
        };
        dictionary.browser()
    }
}

/// Builds an immutable [`FSTDictionary`].
///
/// Port of the static nested class `FSTDictionary.Builder`
/// (`@lucene.experimental`).
pub struct Builder {
    fst_compiler: Option<FSTCompiler<'static, PositiveIntOutputs>>,
    scratch_ints: IntsRefBuilder,
}

impl Builder {
    /// Port of `FSTDictionary.Builder()`.
    ///
    /// # Errors
    ///
    /// The errors of creating the FST compiler.
    pub fn new() -> Result<Self> {
        let outputs = PositiveIntOutputs::get_singleton();
        let fst_compiler = FSTCompilerBuilder::new(InputType::Byte1, outputs).build()?;
        Ok(Self {
            fst_compiler: Some(fst_compiler),
            scratch_ints: IntsRefBuilder::new(),
        })
    }

    fn compiler(&mut self) -> Result<&mut FSTCompiler<'static, PositiveIntOutputs>> {
        self.fst_compiler
            .as_mut()
            .ok_or_else(|| LuceneError::illegal_state("FST was already compiled"))
    }
}

impl index_dictionary::Builder for Builder {
    fn add(&mut self, block_key: &BytesRef, block_file_pointer: i64) -> Result<()> {
        let input = Util::to_ints_ref(block_key, &mut self.scratch_ints);
        self.compiler()?.add(&input, block_file_pointer)
    }

    fn build(&mut self) -> Result<Box<dyn IndexDictionary>> {
        let metadata = self.compiler()?.compile()?;
        let compiler = self
            .fst_compiler
            .take()
            .ok_or_else(|| LuceneError::illegal_state("FST was already compiled"))?;
        let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader()?).ok_or_else(|| {
            super::npe(
                "Cannot invoke \"org.apache.lucene.util.fst.FST.save(org.apache.lucene.store.\
                 DataOutput, org.apache.lucene.store.DataOutput)\" because \"this.fst\" is null",
            )
        })?;
        Ok(Box::new(FSTDictionary::new(fst)))
    }
}
