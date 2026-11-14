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

//! Port of `PayloadDecoder.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use rucene::util::BytesRef;

/// Defines a way of converting payloads to float values, for use by
/// [`PayloadScoreQuery`](crate::payloads::payload_score_query::PayloadScoreQuery).
///
/// Equivalent to the functional interface
/// `org.apache.lucene.queries.payloads.PayloadDecoder`.
///
/// **Divergence from Lucene 10.5.0.** A Java `PayloadDecoder` — a lambda, as
/// [`FLOAT_DECODER`] is — inherits `Object.equals` and `Object.hashCode`, which
/// [`PayloadScoreQuery`](crate::payloads::payload_score_query::PayloadScoreQuery)
/// reads (`PayloadScoreQuery.java:136,141`). Rust has no object identity for a
/// value behind a trait object, so equality is declared as
/// [`decoder_eq`](Self::decoder_eq), whose default compares the concrete types
/// — what identity is for the singleton `FLOAT_DECODER` — and the identity
/// hash as [`decoder_hash`](Self::decoder_hash).
pub trait PayloadDecoder: Debug + Send + Sync + 'static {
    /// Computes a float value for the given payload, `None` where Java's is
    /// `null`.
    ///
    /// Equivalent to `PayloadDecoder.computePayloadFactor(BytesRef)`.
    fn compute_payload_factor(&self, payload: Option<&BytesRef>) -> f32;

    /// Returns this decoder as [`Any`], for [`decoder_eq`](Self::decoder_eq).
    fn as_any(&self) -> &dyn Any;

    /// `Objects.equals(decoder, other)`; see the trait documentation.
    fn decoder_eq(&self, other: &dyn PayloadDecoder) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
    }

    /// `decoder.hashCode()`; see the trait documentation.
    fn decoder_hash(&self) -> i32;
}

/// A [`PayloadDecoder`] that interprets the bytes of a payload as a float.
///
/// Equivalent to the lambda behind the constant `PayloadDecoder.FLOAT_DECODER`,
/// `bytes -> bytes == null ? 1 : bytes.bytes[bytes.offset]`: the first byte,
/// signed, widened to a float.
#[derive(Debug, Default, Clone, Copy)]
pub struct FloatDecoder;

/// A [`PayloadDecoder`] that interprets the bytes of a payload as a float.
///
/// Equivalent to the constant `PayloadDecoder.FLOAT_DECODER`.
pub const FLOAT_DECODER: FloatDecoder = FloatDecoder;

/// Returns [`FLOAT_DECODER`] as a shared decoder.
pub fn float_decoder() -> Arc<dyn PayloadDecoder> {
    Arc::new(FLOAT_DECODER)
}

impl PayloadDecoder for FloatDecoder {
    fn compute_payload_factor(&self, payload: Option<&BytesRef>) -> f32 {
        match payload {
            None => 1.0,
            Some(bytes) => {
                // `bytes.bytes[bytes.offset]`, which reads the backing array
                // whatever `length` says.
                match bytes.bytes.get(bytes.offset) {
                    Some(&byte) => f32::from(byte as i8),
                    None => rucene::store::java_nio_file::throw_unchecked(
                        "java.lang.ArrayIndexOutOfBoundsException",
                        Some(&format!(
                            "Index {} out of bounds for length {}",
                            bytes.offset,
                            bytes.bytes.len()
                        )),
                    ),
                }
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn decoder_hash(&self) -> i32 {
        rucene::util::java_collections::java_identity_hash_code_stand_in(
            "org.apache.lucene.queries.payloads.PayloadDecoder.FLOAT_DECODER",
        )
    }
}
