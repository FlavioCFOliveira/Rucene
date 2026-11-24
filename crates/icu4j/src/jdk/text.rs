// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! Reproductions of the `java.text` (JDK 25) types the ICU4J
//! number-formatting classes of this crate extend or call:
//! `AttributedCharacterIterator.Attribute` and `Format.Field` (open class
//! hierarchies: [`Attribute`], [`FormatField`]), [`FieldPosition`],
//! [`AttributedString`] with its iterator ([`AttributedCharacterIterator`])
//! and the abstract class `Format` ([`Format`]). `java.text.ParsePosition`
//! is [`crate::jdk::ParsePosition`]; `java.text.ParseException` is
//! [`rucene::util::extra::ParseException`].
//!
//! The JDK 25 sources (`java/text/AttributedCharacterIterator.java`,
//! `Format.java`, `FieldPosition.java`, `AttributedString.java`) are the
//! specification; the code is written for this crate.
//!
//! **Divergence.** Java's `Format.Field` subclasses (`NumberFormat.Field`,
//! `ListFormatter.Field`, `UFormat.SpanField`, ...) are open classes whose
//! instances are `static final` singletons compared by identity
//! (`Attribute.equals` and `hashCode` are `final` and inherited from
//! `Object`). The port declares each subclass as a Rust type implementing
//! [`FormatField`] (with [`java_format_field_class!`]) and each constant as
//! a `static`, referenced as [`FieldRef`] (`&'static dyn FormatField`);
//! identity is the address ([`same_attribute`]). `getClass()`,
//! `instanceof` and `Class.isAssignableFrom` are answered from the Java
//! class names each type declares ([`Attribute::java_class`],
//! [`Attribute::java_class_chain`]).
//!
//! **Divergence.** Java's `Object` values attached to attributes
//! (`AttributedString.addAttribute(Attribute, Object, int, int)`,
//! `ConstrainedFieldPosition`'s field values) are [`AttributeValue`]s: the
//! two kinds of object the ICU classes of this crate attach, a boxed
//! `Integer` and a field, with `Objects.equals` semantics.

use std::any::Any;
use std::fmt::{self, Debug};
use std::sync::{Arc, RwLock};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

use crate::jdk::ParsePosition;

// ---------------------------------------------------------------------------
// AttributedCharacterIterator.Attribute and Format.Field
// ---------------------------------------------------------------------------

/// The Java class name of `AttributedCharacterIterator.Attribute`.
pub const ATTRIBUTE_CLASS: &str = "java.text.AttributedCharacterIterator$Attribute";

/// The Java class name of `Format.Field`.
pub const FORMAT_FIELD_CLASS: &str = "java.text.Format$Field";

/// A key identifying a text attribute.
///
/// Equivalent to the open class
/// `java.text.AttributedCharacterIterator.Attribute`: a name
/// (`getName()`), identity equality and hash, and `toString()` =
/// `getClass().getName() + "(" + name + ")"`. Implementations are
/// `static` singletons (see the module documentation).
pub trait Attribute: Any + Debug + Send + Sync {
    /// `getName()` (protected in Java): the name given at construction.
    fn get_name(&self) -> &str;

    /// `getClass().getName()`: the Java class this object is an instance
    /// of.
    fn java_class(&self) -> &'static str;

    /// The Java classes this object is an instance of, from its own class
    /// up to `AttributedCharacterIterator.Attribute` (excluding `Object`):
    /// what `instanceof` and `Class.isAssignableFrom` test.
    fn java_class_chain(&self) -> &'static [&'static str];

    /// This object as `Any`, for the downcasts Java's casts perform.
    fn as_any(&self) -> &dyn Any;

    /// `toString()`: `getClass().getName() + "(" + name + ")"`.
    fn to_java_string(&self) -> String {
        format!("{}({})", self.java_class(), self.get_name())
    }

    /// `instanceof <class_name>` (or `Class.forName(class_name)
    /// .isAssignableFrom(getClass())`); `java.lang.Object` matches every
    /// object.
    fn is_instance_of(&self, class_name: &str) -> bool {
        class_name == "java.lang.Object" || self.java_class_chain().contains(&class_name)
    }
}

/// A constant identifying a field of a formatted text.
///
/// Equivalent to the open class `java.text.Format.Field`, which extends
/// `AttributedCharacterIterator.Attribute` and adds nothing but its
/// constructor; an implementation's [`Attribute::java_class_chain`] ends
/// with [`FORMAT_FIELD_CLASS`] and [`ATTRIBUTE_CLASS`].
pub trait FormatField: Attribute {}

/// A reference to a `Format.Field` constant.
pub type FieldRef = &'static dyn FormatField;

/// A reference to an `AttributedCharacterIterator.Attribute` constant.
pub type AttributeRef = &'static dyn Attribute;

/// `a == b` (and `a.equals(b)`, `Attribute.equals` being identity) for two
/// attributes.
pub fn same_attribute(a: &dyn Attribute, b: &dyn Attribute) -> bool {
    std::ptr::addr_eq(a as *const dyn Attribute, b as *const dyn Attribute)
}

/// `a == b` for two optional fields (Java `null` is `None`), what
/// `Objects.equals` answers for fields.
pub fn same_field(a: Option<FieldRef>, b: Option<FieldRef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_attribute(a, b),
        _ => false,
    }
}

/// Declares a Java `Format.Field` subclass: a struct holding the field
/// name, a `const fn new(name)` (the protected constructor), and the
/// [`Attribute`] and [`FormatField`] implementations.
///
/// `$class` is the Java binary class name; `$chain` lists the Java
/// superclasses between it and `Format.Field` (most derived first), for
/// example `["com.ibm.icu.text.UFormat$SpanField"]`, or `[]`.
#[macro_export]
macro_rules! java_format_field_class {
    ($(#[$doc:meta])* $vis:vis $name:ident, $class:literal, [$($super:literal),* $(,)?]) => {
        $(#[$doc])*
        #[derive(Debug)]
        $vis struct $name {
            name: &'static str,
        }

        impl $name {
            /// The Java binary class name.
            pub const CLASS: &'static str = $class;

            /// The protected constructor `(String name)`.
            pub const fn new(name: &'static str) -> Self {
                Self { name }
            }
        }

        impl $crate::jdk::text::Attribute for $name {
            fn get_name(&self) -> &str {
                self.name
            }

            fn java_class(&self) -> &'static str {
                $class
            }

            fn java_class_chain(&self) -> &'static [&'static str] {
                &[$class, $($super,)* $crate::jdk::text::FORMAT_FIELD_CLASS, $crate::jdk::text::ATTRIBUTE_CLASS]
            }

            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        impl $crate::jdk::text::FormatField for $name {}
    };
}

/// An object attached to an attribute or a field position.
///
/// Stands for the Java `Object` values the ICU classes of this crate
/// attach (see the module documentation).
#[derive(Debug, Clone, Copy)]
pub enum AttributeValue {
    /// A `java.lang.Integer`.
    Integer(i32),
    /// An attribute (a `Format.Field` used as its own value).
    Attribute(AttributeRef),
}

impl AttributeValue {
    /// `value.equals(other)` (`Integer.equals` by value, attributes by
    /// identity).
    pub fn java_equals(&self, other: &AttributeValue) -> bool {
        match (self, other) {
            (AttributeValue::Integer(a), AttributeValue::Integer(b)) => a == b,
            (AttributeValue::Attribute(a), AttributeValue::Attribute(b)) => same_attribute(*a, *b),
            _ => false,
        }
    }

    /// `toString()`.
    pub fn to_java_string(&self) -> String {
        match self {
            AttributeValue::Integer(i) => i.to_string(),
            AttributeValue::Attribute(a) => a.to_java_string(),
        }
    }

    /// The field value of a `Format.Field` used as a value.
    pub fn of_field(field: FieldRef) -> Self {
        AttributeValue::Attribute(field)
    }
}

/// `Objects.equals(a, b)` for optional values.
pub fn values_equal(a: Option<&AttributeValue>, b: Option<&AttributeValue>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.java_equals(b),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// FieldPosition
// ---------------------------------------------------------------------------

/// The state a subclass of `FieldPosition` adds (see [`FieldPosition`]).
pub trait FieldPositionSubclass: Any + Debug + Send + Sync {
    /// `getClass().getName()` of the subclass.
    fn java_class(&self) -> &'static str;

    /// This state as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// This state as mutable `Any`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// A copy of this state.
    fn clone_box(&self) -> Box<dyn FieldPositionSubclass>;
}

/// The field a formatter is asked to locate, and the begin and end indexes
/// of that field in the formatted text once found.
///
/// Equivalent to `java.text.FieldPosition`.
///
/// **Divergence.** Java's `FieldPosition` is an open class (ICU extends it
/// with `UFieldPosition` and tests it with `instanceof`); the port keeps a
/// subclass's added state in an optional extension
/// ([`subclass`](Self::subclass)), which also answers
/// `getClass().getName()`.
#[derive(Debug)]
pub struct FieldPosition {
    field: i32,
    end_index: i32,
    begin_index: i32,
    attribute: Option<FieldRef>,
    subclass: Option<Box<dyn FieldPositionSubclass>>,
}

impl Clone for FieldPosition {
    fn clone(&self) -> Self {
        Self {
            field: self.field,
            end_index: self.end_index,
            begin_index: self.begin_index,
            attribute: self.attribute,
            subclass: self.subclass.as_ref().map(|s| s.clone_box()),
        }
    }
}

impl FieldPosition {
    /// `new FieldPosition(int field)`.
    pub fn new(field: i32) -> Self {
        Self {
            field,
            end_index: 0,
            begin_index: 0,
            attribute: None,
            subclass: None,
        }
    }

    /// `new FieldPosition(Format.Field attribute)`: field id -1.
    pub fn with_attribute(attribute: Option<FieldRef>) -> Self {
        Self::with_attribute_and_field(attribute, -1)
    }

    /// `new FieldPosition(Format.Field attribute, int fieldID)`.
    pub fn with_attribute_and_field(attribute: Option<FieldRef>, field_id: i32) -> Self {
        Self {
            field: field_id,
            end_index: 0,
            begin_index: 0,
            attribute,
            subclass: None,
        }
    }

    /// The constructor of a subclass: this position with `subclass`'s added
    /// state.
    pub fn with_subclass(mut self, subclass: Box<dyn FieldPositionSubclass>) -> Self {
        self.subclass = Some(subclass);
        self
    }

    /// The added state of the subclass this position is an instance of,
    /// if any.
    pub fn subclass(&self) -> Option<&dyn FieldPositionSubclass> {
        self.subclass.as_deref()
    }

    /// The added state of the subclass, mutable.
    pub fn subclass_mut(&mut self) -> Option<&mut (dyn FieldPositionSubclass + 'static)> {
        self.subclass.as_deref_mut()
    }

    /// `getFieldAttribute()`.
    pub fn get_field_attribute(&self) -> Option<FieldRef> {
        self.attribute
    }

    /// `getField()`.
    pub fn get_field(&self) -> i32 {
        self.field
    }

    /// `getBeginIndex()`.
    pub fn get_begin_index(&self) -> i32 {
        self.begin_index
    }

    /// `getEndIndex()`.
    pub fn get_end_index(&self) -> i32 {
        self.end_index
    }

    /// `setBeginIndex(int bi)`.
    pub fn set_begin_index(&mut self, bi: i32) {
        self.begin_index = bi;
    }

    /// `setEndIndex(int ei)`.
    pub fn set_end_index(&mut self, ei: i32) {
        self.end_index = ei;
    }

    /// `equals(Object obj)`: same attribute (identity), begin index, end
    /// index and field id.
    pub fn java_equals(&self, other: &FieldPosition) -> bool {
        same_field(self.attribute, other.attribute)
            && self.begin_index == other.begin_index
            && self.end_index == other.end_index
            && self.field == other.field
    }

    /// `hashCode()`: `(field << 24) | (beginIndex << 16) | endIndex`.
    pub fn hash_code(&self) -> i32 {
        (self.field << 24) | (self.begin_index << 16) | self.end_index
    }

    /// `getClass().getName()`.
    pub fn java_class(&self) -> &'static str {
        self.subclass
            .as_ref()
            .map_or("java.text.FieldPosition", |s| s.java_class())
    }
}

impl fmt::Display for FieldPosition {
    /// `toString()`: `<class>[field=<f>,attribute=<a>,beginIndex=<b>,endIndex=<e>]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let attribute = self
            .attribute
            .map_or_else(|| "null".to_string(), |a| a.to_java_string());
        write!(
            f,
            "{}[field={},attribute={},beginIndex={},endIndex={}]",
            self.java_class(),
            self.field,
            attribute,
            self.begin_index,
            self.end_index
        )
    }
}

// ---------------------------------------------------------------------------
// AttributedCharacterIterator
// ---------------------------------------------------------------------------

/// Iteration over a text and its attribute runs.
///
/// Equivalent to the interface `java.text.AttributedCharacterIterator`.
/// Java's `Set<Attribute>` parameters are slices; `getAttributes()`
/// answers the run's `(key, value)` pairs in the order the run holds
/// them, and `getAllAttributeKeys()` the keys in run order.
///
/// **Divergence.** `getAllAttributeKeys()` returns a `HashSet` in Java,
/// whose order follows the identity hash codes of the keys (unspecified,
/// different per run); the port answers the keys without duplicates in
/// the order the runs hold them.
pub trait AttributedCharacterIterator: CharacterIterator {
    /// `getRunStart()`.
    fn get_run_start(&self) -> i32;

    /// `getRunStart(Attribute attribute)`.
    fn get_run_start_attribute(&self, attribute: AttributeRef) -> i32;

    /// `getRunStart(Set<? extends Attribute> attributes)`.
    fn get_run_start_attributes(&self, attributes: &[AttributeRef]) -> i32;

    /// `getRunLimit()`.
    fn get_run_limit(&self) -> i32;

    /// `getRunLimit(Attribute attribute)`.
    fn get_run_limit_attribute(&self, attribute: AttributeRef) -> i32;

    /// `getRunLimit(Set<? extends Attribute> attributes)`.
    fn get_run_limit_attributes(&self, attributes: &[AttributeRef]) -> i32;

    /// `getAttributes()`: the attributes of the current run.
    fn get_attributes(&self) -> Vec<(AttributeRef, AttributeValue)>;

    /// `getAttribute(Attribute attribute)`: the value of `attribute` on the
    /// current character, `None` for Java's `null`.
    fn get_attribute(&self, attribute: AttributeRef) -> Option<AttributeValue>;

    /// `getAllAttributeKeys()`.
    fn get_all_attribute_keys(&self) -> Vec<AttributeRef>;
}

// ---------------------------------------------------------------------------
// AttributedString
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct AttributedStringData {
    text: Vec<u16>,
    run_count: i32,
    run_starts: Vec<i32>,
    run_attributes: Vec<Option<Vec<AttributeRef>>>,
    run_attribute_values: Vec<Option<Vec<AttributeValue>>>,
}

impl AttributedStringData {
    fn length(&self) -> i32 {
        self.text.len() as i32
    }

    fn create_run_attribute_data_vectors(&mut self) {
        self.run_starts = vec![0];
        self.run_attributes = vec![None];
        self.run_attribute_values = vec![None];
        self.run_count = 1;
    }

    /// `ensureRunBreak(int offset, boolean copyAttrs)` with `copyAttrs`
    /// `true`.
    fn ensure_run_break(&mut self, offset: i32) -> i32 {
        if offset == self.length() {
            return self.run_count;
        }
        let mut run_index = 0;
        while run_index < self.run_count && self.run_starts[run_index as usize] < offset {
            run_index += 1;
        }
        if run_index < self.run_count && self.run_starts[run_index as usize] == offset {
            return run_index;
        }
        let prev = (run_index - 1) as usize;
        let new_run_attributes = self.run_attributes[prev].clone();
        let new_run_attribute_values = self.run_attribute_values[prev].clone();
        self.run_count += 1;
        let at = run_index as usize;
        self.run_starts.insert(at, offset);
        self.run_attributes.insert(at, new_run_attributes);
        self.run_attribute_values
            .insert(at, new_run_attribute_values);
        run_index
    }

    fn add_attribute_run_data(
        &mut self,
        attribute: AttributeRef,
        value: AttributeValue,
        begin_run_index: i32,
        end_run_index: i32,
    ) {
        for i in begin_run_index as usize..end_run_index as usize {
            let mut key_value_index = None;
            if self.run_attributes[i].is_none() {
                self.run_attributes[i] = Some(Vec::new());
                self.run_attribute_values[i] = Some(Vec::new());
            } else if let Some(attrs) = &self.run_attributes[i] {
                key_value_index = attrs.iter().position(|a| same_attribute(*a, attribute));
            }
            match key_value_index {
                None => {
                    if let Some(attrs) = self.run_attributes[i].as_mut() {
                        attrs.push(attribute);
                    }
                    if let Some(values) = self.run_attribute_values[i].as_mut() {
                        values.push(value);
                    }
                }
                Some(k) => {
                    if let Some(values) = self.run_attribute_values[i].as_mut() {
                        values[k] = value;
                    }
                }
            }
        }
    }

    fn add_attribute_impl(
        &mut self,
        attribute: AttributeRef,
        value: AttributeValue,
        begin_index: i32,
        end_index: i32,
    ) {
        if self.run_count == 0 {
            self.create_run_attribute_data_vectors();
        }
        let begin_run_index = self.ensure_run_break(begin_index);
        let end_run_index = self.ensure_run_break(end_index);
        self.add_attribute_run_data(attribute, value, begin_run_index, end_run_index);
    }

    /// `getAttribute(Attribute attribute, int runIndex)`.
    fn get_attribute(&self, attribute: AttributeRef, run_index: i32) -> Option<AttributeValue> {
        let attrs = self.run_attributes.get(run_index as usize)?.as_ref()?;
        let values = self.run_attribute_values[run_index as usize].as_ref()?;
        attrs
            .iter()
            .position(|a| same_attribute(*a, attribute))
            .map(|i| values[i])
    }

    /// `attributeValuesMatch(Set<? extends Attribute> attributes, int
    /// runIndex1, int runIndex2)`.
    fn attribute_values_match(
        &self,
        attributes: &[AttributeRef],
        run_index1: i32,
        run_index2: i32,
    ) -> bool {
        attributes.iter().all(|key| {
            values_equal(
                self.get_attribute(*key, run_index1).as_ref(),
                self.get_attribute(*key, run_index2).as_ref(),
            )
        })
    }
}

/// A text with attribute runs.
///
/// Equivalent to `java.text.AttributedString`, for the members the ICU
/// classes of this crate reach (`AttributedString(String)`,
/// `addAttribute`, `getIterator`). Its iterators share the string, as
/// Java's inner-class iterators do.
///
/// `getAttributeCheckRange` only differs from `getAttribute` for
/// `java.text.Annotation` values, which no [`AttributeValue`] is, so the
/// iterators read the runs directly.
#[derive(Debug, Clone)]
pub struct AttributedString {
    data: Arc<RwLock<AttributedStringData>>,
}

impl AttributedString {
    /// `new AttributedString(String text)`.
    pub fn new(text: Vec<u16>) -> Self {
        Self {
            data: Arc::new(RwLock::new(AttributedStringData {
                text,
                ..Default::default()
            })),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, AttributedStringData> {
        self.data.read().unwrap_or_else(|e| e.into_inner())
    }

    /// `addAttribute(Attribute attribute, Object value)`: the attribute on
    /// the whole text.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Can't add attribute to 0-length text")`
    /// for an empty text.
    pub fn add_attribute(&self, attribute: AttributeRef, value: AttributeValue) -> Result<()> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let len = data.length();
        if len == 0 {
            return Err(LuceneError::illegal_argument(
                "Can't add attribute to 0-length text",
            ));
        }
        data.add_attribute_impl(attribute, value, 0, len);
        Ok(())
    }

    /// `addAttribute(Attribute attribute, Object value, int beginIndex, int
    /// endIndex)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid substring range")` unless `0 <=
    /// begin_index < end_index <= length`.
    pub fn add_attribute_range(
        &self,
        attribute: AttributeRef,
        value: AttributeValue,
        begin_index: i32,
        end_index: i32,
    ) -> Result<()> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        if begin_index < 0 || end_index > data.length() || begin_index >= end_index {
            return Err(LuceneError::illegal_argument("Invalid substring range"));
        }
        data.add_attribute_impl(attribute, value, begin_index, end_index);
        Ok(())
    }

    /// `getIterator()`: an iterator over the whole text with every
    /// attribute.
    pub fn get_iterator(&self) -> AttributedStringIterator {
        let len = self.read().length();
        AttributedStringIterator::new(self.data.clone(), None, 0, len)
    }

    /// `getIterator(Attribute[] attributes, int beginIndex, int endIndex)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Invalid substring range")` unless `0 <=
    /// begin_index <= end_index <= length`.
    pub fn get_iterator_range(
        &self,
        attributes: Option<&[AttributeRef]>,
        begin_index: i32,
        end_index: i32,
    ) -> Result<AttributedStringIterator> {
        if begin_index < 0 || begin_index > end_index || end_index > self.read().length() {
            return Err(LuceneError::illegal_argument("Invalid substring range"));
        }
        Ok(AttributedStringIterator::new(
            self.data.clone(),
            attributes.map(|a| a.to_vec()),
            begin_index,
            end_index,
        ))
    }
}

/// The iterator of an [`AttributedString`].
///
/// Equivalent to the private class
/// `java.text.AttributedString.AttributedStringIterator`.
#[derive(Debug, Clone)]
pub struct AttributedStringIterator {
    string: Arc<RwLock<AttributedStringData>>,
    begin_index: i32,
    end_index: i32,
    #[allow(dead_code)]
    relevant_attributes: Option<Vec<AttributeRef>>,
    current_index: i32,
    current_run_index: i32,
    current_run_start: i32,
    current_run_limit: i32,
}

impl AttributedStringIterator {
    fn new(
        string: Arc<RwLock<AttributedStringData>>,
        attributes: Option<Vec<AttributeRef>>,
        begin_index: i32,
        end_index: i32,
    ) -> Self {
        let mut it = Self {
            string,
            begin_index,
            end_index,
            relevant_attributes: attributes,
            current_index: begin_index,
            current_run_index: 0,
            current_run_start: 0,
            current_run_limit: 0,
        };
        it.update_run_info();
        it
    }

    fn data(&self) -> std::sync::RwLockReadGuard<'_, AttributedStringData> {
        self.string.read().unwrap_or_else(|e| e.into_inner())
    }

    /// `equals(Object obj)`: the same string, index, begin and end.
    pub fn java_equals(&self, that: &AttributedStringIterator) -> bool {
        Arc::ptr_eq(&self.string, &that.string)
            && self.current_index == that.current_index
            && self.begin_index == that.begin_index
            && self.end_index == that.end_index
    }

    /// `hashCode()`: `text.hashCode() ^ currentIndex ^ beginIndex ^
    /// endIndex`.
    pub fn hash_code(&self) -> i32 {
        let mut h: i32 = 0;
        for &c in &self.data().text {
            h = h.wrapping_mul(31).wrapping_add(c as i32);
        }
        h ^ self.current_index ^ self.begin_index ^ self.end_index
    }

    fn char_at(&self, index: i32) -> u16 {
        self.data().text[index as usize]
    }

    fn internal_set_index(&mut self, position: i32) -> u16 {
        self.current_index = position;
        if position < self.current_run_start || position >= self.current_run_limit {
            self.update_run_info();
        }
        if self.current_index == self.end_index {
            DONE
        } else {
            self.char_at(position)
        }
    }

    fn update_run_info(&mut self) {
        if self.current_index == self.end_index {
            self.current_run_start = self.end_index;
            self.current_run_limit = self.end_index;
            self.current_run_index = -1;
        } else {
            let data = self.string.read().unwrap_or_else(|e| e.into_inner());
            let mut run_index = -1;
            while run_index < data.run_count - 1
                && data.run_starts[(run_index + 1) as usize] <= self.current_index
            {
                run_index += 1;
            }
            let mut start = self.begin_index;
            if run_index >= 0 {
                start = data.run_starts[run_index as usize];
                if start < self.begin_index {
                    start = self.begin_index;
                }
            }
            let limit = if run_index < data.run_count - 1 {
                data.run_starts[(run_index + 1) as usize].min(self.end_index)
            } else {
                self.end_index
            };
            drop(data);
            self.current_run_index = run_index;
            self.current_run_start = start;
            self.current_run_limit = limit;
        }
    }
}

impl CharacterIterator for AttributedStringIterator {
    fn first(&mut self) -> u16 {
        self.internal_set_index(self.begin_index)
    }

    fn last(&mut self) -> u16 {
        if self.end_index == self.begin_index {
            self.internal_set_index(self.end_index)
        } else {
            self.internal_set_index(self.end_index - 1)
        }
    }

    fn current(&self) -> u16 {
        if self.current_index == self.end_index {
            DONE
        } else {
            self.char_at(self.current_index)
        }
    }

    fn next(&mut self) -> u16 {
        if self.current_index < self.end_index {
            self.internal_set_index(self.current_index + 1)
        } else {
            DONE
        }
    }

    fn previous(&mut self) -> u16 {
        if self.current_index > self.begin_index {
            self.internal_set_index(self.current_index - 1)
        } else {
            DONE
        }
    }

    fn set_index(&mut self, position: i32) -> Result<u16> {
        if position < self.begin_index || position > self.end_index {
            return Err(LuceneError::illegal_argument("Invalid index"));
        }
        Ok(self.internal_set_index(position))
    }

    fn get_begin_index(&self) -> i32 {
        self.begin_index
    }

    fn get_end_index(&self) -> i32 {
        self.end_index
    }

    fn get_index(&self) -> i32 {
        self.current_index
    }

    fn clone_box(&self) -> Box<dyn CharacterIterator> {
        Box::new(self.clone())
    }
}

impl AttributedCharacterIterator for AttributedStringIterator {
    fn get_run_start(&self) -> i32 {
        self.current_run_start
    }

    fn get_run_start_attribute(&self, attribute: AttributeRef) -> i32 {
        if self.current_run_start == self.begin_index || self.current_run_index == -1 {
            return self.current_run_start;
        }
        let value = self.get_attribute(attribute);
        let data = self.data();
        let mut run_start = self.current_run_start;
        let mut run_index = self.current_run_index;
        while run_start > self.begin_index
            && values_equal(
                value.as_ref(),
                data.get_attribute(attribute, run_index - 1).as_ref(),
            )
        {
            run_index -= 1;
            run_start = data.run_starts[run_index as usize];
        }
        run_start.max(self.begin_index)
    }

    fn get_run_start_attributes(&self, attributes: &[AttributeRef]) -> i32 {
        if self.current_run_start == self.begin_index || self.current_run_index == -1 {
            return self.current_run_start;
        }
        let data = self.data();
        let mut run_start = self.current_run_start;
        let mut run_index = self.current_run_index;
        while run_start > self.begin_index
            && data.attribute_values_match(attributes, self.current_run_index, run_index - 1)
        {
            run_index -= 1;
            run_start = data.run_starts[run_index as usize];
        }
        run_start.max(self.begin_index)
    }

    fn get_run_limit(&self) -> i32 {
        self.current_run_limit
    }

    fn get_run_limit_attribute(&self, attribute: AttributeRef) -> i32 {
        if self.current_run_limit == self.end_index || self.current_run_index == -1 {
            return self.current_run_limit;
        }
        let value = self.get_attribute(attribute);
        let data = self.data();
        let mut run_limit = self.current_run_limit;
        let mut run_index = self.current_run_index;
        while run_limit < self.end_index
            && values_equal(
                value.as_ref(),
                data.get_attribute(attribute, run_index + 1).as_ref(),
            )
        {
            run_index += 1;
            run_limit = if run_index < data.run_count - 1 {
                data.run_starts[(run_index + 1) as usize]
            } else {
                self.end_index
            };
        }
        run_limit.min(self.end_index)
    }

    fn get_run_limit_attributes(&self, attributes: &[AttributeRef]) -> i32 {
        if self.current_run_limit == self.end_index || self.current_run_index == -1 {
            return self.current_run_limit;
        }
        let data = self.data();
        let mut run_limit = self.current_run_limit;
        let mut run_index = self.current_run_index;
        while run_limit < self.end_index
            && data.attribute_values_match(attributes, self.current_run_index, run_index + 1)
        {
            run_index += 1;
            run_limit = if run_index < data.run_count - 1 {
                data.run_starts[(run_index + 1) as usize]
            } else {
                self.end_index
            };
        }
        run_limit.min(self.end_index)
    }

    fn get_attributes(&self) -> Vec<(AttributeRef, AttributeValue)> {
        let data = self.data();
        if data.run_count == 0 || self.current_run_index == -1 {
            return Vec::new();
        }
        let i = self.current_run_index as usize;
        match (&data.run_attributes[i], &data.run_attribute_values[i]) {
            (Some(keys), Some(values)) => {
                keys.iter().copied().zip(values.iter().copied()).collect()
            }
            _ => Vec::new(),
        }
    }

    fn get_attribute(&self, attribute: AttributeRef) -> Option<AttributeValue> {
        if self.current_run_index < 0 {
            return None;
        }
        self.data().get_attribute(attribute, self.current_run_index)
    }

    fn get_all_attribute_keys(&self) -> Vec<AttributeRef> {
        let data = self.data();
        let mut keys: Vec<AttributeRef> = Vec::new();
        for i in 0..data.run_count {
            let iu = i as usize;
            if data.run_starts[iu] < self.end_index
                && (i == data.run_count - 1 || data.run_starts[iu + 1] > self.begin_index)
            {
                if let Some(attrs) = &data.run_attributes[iu] {
                    for a in attrs {
                        if !keys.iter().any(|k| same_attribute(*k, *a)) {
                            keys.push(*a);
                        }
                    }
                }
            }
        }
        keys
    }
}

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

/// A Java object a `Format` parses to.
pub type ParsedObject = Box<dyn Any + Send + Sync>;

/// The abstract base of locale-sensitive formatters.
///
/// Equivalent to the abstract class `java.text.Format`. Java's `Object`
/// arguments are `&dyn Any` (an implementation downcasts them, as Java's
/// `instanceof` tests do); `StringBuffer` results are appended to a
/// `&mut Vec<u16>`.
pub trait Format: Any + Debug + Send + Sync {
    /// `format(Object obj, StringBuffer toAppendTo, FieldPosition pos)`
    /// (abstract): appends the formatted `obj`.
    ///
    /// # Errors
    ///
    /// The implementation's (`IllegalArgumentException` for an object it
    /// cannot format, in the JDK's contract).
    fn format_object_to(
        &self,
        obj: &dyn Any,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()>;

    /// `parseObject(String source, ParsePosition pos)` (abstract): the
    /// parsed object, `None` for Java's `null` on failure.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn parse_object_at(
        &self,
        source: &[u16],
        pos: &mut ParsePosition,
    ) -> Result<Option<ParsedObject>>;

    /// `clone()`.
    fn clone_format(&self) -> Box<dyn Format>;

    /// This object as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// `format(Object obj)` (final): `format(obj, new StringBuffer(), new
    /// FieldPosition(0)).toString()`.
    ///
    /// # Errors
    ///
    /// Those of [`format_object_to`](Self::format_object_to).
    fn format_object(&self, obj: &dyn Any) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_object_to(obj, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// `formatToCharacterIterator(Object obj)`: an iterator over
    /// `format(obj)` without attributes.
    ///
    /// # Errors
    ///
    /// Those of [`format_object`](Self::format_object).
    fn format_to_character_iterator(
        &self,
        obj: &dyn Any,
    ) -> Result<Box<dyn AttributedCharacterIterator>> {
        let s = self.format_object(obj)?;
        Ok(Box::new(AttributedString::new(s).get_iterator()))
    }

    /// `parseObject(String source)`.
    ///
    /// # Errors
    ///
    /// `ParseException("Format.parseObject(String) failed", errorIndex)`
    /// when the parse position did not move, and those of
    /// [`parse_object_at`](Self::parse_object_at).
    fn parse_object(&self, source: &[u16]) -> Result<Option<ParsedObject>> {
        let mut pos = ParsePosition::new(0);
        let result = self.parse_object_at(source, &mut pos)?;
        if pos.get_index() == 0 {
            return Err(rucene::util::extra::ParseException::error(
                "Format.parseObject(String) failed",
                pos.get_error_index(),
            ));
        }
        Ok(result)
    }
}
