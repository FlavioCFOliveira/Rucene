// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.Replaceable`.

use std::fmt::Debug;

/// A string of UTF-16 units that supports replacement of substrings,
/// optionally carrying out-of-band metadata (styles).
///
/// Equivalent to the interface `com.ibm.icu.text.Replaceable`.
///
/// **Divergence.** Java shares one `Replaceable` between its owner and the
/// objects that read it while it changes (`UCharacterIterator.getInstance`
/// keeps the caller's object, and the caller may go on editing it); in Rust
/// such sharing needs interior mutability, so every method takes `&self`
/// and an implementation keeps its text behind a lock, as
/// [`ReplaceableString`](crate::text::replaceable_string::ReplaceableString)
/// does, and the sharers hold an `Arc<dyn Replaceable>`.
///
/// Offsets outside the text panic, where Java's implementations throw the
/// `IndexOutOfBoundsException` of the underlying `StringBuffer` access.
pub trait Replaceable: Debug + Send + Sync {
    /// `length()`: the number of 16-bit code units in the text.
    fn length(&self) -> i32;

    /// `charAt(int offset)`: the 16-bit code unit at `offset`.
    fn char_at(&self, offset: i32) -> u16;

    /// `char32At(int offset)`: the 32-bit code point at `offset`, as
    /// `UTF16.charAt` reads it (an unpaired surrogate is returned as is).
    fn char32_at(&self, offset: i32) -> i32;

    /// `getChars(int srcStart, int srcLimit, char[] dst, int dstStart)`:
    /// copies the units `srcStart..srcLimit` into `dst` at `dstStart`.
    fn get_chars(&self, src_start: i32, src_limit: i32, dst: &mut [u16], dst_start: i32);

    /// `replace(int start, int limit, String text)`: replaces the units
    /// `start..limit` with `text`.
    fn replace(&self, start: i32, limit: i32, text: &[u16]);

    /// `replace(int start, int limit, char[] chars, int charsStart, int
    /// charsLen)`: replaces the units `start..limit` with
    /// `chars[charsStart..charsStart + charsLen]`.
    fn replace_chars(
        &self,
        start: i32,
        limit: i32,
        chars: &[u16],
        chars_start: i32,
        chars_len: i32,
    );

    /// `copy(int start, int limit, int dest)`: copies the units
    /// `start..limit` (with their metadata) to `dest`, which is outside
    /// that range; the text grows by `limit - start`.
    fn copy(&self, start: i32, limit: i32, dest: i32);

    /// `hasMetaData()`: whether the text carries metadata that `copy` must
    /// preserve.
    fn has_meta_data(&self) -> bool;
}
