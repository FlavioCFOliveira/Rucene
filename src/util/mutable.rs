//! Mutable value holders, ported from `org.apache.lucene.util.mutable`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`MutableValue`] | `MutableValue` |
//! | [`MutableValueBool`] | `MutableValueBool` |
//! | [`MutableValueDate`] | `MutableValueDate` |
//! | [`MutableValueDouble`] | `MutableValueDouble` |
//! | [`MutableValueFloat`] | `MutableValueFloat` |
//! | [`MutableValueInt`] | `MutableValueInt` |
//! | [`MutableValueLong`] | `MutableValueLong` |
//! | [`MutableValueStr`] | `MutableValueStr` |
//!
//! Lucene models these as an abstract class plus seven concrete subclasses, using
//! `Object` for the boxed value and reflection (`getClass()`) both to guard the
//! downcasts inside `copy`/`equalsSameType`/`compareSameType` and to order values of
//! different types. This port keeps the same semantics with a [`MutableValue`] trait,
//! [`std::any::Any`] downcasts, and a [`MutableValueObject`] enum in place of
//! `Object`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cmp::Ordering;

use crate::util::byte_block_pool::hash_bytes;
use crate::util::{BytesRef, BytesRefBuilder};

/// The boxed form of a [`MutableValue`], standing in for Java's `Object`.
///
/// Equivalent to the return type of `MutableValue.toObject()`.
#[derive(Clone, Debug, PartialEq)]
pub enum MutableValueObject {
    /// A `java.lang.Boolean`.
    Bool(bool),
    /// A `java.lang.Double`.
    Double(f64),
    /// A `java.lang.Float`.
    Float(f32),
    /// A `java.lang.Integer`.
    Int(i32),
    /// A `java.lang.Long`.
    Long(i64),
    /// A `java.util.Date`, as milliseconds since the epoch.
    Date(i64),
    /// A `java.lang.String`.
    Str(String),
}

impl std::fmt::Display for MutableValueObject {
    /// Renders the value the way `Object.toString()` would.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// `Float`/`Double` follow Java's convention of always showing a decimal point
    /// (`1.0`, not `1`), but the digits come from Rust's shortest round-trip
    /// formatting rather than from `Float.toString`/`Double.toString`, which can
    /// differ in the exponent form. `Date` is rendered as milliseconds since the
    /// epoch instead of `java.util.Date.toString()`, which depends on the default
    /// locale and time zone and so is not reproducible.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutableValueObject::Bool(v) => write!(f, "{}", v),
            MutableValueObject::Double(v) => write!(f, "{}", java_style_float(*v)),
            MutableValueObject::Float(v) => write!(f, "{}", java_style_float(f64::from(*v))),
            MutableValueObject::Int(v) => write!(f, "{}", v),
            MutableValueObject::Long(v) => write!(f, "{}", v),
            MutableValueObject::Date(v) => write!(f, "{}", v),
            MutableValueObject::Str(v) => f.write_str(v),
        }
    }
}

fn java_style_float(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let s = format!("{}", v);
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// Base contract for all mutable values.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValue`.
pub trait MutableValue: Any {
    /// Returns this value as `&dyn Any`, so implementations can downcast the
    /// argument of [`MutableValue::copy`] and friends the way Java casts it.
    fn as_any(&self) -> &dyn Any;

    /// The canonical Java class name of this value, used to order values of
    /// different types.
    fn canonical_name(&self) -> &'static str;

    /// Whether this value exists (i.e. is not the SQL-style `null`).
    fn exists(&self) -> bool;

    /// Sets whether this value exists.
    fn set_exists(&mut self, exists: bool);

    /// Copies `source` into this value.
    ///
    /// # Panics
    ///
    /// Panics if `source` is not of the same type, mirroring Java's
    /// `ClassCastException`.
    fn copy(&mut self, source: &dyn MutableValue);

    /// Returns an independent copy of this value.
    fn duplicate(&self) -> Box<dyn MutableValue>;

    /// Compares this value with another of the same type for equality.
    ///
    /// # Panics
    ///
    /// Panics if `other` is not of the same type, mirroring Java's
    /// `ClassCastException`.
    fn equals_same_type(&self, other: &dyn MutableValue) -> bool;

    /// Orders this value against another of the same type.
    ///
    /// # Panics
    ///
    /// Panics if `other` is not of the same type, mirroring Java's
    /// `ClassCastException`.
    fn compare_same_type(&self, other: &dyn MutableValue) -> i32;

    /// Returns the boxed form of this value, or `None` when it does not exist.
    fn to_object(&self) -> Option<MutableValueObject>;

    /// Returns the hash code of this value, matching Java's `hashCode()`.
    fn hash_code(&self) -> i32;

    /// Orders this value against any other mutable value.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java first compares `getClass().hashCode()`, which is the JVM's identity hash
    /// of the `Class` object and therefore varies from run to run, and only falls
    /// back to comparing the canonical class names when those hashes collide. This
    /// port always uses the canonical class name, which is Lucene's own tie-break and
    /// is the only deterministic half of the comparison.
    fn compare_to(&self, other: &dyn MutableValue) -> i32 {
        let c1 = self.canonical_name();
        let c2 = other.canonical_name();
        if c1 != c2 {
            return match c1.cmp(c2) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            };
        }
        self.compare_same_type(other)
    }

    /// Compares this value with `other`, returning false when the types differ.
    ///
    /// Equivalent to `MutableValue.equals(Object)`.
    fn value_equals(&self, other: &dyn MutableValue) -> bool {
        self.canonical_name() == other.canonical_name() && self.equals_same_type(other)
    }

    /// Renders this value, matching `MutableValue.toString()`.
    fn to_display_string(&self) -> String {
        match self.to_object() {
            Some(o) => o.to_string(),
            None => "(null)".to_string(),
        }
    }
}

fn downcast<'a, T: 'static>(value: &'a dyn MutableValue, expected: &str) -> &'a T {
    value
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("class cast: expected {}", expected))
}

macro_rules! ordering_of {
    ($c:expr) => {
        match $c {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        }
    };
}

// -----------------------------------------------------------------------------
// MutableValueBool
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `bool`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueBool`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has `value` set to `false` for proper
/// operation.
#[derive(Clone, Debug)]
pub struct MutableValueBool {
    /// The current value.
    pub value: bool,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueBool {
    fn default() -> Self {
        Self {
            value: false,
            exists: true,
        }
    }
}

impl MutableValueBool {
    /// Creates a value that exists and holds `false`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueBool {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueBool"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let s: &MutableValueBool = downcast(source, "MutableValueBool");
        self.value = s.value;
        self.exists = s.exists;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        debug_assert!(self.exists || !self.value);
        let b: &MutableValueBool = downcast(other, "MutableValueBool");
        self.value == b.value && self.exists == b.exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        debug_assert!(self.exists || !self.value);
        let b: &MutableValueBool = downcast(other, "MutableValueBool");
        if self.value != b.value {
            return if self.value { 1 } else { -1 };
        }
        if self.exists == b.exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        debug_assert!(self.exists || !self.value);
        if self.exists {
            Some(MutableValueObject::Bool(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        debug_assert!(self.exists || !self.value);
        if self.value {
            2
        } else if self.exists {
            1
        } else {
            0
        }
    }
}

// -----------------------------------------------------------------------------
// MutableValueDouble
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `f64`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueDouble`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has `value` set to `0.0` for proper
/// operation.
#[derive(Clone, Debug)]
pub struct MutableValueDouble {
    /// The current value.
    pub value: f64,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueDouble {
    fn default() -> Self {
        Self {
            value: 0.0,
            exists: true,
        }
    }
}

impl MutableValueDouble {
    /// Creates a value that exists and holds `0.0`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueDouble {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueDouble"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let s: &MutableValueDouble = downcast(source, "MutableValueDouble");
        self.value = s.value;
        self.exists = s.exists;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        let b: &MutableValueDouble = downcast(other, "MutableValueDouble");
        self.value == b.value && self.exists == b.exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        let b: &MutableValueDouble = downcast(other, "MutableValueDouble");
        let c = java_compare_f64(self.value, b.value);
        if c != 0 {
            return c;
        }
        if self.exists == b.exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        if self.exists {
            Some(MutableValueObject::Double(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        let x = self.value.to_bits() as i64;
        (x as i32).wrapping_add((x >> 32) as i32)
    }
}

/// Total ordering of doubles, matching `java.lang.Double.compare`: `-0.0` sorts
/// before `0.0` and `NaN` sorts above everything.
fn java_compare_f64(a: f64, b: f64) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let a_bits = a.to_bits() as i64;
    let b_bits = b.to_bits() as i64;
    match a_bits.cmp(&b_bits) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Total ordering of floats, matching `java.lang.Float.compare`.
fn java_compare_f32(a: f32, b: f32) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let a_bits = a.to_bits() as i32;
    let b_bits = b.to_bits() as i32;
    match a_bits.cmp(&b_bits) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

// -----------------------------------------------------------------------------
// MutableValueFloat
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `f32`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueFloat`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has `value` set to `0.0` for proper
/// operation.
#[derive(Clone, Debug)]
pub struct MutableValueFloat {
    /// The current value.
    pub value: f32,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueFloat {
    fn default() -> Self {
        Self {
            value: 0.0,
            exists: true,
        }
    }
}

impl MutableValueFloat {
    /// Creates a value that exists and holds `0.0`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueFloat {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueFloat"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let s: &MutableValueFloat = downcast(source, "MutableValueFloat");
        self.value = s.value;
        self.exists = s.exists;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        let b: &MutableValueFloat = downcast(other, "MutableValueFloat");
        self.value == b.value && self.exists == b.exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        let b: &MutableValueFloat = downcast(other, "MutableValueFloat");
        let c = java_compare_f32(self.value, b.value);
        if c != 0 {
            return c;
        }
        if self.exists == b.exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        if self.exists {
            Some(MutableValueObject::Float(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        self.value.to_bits() as i32
    }
}

// -----------------------------------------------------------------------------
// MutableValueInt
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `i32`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueInt`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has `value` set to `0` for proper operation.
#[derive(Clone, Debug)]
pub struct MutableValueInt {
    /// The current value.
    pub value: i32,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueInt {
    fn default() -> Self {
        Self {
            value: 0,
            exists: true,
        }
    }
}

impl MutableValueInt {
    /// Creates a value that exists and holds `0`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueInt {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueInt"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let s: &MutableValueInt = downcast(source, "MutableValueInt");
        self.value = s.value;
        self.exists = s.exists;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        debug_assert!(self.exists || self.value == 0);
        let b: &MutableValueInt = downcast(other, "MutableValueInt");
        self.value == b.value && self.exists == b.exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        let b: &MutableValueInt = downcast(other, "MutableValueInt");
        let c = ordering_of!(self.value.cmp(&b.value));
        if c != 0 {
            return c;
        }
        if self.exists == b.exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        debug_assert!(self.exists || self.value == 0);
        if self.exists {
            Some(MutableValueObject::Int(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        // NOTE (Lucene): if used in a HashMap, it already mixes the value.
        (self.value >> 8).wrapping_add(self.value >> 16)
    }
}

// -----------------------------------------------------------------------------
// MutableValueLong
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `i64`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueLong`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has `value` set to `0` for proper operation.
#[derive(Clone, Debug)]
pub struct MutableValueLong {
    /// The current value.
    pub value: i64,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueLong {
    fn default() -> Self {
        Self {
            value: 0,
            exists: true,
        }
    }
}

impl MutableValueLong {
    /// Creates a value that exists and holds `0`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Reads a long-shaped value out of either a [`MutableValueLong`] or a
/// [`MutableValueDate`], mirroring Java's `(MutableValueLong) source` cast, which
/// succeeds for the subclass too.
fn as_long_view(value: &dyn MutableValue) -> (i64, bool) {
    if let Some(v) = value.as_any().downcast_ref::<MutableValueLong>() {
        return (v.value, v.exists);
    }
    if let Some(v) = value.as_any().downcast_ref::<MutableValueDate>() {
        return (v.value, v.exists);
    }
    panic!("class cast: expected MutableValueLong");
}

impl MutableValue for MutableValueLong {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueLong"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let (value, exists) = as_long_view(source);
        self.exists = exists;
        self.value = value;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        debug_assert!(self.exists || self.value == 0);
        let (value, exists) = as_long_view(other);
        self.value == value && self.exists == exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        let (value, exists) = as_long_view(other);
        if self.value < value {
            return -1;
        }
        if self.value > value {
            return 1;
        }
        if self.exists == exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        debug_assert!(self.exists || self.value == 0);
        if self.exists {
            Some(MutableValueObject::Long(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        (self.value as i32).wrapping_add((self.value >> 32) as i32)
    }
}

// -----------------------------------------------------------------------------
// MutableValueDate
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `java.util.Date`, holding milliseconds
/// since the epoch.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueDate`, which extends
/// `MutableValueLong`; this port repeats the long behaviour because Rust has no
/// implementation inheritance, and only [`MutableValue::to_object`] and
/// [`MutableValue::duplicate`] actually differ.
#[derive(Clone, Debug)]
pub struct MutableValueDate {
    /// The current value, in milliseconds since the epoch.
    pub value: i64,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueDate {
    fn default() -> Self {
        Self {
            value: 0,
            exists: true,
        }
    }
}

impl MutableValueDate {
    /// Creates a value that exists and holds the epoch.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueDate {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueDate"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let (value, exists) = as_long_view(source);
        self.exists = exists;
        self.value = value;
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        debug_assert!(self.exists || self.value == 0);
        let (value, exists) = as_long_view(other);
        self.value == value && self.exists == exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        let (value, exists) = as_long_view(other);
        if self.value < value {
            return -1;
        }
        if self.value > value {
            return 1;
        }
        if self.exists == exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        if self.exists {
            Some(MutableValueObject::Date(self.value))
        } else {
            None
        }
    }

    fn hash_code(&self) -> i32 {
        debug_assert!(self.exists || self.value == 0);
        (self.value as i32).wrapping_add((self.value >> 32) as i32)
    }
}

// -----------------------------------------------------------------------------
// MutableValueStr
// -----------------------------------------------------------------------------

/// [`MutableValue`] implementation of type `String`.
///
/// Equivalent to `org.apache.lucene.util.mutable.MutableValueStr`. When mutating
/// instances of this object, the caller is responsible for ensuring that any instance
/// where `exists` is set to `false` also has a `value` of length 0.
#[derive(Clone, Debug)]
pub struct MutableValueStr {
    /// The current value, as UTF-8 bytes.
    pub value: BytesRefBuilder,
    /// Whether this value exists.
    pub exists: bool,
}

impl Default for MutableValueStr {
    fn default() -> Self {
        Self {
            value: BytesRefBuilder::new(),
            exists: true,
        }
    }
}

impl MutableValueStr {
    /// Creates a value that exists and holds the empty string.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MutableValue for MutableValueStr {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn canonical_name(&self) -> &'static str {
        "org.apache.lucene.util.mutable.MutableValueStr"
    }

    fn exists(&self) -> bool {
        self.exists
    }

    fn set_exists(&mut self, exists: bool) {
        self.exists = exists;
    }

    fn copy(&mut self, source: &dyn MutableValue) {
        let s: &MutableValueStr = downcast(source, "MutableValueStr");
        self.exists = s.exists;
        let bytes = s.value.get();
        self.value.copy_ref(&bytes);
    }

    fn duplicate(&self) -> Box<dyn MutableValue> {
        Box::new(self.clone())
    }

    fn equals_same_type(&self, other: &dyn MutableValue) -> bool {
        debug_assert!(self.exists || self.value.length() == 0);
        let b: &MutableValueStr = downcast(other, "MutableValueStr");
        self.value.get() == b.value.get() && self.exists == b.exists
    }

    fn compare_same_type(&self, other: &dyn MutableValue) -> i32 {
        debug_assert!(self.exists || self.value.length() == 0);
        let b: &MutableValueStr = downcast(other, "MutableValueStr");
        let c = ordering_of!(self.value.get().cmp(&b.value.get()));
        if c != 0 {
            return c;
        }
        if self.exists == b.exists {
            return 0;
        }
        if self.exists {
            1
        } else {
            -1
        }
    }

    fn to_object(&self) -> Option<MutableValueObject> {
        debug_assert!(self.exists || self.value.length() == 0);
        if self.exists {
            let bytes: BytesRef = self.value.get();
            Some(MutableValueObject::Str(
                bytes.utf8_to_string().unwrap_or_default(),
            ))
        } else {
            None
        }
    }

    /// Returns the hash code of the held bytes.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Lucene returns `BytesRef.hashCode()`, which hashes with
    /// `StringHelper.GOOD_FAST_HASH_SEED` — a per-JVM random seed — so the value is
    /// not reproducible across runs. This port uses the crate's fixed-seed
    /// `hash_bytes`, which is deterministic and never reaches a file.
    fn hash_code(&self) -> i32 {
        debug_assert!(self.exists || self.value.length() == 0);
        let bytes = self.value.get();
        hash_bytes(bytes.slice())
    }
}
