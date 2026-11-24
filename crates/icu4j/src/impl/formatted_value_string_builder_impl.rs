// © 2019 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.FormattedValueStringBuilderImpl`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::text::{
    same_attribute, AttributeRef, AttributeValue, AttributedCharacterIterator, AttributedString, FieldPosition, FieldRef,
};
use crate::r#impl::formatted_string_builder::{FieldObject, FieldWrapper, FormattedStringBuilder};
use crate::r#impl::static_unicode_sets::{Key, StaticUnicodeSets};
use crate::text::constrained_field_position::ConstrainedFieldPosition;
use crate::text::list_formatter;
use crate::text::number_format::{self, field as nf_field};
use crate::text::u_format::is_span_field;
use crate::text::unicode_set::SpanCondition;

/// The Java class name of `NumberFormat.Field`.
const NUMBER_FORMAT_FIELD_CLASS: &str = "com.ibm.icu.text.NumberFormat$Field";

/// Placeholder field used for calculating spans. Does not currently support
/// nested fields beyond one level. Equivalent to
/// `FormattedValueStringBuilderImpl.SpanFieldPlaceholder` (public mutable
/// fields, set before the placeholder is stored and shared).
#[derive(Debug, Clone, Default)]
pub struct SpanFieldPlaceholder {
    /// `spanField` (a `UFormat.SpanField`).
    pub span_field: Option<FieldRef>,
    /// `normalField`.
    pub normal_field: Option<FieldRef>,
    /// `value`.
    pub value: Option<AttributeValue>,
    /// `start`.
    pub start: i32,
    /// `length`.
    pub length: i32,
}

impl FieldWrapper for SpanFieldPlaceholder {
    fn unwrap(&self) -> Option<FieldRef> {
        self.normal_field
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `fields[i] instanceof SpanFieldPlaceholder ? (SpanFieldPlaceholder)
/// fields[i] : null`.
fn as_placeholder(field: Option<&FieldObject>) -> Option<&SpanFieldPlaceholder> {
    match field {
        Some(FieldObject::Wrapper(w)) => w.as_any().downcast_ref::<SpanFieldPlaceholder>(),
        _ => None,
    }
}

// A Java `Format.Field` subclass whose instances are local to this
// implementation.
crate::java_format_field_class!(
    /// A `Format.Field` marking the end of the text. Equivalent to
    /// `FormattedValueStringBuilderImpl.NullField`.
    pub NullField, "com.ibm.icu.impl.FormattedValueStringBuilderImpl$NullField", []
);

/// `NullField.END`.
pub static NULL_FIELD_END: NullField = NullField::new("end");

/// `cfpos.matchesField(field, value)` for the non-null fields this class
/// passes (Java's `IllegalArgumentException` for a null field cannot occur).
fn matches(cfpos: &ConstrainedFieldPosition, field: FieldRef, value: Option<&AttributeValue>) -> bool {
    cfpos.matches_field(Some(field), value).expect("the field is not null")
}

/// Implementation of FormattedValue based on FormattedStringBuilder.
///
/// Equivalent to `com.ibm.icu.impl.FormattedValueStringBuilderImpl` (static
/// members). Fields are compared by identity (Java `==`).
pub struct FormattedValueStringBuilderImpl;

impl FormattedValueStringBuilderImpl {
    /// Finds the index at which a span field begins (`-1` when absent).
    /// Equivalent to `findSpan(FormattedStringBuilder, Object)`.
    pub fn find_span(self_: &FormattedStringBuilder, value: &AttributeValue) -> i32 {
        for i in self_.zero..self_.zero + self_.length {
            let ph = match as_placeholder(self_.fields[i as usize].as_ref()) {
                None => continue,
                Some(ph) => ph,
            };
            let ph_value = ph.value.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException: value"));
            if ph_value.java_equals(value) {
                return i - self_.zero;
            }
        }
        -1
    }

    /// Upgrade a range of a string to a span field. Equivalent to
    /// `applySpanRange(FormattedStringBuilder, UFormat.SpanField, Object,
    /// int, int)`.
    ///
    /// # Panics
    ///
    /// Java's `ClassCastException` when a field of the range is already a
    /// placeholder (`(java.text.Format.Field) oldField`).
    pub fn apply_span_range(self_: &mut FormattedStringBuilder, span_field: FieldRef, value: Option<AttributeValue>, start: i32, end: i32) {
        for i in start + self_.zero..end + self_.zero {
            let old_field = match &self_.fields[i as usize] {
                None => None,
                Some(FieldObject::Field(f)) => Some(*f),
                Some(FieldObject::Wrapper(_)) => panic!(
                    "java.lang.ClassCastException: class \
                     com.ibm.icu.impl.FormattedValueStringBuilderImpl$SpanFieldPlaceholder cannot be cast to class \
                     java.text.Format$Field"
                ),
            };
            let new_field = SpanFieldPlaceholder {
                span_field: Some(span_field),
                normal_field: old_field,
                value: value,
                start,
                length: end - start,
            };
            self_.fields[i as usize] = Some(FieldObject::Wrapper(Arc::new(new_field)));
        }
    }

    /// Equivalent to `nextFieldPosition(FormattedStringBuilder,
    /// FieldPosition)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("You must pass an instance of
    /// com.ibm.icu.text.NumberFormat.Field as your FieldPosition attribute.
    /// You passed: <class>")`.
    pub fn next_field_position(self_: &FormattedStringBuilder, fp: &mut FieldPosition) -> Result<bool> {
        let raw_field: FieldRef = match fp.get_field_attribute() {
            Some(f) => f,
            None => {
                // Backwards compatibility: read from fp.getField()
                if fp.get_field() == number_format::INTEGER_FIELD {
                    &nf_field::INTEGER
                } else if fp.get_field() == number_format::FRACTION_FIELD {
                    &nf_field::FRACTION
                } else {
                    // No field is set
                    return Ok(false);
                }
            }
        };

        if !raw_field.is_instance_of(NUMBER_FORMAT_FIELD_CLASS) {
            return Err(LuceneError::illegal_argument(format!(
                "You must pass an instance of com.ibm.icu.text.NumberFormat.Field as your FieldPosition attribute.  \
                 You passed: class {}",
                raw_field.java_class()
            )));
        }

        let mut cfpos = ConstrainedFieldPosition::new();
        cfpos.constrain_field(Some(raw_field))?;
        cfpos.set_state(Some(raw_field), None, fp.get_begin_index(), fp.get_end_index());
        if Self::next_position(self_, &mut cfpos, None) {
            fp.set_begin_index(cfpos.get_start());
            fp.set_end_index(cfpos.get_limit());
            return Ok(true);
        }

        // Special case: fraction should start after integer if fraction is
        // not present
        if same_attribute(raw_field, &nf_field::FRACTION) && fp.get_end_index() == 0 {
            let mut inside = false;
            let mut i = self_.zero;
            while i < self_.zero + self_.length {
                let f = self_.fields[i as usize].as_ref();
                if Self::is_int_or_group(f) || is_exactly(f, &nf_field::DECIMAL_SEPARATOR) {
                    inside = true;
                } else if inside {
                    break;
                }
                i += 1;
            }
            fp.set_begin_index(i - self_.zero);
            fp.set_end_index(i - self_.zero);
        }

        Ok(false)
    }

    /// Equivalent to `toCharacterIterator(FormattedStringBuilder, Field)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `AttributedString.addAttribute`.
    pub fn to_character_iterator(
        self_: &FormattedStringBuilder,
        numeric_field: Option<FieldRef>,
    ) -> Result<Box<dyn AttributedCharacterIterator>> {
        let mut cfpos = ConstrainedFieldPosition::new();
        let as_ = AttributedString::new(self_.to_java_string());
        while Self::next_position(self_, &mut cfpos, numeric_field) {
            // Backwards compatibility: field value = field
            let field = cfpos.get_field().expect("a position found has a field");
            let value = cfpos.get_field_value().unwrap_or_else(|| AttributeValue::of_field(field));
            as_.add_attribute_range(field as AttributeRef, value, cfpos.get_start(), cfpos.get_limit())?;
        }
        Ok(Box::new(as_.get_iterator()))
    }

    /// Implementation of nextPosition consistent with the contract of
    /// FormattedValue. Equivalent to `nextPosition(FormattedStringBuilder,
    /// ConstrainedFieldPosition, Field)`.
    pub fn next_position(self_: &FormattedStringBuilder, cfpos: &mut ConstrainedFieldPosition, numeric_field: Option<FieldRef>) -> bool {
        let end_field = Some(FieldObject::Field(&NULL_FIELD_END));
        let mut field_start: i32 = -1;
        let mut curr_field: Option<FieldObject> = None;
        let mut prev_is_span = false;
        if cfpos.get_limit() > 0 {
            prev_is_span = cfpos.get_field().is_some_and(|f| is_span_field(f)) && cfpos.get_start() < cfpos.get_limit();
        }
        let mut prev_is_numeric = false;
        if let Some(nf) = numeric_field {
            prev_is_numeric = cfpos.get_field().is_some_and(|f| same_attribute(f, nf));
        }
        let mut prev_is_integer = cfpos.get_field().is_some_and(|f| same_attribute(f, &nf_field::INTEGER));

        let mut i = self_.zero + cfpos.get_limit();
        while i <= self_.zero + self_.length {
            let mut field: Option<FieldObject> =
                if i < self_.zero + self_.length { self_.fields[i as usize].clone() } else { end_field.clone() };
            // Case 1: currently scanning a field.
            if curr_field.is_some() {
                if curr_field != field {
                    let mut end = i - self_.zero;
                    // Grouping separators can be whitespace; don't throw
                    // them out!
                    if Self::is_trimmable(curr_field.as_ref()) {
                        end = Self::trim_back(self_, end);
                    }
                    if end <= field_start {
                        // Entire field position is ignorable; skip.
                        field_start = -1;
                        curr_field = None;
                        // i-- and continue: the loop increment is skipped.
                        continue;
                    }
                    let mut start = field_start;
                    if Self::is_trimmable(curr_field.as_ref()) {
                        start = Self::trim_front(self_, start);
                    }
                    let cf = match &curr_field {
                        Some(FieldObject::Field(f)) => Some(*f),
                        _ => None,
                    };
                    cfpos.set_state(cf, None, start, end);
                    return true;
                }
                i += 1;
                continue;
            }
            // Special case: emit normalField if we are pointing at the end
            // of spanField.
            if i > self_.zero && prev_is_span {
                let ph = as_placeholder(self_.fields[(i - 1) as usize].as_ref());
                debug_assert!(ph.is_some());
                let ph = ph.expect("the field before a span end is a placeholder");
                if ph.normal_field.is_some_and(|f| same_attribute(f, &list_formatter::field::ELEMENT)) {
                    // Special handling for ULISTFMT_ELEMENT_FIELD
                    if matches(cfpos, &list_formatter::field::ELEMENT, None) {
                        field_start = i - self_.zero - ph.length;
                        let end = field_start + ph.length;
                        cfpos.set_state(Some(&list_formatter::field::ELEMENT), None, field_start, end);
                        return true;
                    }
                } else {
                    // Re-wind, since there may be multiple fields in the
                    // span.
                    i -= ph.length;
                    debug_assert!(i >= self_.zero);
                    let ph2 = as_placeholder(self_.fields[i as usize].as_ref())
                        .expect("the start of a span is a placeholder");
                    field = ph2.normal_field.map(FieldObject::Field);
                }
            }
            // Special case: coalesce the INTEGER if we are pointing at the
            // end of the INTEGER.
            if matches(cfpos, &nf_field::INTEGER, None)
                && i > self_.zero
                && !prev_is_integer
                && !prev_is_numeric
                && Self::is_int_or_group(self_.fields[(i - 1) as usize].as_ref())
                && !Self::is_int_or_group(field.as_ref())
            {
                let mut j = i - 1;
                while j >= self_.zero && Self::is_int_or_group(self_.fields[j as usize].as_ref()) {
                    j -= 1;
                }
                cfpos.set_state(Some(&nf_field::INTEGER), None, j - self_.zero + 1, i - self_.zero);
                return true;
            }
            // Special case: coalesce NUMERIC if we are pointing at the end
            // of the NUMERIC.
            if let Some(nf) = numeric_field {
                if matches(cfpos, nf, None)
                    && i > self_.zero
                    && !prev_is_numeric
                    && Self::is_numeric_field(self_.fields[(i - 1) as usize].as_ref())
                    && !Self::is_numeric_field(field.as_ref())
                {
                    // Re-wind to the beginning of the field and then emit
                    // it
                    let mut j = i - 1;
                    while j >= self_.zero && Self::is_numeric_field(self_.fields[j as usize].as_ref()) {
                        j -= 1;
                    }
                    cfpos.set_state(Some(nf), None, j - self_.zero + 1, i - self_.zero);
                    return true;
                }
            }
            // Check for span field
            let mut ph: Option<SpanFieldPlaceholder> = None;
            if let Some(p) = as_placeholder(field.as_ref()) {
                let p = p.clone();
                field = p.normal_field.map(FieldObject::Field);
                ph = Some(p);
            }
            if let Some(ph) = ph.as_ref().filter(|ph| ph.start == -1 || ph.start == i - self_.zero) {
                let span_field = ph.span_field.expect("a placeholder has a span field");
                if matches(cfpos, span_field, ph.value.as_ref()) {
                    field_start = i - self_.zero;
                    let end = field_start + ph.length;
                    cfpos.set_state(Some(span_field), ph.value, field_start, end);
                    return true;
                } else if ph.normal_field.is_some_and(|f| same_attribute(f, &list_formatter::field::ELEMENT)) {
                    // Special handling for ULISTFMT_ELEMENT_FIELD
                    if matches(cfpos, &list_formatter::field::ELEMENT, None) {
                        field_start = i - self_.zero;
                        let end = field_start + ph.length;
                        cfpos.set_state(Some(&list_formatter::field::ELEMENT), None, field_start, end);
                        return true;
                    } else {
                        // Failed to match; jump ahead
                        i += ph.length - 1;
                        // continue (below, with the flags reset)
                    }
                }
            }
            // Special case: skip over INTEGER; will be coalesced later.
            else if is_exactly(field.as_ref(), &nf_field::INTEGER) {
                field = None;
            }
            // No field starting at this position.
            else if field.is_none() || field == end_field {
                // continue
            }
            // No SpanField
            else if let Some(FieldObject::Field(f)) = &field {
                if matches(cfpos, *f, None) {
                    field_start = i - self_.zero;
                    curr_field = field.clone();
                }
            }
            let _ = &field;

            prev_is_span = false;
            prev_is_numeric = false;
            prev_is_integer = false;
            i += 1;
        }

        debug_assert!(curr_field.is_none());
        // Always set the position to the end so that we don't revisit
        // previous sections
        cfpos.set_state(cfpos.get_field(), cfpos.get_field_value(), self_.length, self_.length);
        false
    }

    /// `isIntOrGroup(Object)`.
    fn is_int_or_group(field: Option<&FieldObject>) -> bool {
        match FormattedStringBuilder::unwrap_field(field) {
            Some(f) => same_attribute(f, &nf_field::INTEGER) || same_attribute(f, &nf_field::GROUPING_SEPARATOR),
            None => false,
        }
    }

    /// `isNumericField(Object)`.
    fn is_numeric_field(field: Option<&FieldObject>) -> bool {
        match FormattedStringBuilder::unwrap_field(field) {
            None => true,
            Some(f) => f.is_instance_of(NUMBER_FORMAT_FIELD_CLASS),
        }
    }

    /// `isTrimmable(Object)`.
    fn is_trimmable(field: Option<&FieldObject>) -> bool {
        !is_exactly(field, &nf_field::GROUPING_SEPARATOR)
            && !matches!(field, Some(FieldObject::Field(f)) if f.is_instance_of(list_formatter::FIELD_CLASS))
    }

    /// `trimBack(FormattedStringBuilder, int)`.
    fn trim_back(self_: &FormattedStringBuilder, limit: i32) -> i32 {
        let text = &self_.chars[self_.zero as usize..(self_.zero + self_.length) as usize];
        StaticUnicodeSets::get(Key::DefaultIgnorables).span_back(text, limit, SpanCondition::Contained)
    }

    /// `trimFront(FormattedStringBuilder, int)`.
    fn trim_front(self_: &FormattedStringBuilder, start: i32) -> i32 {
        let text = &self_.chars[self_.zero as usize..(self_.zero + self_.length) as usize];
        StaticUnicodeSets::get(Key::DefaultIgnorables).span(text, start, SpanCondition::Contained)
    }
}

/// `object == field` for a nullable field object.
fn is_exactly(object: Option<&FieldObject>, field: FieldRef) -> bool {
    matches!(object, Some(FieldObject::Field(f)) if same_attribute(*f, field))
}
