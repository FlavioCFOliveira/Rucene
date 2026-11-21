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

//! The type declared in `BinaryDictionary.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::io::Read;

use rucene::codecs::codec_util::check_header;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::{JavaIoException, EOF_EXCEPTION};
use rucene::store::{DataInput, InputStreamDataInput};
use rucene::util::{IOSupplier, IntsRef};

use super::dictionary::Dictionary;
use super::morph_data::MorphData;
use crate::jdk::byte_buffer::ByteBuffer;

/// `new int[n]`: `NegativeArraySizeException` (message `n`) for a negative
/// size.
pub(crate) fn new_int_array(n: i32) -> Result<Vec<i32>> {
    if n < 0 {
        return Err(LuceneError::unchecked(
            "java.lang.NegativeArraySizeException",
            n.to_string(),
        ));
    }
    Ok(vec![0; n as usize])
}

/// `ByteBuffer.allocateDirect(capacity)`: `IllegalArgumentException`
/// (`capacity < 0: (n < 0)`) for a negative capacity.
pub(crate) fn allocate_direct(capacity: i32) -> Result<ByteBuffer> {
    if capacity < 0 {
        return Err(LuceneError::illegal_argument(format!(
            "capacity < 0: ({capacity} < 0)"
        )));
    }
    Ok(ByteBuffer::allocate(capacity as usize))
}

/// Abstract dictionary base class.
///
/// Equivalent to `org.apache.lucene.analysis.morph.BinaryDictionary<T extends
/// MorphData>`, an abstract class implementing `Dictionary<T>`: this struct
/// holds its fields, and a concrete dictionary holds a `BinaryDictionary`
/// and implements [`BinaryDictionarySubclass`].
#[derive(Debug, Clone)]
pub struct BinaryDictionary {
    target_map_offsets: Vec<i32>,
    target_map: Vec<i32>,
    /// Equivalent to the protected `BinaryDictionary.buffer`, a read-only
    /// view of the dictionary bytes.
    pub buffer: ByteBuffer,
}

impl BinaryDictionary {
    /// Equivalent to `BinaryDictionary.DICT_FILENAME_SUFFIX`.
    pub const DICT_FILENAME_SUFFIX: &'static str = "$buffer.dat";
    /// Equivalent to `BinaryDictionary.TARGETMAP_FILENAME_SUFFIX`.
    pub const TARGETMAP_FILENAME_SUFFIX: &'static str = "$targetMap.dat";
    /// Equivalent to `BinaryDictionary.POSDICT_FILENAME_SUFFIX`.
    pub const POSDICT_FILENAME_SUFFIX: &'static str = "$posDict.dat";

    /// Equivalent to the protected `BinaryDictionary(IOSupplier<InputStream>
    /// targetMapResource, IOSupplier<InputStream> dictResource, String
    /// targetMapCodecHeader, String dictCodecHeader, int dictCodecVersion)`.
    ///
    /// The dictionary bytes after the header are read as one
    /// `ReadableByteChannel.read` of `Channels.newChannel(dictIS)` reads
    /// them: until the buffer is full or the stream ends.
    /// **Divergence:** the JDK's channel also stops early when the stream's
    /// `available()` is 0 after a partial read; a Rust [`Read`] has no
    /// `available()`, so the read goes on.
    ///
    /// # Errors
    ///
    /// What the resources and `CodecUtil.checkHeader` raise, `IOException`
    /// (`targetMap file format broken; …`) for a malformed target map,
    /// `EOFException` (`Cannot read whole dictionary`) for a short dictionary,
    /// and the JDK's exceptions for a negative size.
    pub fn new(
        target_map_resource: &mut dyn IOSupplier<Box<dyn Read + Send>>,
        dict_resource: &mut dyn IOSupplier<Box<dyn Read + Send>>,
        target_map_codec_header: &str,
        dict_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<Self> {
        let (target_map, target_map_offsets) = {
            let map_is = target_map_resource.get()?;
            let mut input = InputStreamDataInput::new(map_is);
            check_header(
                &mut input,
                target_map_codec_header,
                dict_codec_version,
                dict_codec_version,
            )?;
            let mut target_map = new_int_array(input.read_v_int()?)?;
            let mut target_map_offsets = new_int_array(input.read_v_int()?)?;
            Self::populate_target_map(&mut input, &mut target_map, &mut target_map_offsets)?;
            (target_map, target_map_offsets)
        };
        // no buffering here, as we load in one large buffer
        let buffer = {
            let mut dict_is = dict_resource.get()?;
            let size = {
                let mut input = InputStreamDataInput::new(&mut dict_is);
                check_header(
                    &mut input,
                    dict_codec_header,
                    dict_codec_version,
                    dict_codec_version,
                )?;
                input.read_v_int()?
            };
            let mut tmp_buffer = allocate_direct(size)?;
            let mut bytes = vec![0u8; size as usize];
            let mut total = 0usize;
            let mut at_eof = false;
            while total < bytes.len() {
                let n = dict_is.read(&mut bytes[total..])?;
                if n == 0 {
                    at_eof = true;
                    break;
                }
                total += n;
            }
            // `channel.read` answers -1 for a stream already at its end.
            let read: i64 = if at_eof && total == 0 && size > 0 {
                -1
            } else {
                total as i64
            };
            if read != size as i64 {
                return Err(LuceneError::from(LuceneErrorKind::Io(
                    JavaIoException::new(
                        EOF_EXCEPTION,
                        Some("Cannot read whole dictionary".to_string()),
                    )
                    .into_io_error(),
                )));
            }
            tmp_buffer.put_bytes(&bytes)?;
            tmp_buffer
        };
        Ok(Self {
            target_map_offsets,
            target_map,
            buffer,
        })
    }

    /// Equivalent to the private `BinaryDictionary.populateTargetMap(DataInput, int[], int[])`.
    fn populate_target_map(
        input: &mut dyn DataInput,
        target_map: &mut [i32],
        target_map_offsets: &mut [i32],
    ) -> Result<()> {
        let mut accum: i32 = 0;
        let mut source_id: usize = 0;
        for (ofs, slot) in target_map.iter_mut().enumerate() {
            let val = input.read_v_int()?;
            if (val & 0x01) != 0 {
                target_map_offsets[source_id] = ofs as i32;
                source_id += 1;
            }
            accum = accum.wrapping_add(((val as u32) >> 1) as i32);
            *slot = accum;
        }
        if source_id + 1 != target_map_offsets.len() {
            return Err(LuceneError::from(LuceneErrorKind::Io(
                JavaIoException::new(
                    "java.io.IOException",
                    Some(format!(
                        "targetMap file format broken; targetMap.length={}, targetMapOffsets.length={}, sourceId={}",
                        target_map.len(),
                        target_map_offsets.len(),
                        source_id
                    )),
                )
                .into_io_error(),
            )));
        }
        target_map_offsets[source_id] = target_map.len() as i32;
        Ok(())
    }

    /// Equivalent to `BinaryDictionary.lookupWordIds(int sourceId, IntsRef ref)`.
    ///
    /// **Divergence:** Java points `ref.ints` at the whole target map, with
    /// `ref.offset` at the source's first word id; [`IntsRef`] owns its
    /// array, so `ref.ints` receives a copy of the source's word ids and
    /// `ref.offset` is 0. `ref.ints[ref.offset + i]` holds the same ids.
    ///
    /// # Panics
    ///
    /// For a `source_id` outside the map, where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn lookup_word_ids(&self, source_id: i32, r#ref: &mut IntsRef) {
        let offset = self.target_map_offsets[source_id as usize];
        // targetMapOffsets always has one more entry pointing behind last:
        let length = self.target_map_offsets[source_id as usize + 1] - offset;
        r#ref.ints = self.target_map[offset as usize..(offset + length) as usize].to_vec();
        r#ref.offset = 0;
        r#ref.length = length as usize;
    }
}

/// A concrete dictionary: a subclass of `BinaryDictionary<T>`, which
/// implements `Dictionary<T>`.
pub trait BinaryDictionarySubclass<T: ?Sized + MorphData>: Dictionary<T> {
    /// The `BinaryDictionary` this dictionary extends.
    fn binary_dictionary(&self) -> &BinaryDictionary;

    /// Equivalent to the inherited `BinaryDictionary.lookupWordIds(int, IntsRef)`.
    fn lookup_word_ids(&self, source_id: i32, r#ref: &mut IntsRef) {
        self.binary_dictionary().lookup_word_ids(source_id, r#ref);
    }
}
