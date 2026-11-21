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

//! `java.util.Base64.getDecoder()`, the basic RFC 4648 decoder, as JDK 25
//! implements it (see the [`jdk`](crate::jdk) module documentation).

use rucene::error::{LuceneError, Result};

/// The value of each base64 alphabet byte, `-1` for a byte outside the
/// alphabet and `-2` for the padding byte `=` (`Base64.Decoder.fromBase64`).
fn from_base64(b: u8) -> i32 {
    match b {
        b'A'..=b'Z' => i32::from(b - b'A'),
        b'a'..=b'z' => i32::from(b - b'a') + 26,
        b'0'..=b'9' => i32::from(b - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        b'=' => -2,
        _ => -1,
    }
}

/// `decodedOutLength(byte[], int, int)` of the basic decoder.
fn decoded_out_length(src: &[u8]) -> Result<usize> {
    let mut paddings = 0;
    let len = src.len();
    if len == 0 {
        return Ok(0);
    }
    if len < 2 {
        return Err(LuceneError::illegal_argument(
            "Input byte[] should at least have 2 bytes for base64 bytes",
        ));
    }
    if src[len - 1] == b'=' {
        paddings += 1;
        if src[len - 2] == b'=' {
            paddings += 1;
        }
    }
    if paddings == 0 && (len & 0x3) != 0 {
        paddings = 4 - (len & 0x3);
    }
    Ok(3 * len.div_ceil(4) - paddings)
}

/// `Integer.toString(b, 16)` of the signed Java `byte` `b`.
fn signed_byte_hex(b: u8) -> String {
    let value = b as i8;
    if value < 0 {
        format!("-{:x}", -i32::from(value))
    } else {
        format!("{value:x}")
    }
}

/// Decodes all bytes of `src` with the basic base64 scheme.
///
/// Equivalent to `Base64.getDecoder().decode(byte[])`: `decode0` of JDK 25's
/// basic decoder, without its block fast path, which decodes the same bytes.
///
/// # Errors
///
/// `IllegalArgumentException` with the JDK's messages: `Input byte[] should
/// at least have 2 bytes for base64 bytes`, `Input byte array has wrong
/// 4-byte ending unit`, `Illegal base64 character <hex>`, `Last unit does
/// not have enough valid bits` and `Input byte array has incorrect ending
/// byte at <n>`.
///
/// # Panics
///
/// Where Java throws `ArrayIndexOutOfBoundsException` writing past the
/// array `decodedOutLength` sized.
pub fn decode(src: &[u8]) -> Result<Vec<u8>> {
    let mut dst = vec![0u8; decoded_out_length(src)?];
    let sl = src.len();
    let mut sp = 0;
    let mut dp = 0;
    let mut bits: i32 = 0;
    let mut shiftto = 18; // pos of first byte of 4-byte atom

    while sp < sl {
        let mut b = from_base64(src[sp]);
        sp += 1;
        if b < 0 {
            if b == -2 {
                // padding byte '='
                //  =     shiftto==18 unnecessary padding
                //  x=    shiftto==12 a dangling single x
                //  x     to be handled together with non-padding case
                //  xx=   shiftto==6&&sp==sl missing last =
                //  xx=y  shiftto==6 last is not =
                let wrong_ending = if shiftto == 6 {
                    let missing = sp == sl || src[sp] != b'=';
                    if sp < sl {
                        sp += 1;
                    }
                    missing
                } else {
                    shiftto == 18
                };
                if wrong_ending {
                    return Err(LuceneError::illegal_argument(
                        "Input byte array has wrong 4-byte ending unit",
                    ));
                }
                break;
            }
            return Err(LuceneError::illegal_argument(format!(
                "Illegal base64 character {}",
                signed_byte_hex(src[sp - 1])
            )));
        }
        b <<= shiftto;
        bits |= b;
        shiftto -= 6;
        if shiftto < 0 {
            dst[dp] = (bits >> 16) as u8;
            dst[dp + 1] = (bits >> 8) as u8;
            dst[dp + 2] = bits as u8;
            dp += 3;
            shiftto = 18;
            bits = 0;
        }
    }
    // reached end of byte array or hit padding '=' characters.
    if shiftto == 6 {
        dst[dp] = (bits >> 16) as u8;
        dp += 1;
    } else if shiftto == 0 {
        dst[dp] = (bits >> 16) as u8;
        dst[dp + 1] = (bits >> 8) as u8;
        dp += 2;
    } else if shiftto == 12 {
        // dangling single "x", incorrectly encoded.
        return Err(LuceneError::illegal_argument(
            "Last unit does not have enough valid bits",
        ));
    }
    // anything left is invalid (Java's `src[sp++]` is only read by the
    // MIME decoder, so the position reported is the first byte left)
    if sp < sl {
        return Err(LuceneError::illegal_argument(format!(
            "Input byte array has incorrect ending byte at {sp}"
        )));
    }
    dst.truncate(dp);
    Ok(dst)
}

/// Decodes a base64 encoded string, given as its UTF-16 units.
///
/// Equivalent to `Base64.getDecoder().decode(String)`,
/// `decode(src.getBytes(ISO_8859_1))`: a unit above U+00FF becomes `?`.
///
/// # Errors
///
/// As [`decode`].
pub fn decode_string(src: &[u16]) -> Result<Vec<u8>> {
    let latin1: Vec<u8> = src
        .iter()
        .map(|&unit| u8::try_from(unit).unwrap_or(b'?'))
        .collect();
    decode(&latin1)
}
