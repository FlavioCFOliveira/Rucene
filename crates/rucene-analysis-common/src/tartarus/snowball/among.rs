/*
Copyright (c) 2001, Dr Martin Porter
Copyright (c) 2004,2005, Richard Boulton
Copyright (c) 2013, Yoshiki Shibukawa
Copyright (c) 2006,2007,2009,2010,2011,2014-2019, Olly Betts
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

  1. Redistributions of source code must retain the above copyright notice,
     this list of conditions and the following disclaimer.
  2. Redistributions in binary form must reproduce the above copyright notice,
     this list of conditions and the following disclaimer in the documentation
     and/or other materials provided with the distribution.
  3. Neither the name of the Snowball project nor the names of its contributors
     may be used to endorse or promote products derived from this software
     without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
*/

//! Internal class used by Snowball stemmers, ported from
//! `org.tartarus.snowball.Among`.

use std::fmt;

/// The routine an [`Among`] entry runs when its string matches: a method of
/// the program `P` taking no argument and returning `boolean`.
pub type AmongMethod<P> = fn(&mut P) -> bool;

/// Internal class used by Snowball stemmers: one entry of a generated
/// `among` table, the string it searches for, the index of its longest
/// matching substring entry, the result of the lookup and, optionally, the
/// routine that must also succeed.
///
/// Equivalent to `org.tartarus.snowball.Among`.
///
/// `P` is the program class whose routines the entry names; an entry
/// without a routine may use any `P`, [`SnowballProgram`] by default.
///
/// **Divergence from Lucene 10.5.0.** Java's five-argument constructor finds
/// the routine by name through `MethodHandles.Lookup.findVirtual` and holds
/// a `MethodHandle` that `SnowballProgram.find_among` calls with
/// `invokeExact`; a routine the class lacks fails at class initialisation
/// with `RuntimeException("Snowball program '<class>' is broken, cannot
/// access method: boolean <name>()")`. Rust has no reflection, so the entry
/// holds the routine itself, an [`AmongMethod`] function pointer that
/// `find_among` calls directly: the routine run, and the result, are the
/// same; a routine the class lacks is a compile error instead of that
/// `RuntimeException`. The search string is the UTF-16 units Java's
/// `s.toCharArray()` holds, given as a `&'static [u16]` (the generated
/// tables are `static`, as Java's `static final` arrays are).
///
/// [`SnowballProgram`]: super::snowball_program::SnowballProgram
pub struct Among<P = super::snowball_program::SnowballProgram> {
    /// Search string (`final char[] s`).
    pub(super) s: &'static [u16],
    /// Index to longest matching substring (`final int substring_i`).
    pub(super) substring_i: i32,
    /// Result of the lookup (`final int result`).
    pub(super) result: i32,
    /// Method to use if substring matches (`final MethodHandle method`).
    pub(super) method: Option<AmongMethod<P>>,
}

impl<P> Among<P> {
    /// Equivalent to `Among(String, int, int)`: an entry without a routine.
    pub const fn new(s: &'static [u16], substring_i: i32, result: i32) -> Self {
        Self {
            s,
            substring_i,
            result,
            method: None,
        }
    }

    /// Equivalent to `Among(String, int, int, String, MethodHandles.Lookup)`
    /// with a non-empty method name: an entry whose match also requires
    /// `method` to return `true`. Java's empty method name, which leaves the
    /// entry without a routine, is [`Among::new`].
    pub const fn with_method(
        s: &'static [u16],
        substring_i: i32,
        result: i32,
        method: AmongMethod<P>,
    ) -> Self {
        Self {
            s,
            substring_i,
            result,
            method: Some(method),
        }
    }
}

impl<P> fmt::Debug for Among<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Among")
            .field("s", &String::from_utf16_lossy(self.s))
            .field("substring_i", &self.substring_i)
            .field("result", &self.result)
            .field("method", &self.method.is_some())
            .finish()
    }
}
