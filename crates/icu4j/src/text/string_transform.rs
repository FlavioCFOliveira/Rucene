// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.StringTransform`.

use crate::text::transform::Transform;

/// A transform from a string to a string.
///
/// Equivalent to the interface `com.ibm.icu.text.StringTransform`, which
/// extends `Transform<String,String>`; its `transform(String)` is the
/// inherited method with `S` and `D` the UTF-16 strings of this crate.
///
/// Equivalent to `StringTransform.transform(String)`: Java re-declares the
/// inherited `Transform.transform(S)` with `S` and `D` bound to `String`;
/// here it is [`Transform::transform`] of the supertrait, bound to
/// `&[u16]` and `Vec<u16>`.
pub trait StringTransform: for<'a> Transform<&'a [u16], Vec<u16>> {}
