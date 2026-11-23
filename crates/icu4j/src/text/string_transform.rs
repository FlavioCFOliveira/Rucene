// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.StringTransform`.

use crate::text::transform::Transform;

/// A transform from a string to a string.
///
/// Equivalent to the interface `com.ibm.icu.text.StringTransform`, which
/// extends `Transform<String,String>`; its `transform(String)` is the
/// inherited method with `S` and `D` the UTF-16 strings of this crate.
pub trait StringTransform: for<'a> Transform<&'a [u16], Vec<u16>> {}
