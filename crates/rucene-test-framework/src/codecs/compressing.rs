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

//! Port of `org.apache.lucene.tests.codecs.compressing`: support for testing
//! `Lucene90CompressingStoredFieldsFormat` and
//! `Lucene90CompressingTermVectorsFormat`.

pub mod compressing_codec;
pub mod deflate_with_preset_compressing_codec;
pub mod dummy;
pub mod fast_compressing_codec;
pub mod fast_decompression_compressing_codec;
pub mod high_compression_compressing_codec;
pub mod lz4_with_preset_compressing_codec;

pub use compressing_codec::CompressingCodec;
pub use deflate_with_preset_compressing_codec::DeflateWithPresetCompressingCodec;
pub use fast_compressing_codec::FastCompressingCodec;
pub use fast_decompression_compressing_codec::FastDecompressionCompressingCodec;
pub use high_compression_compressing_codec::HighCompressionCompressingCodec;
pub use lz4_with_preset_compressing_codec::LZ4WithPresetCompressingCodec;
