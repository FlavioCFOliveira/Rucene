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

//! Parent class of all snowball stemmers, ported from
//! `org.tartarus.snowball.SnowballStemmer`.

use std::fmt;

use super::snowball_program::SnowballProgram;

/// Parent class of all snowball stemmers, which must implement `stem`.
///
/// Equivalent to `org.tartarus.snowball.SnowballStemmer`, the abstract
/// class `extends SnowballProgram`: a stemmer is a type that holds a
/// [`SnowballProgram`] (reached through [`AsRef`]/[`AsMut`], where Java
/// inherits its members) and implements [`stem`](Self::stem). It is
/// `Send + Sync`, as the filters holding one must be, and `Debug`.
///
/// `SnowballStemmer`'s `serialVersionUID` is not ported (see
/// [`SnowballProgram`]'s divergences).
pub trait SnowballStemmer:
    AsRef<SnowballProgram> + AsMut<SnowballProgram> + Send + Sync + fmt::Debug
{
    /// Stems the current string of the program.
    ///
    /// Equivalent to `public abstract boolean stem()`.
    fn stem(&mut self) -> bool;
}
