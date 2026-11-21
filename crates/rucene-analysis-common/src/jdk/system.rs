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

//! The static methods of `java.lang.System` the module calls.

use rucene::error::{LuceneError, LuceneErrorKind, Result};

/// `System.arraycopy(buf, src_pos, buf, dest_pos, length)` on one `char[]`:
/// the units are copied as if through a temporary array, so overlapping
/// ranges are safe.
///
/// # Errors
///
/// Java's `ArrayIndexOutOfBoundsException`, with JDK 25's message, for a
/// negative `length` or a range past the end of `buf`.
pub fn arraycopy_within(
    buf: &mut [u16],
    src_pos: usize,
    dest_pos: usize,
    length: i64,
) -> Result<()> {
    let aioobe = |message: String| {
        LuceneError::from(LuceneErrorKind::Unchecked {
            class: "java.lang.ArrayIndexOutOfBoundsException",
            message: Some(message),
            cause: None,
        })
    };
    if length < 0 {
        return Err(aioobe(format!("arraycopy: length {length} is negative")));
    }
    let len = buf.len();
    if src_pos as i64 + length > len as i64 {
        return Err(aioobe(format!(
            "arraycopy: last source index {} out of bounds for char[{len}]",
            src_pos as i64 + length
        )));
    }
    if dest_pos as i64 + length > len as i64 {
        return Err(aioobe(format!(
            "arraycopy: last destination index {} out of bounds for char[{len}]",
            dest_pos as i64 + length
        )));
    }
    let length = length as usize;
    buf.copy_within(src_pos..src_pos + length, dest_pos);
    Ok(())
}
