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

//! The type declared in `ConnectionCostsWriter.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use rucene::codecs::codec_util::write_header;
use rucene::error::Result;
use rucene::store::java_nio_file::{Files, Path};
use rucene::store::DataOutput;

use super::binary_dictionary::allocate_direct;
use super::connection_costs::ConnectionCosts;
use super::dictionary_entry_writer::write_file;
use crate::jdk::byte_buffer::ByteBuffer;

/// Writes connection costs
///
/// Equivalent to `org.apache.lucene.analysis.morph.ConnectionCostsWriter<T
/// extends ConnectionCosts>`; Java's `Class<T> implClazz` is held as the
/// class's binary name, the only thing the writer reads from it.
#[derive(Debug, Clone)]
pub struct ConnectionCostsWriter {
    impl_class_name: String,
    /// array is backward IDs first since get is called using the same
    /// backward ID consecutively. maybe doesn't matter.
    costs: ByteBuffer,
    forward_size: i32,
    backward_size: i32,
}

impl ConnectionCostsWriter {
    /// Constructor for building. TODO: remove write access
    ///
    /// Equivalent to `ConnectionCostsWriter(Class<T> implClazz, int
    /// forwardSize, int backwardSize)`; `impl_class_name` is
    /// `implClazz.getName()`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`capacity < 0: …`) when the matrix size
    /// is negative, as `ByteBuffer.allocateDirect` throws.
    pub fn new(
        impl_class_name: impl Into<String>,
        forward_size: i32,
        backward_size: i32,
    ) -> Result<Self> {
        Ok(Self {
            impl_class_name: impl_class_name.into(),
            forward_size,
            backward_size,
            costs: allocate_direct(2i32.wrapping_mul(backward_size).wrapping_mul(forward_size))?,
        })
    }

    /// Equivalent to `ConnectionCostsWriter.add(int forwardId, int backwardId, int cost)`.
    ///
    /// # Errors
    ///
    /// `IndexOutOfBoundsException` for ids outside the matrix, as
    /// `ByteBuffer.putShort` throws.
    pub fn add(&mut self, forward_id: i32, backward_id: i32, cost: i32) -> Result<()> {
        let offset = backward_id
            .wrapping_mul(self.forward_size)
            .wrapping_add(forward_id)
            .wrapping_mul(2);
        self.costs.put_short_at(offset, cost as i16)
    }

    /// Equivalent to the private `ConnectionCostsWriter.getBaseFileName()`.
    fn get_base_file_name(&self) -> String {
        self.impl_class_name.replace('.', "/")
    }

    /// Equivalent to `ConnectionCostsWriter.write(Path baseDir, String
    /// connectionCostsCodecHeader, int dictCodecVersion)`: as in Java, it
    /// creates `baseDir` (not the file's parent).
    ///
    /// # Errors
    ///
    /// What creating and writing the file raise.
    pub fn write(
        &self,
        base_dir: &Path,
        connection_costs_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<()> {
        Files::create_directories(base_dir)?;
        let file_name = format!(
            "{}{}",
            self.get_base_file_name(),
            ConnectionCosts::FILENAME_SUFFIX
        );
        write_file(&base_dir.resolve_str(&file_name), |out| {
            write_header(out, connection_costs_codec_header, dict_codec_version)?;
            out.write_v_int(self.forward_size)?;
            out.write_v_int(self.backward_size)?;
            let mut last: i32 = 0;
            for i in 0..self.costs.limit() / 2 {
                let cost = self.costs.get_short_at((i * 2) as i32)?;
                let delta = i32::from(cost) - last;
                out.write_z_int(delta)?;
                last = i32::from(cost);
            }
            Ok(())
        })
    }
}
