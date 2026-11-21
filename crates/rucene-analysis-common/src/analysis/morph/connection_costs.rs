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

//! The type declared in `ConnectionCosts.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::io::Read;

use rucene::codecs::codec_util::check_header;
use rucene::error::Result;
use rucene::store::{DataInput, InputStreamDataInput};
use rucene::util::IOSupplier;

use super::binary_dictionary::allocate_direct;
use crate::jdk::byte_buffer::ByteBuffer;

/// n-gram connection cost data
///
/// Equivalent to `org.apache.lucene.analysis.morph.ConnectionCosts`, an
/// abstract class without abstract methods: a concrete cost table holds a
/// `ConnectionCosts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCosts {
    buffer: ByteBuffer,
    forward_size: i32,
}

impl ConnectionCosts {
    /// Equivalent to `ConnectionCosts.FILENAME_SUFFIX`.
    pub const FILENAME_SUFFIX: &'static str = ".dat";

    /// Equivalent to the protected `ConnectionCosts(IOSupplier<InputStream>
    /// connectionCostResource, String connectionCostsCodecHeader, int
    /// dictCodecVersion)`.
    ///
    /// # Errors
    ///
    /// What the resource and `CodecUtil.checkHeader` raise, `EOFException`
    /// for a short file, and `IllegalArgumentException` for a negative size.
    pub fn new(
        connection_cost_resource: &mut dyn IOSupplier<Box<dyn Read + Send>>,
        connection_costs_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<Self> {
        let is = connection_cost_resource.get()?;
        let mut input = InputStreamDataInput::new(is);
        check_header(
            &mut input,
            connection_costs_codec_header,
            dict_codec_version,
            dict_codec_version,
        )?;
        let forward_size = input.read_v_int()?;
        let backward_size = input.read_v_int()?;
        let size = forward_size.wrapping_mul(backward_size);
        // copy the matrix into a direct byte buffer
        let mut tmp_buffer = allocate_direct(size.wrapping_mul(2))?;
        let mut accum: i32 = 0;
        for _j in 0..backward_size {
            for _i in 0..forward_size {
                accum = accum.wrapping_add(input.read_z_int()?);
                tmp_buffer.put_short(accum as i16)?;
            }
        }
        Ok(Self {
            buffer: tmp_buffer,
            forward_size,
        })
    }

    /// Equivalent to `ConnectionCosts.get(int forwardId, int backwardId)`.
    ///
    /// # Panics
    ///
    /// For ids outside the matrix, where Java's `ByteBuffer.getShort` throws
    /// `IndexOutOfBoundsException`.
    pub fn get(&self, forward_id: i32, backward_id: i32) -> i32 {
        // map 2d matrix into a single dimension short array
        let offset = backward_id
            .wrapping_mul(self.forward_size)
            .wrapping_add(forward_id)
            .wrapping_mul(2);
        match self.buffer.get_short_at(offset) {
            Ok(cost) => i32::from(cost),
            Err(e) => panic!("{e}"),
        }
    }
}
