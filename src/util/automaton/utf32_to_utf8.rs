//! Port of `org.apache.lucene.util.automaton.UTF32ToUTF8`.

use super::automaton::{Automaton, AutomatonBuilder, Transition, TransitionAccessor};

/// Unicode boundaries for UTF-8 bytes 1, 2, 3, 4.
const START_CODES: [i32; 4] = [0, 128, 2048, 65536];
const END_CODES: [i32; 4] = [127, 2047, 65535, 1114111];

/// `MASKS[n]` has the low `n` bits set; `MASKS[0]` is zero.
const MASKS: [i32; 8] = [0, 1, 3, 7, 15, 31, 63, 127];

/// Holds a single code point as a sequence of 1 to 4 UTF-8 bytes.
///
/// Equivalent to the private `UTF32ToUTF8.UTF8Sequence`; `value` is the byte value
/// and `bits` is how many bits are "used" by UTF-8 at that byte.
#[derive(Clone, Copy, Debug, Default)]
struct UTF8Sequence {
    values: [i32; 4],
    bits: [i32; 4],
    len: usize,
}

impl UTF8Sequence {
    fn byte_at(&self, idx: usize) -> i32 {
        self.values[idx] & 0xFF
    }

    fn num_bits(&self, idx: usize) -> i32 {
        self.bits[idx]
    }

    fn set(&mut self, code: i32) {
        if code < 128 {
            // 0xxxxxxx
            self.values[0] = code;
            self.bits[0] = 7;
            self.len = 1;
        } else if code < 2048 {
            // 110yyyxx 10xxxxxx
            self.values[0] = (6 << 5) | (code >> 6);
            self.bits[0] = 5;
            self.set_rest(code, 1);
            self.len = 2;
        } else if code < 65536 {
            // 1110yyyy 10yyyyxx 10xxxxxx
            self.values[0] = (14 << 4) | (code >> 12);
            self.bits[0] = 4;
            self.set_rest(code, 2);
            self.len = 3;
        } else {
            // 11110zzz 10zzyyyy 10yyyyxx 10xxxxxx
            self.values[0] = (30 << 3) | (code >> 18);
            self.bits[0] = 3;
            self.set_rest(code, 3);
            self.len = 4;
        }
    }

    /// Only sets the first byte value, for the temporary sequences.
    fn set_first_byte(&mut self, code: i32) {
        if code < 128 {
            // 0xxxxxxx
            self.values[0] = code;
            self.len = 1;
        } else if code < 2048 {
            // 110yyyxx 10xxxxxx
            self.values[0] = (6 << 5) | (code >> 6);
            self.len = 2;
        } else if code < 65536 {
            // 1110yyyy 10yyyyxx 10xxxxxx
            self.values[0] = (14 << 4) | (code >> 12);
            self.len = 3;
        } else {
            // 11110zzz 10zzyyyy 10yyyyxx 10xxxxxx
            self.values[0] = (30 << 3) | (code >> 18);
            self.len = 4;
        }
    }

    fn set_rest(&mut self, code: i32, num_bytes: usize) {
        let mut code = code;
        for i in 0..num_bytes {
            self.values[num_bytes - i] = 128 | (code & MASKS[6]);
            self.bits[num_bytes - i] = 6;
            code >>= 6;
        }
    }
}

/// Converts UTF-32 automata to the equivalent UTF-8 representation.
///
/// Equivalent to `org.apache.lucene.util.automaton.UTF32ToUTF8`. The byte ranges
/// this produces decide which terms a query matches against the on-disk terms
/// dictionary, so the construction is reproduced exactly.
pub struct UTF32ToUTF8 {
    start_utf8: UTF8Sequence,
    end_utf8: UTF8Sequence,
    tmp_utf8a: UTF8Sequence,
    tmp_utf8b: UTF8Sequence,
    utf8: AutomatonBuilder,
}

impl Default for UTF32ToUTF8 {
    fn default() -> Self {
        Self::new()
    }
}

impl UTF32ToUTF8 {
    /// Sole constructor.
    pub fn new() -> Self {
        Self {
            start_utf8: UTF8Sequence::default(),
            end_utf8: UTF8Sequence::default(),
            tmp_utf8a: UTF8Sequence::default(),
            tmp_utf8b: UTF8Sequence::default(),
            utf8: AutomatonBuilder::new(),
        }
    }

    /// Builds the necessary UTF-8 edges between `start` and `end`.
    fn convert_one_edge(
        &mut self,
        start: i32,
        end: i32,
        start_code_point: i32,
        end_code_point: i32,
    ) {
        let mut start_utf8 = self.start_utf8;
        let mut end_utf8 = self.end_utf8;
        start_utf8.set(start_code_point);
        end_utf8.set(end_code_point);
        self.start_utf8 = start_utf8;
        self.end_utf8 = end_utf8;
        self.build(start, end, &start_utf8, &end_utf8, 0);
    }

    fn build(
        &mut self,
        start: i32,
        end: i32,
        start_utf8: &UTF8Sequence,
        end_utf8: &UTF8Sequence,
        upto: usize,
    ) {
        // Break into start, middle, end:
        if start_utf8.byte_at(upto) == end_utf8.byte_at(upto) {
            // Degen case: lead with the same byte:
            if upto == start_utf8.len - 1 && upto == end_utf8.len - 1 {
                // Super degen: just a single edge, one UTF-8 byte:
                self.utf8.add_transition_range(
                    start,
                    end,
                    start_utf8.byte_at(upto),
                    end_utf8.byte_at(upto),
                );
            } else {
                debug_assert!(start_utf8.len > upto + 1);
                debug_assert!(end_utf8.len > upto + 1);
                let n = self.utf8.create_state();

                // Single value leading edge
                self.utf8.add_transition(start, n, start_utf8.byte_at(upto));

                // Recurse for the rest
                self.build(n, end, start_utf8, end_utf8, 1 + upto);
            }
        } else if start_utf8.len == end_utf8.len {
            if upto == start_utf8.len - 1 {
                self.utf8.add_transition_range(
                    start,
                    end,
                    start_utf8.byte_at(upto),
                    end_utf8.byte_at(upto),
                );
            } else {
                self.start(start, end, start_utf8, upto, false);
                if end_utf8.byte_at(upto) - start_utf8.byte_at(upto) > 1 {
                    // There is a middle
                    self.all(
                        start,
                        end,
                        start_utf8.byte_at(upto) + 1,
                        end_utf8.byte_at(upto) - 1,
                        start_utf8.len - upto - 1,
                    );
                }
                self.end(start, end, end_utf8, upto, false);
            }
        } else {
            // start
            self.start(start, end, start_utf8, upto, true);

            // possibly middle, spanning multiple num bytes
            let mut byte_count = 1 + start_utf8.len - upto;
            let limit = end_utf8.len - upto;
            while byte_count < limit {
                let mut a = self.tmp_utf8a;
                let mut b = self.tmp_utf8b;
                a.set_first_byte(START_CODES[byte_count - 1]);
                b.set_first_byte(END_CODES[byte_count - 1]);
                self.tmp_utf8a = a;
                self.tmp_utf8b = b;
                self.all(start, end, a.byte_at(0), b.byte_at(0), a.len - 1);
                byte_count += 1;
            }

            // end
            self.end(start, end, end_utf8, upto, true);
        }
    }

    fn start(
        &mut self,
        start: i32,
        end: i32,
        start_utf8: &UTF8Sequence,
        upto: usize,
        do_all: bool,
    ) {
        if upto == start_utf8.len - 1 {
            // Done recursing
            self.utf8.add_transition_range(
                start,
                end,
                start_utf8.byte_at(upto),
                start_utf8.byte_at(upto) | MASKS[start_utf8.num_bits(upto) as usize],
            ); // type=start
        } else {
            let n = self.utf8.create_state();
            self.utf8.add_transition(start, n, start_utf8.byte_at(upto));
            self.start(n, end, start_utf8, 1 + upto, true);
            let end_code = start_utf8.byte_at(upto) | MASKS[start_utf8.num_bits(upto) as usize];
            if do_all && start_utf8.byte_at(upto) != end_code {
                self.all(
                    start,
                    end,
                    start_utf8.byte_at(upto) + 1,
                    end_code,
                    start_utf8.len - upto - 1,
                );
            }
        }
    }

    fn end(&mut self, start: i32, end: i32, end_utf8: &UTF8Sequence, upto: usize, do_all: bool) {
        if upto == end_utf8.len - 1 {
            // Done recursing
            self.utf8.add_transition_range(
                start,
                end,
                end_utf8.byte_at(upto) & !MASKS[end_utf8.num_bits(upto) as usize],
                end_utf8.byte_at(upto),
            );
        } else {
            // GH-ISSUE#12472: UTF-8 special case for the different start byte of the
            // different length=2,3,4
            let start_code = if end_utf8.len == 2 {
                debug_assert!(upto == 0); // the upto==1 case is handled by the first if above
                                          // the first length=2 UTF8 Unicode character is C2 80,
                                          // so we must special case 0xC2 as the 1st byte.
                0xC2
            } else if end_utf8.len == 3 && upto == 1 && end_utf8.byte_at(0) == 0xE0 {
                // the first length=3 UTF8 Unicode character is E0 A0 80,
                // so we must special case 0xA0 as the 2nd byte when E0 was the first
                // byte of endUTF8.
                0xA0
            } else if end_utf8.len == 4 && upto == 1 && end_utf8.byte_at(0) == 0xF0 {
                // the first length=4 UTF8 Unicode character is F0 90 80 80,
                // so we must special case 0x90 as the 2nd byte when F0 was the first
                // byte of endUTF8.
                0x90
            } else {
                end_utf8.byte_at(upto) & !MASKS[end_utf8.num_bits(upto) as usize]
            };
            if do_all && end_utf8.byte_at(upto) != start_code {
                self.all(
                    start,
                    end,
                    start_code,
                    end_utf8.byte_at(upto) - 1,
                    end_utf8.len - upto - 1,
                );
            }
            let n = self.utf8.create_state();
            self.utf8.add_transition(start, n, end_utf8.byte_at(upto));
            self.end(n, end, end_utf8, 1 + upto, true);
        }
    }

    fn all(&mut self, start: i32, end: i32, start_code: i32, end_code: i32, left: usize) {
        if left == 0 {
            self.utf8
                .add_transition_range(start, end, start_code, end_code);
        } else {
            let mut left = left;
            let mut last_n = self.utf8.create_state();
            self.utf8
                .add_transition_range(start, last_n, start_code, end_code);
            while left > 1 {
                let n = self.utf8.create_state();
                self.utf8.add_transition_range(last_n, n, 128, 191); // type=all*
                left -= 1;
                last_n = n;
            }
            self.utf8.add_transition_range(last_n, end, 128, 191); // type=all*
        }
    }

    /// Converts an incoming UTF-32 automaton to an equivalent UTF-8 one.
    ///
    /// The incoming automaton need not be deterministic. Note that the returned
    /// automaton will not in general be deterministic, so you must determinize it if
    /// that is needed.
    pub fn convert(&mut self, utf32: &Automaton) -> Automaton {
        if utf32.get_num_states() == 0 {
            return utf32.clone();
        }

        let mut map = vec![-1i32; utf32.get_num_states() as usize];

        let mut pending: Vec<i32> = Vec::new();
        let mut utf32_state = 0i32;
        pending.push(utf32_state);
        self.utf8 = AutomatonBuilder::new();

        let mut utf8_state = self.utf8.create_state();

        self.utf8
            .set_accept(utf8_state, utf32.is_accept(utf32_state));

        map[utf32_state as usize] = utf8_state;

        let mut scratch = Transition::new();

        while let Some(next) = pending.pop() {
            utf32_state = next;
            utf8_state = map[utf32_state as usize];
            debug_assert!(utf8_state != -1);

            let num_transitions = utf32.get_num_transitions(utf32_state);
            utf32.init_transition(utf32_state, &mut scratch);
            for _ in 0..num_transitions {
                utf32.get_next_transition(&mut scratch);
                let dest_utf32 = scratch.dest;
                let mut dest_utf8 = map[dest_utf32 as usize];
                if dest_utf8 == -1 {
                    dest_utf8 = self.utf8.create_state();
                    self.utf8.set_accept(dest_utf8, utf32.is_accept(dest_utf32));
                    map[dest_utf32 as usize] = dest_utf8;
                    pending.push(dest_utf32);
                }

                // Writes new transitions into the builder:
                self.convert_one_edge(utf8_state, dest_utf8, scratch.min, scratch.max);
            }
        }

        self.utf8.finish()
    }
}
