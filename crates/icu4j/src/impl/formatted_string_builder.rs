// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.FormattedStringBuilder`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{same_attribute, FieldRef};
use crate::jdk::{index_out_of_bounds, to_rust_string};
use crate::jdk::text::same_field;
use crate::text::number_format::field as nf_field;

/// An object that wraps a field: `unwrap` gives the field the wrapped
/// position reports. Equivalent to `FormattedStringBuilder.FieldWrapper`;
/// [`as_any`](Self::as_any) answers Java's casts to the implementing class.
pub trait FieldWrapper: fmt::Debug + Send + Sync + 'static {
    /// Equivalent to `unwrap()`.
    fn unwrap(&self) -> Option<FieldRef>;

    /// This object as `Any`, for Java's casts.
    fn as_any(&self) -> &dyn Any;
}

/// The `Object` a `FormattedStringBuilder` stores per code unit (Java
/// `null` is `None` of an `Option<FieldObject>`): a `java.text.Format.Field`
/// or a [`FieldWrapper`].
///
/// Java compares these objects by identity (`==`): two fields are equal when
/// they are the same `Format.Field` constant, two wrappers when they are the
/// same object.
#[derive(Debug, Clone)]
pub enum FieldObject {
    /// A `java.text.Format.Field`.
    Field(FieldRef),
    /// A `FormattedStringBuilder.FieldWrapper`.
    Wrapper(Arc<dyn FieldWrapper>),
}

impl PartialEq for FieldObject {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (FieldObject::Field(a), FieldObject::Field(b)) => same_attribute(*a, *b),
            (FieldObject::Wrapper(a), FieldObject::Wrapper(b)) => {
                std::ptr::eq(Arc::as_ptr(a) as *const (), Arc::as_ptr(b) as *const ())
            }
            _ => false,
        }
    }
}

impl From<FieldRef> for FieldObject {
    fn from(field: FieldRef) -> Self {
        FieldObject::Field(field)
    }
}

impl FieldObject {
    /// Whether this object is the field `field` (Java `object == field`).
    pub fn is_field(&self, field: FieldRef) -> bool {
        matches!(self, FieldObject::Field(f) if same_attribute(*f, field))
    }
}

/// Whether the nullable field object `a` is the field `field` (Java
/// `a == field`).
pub fn is_field(a: Option<&FieldObject>, field: FieldRef) -> bool {
    matches!(a, Some(o) if o.is_field(field))
}

/// A string builder that keeps a field per code unit, with an efficient
/// prepend as well as append.
///
/// Equivalent to `com.ibm.icu.impl.FormattedStringBuilder`. Its
/// `CharSequence` text is UTF-16; the `Appendable` methods are
/// [`append_csq`](Self::append_csq), [`append_csq_range`](Self::append_csq_range)
/// and [`append_char`](Self::append_char) (appending cannot fail, so they
/// return nothing). `hashCode()` and `equals(Object)` throw
/// `UnsupportedOperationException("Don't call #hashCode() or #equals() on a
/// mutable.")` in Java; this type implements neither `Hash` nor `PartialEq`,
/// which keeps those calls out of the port.
#[derive(Debug, Clone)]
pub struct FormattedStringBuilder {
    /// `chars`.
    pub(crate) chars: Vec<u16>,
    /// `fields`.
    pub(crate) fields: Vec<Option<FieldObject>>,
    /// `zero`.
    pub(crate) zero: i32,
    /// `length`.
    pub(crate) length: i32,
    /// `appendOffset`: number of characters from the end where .append()
    /// operations insert.
    pub(crate) append_offset: i32,
    /// `appendableField`: the field used in the `Appendable` methods.
    pub(crate) appendable_field: Option<FieldObject>,
}

/// `fieldToDebugChar`.
static FIELD_TO_DEBUG_CHAR: LazyLock<Vec<(FieldRef, char)>> = LazyLock::new(|| {
    vec![
        (&nf_field::SIGN as FieldRef, '-'),
        (&nf_field::INTEGER as FieldRef, 'i'),
        (&nf_field::FRACTION as FieldRef, 'f'),
        (&nf_field::EXPONENT as FieldRef, 'e'),
        (&nf_field::EXPONENT_SIGN as FieldRef, '+'),
        (&nf_field::EXPONENT_SYMBOL as FieldRef, 'E'),
        (&nf_field::DECIMAL_SEPARATOR as FieldRef, '.'),
        (&nf_field::GROUPING_SEPARATOR as FieldRef, ','),
        (&nf_field::PERCENT as FieldRef, '%'),
        (&nf_field::PERMILLE as FieldRef, '\u{2030}'),
        (&nf_field::CURRENCY as FieldRef, '$'),
        (&nf_field::MEASURE_UNIT as FieldRef, 'u'),
        (&nf_field::COMPACT as FieldRef, 'C'),
    ]
});

/// `EMPTY`: a constant, empty `FormattedStringBuilder`. Do NOT call mutative
/// operations on this.
pub static EMPTY: LazyLock<FormattedStringBuilder> = LazyLock::new(FormattedStringBuilder::new);

impl Default for FormattedStringBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FormattedStringBuilder {
    /// The field a field object stands for: the field itself, or what a
    /// wrapper unwraps to. Equivalent to `unwrapField(Object)`.
    pub fn unwrap_field(field: Option<&FieldObject>) -> Option<FieldRef> {
        match field {
            None => None,
            Some(FieldObject::Wrapper(w)) => w.unwrap(),
            Some(FieldObject::Field(f)) => Some(*f),
        }
    }

    /// Equivalent to `FormattedStringBuilder()`.
    pub fn new() -> Self {
        Self::with_capacity(40)
    }

    /// Equivalent to `FormattedStringBuilder(int)`.
    pub fn with_capacity(capacity: i32) -> Self {
        Self {
            chars: vec![0; capacity as usize],
            fields: vec![None; capacity as usize],
            zero: capacity / 2,
            length: 0,
            append_offset: 0,
            appendable_field: None,
        }
    }

    /// Equivalent to `FormattedStringBuilder(FormattedStringBuilder)`: the
    /// characters, fields, zero and length of `source` (the append offset and
    /// the appendable field keep their initial values).
    pub fn from_other(source: &FormattedStringBuilder) -> Self {
        let mut b = Self {
            chars: Vec::new(),
            fields: Vec::new(),
            zero: 0,
            length: 0,
            append_offset: 0,
            appendable_field: None,
        };
        b.copy_from(source);
        b
    }

    /// Equivalent to `copyFrom(FormattedStringBuilder)`.
    pub fn copy_from(&mut self, source: &FormattedStringBuilder) {
        self.chars = source.chars.clone();
        self.fields = source.fields.clone();
        self.zero = source.zero;
        self.length = source.length;
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> i32 {
        self.length
    }

    /// Equivalent to `codePointCount()`.
    pub fn code_point_count(&self) -> i32 {
        let s = self.zero as usize;
        let text = &self.chars[s..s + self.length as usize];
        let mut n = 0;
        let mut i = 0;
        while i < text.len() {
            if is_high(text[i]) && i + 1 < text.len() && is_low(text[i + 1]) {
                i += 1;
            }
            n += 1;
            i += 1;
        }
        n
    }

    /// Equivalent to `charAt(int)`.
    pub fn char_at(&self, index: i32) -> u16 {
        debug_assert!(index >= 0);
        debug_assert!(index < self.length);
        self.chars[(self.zero + index) as usize]
    }

    /// Equivalent to `fieldAt(int)`.
    pub fn field_at(&self, index: i32) -> Option<&FieldObject> {
        debug_assert!(index >= 0);
        debug_assert!(index < self.length);
        self.fields[(self.zero + index) as usize].as_ref()
    }

    /// Equivalent to `getFirstCodePoint()`.
    pub fn get_first_code_point(&self) -> i32 {
        if self.length == 0 {
            return -1;
        }
        code_point_at_limit(&self.chars, self.zero, self.zero + self.length)
    }

    /// Equivalent to `getLastCodePoint()`.
    pub fn get_last_code_point(&self) -> i32 {
        if self.length == 0 {
            return -1;
        }
        code_point_before_start(&self.chars, self.zero + self.length, self.zero)
    }

    /// Equivalent to `codePointAt(int)`.
    pub fn code_point_at(&self, index: i32) -> i32 {
        code_point_at_limit(&self.chars, self.zero + index, self.zero + self.length)
    }

    /// Equivalent to `codePointBefore(int)`.
    pub fn code_point_before(&self, index: i32) -> i32 {
        code_point_before_start(&self.chars, self.zero + index, self.zero)
    }

    /// Equivalent to `clear()`.
    pub fn clear(&mut self) -> &mut Self {
        self.zero = self.get_capacity() / 2;
        self.length = 0;
        self
    }

    /// Sets the index at which append operations insert. Defaults to the end.
    /// Equivalent to `setAppendIndex(int)`.
    pub fn set_append_index(&mut self, index: i32) {
        self.append_offset = self.length - index;
    }

    /// Equivalent to `appendChar16(char, Object)`.
    pub fn append_char16(&mut self, code_unit: u16, field: Option<FieldObject>) -> i32 {
        self.insert_char16(self.length - self.append_offset, code_unit, field)
    }

    /// Equivalent to `insertChar16(int, char, Object)`.
    pub fn insert_char16(&mut self, index: i32, code_unit: u16, field: Option<FieldObject>) -> i32 {
        let count = 1;
        let position = self.prepare_for_insert(index, count) as usize;
        self.chars[position] = code_unit;
        self.fields[position] = field;
        count
    }

    /// Appends the specified codePoint to the end of the string. Equivalent
    /// to `appendCodePoint(int, Object)`.
    pub fn append_code_point(&mut self, code_point: i32, field: Option<FieldObject>) -> i32 {
        self.insert_code_point(self.length - self.append_offset, code_point, field)
    }

    /// Inserts the specified codePoint at the specified index in the string.
    /// Equivalent to `insertCodePoint(int, int, Object)`.
    ///
    /// # Panics
    ///
    /// With `Character.toChars`'s `IllegalArgumentException` text for an
    /// invalid code point (an unchecked exception ICU never catches).
    pub fn insert_code_point(&mut self, index: i32, code_point: i32, field: Option<FieldObject>) -> i32 {
        let count = if code_point >= 0x10000 { 2 } else { 1 };
        if !(0..=0x10ffff).contains(&code_point) {
            panic!("java.lang.IllegalArgumentException: Not a valid Unicode code point: 0x{:X}", code_point);
        }
        let position = self.prepare_for_insert(index, count) as usize;
        if count == 2 {
            let c = code_point - 0x10000;
            self.chars[position] = (0xd800 + (c >> 10)) as u16;
            self.chars[position + 1] = (0xdc00 + (c & 0x3ff)) as u16;
            self.fields[position] = field.clone();
            self.fields[position + 1] = field;
        } else {
            self.chars[position] = code_point as u16;
            self.fields[position] = field;
        }
        count
    }

    /// Appends the specified CharSequence to the end of the string.
    /// Equivalent to `append(CharSequence, Object)`.
    pub fn append(&mut self, sequence: &[u16], field: Option<FieldObject>) -> i32 {
        self.insert(self.length - self.append_offset, sequence, field)
    }

    /// Inserts the specified CharSequence at the specified index in the
    /// string. Equivalent to `insert(int, CharSequence, Object)`.
    pub fn insert(&mut self, index: i32, sequence: &[u16], field: Option<FieldObject>) -> i32 {
        if sequence.is_empty() {
            // Nothing to insert.
            0
        } else if sequence.len() == 1 {
            // Fast path: on a single-char string, using insertCodePoint below
            // is 70% faster than the CharSequence method: 12.2 ns versus 41.9
            // ns for five operations on my Linux x86-64 workstation.
            self.insert_code_point(index, sequence[0] as i32, field)
        } else {
            self.insert_range(index, sequence, 0, sequence.len() as i32, field)
        }
    }

    /// Inserts the specified CharSequence at the specified index in the
    /// string, reading from the CharSequence from start (inclusive) to end
    /// (exclusive). Equivalent to `insert(int, CharSequence, int, int,
    /// Object)`.
    pub fn insert_range(&mut self, index: i32, sequence: &[u16], start: i32, end: i32, field: Option<FieldObject>) -> i32 {
        let count = end - start;
        let position = self.prepare_for_insert(index, count);
        for i in 0..count {
            self.chars[(position + i) as usize] = sequence[(start + i) as usize];
            self.fields[(position + i) as usize] = field.clone();
        }
        count
    }

    /// Replaces the chars between startThis and endThis with the chars
    /// between startOther and endOther of the given CharSequence. Calling
    /// this method with startThis == endThis is equivalent to calling insert.
    /// Equivalent to `splice(int, int, CharSequence, int, int, Object)`.
    pub fn splice(
        &mut self,
        start_this: i32,
        end_this: i32,
        sequence: &[u16],
        start_other: i32,
        end_other: i32,
        field: Option<FieldObject>,
    ) -> i32 {
        let this_length = end_this - start_this;
        let other_length = end_other - start_other;
        let count = other_length - this_length;
        let position = if count > 0 {
            // Overall, chars need to be added.
            self.prepare_for_insert(start_this, count)
        } else {
            // Overall, chars need to be removed or kept the same.
            self.remove(start_this, -count)
        };
        for i in 0..other_length {
            self.chars[(position + i) as usize] = sequence[(start_other + i) as usize];
            self.fields[(position + i) as usize] = field.clone();
        }
        count
    }

    /// Appends the chars in the specified char array to the end of the
    /// string, and associates them with the fields in the specified field
    /// array, which must have the same length as chars. Equivalent to
    /// `append(char[], Object[])`.
    pub fn append_chars(&mut self, chars: &[u16], fields: Option<&[Option<FieldObject>]>) -> i32 {
        self.insert_chars(self.length - self.append_offset, chars, fields)
    }

    /// Inserts the chars in the specified char array at the specified index
    /// in the string, and associates them with the fields in the specified
    /// field array, which must have the same length as chars. Equivalent to
    /// `insert(int, char[], Object[])`.
    pub fn insert_chars(&mut self, index: i32, chars: &[u16], fields: Option<&[Option<FieldObject>]>) -> i32 {
        debug_assert!(fields.is_none() || chars.len() == fields.map_or(0, |f| f.len()));
        let count = chars.len() as i32;
        if count == 0 {
            return 0; // nothing to insert
        }
        let position = self.prepare_for_insert(index, count);
        for i in 0..count {
            self.chars[(position + i) as usize] = chars[i as usize];
            self.fields[(position + i) as usize] = match fields {
                None => None,
                Some(f) => f[i as usize].clone(),
            };
        }
        count
    }

    /// Appends the contents of another `FormattedStringBuilder` to the end
    /// of this instance. Equivalent to `append(FormattedStringBuilder)`.
    pub fn append_builder(&mut self, other: &FormattedStringBuilder) -> i32 {
        self.insert_builder(self.length - self.append_offset, other)
    }

    /// Inserts the contents of another `FormattedStringBuilder` into this
    /// instance at the given index. Equivalent to `insert(int,
    /// FormattedStringBuilder)`; Java throws
    /// `IllegalArgumentException("Cannot call insert/append on myself")`
    /// when `other` is this instance, a call Rust's borrow rules reject at
    /// compile time.
    pub fn insert_builder(&mut self, index: i32, other: &FormattedStringBuilder) -> i32 {
        let count = other.length;
        if count == 0 {
            // Nothing to insert.
            return 0;
        }
        let position = self.prepare_for_insert(index, count);
        for i in 0..count {
            self.chars[(position + i) as usize] = other.char_at(i);
            self.fields[(position + i) as usize] = other.field_at(i).cloned();
        }
        count
    }

    /// Shifts and sets up an insertion; answers the position in the buffers
    /// at which to insert. Equivalent to `prepareForInsert(int, int)`.
    fn prepare_for_insert(&mut self, mut index: i32, count: i32) -> i32 {
        if index == -1 {
            index = self.length;
        }
        if index == 0 && self.zero - count >= 0 {
            // Append to start
            self.zero -= count;
            self.length += count;
            self.zero
        } else if index == self.length && self.zero + self.length + count < self.get_capacity() {
            // Append to end
            self.length += count;
            self.zero + self.length - count
        } else {
            // Move chars around and/or allocate more space
            self.prepare_for_insert_helper(index, count)
        }
    }

    /// `prepareForInsertHelper(int, int)`.
    fn prepare_for_insert_helper(&mut self, index: i32, count: i32) -> i32 {
        // Keeping this code out of prepareForInsert() increases the speed of
        // append operations.
        let old_capacity = self.get_capacity();
        let old_zero = self.zero;
        let length = self.length;
        if length + count > old_capacity {
            let new_capacity = (length + count) * 2;
            let new_zero = new_capacity / 2 - (length + count) / 2;

            // C++ note: memcpy is required because src and dest might be the
            // same.
            let mut new_chars = vec![0u16; new_capacity as usize];
            let mut new_fields: Vec<Option<FieldObject>> = vec![None; new_capacity as usize];

            arraycopy(&self.chars, old_zero, &mut new_chars, new_zero, index);
            arraycopy(&self.chars, old_zero + index, &mut new_chars, new_zero + index + count, length - index);
            arraycopy(&self.fields, old_zero, &mut new_fields, new_zero, index);
            arraycopy(&self.fields, old_zero + index, &mut new_fields, new_zero + index + count, length - index);

            self.chars = new_chars;
            self.fields = new_fields;
            self.zero = new_zero;
            self.length += count;
        } else {
            let new_zero = old_capacity / 2 - (length + count) / 2;

            // C++ note: memmove is required because src and dest might be the
            // same.
            arraycopy_within(&mut self.chars, old_zero, new_zero, length);
            arraycopy_within(&mut self.chars, new_zero + index, new_zero + index + count, length - index);
            arraycopy_within(&mut self.fields, old_zero, new_zero, length);
            arraycopy_within(&mut self.fields, new_zero + index, new_zero + index + count, length - index);

            self.zero = new_zero;
            self.length += count;
        }
        self.zero + index
    }

    /// Removes `count` chars at `index`; answers the position at which the
    /// chars were removed. Equivalent to `remove(int, int)`.
    fn remove(&mut self, index: i32, count: i32) -> i32 {
        let position = index + self.zero;
        let n = self.length - index - count;
        arraycopy_within(&mut self.chars, position + count, position, n);
        arraycopy_within(&mut self.fields, position + count, position, n);
        self.length -= count;
        position
    }

    /// `getCapacity()`.
    fn get_capacity(&self) -> i32 {
        self.chars.len() as i32
    }

    /// Equivalent to `subSequence(int, int)`: a copy sharing nothing, whose
    /// window is `[start, end)`.
    pub fn sub_sequence(&self, start: i32, end: i32) -> FormattedStringBuilder {
        debug_assert!(start >= 0);
        debug_assert!(end <= self.length);
        debug_assert!(end >= start);
        let mut other = FormattedStringBuilder::from_other(self);
        other.zero = self.zero + start;
        other.length = end - start;
        other
    }

    /// Use this instead of subSequence if returning a String; the
    /// subSequence method returns a `FormattedStringBuilder`. Equivalent to
    /// `subString(int, int)`.
    ///
    /// # Panics
    ///
    /// `IndexOutOfBoundsException` when the range is invalid.
    pub fn sub_string(&self, start: i32, end: i32) -> Vec<u16> {
        if start < 0 || end > self.length || end < start {
            index_out_of_bounds(None);
        }
        self.chars[(start + self.zero) as usize..(end + self.zero) as usize].to_vec()
    }

    /// The string of this builder (its UTF-16 text). Equivalent to
    /// `toString()`.
    pub fn to_java_string(&self) -> Vec<u16> {
        self.chars[self.zero as usize..(self.zero + self.length) as usize].to_vec()
    }

    /// The text of this builder and its fields, one character per field
    /// (`n` for no field, `?` for an unknown one). Equivalent to
    /// `toDebugString()`.
    pub fn to_debug_string(&self) -> String {
        let mut sb = String::new();
        sb.push_str("<FormattedStringBuilder [");
        sb.push_str(&to_rust_string(&self.to_java_string()));
        sb.push_str("] [");
        for i in self.zero..self.zero + self.length {
            match &self.fields[i as usize] {
                None => sb.push('n'),
                Some(field) => {
                    let found = FIELD_TO_DEBUG_CHAR.iter().find(|(f, _)| field.is_field(*f));
                    match found {
                        Some((_, c)) => sb.push(*c),
                        None => sb.push('?'),
                    }
                }
            }
        }
        sb.push_str("]>");
        sb
    }

    /// Equivalent to `toCharArray()`.
    pub fn to_char_array(&self) -> Vec<u16> {
        self.chars[self.zero as usize..(self.zero + self.length) as usize].to_vec()
    }

    /// Equivalent to `toFieldArray()`.
    pub fn to_field_array(&self) -> Vec<Option<FieldObject>> {
        self.fields[self.zero as usize..(self.zero + self.length) as usize].to_vec()
    }

    /// Sets the field used in the `Appendable` methods. Equivalent to
    /// `setAppendableField(Object)`.
    pub fn set_appendable_field(&mut self, field: Option<FieldObject>) {
        self.appendable_field = field;
    }

    /// This method is provided for Java Appendable compatibility. In most
    /// cases, please use the append methods that take a field parameter.
    /// Equivalent to `append(CharSequence)` (`Appendable`).
    pub fn append_csq(&mut self, csq: &[u16]) -> &mut Self {
        debug_assert!(self.appendable_field.is_some());
        let field = self.appendable_field.clone();
        self.insert(self.length - self.append_offset, csq, field);
        self
    }

    /// Equivalent to `append(CharSequence, int, int)` (`Appendable`).
    pub fn append_csq_range(&mut self, csq: &[u16], start: i32, end: i32) -> &mut Self {
        debug_assert!(self.appendable_field.is_some());
        let field = self.appendable_field.clone();
        self.insert_range(self.length - self.append_offset, csq, start, end, field);
        self
    }

    /// Equivalent to `append(char)` (`Appendable`).
    pub fn append_char(&mut self, c: u16) -> &mut Self {
        debug_assert!(self.appendable_field.is_some());
        let field = self.appendable_field.clone();
        self.insert_char16(self.length - self.append_offset, c, field);
        self
    }

    /// Whether the contents of this builder are equal to the given char and
    /// field arrays (fields compared after unwrapping, by identity).
    /// Equivalent to `contentEquals(char[], Object[])`.
    pub fn content_equals_arrays(&self, chars: &[u16], fields: &[Option<FieldObject>]) -> bool {
        if chars.len() as i32 != self.length {
            return false;
        }
        if fields.len() as i32 != self.length {
            return false;
        }
        for i in 0..self.length {
            if self.chars[(self.zero + i) as usize] != chars[i as usize] {
                return false;
            }
            if !same_field(
                Self::unwrap_field(self.fields[(self.zero + i) as usize].as_ref()),
                Self::unwrap_field(fields[i as usize].as_ref()),
            ) {
                return false;
            }
        }
        true
    }

    /// Whether the contents of this builder are equal to those of `other`.
    /// Equivalent to `contentEquals(FormattedStringBuilder)`.
    pub fn content_equals(&self, other: &FormattedStringBuilder) -> bool {
        if self.length != other.length {
            return false;
        }
        for i in 0..self.length {
            if self.char_at(i) != other.char_at(i) {
                return false;
            }
            if !same_field(Self::unwrap_field(self.field_at(i)), Self::unwrap_field(other.field_at(i))) {
                return false;
            }
        }
        true
    }

    /// Equivalent to `hashCode()`.
    ///
    /// # Errors
    ///
    /// Always: `UnsupportedOperationException("Don't call #hashCode() or
    /// #equals() on a mutable.")`.
    pub fn hash_code(&self) -> Result<i32> {
        Err(LuceneError::unsupported_operation("Don't call #hashCode() or #equals() on a mutable."))
    }

    /// Equivalent to `equals(Object)`.
    ///
    /// # Errors
    ///
    /// Always: `UnsupportedOperationException("Don't call #hashCode() or
    /// #equals() on a mutable.")`.
    pub fn equals(&self, _other: &FormattedStringBuilder) -> Result<bool> {
        Err(LuceneError::unsupported_operation("Don't call #hashCode() or #equals() on a mutable."))
    }
}

impl fmt::Display for FormattedStringBuilder {
    /// `toString()`, rendered lossily for display.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_rust_string(&self.to_java_string()))
    }
}

/// `System.arraycopy` between two arrays.
fn arraycopy<T: Clone>(src: &[T], src_pos: i32, dest: &mut [T], dest_pos: i32, length: i32) {
    let (s, d, n) = (src_pos as usize, dest_pos as usize, length as usize);
    dest[d..d + n].clone_from_slice(&src[s..s + n]);
}

/// `System.arraycopy` within one array (overlapping ranges copied as if
/// through a temporary array).
fn arraycopy_within<T: Clone>(array: &mut [T], src_pos: i32, dest_pos: i32, length: i32) {
    let (s, d, n) = (src_pos as usize, dest_pos as usize, length as usize);
    if n == 0 {
        return;
    }
    let tmp: Vec<T> = array[s..s + n].to_vec();
    array[d..d + n].clone_from_slice(&tmp);
}

fn is_high(c: u16) -> bool {
    (0xd800..0xdc00).contains(&c)
}

fn is_low(c: u16) -> bool {
    (0xdc00..0xe000).contains(&c)
}

/// `Character.codePointAt(char[], int, int)`.
fn code_point_at_limit(a: &[u16], index: i32, limit: i32) -> i32 {
    let c1 = a[index as usize];
    if is_high(c1) && index + 1 < limit {
        let c2 = a[(index + 1) as usize];
        if is_low(c2) {
            return (((c1 as i32) - 0xd800) << 10) + ((c2 as i32) - 0xdc00) + 0x10000;
        }
    }
    c1 as i32
}

/// `Character.codePointBefore(char[], int, int)`.
fn code_point_before_start(a: &[u16], index: i32, start: i32) -> i32 {
    let i = index - 1;
    let c2 = a[i as usize];
    if is_low(c2) && i > start {
        let c1 = a[(i - 1) as usize];
        if is_high(c1) {
            return (((c1 as i32) - 0xd800) << 10) + ((c2 as i32) - 0xdc00) + 0x10000;
        }
    }
    c2 as i32
}
