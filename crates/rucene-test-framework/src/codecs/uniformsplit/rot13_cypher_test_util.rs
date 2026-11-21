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

//! Port of `org.apache.lucene.tests.codecs.uniformsplit.Rot13CypherTestUtil`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{to_int_exact, LuceneError, Result};
use rucene::store::{DataInput, DataOutput};
use rucene::util::BytesRef;
use rucene_codecs::uniformsplit::{BlockDecoder, BlockEncoder, WritableBytes};

const ENCODING_OFFSET: i32 = 7;
const ENCODING_ROTATION: u8 = 13;

/// Test utility for simple ROT13 cipher (<https://en.wikipedia.org/wiki/ROT13>).
///
/// Port of `Rot13CypherTestUtil`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rot13CypherTestUtil;

/// `new byte[length]`: `NegativeArraySizeException` (message: the length)
/// for a negative length.
fn new_byte_array(length: i32) -> Result<Vec<u8>> {
    if length < 0 {
        return Err(LuceneError::unchecked(
            "java.lang.NegativeArraySizeException",
            length.to_string(),
        ));
    }
    Ok(vec![0u8; length as usize])
}

impl Rot13CypherTestUtil {
    /// `encode(DataInput bytesInput, int length)`: `length` bytes of
    /// `bytes_input`, each plus 13 (wrapping, as Java's `(byte)` cast does),
    /// after 7 zero bytes.
    ///
    /// # Errors
    ///
    /// `NegativeArraySizeException` when `length + 7` is negative (Java's
    /// `int` addition wraps); the errors of reading the input.
    pub fn encode(bytes_input: &mut dyn DataInput, length: i32) -> Result<Vec<u8>> {
        let mut encoded_bytes = new_byte_array(length.wrapping_add(ENCODING_OFFSET))?;
        for i in 0..length.max(0) as usize {
            encoded_bytes[i + ENCODING_OFFSET as usize] =
                bytes_input.read_byte()?.wrapping_add(ENCODING_ROTATION);
        }
        Ok(encoded_bytes)
    }

    /// `decode(DataInput bytesInput, long length)`: skips the 7 leading
    /// bytes, then reads `length - 7` bytes, each minus 13.
    ///
    /// # Errors
    ///
    /// `ArithmeticException` (`Math.toIntExact`) when `length - 7` is outside
    /// the `int` range, `NegativeArraySizeException` when it is negative; the
    /// errors of reading the input.
    pub fn decode(bytes_input: &mut dyn DataInput, length: i64) -> Result<Vec<u8>> {
        let length = length.wrapping_sub(i64::from(ENCODING_OFFSET));
        bytes_input.skip_bytes(i64::from(ENCODING_OFFSET))?;
        let mut decoded_bytes = new_byte_array(to_int_exact(length)?)?;
        for byte in decoded_bytes.iter_mut() {
            *byte = bytes_input.read_byte()?.wrapping_sub(ENCODING_ROTATION);
        }
        Ok(decoded_bytes)
    }

    /// `getBlockEncoder()`: a [`BlockEncoder`] that [`encode`](Self::encode)s
    /// each block.
    pub fn get_block_encoder() -> Arc<dyn BlockEncoder> {
        Arc::new(Rot13Encoder { on_write: None })
    }

    /// `getBlockDecoder()`: a [`BlockDecoder`] that [`decode`](Self::decode)s
    /// each block.
    pub fn get_block_decoder() -> Arc<dyn BlockDecoder> {
        Arc::new(Rot13Decoder { on_decode: None })
    }
}

/// The lambda `(blockBytes, length) -> { byte[] encodedBytes =
/// Rot13CypherTestUtil.encode(blockBytes, Math.toIntExact(length)); return
/// new WritableBytes() {...}; }` of `getBlockEncoder()`, and of
/// `UniformSplitRot13PostingsFormat.getBlockEncoder()`, whose `writeTo` also
/// runs `on_write` (`encoderCalled = true`) before writing.
pub(crate) struct Rot13Encoder {
    pub(crate) on_write: Option<fn()>,
}

impl fmt::Display for Rot13Encoder {
    /// A lambda's `Object.toString()` names a JVM-generated hidden class
    /// (`...$$Lambda/0x...@hash`), which no port can reproduce; this names
    /// the lambda's enclosing class.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("org.apache.lucene.tests.codecs.uniformsplit.Rot13CypherTestUtil$$Lambda")
    }
}

impl BlockEncoder for Rot13Encoder {
    fn encode(
        &self,
        block_bytes: &mut dyn DataInput,
        length: i64,
    ) -> Result<Box<dyn WritableBytes>> {
        let encoded_bytes = Rot13CypherTestUtil::encode(block_bytes, to_int_exact(length)?)?;
        Ok(Box::new(Rot13Bytes {
            encoded_bytes,
            on_write: self.on_write,
        }))
    }
}

/// The anonymous `BlockEncoder.WritableBytes` of the encoder lambdas.
struct Rot13Bytes {
    encoded_bytes: Vec<u8>,
    on_write: Option<fn()>,
}

impl WritableBytes for Rot13Bytes {
    fn size(&self) -> i64 {
        self.encoded_bytes.len() as i64
    }

    fn write_to(&self, data_output: &mut dyn DataOutput) -> Result<()> {
        if let Some(on_write) = self.on_write {
            on_write();
        }
        data_output.write_bytes(&self.encoded_bytes, 0, self.encoded_bytes.len())
    }
}

/// The lambda `(blockBytes, length) -> new
/// BytesRef(Rot13CypherTestUtil.decode(blockBytes, length))` of
/// `getBlockDecoder()`, and of
/// `UniformSplitRot13PostingsFormat.getBlockDecoder()`, which first runs
/// `on_decode` (`decoderCalled = true`).
pub(crate) struct Rot13Decoder {
    pub(crate) on_decode: Option<fn()>,
}

impl fmt::Display for Rot13Decoder {
    /// See [`Rot13Encoder`]'s `Display`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("org.apache.lucene.tests.codecs.uniformsplit.Rot13CypherTestUtil$$Lambda")
    }
}

impl BlockDecoder for Rot13Decoder {
    fn decode(&self, block_bytes: &mut dyn DataInput, length: i64) -> Result<BytesRef> {
        if let Some(on_decode) = self.on_decode {
            on_decode();
        }
        Ok(BytesRef::new(Rot13CypherTestUtil::decode(
            block_bytes,
            length,
        )?))
    }
}
