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

//! Port of `org.apache.lucene.tests.codecs.vector.ConfigurableMCodec`.

use std::fmt;

use rucene::codecs::{Codec, FilterCodec, Lucene99HnswVectorsFormat};

use crate::codecs::forward_codec;
use crate::util::TestUtil;

/// This codec allows customization of the number of connections made for an
/// HNSW index.
///
/// Port of `ConfigurableMCodec` (`extends FilterCodec`, named
/// `ConfigurableMCodec`): the default codec with
/// `new Lucene99HnswVectorsFormat(maxConn, 100)` as its KNN-vectors format.
pub struct ConfigurableMCodec {
    inner: FilterCodec,
}

impl ConfigurableMCodec {
    /// `new ConfigurableMCodec()`: `maxConn` 128.
    pub fn new() -> Self {
        Self::with_max_conn(128)
    }

    /// `new ConfigurableMCodec(maxConn)`.
    ///
    /// # Panics
    ///
    /// The `IllegalArgumentException` of `Lucene99HnswVectorsFormat`'s
    /// constructor for a `max_conn` out of range.
    pub fn with_max_conn(max_conn: i32) -> Self {
        ConfigurableMCodec {
            inner: FilterCodec::new("ConfigurableMCodec", TestUtil::get_default_codec())
                .with_knn_vectors_format(Lucene99HnswVectorsFormat::with_max_conn_beam_width(
                    max_conn, 100,
                )),
        }
    }
}

impl Default for ConfigurableMCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ConfigurableMCodec {
    /// `Codec.toString()`: the name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConfigurableMCodec")
    }
}

impl Codec for ConfigurableMCodec {
    forward_codec!(inner);
}
