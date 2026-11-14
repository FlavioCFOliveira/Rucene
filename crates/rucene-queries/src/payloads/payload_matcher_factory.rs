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

//! Port of `PayloadMatcherFactory.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::cmp::Ordering;

use rucene::util::java_fmt::java_utf8_to_string;
use rucene::util::string_helper::compare_utf16;
use rucene::util::BytesRef;

use crate::payloads::payload_matcher::PayloadMatcher;
use crate::payloads::span_payload_check_query::{MatchOperation, PayloadType};

/// Creates a payload matcher object based on a payload type and an operation.
///
/// Equivalent to `org.apache.lucene.queries.payloads.PayloadMatcherFactory`.
/// Payload types of `INT`, `FLOAT` or `STRING` are supported, and inequality
/// operations are supported.
#[derive(Debug, Default, Clone, Copy)]
pub struct PayloadMatcherFactory;

static EQ_PAYLOAD_MATCHER: EQPayloadMatcher = EQPayloadMatcher;
static LT_INT_PAYLOAD_MATCHER: LTIntPayloadMatcher = LTIntPayloadMatcher;
static LTE_INT_PAYLOAD_MATCHER: LTEIntPayloadMatcher = LTEIntPayloadMatcher;
static GT_INT_PAYLOAD_MATCHER: GTIntPayloadMatcher = GTIntPayloadMatcher;
static GTE_INT_PAYLOAD_MATCHER: GTEIntPayloadMatcher = GTEIntPayloadMatcher;
static LT_FLOAT_PAYLOAD_MATCHER: LTFloatPayloadMatcher = LTFloatPayloadMatcher;
static LTE_FLOAT_PAYLOAD_MATCHER: LTEFloatPayloadMatcher = LTEFloatPayloadMatcher;
static GT_FLOAT_PAYLOAD_MATCHER: GTFloatPayloadMatcher = GTFloatPayloadMatcher;
static GTE_FLOAT_PAYLOAD_MATCHER: GTEFloatPayloadMatcher = GTEFloatPayloadMatcher;
static LT_STRING_PAYLOAD_MATCHER: LTStringPayloadMatcher = LTStringPayloadMatcher;
static LTE_STRING_PAYLOAD_MATCHER: LTEStringPayloadMatcher = LTEStringPayloadMatcher;
static GT_STRING_PAYLOAD_MATCHER: GTStringPayloadMatcher = GTStringPayloadMatcher;
static GTE_STRING_PAYLOAD_MATCHER: GTEStringPayloadMatcher = GTEStringPayloadMatcher;

impl PayloadMatcherFactory {
    /// The matcher of `op` for `payload_type`.
    ///
    /// Equivalent to `payloadCheckerOpTypeMap.get(payloadType).get(op)`: the
    /// private static map from payload type to the map of its four inequality
    /// matchers, which answers `null` for `EQ`.
    fn payload_checker_op_type_map(
        payload_type: PayloadType,
        op: MatchOperation,
    ) -> Option<&'static dyn PayloadMatcher> {
        match (payload_type, op) {
            // ints
            (PayloadType::Int, MatchOperation::Lt) => Some(&LT_INT_PAYLOAD_MATCHER),
            (PayloadType::Int, MatchOperation::Lte) => Some(&LTE_INT_PAYLOAD_MATCHER),
            (PayloadType::Int, MatchOperation::Gt) => Some(&GT_INT_PAYLOAD_MATCHER),
            (PayloadType::Int, MatchOperation::Gte) => Some(&GTE_INT_PAYLOAD_MATCHER),
            // floats
            (PayloadType::Float, MatchOperation::Lt) => Some(&LT_FLOAT_PAYLOAD_MATCHER),
            (PayloadType::Float, MatchOperation::Lte) => Some(&LTE_FLOAT_PAYLOAD_MATCHER),
            (PayloadType::Float, MatchOperation::Gt) => Some(&GT_FLOAT_PAYLOAD_MATCHER),
            (PayloadType::Float, MatchOperation::Gte) => Some(&GTE_FLOAT_PAYLOAD_MATCHER),
            // strings
            (PayloadType::String, MatchOperation::Lt) => Some(&LT_STRING_PAYLOAD_MATCHER),
            (PayloadType::String, MatchOperation::Lte) => Some(&LTE_STRING_PAYLOAD_MATCHER),
            (PayloadType::String, MatchOperation::Gt) => Some(&GT_STRING_PAYLOAD_MATCHER),
            (PayloadType::String, MatchOperation::Gte) => Some(&GTE_STRING_PAYLOAD_MATCHER),
            (_, MatchOperation::Eq) => None,
        }
    }

    /// Returns a payload matcher for use in the span payload check query that
    /// will decode the bytes from a payload based on the payload type, and
    /// apply a matching inequality operation (eq, lt, lte, gt and gte).
    ///
    /// Equivalent to the static
    /// `PayloadMatcherFactory.createMatcherForOpAndType(PayloadType, MatchOperation)`;
    /// Java's `null` operation is `None`.
    ///
    /// * `payload_type` — the type of the payload to decode: `STRING`, `INT`,
    ///   `FLOAT`;
    /// * `op` — an inequality operation as the test (for example `EQ` for
    ///   equals, `GT` for greater than).
    pub fn create_matcher_for_op_and_type(
        payload_type: PayloadType,
        op: Option<MatchOperation>,
    ) -> &'static dyn PayloadMatcher {
        // special optimization, binary/byte comparison
        let op = match op {
            None | Some(MatchOperation::Eq) => return &EQ_PAYLOAD_MATCHER,
            Some(op) => op,
        };
        // otherwise, we need to pay attention to the payload type and operation
        match Self::payload_checker_op_type_map(payload_type, op) {
            Some(matcher) => matcher,
            // Unknown op and payload type gets you an equals operator.
            None => &EQ_PAYLOAD_MATCHER,
        }
    }
}

/// Throws the `ArrayIndexOutOfBoundsException` a byte-array `VarHandle` view
/// of a 4-byte type raises: it checks `index` against `length - 3`.
fn view_index_out_of_bounds(index: usize, length: usize) -> ! {
    rucene::store::java_nio_file::throw_unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        Some(&format!(
            "Index {index} out of bounds for length {}",
            length as i64 - 3
        )),
    )
}

/// `BitUtil.VH_BE_INT.get(bytes, offset)`: four bytes of the backing array,
/// big-endian.
fn vh_be_int(bytes: &[u8], offset: usize) -> i32 {
    match bytes.get(offset..offset.saturating_add(4)) {
        Some(b) if b.len() == 4 => i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        _ => view_index_out_of_bounds(offset, bytes.len()),
    }
}

/// Equality is the same for all payload types.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.EQPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct EQPayloadMatcher;

impl PayloadMatcher for EQPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        source.bytes_equals(payload)
    }
}

/// The payload matchers that decode both payloads as UTF-8 strings.
///
/// Equivalent to the private abstract static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.StringPayloadMatcher`;
/// its concrete `comparePayload` is
/// [`compare_string_payload`](Self::compare_string_payload).
pub(crate) trait StringPayloadMatcher {
    /// Equivalent to `StringPayloadMatcher.comparePayload(BytesRef, BytesRef)`.
    fn compare_string_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.string_compare(
            &self.decode_string(&payload.bytes, payload.offset, payload.length),
            &self.decode_string(&source.bytes, source.offset, source.length),
        )
    }

    /// `new String(ArrayUtil.copyOfSubArray(bytes, offset, offset + length), UTF_8)`.
    ///
    /// Equivalent to the private `StringPayloadMatcher.decodeString(byte[], int, int)`.
    fn decode_string(&self, bytes: &[u8], offset: usize, length: usize) -> String {
        java_utf8_to_string(&bytes[offset..offset + length])
    }

    /// Equivalent to the protected abstract
    /// `StringPayloadMatcher.stringCompare(String, String)`.
    fn string_compare(&self, val: &str, threshold: &str) -> bool;
}

/// `String.compareTo(String)`, which compares UTF-16 code units.
fn java_compare_to(val: &str, thresh: &str) -> Ordering {
    compare_utf16(val, thresh)
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTStringPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTStringPayloadMatcher;

impl StringPayloadMatcher for LTStringPayloadMatcher {
    fn string_compare(&self, val: &str, thresh: &str) -> bool {
        let res = java_compare_to(val, thresh);
        res == Ordering::Less
    }
}

impl PayloadMatcher for LTStringPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_string_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTEStringPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTEStringPayloadMatcher;

impl StringPayloadMatcher for LTEStringPayloadMatcher {
    fn string_compare(&self, val: &str, thresh: &str) -> bool {
        // `res < 1`
        let res = java_compare_to(val, thresh);
        res != Ordering::Greater
    }
}

impl PayloadMatcher for LTEStringPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_string_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTStringPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTStringPayloadMatcher;

impl StringPayloadMatcher for GTStringPayloadMatcher {
    fn string_compare(&self, val: &str, thresh: &str) -> bool {
        let res = java_compare_to(val, thresh);
        res == Ordering::Greater
    }
}

impl PayloadMatcher for GTStringPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_string_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTEStringPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTEStringPayloadMatcher;

impl StringPayloadMatcher for GTEStringPayloadMatcher {
    fn string_compare(&self, val: &str, thresh: &str) -> bool {
        // `res > -1`
        let res = java_compare_to(val, thresh);
        res != Ordering::Less
    }
}

impl PayloadMatcher for GTEStringPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_string_payload(source, payload)
    }
}

/// The payload matchers that decode both payloads as big-endian ints.
///
/// Equivalent to the private abstract static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.IntPayloadMatcher`;
/// its concrete `comparePayload` is
/// [`compare_int_payload`](Self::compare_int_payload).
pub(crate) trait IntPayloadMatcher {
    /// Equivalent to `IntPayloadMatcher.comparePayload(BytesRef, BytesRef)`.
    fn compare_int_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.int_compare(
            self.decode_int(&payload.bytes, payload.offset),
            self.decode_int(&source.bytes, source.offset),
        )
    }

    /// `(int) BitUtil.VH_BE_INT.get(bytes, offset)`.
    ///
    /// Equivalent to the private `IntPayloadMatcher.decodeInt(byte[], int)`.
    fn decode_int(&self, bytes: &[u8], offset: usize) -> i32 {
        vh_be_int(bytes, offset)
    }

    /// Equivalent to the protected abstract
    /// `IntPayloadMatcher.intCompare(int, int)`.
    fn int_compare(&self, val: i32, threshold: i32) -> bool;
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTIntPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTIntPayloadMatcher;

impl IntPayloadMatcher for LTIntPayloadMatcher {
    fn int_compare(&self, val: i32, thresh: i32) -> bool {
        val < thresh
    }
}

impl PayloadMatcher for LTIntPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_int_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTEIntPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTEIntPayloadMatcher;

impl IntPayloadMatcher for LTEIntPayloadMatcher {
    fn int_compare(&self, val: i32, thresh: i32) -> bool {
        val <= thresh
    }
}

impl PayloadMatcher for LTEIntPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_int_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTIntPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTIntPayloadMatcher;

impl IntPayloadMatcher for GTIntPayloadMatcher {
    fn int_compare(&self, val: i32, thresh: i32) -> bool {
        val > thresh
    }
}

impl PayloadMatcher for GTIntPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_int_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTEIntPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTEIntPayloadMatcher;

impl IntPayloadMatcher for GTEIntPayloadMatcher {
    fn int_compare(&self, val: i32, thresh: i32) -> bool {
        val >= thresh
    }
}

impl PayloadMatcher for GTEIntPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_int_payload(source, payload)
    }
}

/// The payload matchers that decode both payloads as big-endian floats.
///
/// Equivalent to the private abstract static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.FloatPayloadMatcher`;
/// its concrete `comparePayload` is
/// [`compare_float_payload`](Self::compare_float_payload).
pub(crate) trait FloatPayloadMatcher {
    /// Equivalent to `FloatPayloadMatcher.comparePayload(BytesRef, BytesRef)`.
    fn compare_float_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.float_compare(
            self.decode_float(&payload.bytes, payload.offset),
            self.decode_float(&source.bytes, source.offset),
        )
    }

    /// `(float) BitUtil.VH_BE_FLOAT.get(bytes, offset)`.
    ///
    /// Equivalent to the private `FloatPayloadMatcher.decodeFloat(byte[], int)`.
    fn decode_float(&self, bytes: &[u8], offset: usize) -> f32 {
        f32::from_bits(vh_be_int(bytes, offset) as u32)
    }

    /// Equivalent to the protected abstract
    /// `FloatPayloadMatcher.floatCompare(float, float)`.
    fn float_compare(&self, val: f32, threshold: f32) -> bool;
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTFloatPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTFloatPayloadMatcher;

impl FloatPayloadMatcher for LTFloatPayloadMatcher {
    fn float_compare(&self, val: f32, thresh: f32) -> bool {
        val < thresh
    }
}

impl PayloadMatcher for LTFloatPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_float_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.LTEFloatPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LTEFloatPayloadMatcher;

impl FloatPayloadMatcher for LTEFloatPayloadMatcher {
    fn float_compare(&self, val: f32, thresh: f32) -> bool {
        val <= thresh
    }
}

impl PayloadMatcher for LTEFloatPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_float_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTFloatPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTFloatPayloadMatcher;

impl FloatPayloadMatcher for GTFloatPayloadMatcher {
    fn float_compare(&self, val: f32, thresh: f32) -> bool {
        val > thresh
    }
}

impl PayloadMatcher for GTFloatPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_float_payload(source, payload)
    }
}

/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.payloads.PayloadMatcherFactory.GTEFloatPayloadMatcher`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GTEFloatPayloadMatcher;

impl FloatPayloadMatcher for GTEFloatPayloadMatcher {
    fn float_compare(&self, val: f32, thresh: f32) -> bool {
        val >= thresh
    }
}

impl PayloadMatcher for GTEFloatPayloadMatcher {
    fn compare_payload(&self, source: &BytesRef, payload: &BytesRef) -> bool {
        self.compare_float_payload(source, payload)
    }
}
