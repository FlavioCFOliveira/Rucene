//! The JDK types the port shares between its modules: `java.lang.Object`
//! values, `java.io.Reader` and `java.io.InputStream` handles, the Java
//! exceptions thrown through the parser, and Java `String` helpers.
//!
//! These are not ports of NekoHTML or Xerces files; they are the Rust
//! counterparts of the JDK types those files use (see the crate
//! documentation for the divergences they carry).

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::LuceneError;
use rucene_analysis_common::jdk::character;
use rucene_analysis_common::jdk::io::ReadChars;

use crate::org::apache::xerces::xni::xni_exception::XNIException;
use crate::org::xml::sax::sax_exception::SAXException;

/// A non-null `java.lang.Object` reference: a shared value of any type.
/// Java's `null` is `None` of an `Option<Object>`.
pub type Object = Rc<dyn Any>;

/// A `java.lang.String` as an [`Object`].
pub fn string_object(s: &str) -> Object {
    Rc::new(s.to_string())
}

/// `object instanceof String ? (String) object : null`.
pub fn object_as_str(o: &Object) -> Option<&str> {
    o.downcast_ref::<String>().map(String::as_str)
}

/// `object instanceof Boolean ? (Boolean) object : null`.
pub fn object_as_bool(o: &Object) -> Option<bool> {
    o.downcast_ref::<bool>().copied()
}

/// An interface-typed `Object` (a property value whose Java type is an
/// interface): the `Rc<dyn Interface>` boxed in the [`Object`].
pub fn interface_object<T: ?Sized + 'static>(value: Rc<T>) -> Object {
    Rc::new(value)
}

/// `(Interface) object` of an interface-typed `Object` made by
/// [`interface_object`]; `None` where Java throws `ClassCastException`.
pub fn object_as_interface<T: ?Sized + 'static>(o: &Object) -> Option<Rc<T>> {
    o.downcast_ref::<Rc<T>>().cloned()
}

/// A `java.io.Reader` reference, shared as Java shares it.
pub type Reader = Rc<RefCell<dyn ReadChars>>;

/// A `java.io.InputStream` reference, shared as Java shares it.
pub type InputStream = Rc<RefCell<dyn std::io::Read>>;

/// A Java exception thrown through the parser.
///
/// The checked `java.io.IOException` and `org.xml.sax.SAXException` and the
/// unchecked `org.apache.xerces.xni.XNIException` keep their class and data,
/// because Xerces catches and converts them by class; any other unchecked
/// exception is kept as the [`LuceneError`] that stands for it.
#[derive(Debug, Clone)]
pub enum Exception {
    /// An `org.xml.sax.SAXException` or one of its subclasses.
    Sax(SAXException),
    /// An `org.apache.xerces.xni.XNIException` or one of its subclasses.
    Xni(XNIException),
    /// A `java.io.IOException` or one of its subclasses.
    Io(Arc<LuceneError>),
    /// Any other `java.lang.RuntimeException` (or `Error`).
    Runtime(Arc<LuceneError>),
}

/// The result of a Java method that may throw.
pub type JResult<T> = std::result::Result<T, Exception>;

impl Exception {
    /// A `java.io.IOException` (or subclass) of `error`.
    pub fn io(error: LuceneError) -> Self {
        Exception::Io(Arc::new(error))
    }

    /// An unchecked exception of `error`.
    pub fn runtime(error: LuceneError) -> Self {
        Exception::Runtime(Arc::new(error))
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &'static str {
        match self {
            Exception::Sax(e) => e.class_name(),
            Exception::Xni(e) => e.class_name(),
            Exception::Io(_) => "java.io.IOException",
            Exception::Runtime(_) => "java.lang.RuntimeException",
        }
    }

    /// `Throwable.getMessage()` (as overridden by each class).
    pub fn get_message(&self) -> Option<String> {
        match self {
            Exception::Sax(e) => e.get_message(),
            Exception::Xni(e) => e.get_message().map(str::to_string),
            Exception::Io(e) | Exception::Runtime(e) => Some(e.to_string()),
        }
    }

    /// `Throwable.toString()` (as overridden by each class).
    pub fn to_java_string(&self) -> String {
        match self {
            Exception::Sax(e) => e.to_java_string(),
            Exception::Xni(e) => e.to_java_string(),
            Exception::Io(e) | Exception::Runtime(e) => e.to_string(),
        }
    }

    /// The exception as a [`LuceneError`], for a caller that propagates it
    /// out of the parser: an I/O or unchecked error is returned as it was
    /// thrown; a SAX or XNI exception becomes an error of its class and
    /// message.
    pub fn into_lucene_error(self) -> LuceneError {
        match self {
            Exception::Io(e) | Exception::Runtime(e) => {
                Arc::try_unwrap(e).unwrap_or_else(|e| e.duplicate())
            }
            other => {
                let class = other.class_name();
                LuceneError::unchecked(class, other.get_message().unwrap_or_default())
            }
        }
    }
}

impl From<LuceneError> for Exception {
    /// Classifies a [`LuceneError`] by its Java class: an `IOException` (or
    /// subclass) is [`Exception::Io`], anything else [`Exception::Runtime`].
    fn from(error: LuceneError) -> Self {
        if error.is_io_exception() {
            Exception::io(error)
        } else {
            Exception::runtime(error)
        }
    }
}

impl From<SAXException> for Exception {
    fn from(e: SAXException) -> Self {
        Exception::Sax(e)
    }
}

impl From<XNIException> for Exception {
    fn from(e: XNIException) -> Self {
        Exception::Xni(e)
    }
}

impl fmt::Display for Exception {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// `Throwable.toString()` of a plain Java exception: its class name, then
/// `": "` and its message when it has one.
pub(crate) fn throwable_to_string(class_name: &str, message: Option<&str>) -> String {
    match message {
        Some(m) => format!("{class_name}: {m}"),
        None => class_name.to_string(),
    }
}

/// `String.hashCode()`: the hash of the string's UTF-16 units.
pub fn string_hash_code(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// The UTF-16 units of `s` (`String.toCharArray()`).
pub fn to_chars(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `new String(char[])` (see the crate documentation for unpaired
/// surrogates).
pub fn from_chars(chars: &[u16]) -> String {
    String::from_utf16_lossy(chars)
}

/// `String.length()`: the number of UTF-16 units.
pub fn java_len(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// `String.charAt(index)`.
///
/// # Panics
///
/// When `index` is out of bounds, as Java throws
/// `StringIndexOutOfBoundsException`.
pub fn char_at(s: &str, index: i32) -> u16 {
    s.encode_utf16()
        .nth(index as usize)
        .unwrap_or_else(|| panic!("java.lang.StringIndexOutOfBoundsException: index {index}"))
}

/// `String.toUpperCase(Locale.ENGLISH)` / `Locale.ROOT`: the locale-independent
/// full Unicode upper-casing.
pub fn to_upper_case(s: &str) -> String {
    s.to_uppercase()
}

/// `String.toLowerCase(Locale.ENGLISH)` / `Locale.ROOT`: the
/// locale-independent full Unicode lower-casing.
pub fn to_lower_case(s: &str) -> String {
    s.to_lowercase()
}

/// `String.equalsIgnoreCase(String)`: equal lengths and every UTF-16 unit
/// pair equal, or equal after `Character.toUpperCase`, or after
/// `Character.toLowerCase` of the upper-cased units.
pub fn equals_ignore_case(a: &str, b: &str) -> bool {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(&x, &y)| {
            if x == y {
                return true;
            }
            let ux = char_upper(x);
            let uy = char_upper(y);
            ux == uy || char_lower(ux) == char_lower(uy)
        })
}

/// `Character.toUpperCase(char)` of a UTF-16 unit (JDK 25 tables).
pub(crate) fn char_upper(c: u16) -> u16 {
    character::to_upper_case_unit(c)
}

/// `Character.toLowerCase(char)` of a UTF-16 unit (JDK 25 tables).
pub(crate) fn char_lower(c: u16) -> u16 {
    character::to_lower_case_unit(c)
}

/// `Character.isWhitespace(char)` of a UTF-16 unit (JDK 25 tables).
pub(crate) fn is_java_whitespace(c: u16) -> bool {
    character::is_whitespace(c as i32)
}

/// `Character.isLetterOrDigit(char)` of a UTF-16 unit (JDK 25 tables).
pub(crate) fn is_letter_or_digit(c: u16) -> bool {
    character::is_letter(c as i32) || character::is_digit(c as i32)
}

/// `String.trim()`: strips the leading and trailing units `<= ' '`.
pub fn trim(s: &str) -> String {
    s.trim_matches(|c: char| c <= ' ').to_string()
}

/// `new IOException(message)` of the Java class `class` (an `IOException`
/// or one of its subclasses), as an [`Exception::Io`].
pub fn io_exception(class: &'static str, message: Option<String>) -> Exception {
    Exception::io(LuceneError::from(rucene::error::LuceneErrorKind::Io(
        rucene::store::java_nio_file::JavaIoException::new(class, message).into_io_error(),
    )))
}

/// `Integer.parseInt(String)`: an optional sign then decimal digits
/// (`Character.digit(c, 10)`, which accepts every Unicode decimal digit);
/// `None` where Java throws `NumberFormatException`.
pub fn parse_int(s: &str) -> Option<i32> {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.is_empty() {
        return None;
    }
    let (negative, start) = match units[0] {
        0x2D => (true, 1),
        0x2B => (false, 1),
        _ => (false, 0),
    };
    if start == units.len() {
        return None;
    }
    let mut result: i64 = 0;
    for &u in &units[start..] {
        if !character::is_digit(u as i32) {
            return None;
        }
        let d = character::get_numeric_value(u as i32);
        if !(0..=9).contains(&d) {
            return None;
        }
        result = result * 10 + d as i64;
        if result > i32::MAX as i64 + 1 {
            return None;
        }
    }
    let result = if negative { -result } else { result };
    i32::try_from(result).ok()
}

/// A `java.util.Hashtable` with `String` keys, reproducing its iteration
/// order (`keys()`): initial capacity 11, load factor 0.75, rehash to
/// `2n + 1`, new entries at the head of their bucket, enumeration from the
/// last bucket to the first.
#[derive(Debug, Clone)]
pub struct JavaHashtable<V> {
    /// Each bucket's chain, its head first.
    table: Vec<Vec<(String, V)>>,
    count: usize,
    threshold: usize,
}

impl<V> Default for JavaHashtable<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> JavaHashtable<V> {
    /// `new Hashtable()`.
    pub fn new() -> Self {
        Self {
            table: (0..11).map(|_| Vec::new()).collect(),
            count: 0,
            threshold: 8,
        }
    }

    fn index(key: &str, len: usize) -> usize {
        ((string_hash_code(key) & 0x7FFF_FFFF) as usize) % len
    }

    /// `get(Object)`.
    pub fn get(&self, key: &str) -> Option<&V> {
        let i = Self::index(key, self.table.len());
        self.table[i].iter().find(|e| e.0 == key).map(|e| &e.1)
    }

    /// `put(K, V)`.
    pub fn put(&mut self, key: &str, value: V) -> Option<V> {
        let i = Self::index(key, self.table.len());
        if let Some(e) = self.table[i].iter_mut().find(|e| e.0 == key) {
            return Some(std::mem::replace(&mut e.1, value));
        }
        if self.count >= self.threshold {
            self.rehash();
        }
        let i = Self::index(key, self.table.len());
        self.table[i].insert(0, (key.to_string(), value));
        self.count += 1;
        None
    }

    fn rehash(&mut self) {
        let old = std::mem::take(&mut self.table);
        let new_capacity = (old.len() << 1) + 1;
        self.threshold = (new_capacity as f32 * 0.75) as usize;
        self.table = (0..new_capacity).map(|_| Vec::new()).collect();
        for chain in old.into_iter().rev() {
            for e in chain {
                let i = Self::index(&e.0, new_capacity);
                self.table[i].insert(0, e);
            }
        }
    }

    /// `remove(Object)`.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        let i = Self::index(key, self.table.len());
        let p = self.table[i].iter().position(|e| e.0 == key)?;
        self.count -= 1;
        Some(self.table[i].remove(p).1)
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        for chain in self.table.iter_mut() {
            chain.clear();
        }
        self.count = 0;
    }

    /// `keys()`, in the enumeration order.
    pub fn keys(&self) -> Vec<String> {
        self.iter().map(|(k, _)| k.clone()).collect()
    }

    /// The entries in the enumeration order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.table.iter().rev().flat_map(|c| c.iter().map(|e| (&e.0, &e.1)))
    }

    /// `size()`.
    pub fn len(&self) -> usize {
        self.count
    }

    /// `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}
