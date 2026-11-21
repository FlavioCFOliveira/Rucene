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

//! The decoders of JDK 25's multi-byte charsets other than `UTF-8`, as
//! [`charset`](super::charset) runs them.
//!
//! Two kinds of decoder are emulated:
//!
//! * **Byte-sequence machines**, for the table-driven charsets (the DBCS,
//!   EUC, Shift_JIS, Big5, HKSCS, EBCDIC-stateful and ISO-2022 charsets):
//!   `tools/translate/JdkCharsetData.java` probes the JDK 25 decoder of each
//!   one through the public `java.nio.charset` API, byte by byte, from each of
//!   its shift states, and records, for every shortest byte sequence the
//!   decoder decides on, what it decides: the UTF-16 units it outputs and the
//!   bytes it consumes, malformed or unmappable input of some length, or a
//!   shift that outputs nothing, with the state the decoder is in after it,
//!   and whether it reports `OVERFLOW` first when the output is full. The
//!   generator checks every machine against the JDK decoder on pseudo-random
//!   streams fed in random chunks into random small output buffers, call for
//!   call (result, input position and output); `Machine::decode_loop` runs
//!   it exactly as the generator's model does.
//! * **Algorithms** written from the JDK 25 sources, read as specification
//!   (`sun/nio/cs/UnicodeDecoder.java`, `UTF_32Coder.java`, `CESU_8.java`,
//!   `GB18030.java`, `sun/nio/cs/ext/ISCII91.java`, `JISAutoDetect.java`):
//!   the `UTF-16` and `UTF-32` families and `CESU-8` need no data; `GB18030`
//!   reads its two-byte table and the four-byte table of the Basic
//!   Multilingual Plane, and `x-ISCII91` the unit of each byte, both
//!   generated from the JDK and checked by the generator against the JDK
//!   decoder; `x-JISAutoDetect` runs the `ISO-2022-JP`, `EUC-JP` and
//!   `Shift_JIS` machines.
//!
//! The data is `charset_multibyte.bin`: a directory of items, each
//! zlib-compressed on its own and inflated the first time a decoder of its
//! charset is created.

use std::io::Read;
use std::sync::{Arc, LazyLock, OnceLock};

use super::charset::CoderResult;

/// The compressed data of the multi-byte charsets.
static DATA_BYTES: &[u8] = include_bytes!("charset_multibyte.bin");

/// A byte order of the `UTF-16` and `UTF-32` decoders: `UnicodeDecoder`'s
/// and `UTF_32Coder`'s `NONE`, `BIG` and `LITTLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ByteOrder {
    /// `NONE`: the byte order mark decides.
    None,
    /// `BIG`: big-endian.
    Big,
    /// `LITTLE`: little-endian.
    Little,
}

/// A big-endian reader of the generator's `DataOutputStream` output.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        slice
    }

    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    fn u16(&mut self) -> u16 {
        let b = self.take(2);
        u16::from_be_bytes([b[0], b[1]])
    }

    fn u32(&mut self) -> u32 {
        let b = self.take(4);
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }

    fn len(&mut self) -> usize {
        self.u32() as usize
    }
}

/// An item of the container: its name and the range of its zlib stream.
struct Item {
    name: &'static str,
    offset: usize,
    length: usize,
    inflated: usize,
}

/// The container's directory: `"RJCS"`, the item count, and per item its
/// name, the offset and length of its zlib stream and its inflated length.
static DIRECTORY: LazyLock<Vec<Item>> = LazyLock::new(|| {
    let mut r = Reader::new(DATA_BYTES);
    assert_eq!(r.take(4), b"RJCS", "INVARIANT: the embedded charset data");
    let count = r.len();
    (0..count)
        .map(|_| {
            let name_len = usize::from(r.u16());
            let name = std::str::from_utf8(r.take(name_len))
                .expect("INVARIANT: the generator writes UTF-8 names");
            Item {
                name,
                offset: r.len(),
                length: r.len(),
                inflated: r.len(),
            }
        })
        .collect()
});

/// The inflated bytes of the item `name`.
fn inflate(name: &str) -> Vec<u8> {
    let item = DIRECTORY
        .iter()
        .find(|item| item.name == name)
        .expect("INVARIANT: the generator writes an item for every charset that needs one");
    let mut bytes = Vec::with_capacity(item.inflated);
    flate2::read::ZlibDecoder::new(&DATA_BYTES[item.offset..item.offset + item.length])
        .read_to_end(&mut bytes)
        .expect("INVARIANT: the embedded charset data inflates");
    assert_eq!(
        bytes.len(),
        item.inflated,
        "INVARIANT: the embedded charset data"
    );
    bytes
}

/// Reconstructs `count` values written as `planes` byte planes (low byte
/// first) of their successive differences.
fn read_planes(r: &mut Reader<'_>, count: usize, planes: usize) -> Vec<u32> {
    let bytes = r.take(count * planes);
    let mut values = Vec::with_capacity(count);
    let mut prev = 0u32;
    for i in 0..count {
        let mut delta = 0u32;
        for plane in 0..planes {
            delta |= u32::from(bytes[plane * count + i]) << (8 * plane);
        }
        prev = prev.wrapping_add(delta);
        if planes < 4 {
            prev &= (1u32 << (8 * planes)) - 1;
        }
        values.push(prev);
    }
    values
}

// ------------------------------------------------------------- machines

/// Entry tag: the next byte leads to a child node.
const TAG_NODE: u32 = 0;
/// Entry tag: one unit is output, the state is kept.
const TAG_EMIT1: u32 = 1;
/// Entry tag: malformed or unmappable input, the state is kept.
const TAG_ERROR: u32 = 2;
/// Entry tag: an extra leaf ([`Extra`]).
const TAG_EXTRA: u32 = 3;

/// A trie node of a [`Machine`]: the entries of the bytes `lo..=hi`, every
/// other byte having `default`.
#[derive(Debug)]
struct Node {
    /// Input ending inside this node's sequence reports `OVERFLOW` first
    /// when the output is full.
    overflow_first: bool,
    lo: u8,
    hi: u8,
    default: u32,
    /// The index of the entry of `lo` in [`Machine::entries`].
    start: usize,
}

/// A decision of a [`Machine`] that is not a plain entry: two units, a
/// change of state, or both.
#[derive(Debug)]
struct Extra {
    /// The bytes the decision consumes.
    consumed: u8,
    /// The number of units output (0 to 2).
    units: u8,
    unit1: u16,
    unit2: u16,
    /// The state after the decision.
    next: u16,
    /// `OVERFLOW` first when the output is full.
    overflow_first: bool,
    /// 0: none; 1: malformed input; 2: unmappable input, of `error_length`.
    error: u8,
    error_length: u8,
}

/// The byte-sequence machine of a table-driven JDK 25 decoder (see the
/// module documentation).
#[derive(Debug)]
pub(crate) struct Machine {
    /// The root node of each state.
    roots: Vec<u32>,
    nodes: Vec<Node>,
    entries: Vec<u32>,
    extras: Vec<Extra>,
}

impl Machine {
    /// Reads a machine the generator serialized.
    fn parse(bytes: &[u8]) -> Machine {
        let mut r = Reader::new(bytes);
        let state_count = r.len();
        let roots = (0..state_count).map(|_| r.u32()).collect();
        let node_count = r.len();
        let mut nodes = Vec::with_capacity(node_count);
        let mut total = 0;
        for _ in 0..node_count {
            let flags = r.u8();
            let lo = r.u8();
            let hi = r.u8();
            let default = r.u32();
            nodes.push(Node {
                overflow_first: flags & 1 != 0,
                lo,
                hi,
                default,
                start: total,
            });
            if lo <= hi {
                total += usize::from(hi - lo) + 1;
            }
        }
        let entries = read_planes(&mut r, total, 4);
        let extra_count = r.len();
        let extras = (0..extra_count)
            .map(|_| Extra {
                consumed: r.u8(),
                units: r.u8(),
                unit1: r.u16(),
                unit2: r.u16(),
                next: r.u16(),
                overflow_first: r.u8() != 0,
                error: r.u8(),
                error_length: r.u8(),
            })
            .collect();
        assert_eq!(r.pos, bytes.len(), "INVARIANT: the embedded charset data");
        Machine {
            roots,
            nodes,
            entries,
            extras,
        }
    }

    /// The entry of byte `b` in node `node`.
    fn entry(&self, node: u32, b: u8) -> u32 {
        let n = &self.nodes[node as usize];
        if b < n.lo || b > n.hi {
            n.default
        } else {
            self.entries[n.start + usize::from(b - n.lo)]
        }
    }

    /// Decodes `src[*sp..]` into `dst[*dp..]` from state `*state`, as the
    /// JDK decoder's `decodeLoop` does: until the input is used up or ends
    /// inside a sequence (`UNDERFLOW`), the output lacks room (`OVERFLOW`),
    /// or malformed or unmappable input starts at `*sp`.
    pub(crate) fn decode_loop(
        &self,
        state: &mut u16,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        let sl = src.len();
        let dl = dst.len();
        loop {
            if *sp >= sl {
                return CoderResult::Underflow;
            }
            let mut node = self.roots[usize::from(*state)];
            let mut p = *sp;
            loop {
                if p >= sl {
                    if *dp >= dl && self.nodes[node as usize].overflow_first {
                        return CoderResult::Overflow;
                    }
                    return CoderResult::Underflow;
                }
                let e = self.entry(node, src[p]);
                p += 1;
                match e >> 30 {
                    TAG_NODE => {
                        node = e & 0x3FFF_FFFF;
                        continue;
                    }
                    TAG_EMIT1 => {
                        if *dp >= dl {
                            return CoderResult::Overflow;
                        }
                        dst[*dp] = (e & 0xFFFF) as u16;
                        *dp += 1;
                        *sp += ((e >> 16) & 0xF) as usize;
                    }
                    TAG_ERROR => {
                        if *dp >= dl && e & (1 << 9) != 0 {
                            return CoderResult::Overflow;
                        }
                        let length = (e & 0xFF) as usize;
                        return if e & (1 << 8) != 0 {
                            CoderResult::Unmappable(length)
                        } else {
                            CoderResult::Malformed(length)
                        };
                    }
                    _ => {
                        debug_assert_eq!(e >> 30, TAG_EXTRA);
                        let x = &self.extras[(e & 0x3FFF_FFFF) as usize];
                        if x.error != 0 {
                            if *dp >= dl && x.overflow_first {
                                return CoderResult::Overflow;
                            }
                            *state = x.next;
                            let length = usize::from(x.error_length);
                            return if x.error == 2 {
                                CoderResult::Unmappable(length)
                            } else {
                                CoderResult::Malformed(length)
                            };
                        }
                        if x.units > 0 {
                            if dl - *dp < usize::from(x.units) {
                                return CoderResult::Overflow;
                            }
                            dst[*dp] = x.unit1;
                            *dp += 1;
                            if x.units > 1 {
                                dst[*dp] = x.unit2;
                                *dp += 1;
                            }
                        } else if *dp >= dl && x.overflow_first {
                            return CoderResult::Overflow;
                        }
                        *state = x.next;
                        *sp += usize::from(x.consumed);
                    }
                }
                break;
            }
        }
    }
}

/// A machine item's name and its machine, parsed on first use.
type MachineCell = (&'static str, OnceLock<Arc<Machine>>);

/// The machine of the item `name` (`M:<charset>`), parsed once.
pub(crate) fn machine(name: &'static str) -> Arc<Machine> {
    static MACHINES: LazyLock<Vec<MachineCell>> = LazyLock::new(|| {
        DIRECTORY
            .iter()
            .filter(|item| item.name.starts_with("M:"))
            .map(|item| (item.name, OnceLock::new()))
            .collect()
    });
    let (item, cell) = MACHINES
        .iter()
        .find(|(item, _)| *item == name)
        .expect("INVARIANT: every machine charset has an item");
    Arc::clone(cell.get_or_init(|| Arc::new(Machine::parse(&inflate(item)))))
}

/// A machine with the state of one decoder.
#[derive(Debug, Clone)]
pub(crate) struct MachineDecoder {
    machine: Arc<Machine>,
    state: u16,
}

impl MachineDecoder {
    /// A decoder of the machine `machine` in its initial state.
    pub(crate) fn new(machine: Arc<Machine>) -> Self {
        Self { machine, state: 0 }
    }

    /// `decodeLoop`.
    pub(crate) fn decode_loop(
        &mut self,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        self.machine.decode_loop(&mut self.state, src, sp, dst, dp)
    }

    /// `implReset()`: the initial state.
    pub(crate) fn reset(&mut self) {
        self.state = 0;
    }
}

// ---------------------------------------------------- UTF-16 and UTF-32

/// The state of a `UTF-16` decoder: `sun.nio.cs.UnicodeDecoder`.
#[derive(Debug, Clone)]
pub(crate) struct Utf16Decoder {
    /// `expectedByteOrder`.
    expected: ByteOrder,
    /// `currentByteOrder`.
    current: ByteOrder,
    /// `defaultByteOrder`.
    default: ByteOrder,
}

impl Utf16Decoder {
    /// `UnicodeDecoder(cs, bo, defaultBO)`.
    pub(crate) fn new(expected: ByteOrder, default: ByteOrder) -> Self {
        Self {
            expected,
            current: expected,
            default,
        }
    }

    /// `decode(int, int)`: the unit of the bytes `b1`, `b2` in the current
    /// byte order.
    fn decode(&self, b1: u8, b2: u8) -> u16 {
        if self.current == ByteOrder::Big {
            (u16::from(b1) << 8) | u16::from(b2)
        } else {
            (u16::from(b2) << 8) | u16::from(b1)
        }
    }

    /// `UnicodeDecoder.decodeLoop`: a byte order mark (`FE FF` or `FF FE`)
    /// is read and skipped only while the byte order is `NONE`; an unpaired
    /// high surrogate is malformed input of length 4 (with its follower), an
    /// unpaired low surrogate of length 2.
    pub(crate) fn decode_loop(
        &mut self,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        let mut mark = *sp;
        let mut pos = *sp;
        let result = loop {
            if src.len() - pos <= 1 {
                break CoderResult::Underflow;
            }
            let b1 = src[pos];
            let b2 = src[pos + 1];
            pos += 2;

            // Byte Order Mark interpretation
            if self.current == ByteOrder::None {
                let c = (u16::from(b1) << 8) | u16::from(b2);
                if c == 0xFEFF {
                    self.current = ByteOrder::Big;
                    mark += 2;
                    continue;
                } else if c == 0xFFFE {
                    self.current = ByteOrder::Little;
                    mark += 2;
                    continue;
                } else {
                    self.current = self.default;
                    // FALL THROUGH to process b1, b2 normally
                }
            }

            let c = self.decode(b1, b2);

            // Surrogates
            if (0xD800..=0xDFFF).contains(&c) {
                if c <= 0xDBFF {
                    if src.len() - pos < 2 {
                        break CoderResult::Underflow;
                    }
                    let c2 = self.decode(src[pos], src[pos + 1]);
                    pos += 2;
                    if !(0xDC00..=0xDFFF).contains(&c2) {
                        break CoderResult::Malformed(4);
                    }
                    if dst.len() - *dp < 2 {
                        break CoderResult::Overflow;
                    }
                    mark += 4;
                    dst[*dp] = c;
                    dst[*dp + 1] = c2;
                    *dp += 2;
                    continue;
                }
                // Unpaired low surrogate
                break CoderResult::Malformed(2);
            }

            if *dp >= dst.len() {
                break CoderResult::Overflow;
            }
            mark += 2;
            dst[*dp] = c;
            *dp += 1;
        };
        *sp = mark;
        result
    }

    /// `UnicodeDecoder.implReset()`: the expected byte order again.
    pub(crate) fn reset(&mut self) {
        self.current = self.expected;
    }
}

/// The state of a `UTF-32` decoder: `sun.nio.cs.UTF_32Coder.Decoder`.
#[derive(Debug, Clone)]
pub(crate) struct Utf32Decoder {
    /// `expectedBO`.
    expected: ByteOrder,
    /// `currentBO`.
    current: ByteOrder,
}

impl Utf32Decoder {
    /// `UTF_32Coder.Decoder(cs, bo)`.
    pub(crate) fn new(expected: ByteOrder) -> Self {
        Self {
            expected,
            current: ByteOrder::None,
        }
    }

    /// `getCP(ByteBuffer)`: the four bytes at `pos` in the current byte
    /// order.
    fn get_cp(&self, src: &[u8], pos: usize) -> u32 {
        let b = [src[pos], src[pos + 1], src[pos + 2], src[pos + 3]];
        if self.current == ByteOrder::Big {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    }

    /// `UTF_32Coder.Decoder.decodeLoop`: the first four bytes the decoder
    /// sees decide the byte order (a mark `00 00 FE FF` or `FF FE 00 00` the
    /// expected order allows is skipped); a value outside the code space is
    /// malformed input of length 4.
    pub(crate) fn decode_loop(
        &mut self,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        if src.len() - *sp < 4 {
            return CoderResult::Underflow;
        }
        let mut mark = *sp;
        let mut pos = *sp;
        if self.current == ByteOrder::None {
            let cp = u32::from_be_bytes([src[pos], src[pos + 1], src[pos + 2], src[pos + 3]]);
            pos += 4;
            if cp == 0xFEFF && self.expected != ByteOrder::Little {
                self.current = ByteOrder::Big;
                mark += 4;
            } else if cp == 0xFFFE_0000 && self.expected != ByteOrder::Big {
                self.current = ByteOrder::Little;
                mark += 4;
            } else {
                self.current = if self.expected == ByteOrder::None {
                    ByteOrder::Big
                } else {
                    self.expected
                };
                pos = mark;
            }
        }
        let result = loop {
            if src.len() - pos < 4 {
                break CoderResult::Underflow;
            }
            let cp = self.get_cp(src, pos);
            pos += 4;
            if cp < 0x10000 {
                if *dp >= dst.len() {
                    break CoderResult::Overflow;
                }
                mark += 4;
                dst[*dp] = cp as u16;
                *dp += 1;
            } else if cp <= 0x10FFFF {
                if dst.len() - *dp < 2 {
                    break CoderResult::Overflow;
                }
                mark += 4;
                dst[*dp] = (((cp - 0x10000) >> 10) + 0xD800) as u16;
                dst[*dp + 1] = (((cp - 0x10000) & 0x3FF) + 0xDC00) as u16;
                *dp += 2;
            } else {
                break CoderResult::Malformed(4);
            }
        };
        *sp = mark;
        result
    }

    /// `UTF_32Coder.Decoder.implReset()`: the byte order undecided again.
    pub(crate) fn reset(&mut self) {
        self.current = ByteOrder::None;
    }
}

// --------------------------------------------------------------- CESU-8

/// `isNotContinuation(b)`.
fn is_not_continuation(b: u8) -> bool {
    (b & 0xC0) != 0x80
}

/// `isMalformed3_2(b1, b2)`.
fn is_malformed3_2(b1: u8, b2: u8) -> bool {
    (b1 == 0xE0 && (b2 & 0xE0) == 0x80) || (b2 & 0xC0) != 0x80
}

/// `isMalformed3(b1, b2, b3)`.
fn is_malformed3(b1: u8, b2: u8, b3: u8) -> bool {
    (b1 == 0xE0 && (b2 & 0xE0) == 0x80) || (b2 & 0xC0) != 0x80 || (b3 & 0xC0) != 0x80
}

/// `xflow`: underflow when the input lacks the `nb` bytes of the sequence
/// (or `nb` is 0), overflow when the output lacks room.
fn xflow(remaining: usize, nb: usize) -> CoderResult {
    if nb == 0 || remaining < nb {
        CoderResult::Underflow
    } else {
        CoderResult::Overflow
    }
}

/// The decoding loop of JDK 25's `CESU-8` decoder (`CESU_8.Decoder`): one-,
/// two- and three-byte sequences, each three-byte one a single UTF-16 unit
/// (a surrogate pair is two such sequences, and surrogates are not checked);
/// a lead byte `F0`-`FF` or a lone continuation byte is malformed input of
/// length 1.
pub(crate) fn cesu8_decode_loop(
    src: &[u8],
    sp: &mut usize,
    dst: &mut [u16],
    dp: &mut usize,
) -> CoderResult {
    let sl = src.len();
    let dl = dst.len();
    while *sp < sl {
        let b1 = src[*sp];
        if b1 < 0x80 {
            // 1 byte, 7 bits: 0xxxxxxx
            if *dp >= dl {
                return xflow(sl - *sp, 1);
            }
            dst[*dp] = u16::from(b1);
            *dp += 1;
            *sp += 1;
        } else if (b1 >> 5) == 0b110 && (b1 & 0x1E) != 0 {
            // 2 bytes, 11 bits: 110xxxxx 10xxxxxx
            if sl - *sp < 2 || *dp >= dl {
                return xflow(sl - *sp, 2);
            }
            let b2 = src[*sp + 1];
            if is_not_continuation(b2) {
                return CoderResult::Malformed(1);
            }
            dst[*dp] = (u16::from(b1 & 0x1F) << 6) | u16::from(b2 & 0x3F);
            *dp += 1;
            *sp += 2;
        } else if (b1 >> 4) == 0b1110 {
            // 3 bytes, 16 bits: 1110xxxx 10xxxxxx 10xxxxxx
            let src_remaining = sl - *sp;
            if src_remaining < 3 || *dp >= dl {
                if src_remaining > 1 && is_malformed3_2(b1, src[*sp + 1]) {
                    return CoderResult::Malformed(1);
                }
                return xflow(src_remaining, 3);
            }
            let b2 = src[*sp + 1];
            let b3 = src[*sp + 2];
            if is_malformed3(b1, b2, b3) {
                // `malformedN(src, 3)`: no need to look at b3
                let length = if (b1 == 0xE0 && (b2 & 0xE0) == 0x80) || is_not_continuation(b2) {
                    1
                } else {
                    2
                };
                return CoderResult::Malformed(length);
            }
            dst[*dp] =
                (u16::from(b1 & 0x0F) << 12) | (u16::from(b2 & 0x3F) << 6) | u16::from(b3 & 0x3F);
            *dp += 1;
            *sp += 3;
        } else {
            return CoderResult::Malformed(1);
        }
    }
    CoderResult::Underflow
}

// -------------------------------------------------------------- GB18030

/// The tables of `GB18030` (see [`gb18030_decode_loop`]).
#[derive(Debug)]
pub(crate) struct Gb18030Data {
    /// The unit of each two-byte sequence: lead `0x81`-`0xFE` by trail
    /// `0x40`-`0xFE` (191 per lead; `0x7F` is malformed input).
    double: Vec<u16>,
    /// The unit of each four-byte sequence of the Basic Multilingual Plane,
    /// by linear offset `0` to `0x99FB`.
    four: Vec<u16>,
}

/// The `GB18030` tables, inflated once.
pub(crate) fn gb18030_data() -> &'static Gb18030Data {
    static DATA: LazyLock<Gb18030Data> = LazyLock::new(|| {
        let bytes = inflate("GB18030");
        let mut r = Reader::new(&bytes);
        let double_len = r.len();
        let four_len = r.len();
        let values = read_planes(&mut r, double_len + four_len, 2);
        assert_eq!(r.pos, bytes.len(), "INVARIANT: the embedded charset data");
        let values: Vec<u16> = values.into_iter().map(|v| v as u16).collect();
        Gb18030Data {
            double: values[..double_len].to_vec(),
            four: values[double_len..].to_vec(),
        }
    });
    &DATA
}

/// The decoding loop of JDK 25's `GB18030` decoder (`GB18030.Decoder`, the
/// 2022 mapping JDK 25 uses by default): ASCII; a lead byte `0x81`-`0xFE`
/// followed by a trail `0x40`-`0xFE` but `0x7F` (two bytes, from the table),
/// or by `0x30`-`0x39`, a byte `0x81`-`0xFE` and a byte `0x30`-`0x39` (four
/// bytes: a linear offset, from the table in the Basic Multilingual Plane,
/// computed in the supplementary planes). The malformed lengths are the
/// JDK's: 1 for a bad lead byte or a second byte below `0x30`, 2 for a bad
/// trail, 3 for a bad third byte, 4 for a bad fourth byte or an offset
/// outside both ranges.
pub(crate) fn gb18030_decode_loop(
    data: &Gb18030Data,
    src: &[u8],
    sp: &mut usize,
    dst: &mut [u16],
    dp: &mut usize,
) -> CoderResult {
    let sl = src.len();
    let dl = dst.len();
    while *sp < sl {
        let b1 = usize::from(src[*sp]);
        let size;
        let mut units = [0u16; 2];
        let mut count = 1;
        if b1 < 0x80 {
            // US-ASCII range
            size = 1;
            units[0] = b1 as u16;
        } else if !(0x81..=0xFE).contains(&b1) {
            return CoderResult::Malformed(1);
        } else {
            // Either 2 or 4 byte sequence follows
            if sl - *sp < 2 {
                return CoderResult::Underflow;
            }
            let b2 = usize::from(src[*sp + 1]);
            if b2 < 0x30 {
                return CoderResult::Malformed(1);
            } else if b2 <= 0x39 {
                if sl - *sp < 4 {
                    return CoderResult::Underflow;
                }
                let b3 = usize::from(src[*sp + 2]);
                if !(0x81..=0xFE).contains(&b3) {
                    return CoderResult::Malformed(3);
                }
                let b4 = usize::from(src[*sp + 3]);
                if !(0x30..=0x39).contains(&b4) {
                    return CoderResult::Malformed(4);
                }
                size = 4;
                if *dp >= dl {
                    return CoderResult::Overflow;
                }
                let offset = (((b1 - 0x81) * 10 + (b2 - 0x30)) * 126 + b3 - 0x81) * 10 + b4 - 0x30;
                if offset < data.four.len() {
                    units[0] = data.four[offset];
                } else if (0x2E248..0x12E248).contains(&offset) {
                    // Supplemental UCS planes handled via surrogates
                    let cp = offset - 0x1E248;
                    if dl - *dp < 2 {
                        return CoderResult::Overflow;
                    }
                    units[0] = ((cp - 0x10000) / 0x400 + 0xD800) as u16;
                    units[1] = ((cp - 0x10000) % 0x400 + 0xDC00) as u16;
                    count = 2;
                } else {
                    return CoderResult::Malformed(4);
                }
            } else if b2 == 0x7F || b2 == 0xFF || b2 < 0x40 {
                return CoderResult::Malformed(2);
            } else {
                size = 2;
                units[0] = data.double[(b1 - 0x81) * 191 + (b2 - 0x40)];
            }
        }
        if *dp >= dl {
            return CoderResult::Overflow;
        }
        dst[*dp..*dp + count].copy_from_slice(&units[..count]);
        *dp += count;
        *sp += size;
    }
    CoderResult::Underflow
}

// -------------------------------------------------------------- ISCII91

/// `ISCII91.NUKTA_CHAR`.
const NUKTA_CHAR: u16 = 0x093C;
/// `ISCII91.HALANT_CHAR`.
const HALANT_CHAR: u16 = 0x094D;
/// `ISCII91.Decoder.ZWNJ_CHAR`.
const ZWNJ_CHAR: u16 = 0x200C;
/// `ISCII91.Decoder.ZWJ_CHAR`.
const ZWJ_CHAR: u16 = 0x200D;
/// `ISCII91.Decoder.INVALID_CHAR`.
const INVALID_CHAR: u16 = 0xFFFF;

/// The unit `x-ISCII91` decodes each byte to (`directMapTable` at the index
/// the decoder computes from the byte), [`INVALID_CHAR`] for an unmappable
/// byte; inflated once.
fn iscii91_table() -> &'static [u16; 256] {
    static TABLE: LazyLock<[u16; 256]> = LazyLock::new(|| {
        let bytes = inflate("ISCII91");
        let mut r = Reader::new(&bytes);
        let mut table = [0u16; 256];
        for slot in &mut table {
            *slot = r.u16();
        }
        assert_eq!(r.pos, bytes.len(), "INVARIANT: the embedded charset data");
        table
    });
    &TABLE
}

/// The state of an `x-ISCII91` decoder: `ISCII91.Decoder`.
#[derive(Debug, Clone)]
pub(crate) struct Iscii91Decoder {
    /// `contextChar`.
    context_char: u16,
    /// `needFlushing`.
    need_flushing: bool,
}

impl Iscii91Decoder {
    /// `ISCII91.Decoder(cs)`.
    pub(crate) fn new() -> Self {
        Self {
            context_char: INVALID_CHAR,
            need_flushing: false,
        }
    }

    /// `ISCII91.Decoder.decodeArrayLoop`: a sign that may combine with a
    /// following nukta is held back (`needFlushing`) until the next byte;
    /// an unmappable byte with nothing held is unmappable input of length 1.
    pub(crate) fn decode_loop(
        &mut self,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        let table = iscii91_table();
        let dl = dst.len();
        while *sp < src.len() {
            let current_char = table[usize::from(src[*sp])];

            // if the contextChar is either ATR || EXT
            // set the output to '�'
            if self.context_char == 0xFFFD {
                if dl - *dp < 1 {
                    return CoderResult::Overflow;
                }
                dst[*dp] = 0xFFFD;
                *dp += 1;
                self.context_char = INVALID_CHAR;
                self.need_flushing = false;
                *sp += 1;
                continue;
            }

            match current_char {
                0x0901 | 0x0907 | 0x0908 | 0x090B | 0x093F | 0x0940 | 0x0943 | 0x0964 => {
                    if self.need_flushing {
                        if dl - *dp < 1 {
                            return CoderResult::Overflow;
                        }
                        dst[*dp] = self.context_char;
                        *dp += 1;
                        self.context_char = current_char;
                        *sp += 1;
                        continue;
                    }
                    self.context_char = current_char;
                    self.need_flushing = true;
                    *sp += 1;
                    continue;
                }
                NUKTA_CHAR => {
                    if dl - *dp < 1 {
                        return CoderResult::Overflow;
                    }
                    let mapped = match self.context_char {
                        0x0901 => 0x0950,
                        0x0907 => 0x090C,
                        0x0908 => 0x0961,
                        0x090B => 0x0960,
                        0x093F => 0x0962,
                        0x0940 => 0x0963,
                        0x0943 => 0x0944,
                        0x0964 => 0x093D,
                        HALANT_CHAR => {
                            if self.need_flushing {
                                dst[*dp] = self.context_char;
                                *dp += 1;
                                self.context_char = current_char;
                                *sp += 1;
                                continue;
                            }
                            ZWJ_CHAR
                        }
                        _ => {
                            if self.need_flushing {
                                dst[*dp] = self.context_char;
                                *dp += 1;
                                self.context_char = current_char;
                                *sp += 1;
                                continue;
                            }
                            NUKTA_CHAR
                        }
                    };
                    dst[*dp] = mapped;
                    *dp += 1;
                }
                HALANT_CHAR => {
                    if dl - *dp < 1 {
                        return CoderResult::Overflow;
                    }
                    if self.need_flushing {
                        dst[*dp] = self.context_char;
                        *dp += 1;
                        self.context_char = current_char;
                        *sp += 1;
                        continue;
                    }
                    dst[*dp] = if self.context_char == HALANT_CHAR {
                        ZWNJ_CHAR
                    } else {
                        HALANT_CHAR
                    };
                    *dp += 1;
                }
                INVALID_CHAR => {
                    if self.need_flushing {
                        if dl - *dp < 1 {
                            return CoderResult::Overflow;
                        }
                        dst[*dp] = self.context_char;
                        *dp += 1;
                        self.context_char = current_char;
                        *sp += 1;
                        continue;
                    }
                    return CoderResult::Unmappable(1);
                }
                _ => {
                    if dl - *dp < 1 {
                        return CoderResult::Overflow;
                    }
                    if self.need_flushing {
                        dst[*dp] = self.context_char;
                        *dp += 1;
                        self.context_char = current_char;
                        *sp += 1;
                        continue;
                    }
                    dst[*dp] = current_char;
                    *dp += 1;
                }
            }
            self.context_char = current_char;
            self.need_flushing = false;
            *sp += 1;
        }
        CoderResult::Underflow
    }

    /// `ISCII91.Decoder.implFlush(CharBuffer)`: outputs the held sign, if
    /// any, and forgets the context.
    pub(crate) fn flush(&mut self, dst: &mut [u16], dp: &mut usize) -> CoderResult {
        if self.need_flushing {
            if dst.len() - *dp < 1 {
                return CoderResult::Overflow;
            }
            dst[*dp] = self.context_char;
            *dp += 1;
        }
        self.context_char = INVALID_CHAR;
        self.need_flushing = false;
        CoderResult::Underflow
    }
}

// -------------------------------------------------------- JISAutoDetect

/// `JISAutoDetect.Decoder.SJISName`: `windows-31J` on Windows, `Shift_JIS`
/// elsewhere, as the JDK decides by the operating system it runs on.
fn sjis_machine_item() -> &'static str {
    if cfg!(windows) {
        "M:windows-31j"
    } else {
        "M:Shift_JIS"
    }
}

/// `JISAutoDetect.isPlainASCII(byte)`: an ASCII byte other than `ESC`.
fn is_plain_ascii(b: u8) -> bool {
    b < 0x80 && b != 0x1B
}

/// `JISAutoDetect.looksLikeJapanese(CharBuffer)`: more than one fullwidth
/// Hiragana or more than one halfwidth Katakana unit.
fn looks_like_japanese(units: &[u16]) -> bool {
    let mut hiragana = 0; // Fullwidth Hiragana
    let mut katakana = 0; // Halfwidth Katakana
    for &c in units {
        if (0x3040..=0x309F).contains(&c) {
            hiragana += 1;
            if hiragana > 1 {
                return true;
            }
        }
        if (0xFF65..=0xFF9F).contains(&c) {
            katakana += 1;
            if katakana > 1 {
                return true;
            }
        }
    }
    false
}

/// The state of an `x-JISAutoDetect` decoder: `JISAutoDetect.Decoder`, whose
/// `detectedDecoder` is one of the `ISO-2022-JP`, `EUC-JP` and Shift_JIS
/// machines once the input has told them apart.
#[derive(Debug, Clone, Default)]
pub(crate) struct JisAutoDetectDecoder {
    /// `detectedDecoder`.
    detected: Option<MachineDecoder>,
}

impl JisAutoDetectDecoder {
    /// `JISAutoDetect.Decoder(cs)`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// `decodeLoop(DelegatableDecoder, ByteBuffer, CharBuffer)`: resets
    /// `decoder`, keeps it as the detected decoder and decodes with it.
    fn delegate(
        &mut self,
        mut decoder: MachineDecoder,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        decoder.reset();
        let detected = self.detected.insert(decoder);
        detected.decode_loop(src, sp, dst, dp)
    }

    /// `JISAutoDetect.Decoder.decodeLoop`: copies the leading plain ASCII;
    /// then, until a decoder is detected, decodes the rest of the input with
    /// fresh `ISO-2022-JP`, `EUC-JP` and Shift_JIS decoders into sandboxes
    /// sized from the input's limit and picks one: `ISO-2022-JP` if it
    /// decodes without error, the other one if one of `EUC-JP` and Shift_JIS
    /// fails, the one that got further, nothing (`UNDERFLOW`) if both stop
    /// at the first byte, else `EUC-JP` if its output looks Japanese.
    pub(crate) fn decode_loop(
        &mut self,
        src: &[u8],
        sp: &mut usize,
        dst: &mut [u16],
        dp: &mut usize,
    ) -> CoderResult {
        if let Some(detected) = &mut self.detected {
            return detected.decode_loop(src, sp, dst, dp);
        }

        // `copyLeadingASCII`
        let limit = *sp + (src.len() - *sp).min(dst.len() - *dp);
        while *sp < limit && is_plain_ascii(src[*sp]) {
            dst[*dp] = u16::from(src[*sp]);
            *dp += 1;
            *sp += 1;
        }

        // All ASCII?
        if *sp >= src.len() {
            return CoderResult::Underflow;
        }
        // Overflow only if there is still ascii but no out buffer.
        if *dp >= dst.len() && is_plain_ascii(src[*sp]) {
            return CoderResult::Overflow;
        }

        // `(int)(src.limit() * (double)maxCharsPerByte())`, maxCharsPerByte
        // being 1.0
        let cbufsiz = src.len();
        let mut sandbox = vec![0u16; cbufsiz];

        // First try ISO-2022-JP, since there is no ambiguity
        let dd2022 = MachineDecoder::new(machine("M:ISO-2022-JP"));
        {
            let mut probe = dd2022.clone();
            let (mut p, mut q) = (*sp, 0);
            let res2022 = probe.decode_loop(src, &mut p, &mut sandbox, &mut q);
            if !res2022.is_error() {
                return self.delegate(dd2022, src, sp, dst, dp);
            }
        }

        // We must choose between EUC and SJIS
        let dd_eucj = MachineDecoder::new(machine("M:EUC-JP"));
        let dd_sjis = MachineDecoder::new(machine(sjis_machine_item()));

        let (mut src_eucj, mut sandbox_eucj_len) = (*sp, 0);
        let res_eucj =
            dd_eucj
                .clone()
                .decode_loop(src, &mut src_eucj, &mut sandbox, &mut sandbox_eucj_len);
        // If EUC decoding fails, must be SJIS
        if res_eucj.is_error() {
            return self.delegate(dd_sjis, src, sp, dst, dp);
        }
        let mut sandbox_sjis = vec![0u16; cbufsiz];
        let (mut src_sjis, mut sandbox_sjis_len) = (*sp, 0);
        let res_sjis = dd_sjis.clone().decode_loop(
            src,
            &mut src_sjis,
            &mut sandbox_sjis,
            &mut sandbox_sjis_len,
        );
        // If SJIS decoding fails, must be EUC
        if res_sjis.is_error() {
            return self.delegate(dd_eucj, src, sp, dst, dp);
        }

        // From here on, we have some ambiguity, and must guess.

        // We prefer input that does not appear to end mid-character.
        if src_eucj > src_sjis {
            return self.delegate(dd_eucj, src, sp, dst, dp);
        }

        if src_eucj < src_sjis {
            return self.delegate(dd_sjis, src, sp, dst, dp);
        }

        // end-of-input is after the first byte of the first char?
        if *sp == src_eucj {
            return CoderResult::Underflow;
        }

        // Use heuristic knowledge of typical Japanese text
        if looks_like_japanese(&sandbox[..sandbox_eucj_len]) {
            self.delegate(dd_eucj, src, sp, dst, dp)
        } else {
            self.delegate(dd_sjis, src, sp, dst, dp)
        }
    }

    /// `JISAutoDetect.Decoder.implReset()`: no decoder detected.
    pub(crate) fn reset(&mut self) {
        self.detected = None;
    }
}
