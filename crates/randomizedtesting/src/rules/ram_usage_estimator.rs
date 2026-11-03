//! Port of `com.carrotsearch.randomizedtesting.rules.RamUsageEstimator`
//! (randomizedtesting 2.8.1, `rules/RamUsageEstimator.java`), with the
//! package-private `IdentityHashSet` and `Constants` it declares.
//!
//! **Divergences.**
//! * Java measures the JVM's layout (`sun.misc.Unsafe` for the reference
//!   size, header sizes and field offsets, the HotSpot diagnostic bean for
//!   the alignment; `RamUsageEstimator.java:145`). The port has no JVM: it uses the layout of a 64-bit
//!   HotSpot JVM with compressed references and without compact headers
//!   (reference 4 bytes, object header 12, array header 16, alignment 8),
//!   JDK 25's default for heaps under 32 GiB, and sizes fields without
//!   offsets (Java's fallback branch of `adjustForField`,
//!   `RamUsageEstimator.java:623`).
//! * Java walks any object graph by reflection, and on JDK 9+ fails with
//!   `IllegalStateException` on a class of a JDK module whose reference
//!   fields it may not open (a `String`, a `HashMap`;
//!   `RamUsageEstimator.java:576`). The port measures
//!   the [`Value`]s that have a Java layout it can state: `null`, boxed
//!   primitives, primitive arrays and `Object[]`; any other value (a
//!   string, a list, an opaque Rust object) fails as an inaccessible JDK
//!   object does.

use std::sync::Arc;

use crate::jdk::exception::{Throwable, ILLEGAL_STATE_EXCEPTION};
use crate::jdk::reflect::{modifier, Class, Value};

/// JVM features the estimator may measure (`RamUsageEstimator.JvmFeature`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JvmFeature {
    /// `OBJECT_REFERENCE_SIZE`.
    ObjectReferenceSize,
    /// `ARRAY_HEADER_SIZE`.
    ArrayHeaderSize,
    /// `FIELD_OFFSETS`.
    FieldOffsets,
    /// `OBJECT_ALIGNMENT`.
    ObjectAlignment,
}

impl JvmFeature {
    /// `description`.
    pub fn description(&self) -> &'static str {
        match self {
            JvmFeature::ObjectReferenceSize => {
                "Object reference size estimated using array index scale"
            }
            JvmFeature::ArrayHeaderSize => "Array header size estimated using array based offset",
            JvmFeature::FieldOffsets => "Shallow instance size based on field offsets",
            JvmFeature::ObjectAlignment => {
                "Object alignment retrieved from HotSpotDiagnostic MX bean"
            }
        }
    }
}

/// Estimates the memory a value occupies on the JVM (`RamUsageEstimator`).
pub struct RamUsageEstimator;

impl RamUsageEstimator {
    /// `ONE_KB`.
    pub const ONE_KB: i64 = 1024;
    /// `ONE_MB`.
    pub const ONE_MB: i64 = 1024 * 1024;
    /// `ONE_GB`.
    pub const ONE_GB: i64 = 1024 * 1024 * 1024;
    /// `NUM_BYTES_BOOLEAN`.
    pub const NUM_BYTES_BOOLEAN: i64 = 1;
    /// `NUM_BYTES_BYTE`.
    pub const NUM_BYTES_BYTE: i64 = 1;
    /// `NUM_BYTES_CHAR`.
    pub const NUM_BYTES_CHAR: i64 = 2;
    /// `NUM_BYTES_SHORT`.
    pub const NUM_BYTES_SHORT: i64 = 2;
    /// `NUM_BYTES_INT`.
    pub const NUM_BYTES_INT: i64 = 4;
    /// `NUM_BYTES_FLOAT`.
    pub const NUM_BYTES_FLOAT: i64 = 4;
    /// `NUM_BYTES_LONG`.
    pub const NUM_BYTES_LONG: i64 = 8;
    /// `NUM_BYTES_DOUBLE`.
    pub const NUM_BYTES_DOUBLE: i64 = 8;
    /// `NUM_BYTES_OBJECT_REF` (see the module documentation).
    pub const NUM_BYTES_OBJECT_REF: i64 = 4;
    /// `NUM_BYTES_OBJECT_HEADER`.
    pub const NUM_BYTES_OBJECT_HEADER: i64 = 12;
    /// `NUM_BYTES_ARRAY_HEADER`.
    pub const NUM_BYTES_ARRAY_HEADER: i64 = 16;
    /// `NUM_BYTES_OBJECT_ALIGNMENT`.
    pub const NUM_BYTES_OBJECT_ALIGNMENT: i64 = 8;

    /// `primitiveSizes.get(type)`.
    pub fn primitive_size(type_name: &str) -> Option<i64> {
        Some(match type_name {
            "boolean" => Self::NUM_BYTES_BOOLEAN,
            "byte" => Self::NUM_BYTES_BYTE,
            "char" => Self::NUM_BYTES_CHAR,
            "short" => Self::NUM_BYTES_SHORT,
            "int" => Self::NUM_BYTES_INT,
            "float" => Self::NUM_BYTES_FLOAT,
            "double" => Self::NUM_BYTES_DOUBLE,
            "long" => Self::NUM_BYTES_LONG,
            _ => return None,
        })
    }

    /// `getSupportedFeatures()`: the port measures none of them; the
    /// layout is the fixed one of the module documentation.
    pub fn get_supported_features() -> Vec<JvmFeature> {
        Vec::new()
    }

    /// `getUnsupportedFeatures()`.
    pub fn get_unsupported_features() -> Vec<JvmFeature> {
        vec![
            JvmFeature::ObjectReferenceSize,
            JvmFeature::ArrayHeaderSize,
            JvmFeature::FieldOffsets,
            JvmFeature::ObjectAlignment,
        ]
    }

    /// `isSupportedJVM()`.
    pub fn is_supported_jvm() -> bool {
        Self::get_unsupported_features().is_empty()
    }

    /// `alignObjectSize(size)`: rounds up to the object alignment.
    pub fn align_object_size(size: i64) -> i64 {
        let size = size + Self::NUM_BYTES_OBJECT_ALIGNMENT - 1;
        size - (size % Self::NUM_BYTES_OBJECT_ALIGNMENT)
    }

    /// `sizeOf(<component>[] arr)` for an array of `len` elements.
    pub fn size_of_primitive_array(component: &str, len: usize) -> i64 {
        let size = Self::primitive_size(component).unwrap_or(Self::NUM_BYTES_OBJECT_REF);
        Self::align_object_size(Self::NUM_BYTES_ARRAY_HEADER + size * len as i64)
    }

    /// `sizeOf(Object)`: the value and everything it references.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`Unable to access '...' to estimate memory
    /// usage`) for a value the estimator cannot walk.
    pub fn size_of(obj: &Value) -> Result<i64, Throwable> {
        Self::measure_size_of(vec![obj.clone()])
    }

    /// `sizeOfAll(objects)`: references shared between them count once.
    ///
    /// # Errors
    ///
    /// As [`size_of`](Self::size_of).
    pub fn size_of_all(objects: &[Value]) -> Result<i64, Throwable> {
        Self::measure_size_of(objects.to_vec())
    }

    /// `shallowSizeOf(Object)`: the value alone.
    ///
    /// # Errors
    ///
    /// As [`size_of`](Self::size_of).
    pub fn shallow_size_of(obj: &Value) -> Result<i64, Throwable> {
        Ok(match obj {
            Value::Null | Value::Void => 0,
            Value::PrimitiveArray(a) => Self::size_of_primitive_array(a.component, a.len),
            Value::Array(v) => Self::shallow_size_of_object_array(v.len()),
            other => Self::boxed_size(other).ok_or_else(|| inaccessible(other))?,
        })
    }

    /// `shallowSizeOfAll(objects)`.
    ///
    /// # Errors
    ///
    /// As [`size_of`](Self::size_of).
    pub fn shallow_size_of_all(objects: &[Value]) -> Result<i64, Throwable> {
        objects.iter().map(Self::shallow_size_of).sum()
    }

    /// `shallowSizeOfInstance(clazz)`: the header plus the instance fields
    /// of the class and its superclasses, aligned.
    pub fn shallow_size_of_instance(clazz: &Class) -> i64 {
        let mut size = Self::NUM_BYTES_OBJECT_HEADER;
        let mut c = Some(clazz);
        while let Some(cls) = c {
            for f in cls.declared_fields() {
                if f.get_modifiers() & modifier::STATIC == 0 {
                    size +=
                        Self::primitive_size(f.type_name()).unwrap_or(Self::NUM_BYTES_OBJECT_REF);
                }
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        Self::align_object_size(size)
    }

    fn shallow_size_of_object_array(len: usize) -> i64 {
        Self::align_object_size(
            Self::NUM_BYTES_ARRAY_HEADER + Self::NUM_BYTES_OBJECT_REF * len as i64,
        )
    }

    /// The size of a boxed primitive (`Integer`, `Long`, …): header plus
    /// its one field, aligned.
    fn boxed_size(v: &Value) -> Option<i64> {
        let field = match v {
            Value::Bool(_) => Self::NUM_BYTES_BOOLEAN,
            Value::Int(_) => Self::NUM_BYTES_INT,
            Value::Long(_) => Self::NUM_BYTES_LONG,
            Value::Double(_) => Self::NUM_BYTES_DOUBLE,
            _ => return None,
        };
        Some(Self::align_object_size(
            Self::NUM_BYTES_OBJECT_HEADER + field,
        ))
    }

    /// `measureSizeOf(stack)`: a depth-first walk with an identity set.
    fn measure_size_of(mut stack: Vec<Value>) -> Result<i64, Throwable> {
        let mut seen: IdentityHashSet = IdentityHashSet::default();
        let mut total = 0;
        while let Some(ob) = stack.pop() {
            if matches!(ob, Value::Null | Value::Void) || seen.contains(&ob) {
                continue;
            }
            seen.add(&ob);
            match &ob {
                Value::PrimitiveArray(a) => {
                    total += Self::size_of_primitive_array(a.component, a.len)
                }
                Value::Array(v) => {
                    total += Self::shallow_size_of_object_array(v.len());
                    for o in v.iter().rev() {
                        if !matches!(o, Value::Null) && !seen.contains(o) {
                            stack.push(o.clone());
                        }
                    }
                }
                other => match Self::boxed_size(other) {
                    Some(s) => total += s,
                    None => return Err(inaccessible(other)),
                },
            }
        }
        Ok(total)
    }

    /// `humanReadableUnits(bytes)`: `bytes`, `KB`, `MB` or `GB` with at
    /// most one decimal (`DecimalFormat("0.#")`, half-even).
    pub fn human_readable_units(bytes: i64) -> String {
        let fmt = |v: f32| decimal_format_0_hash(f64::from(v));
        if bytes / Self::ONE_GB > 0 {
            format!("{} GB", fmt(bytes as f32 / Self::ONE_GB as f32))
        } else if bytes / Self::ONE_MB > 0 {
            format!("{} MB", fmt(bytes as f32 / Self::ONE_MB as f32))
        } else if bytes / Self::ONE_KB > 0 {
            format!("{} KB", fmt(bytes as f32 / Self::ONE_KB as f32))
        } else {
            format!("{bytes} bytes")
        }
    }

    /// `humanSizeOf(object)`.
    ///
    /// # Errors
    ///
    /// As [`size_of`](Self::size_of).
    pub fn human_size_of(object: &Value) -> Result<String, Throwable> {
        Ok(Self::human_readable_units(Self::size_of(object)?))
    }
}

/// `new DecimalFormat("0.#").format(v)`: the shortest decimal digits of `v`
/// rounded half-even to one fraction digit, without a trailing `.0`.
fn decimal_format_0_hash(v: f64) -> String {
    let s = format!("{v}");
    let (int_part, frac) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.clone(), String::new()),
    };
    let mut digits: Vec<u8> = int_part.bytes().map(|b| b - b'0').collect();
    let frac_digits: Vec<u8> = frac.bytes().map(|b| b - b'0').collect();
    let first = frac_digits.first().copied().unwrap_or(0);
    let rest = &frac_digits[frac_digits.len().min(1)..];
    let mut kept = first;
    let round_up = match rest.first() {
        None => false,
        Some(&d) if d > 5 => true,
        Some(&d) if d < 5 => false,
        Some(_) => rest[1..].iter().any(|&d| d != 0) || kept % 2 == 1,
    };
    if round_up {
        kept += 1;
        if kept == 10 {
            kept = 0;
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, 1);
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    }
    let mut out: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    if kept != 0 {
        out.push('.');
        out.push(char::from(b'0' + kept));
    }
    out
}

/// The failure of `createCacheEntry` on a value the estimator cannot
/// walk: Java's `IllegalStateException`, caused by the
/// `InaccessibleObjectException` of `Field.setAccessible` on a field of a
/// JDK module. The unnamed module's identity hash, which Java prints, is
/// `0` here.
fn inaccessible(v: &Value) -> Throwable {
    let (field, package) = match v {
        Value::Str(_) => ("private final byte[] java.lang.String.value", "java.lang"),
        Value::List(_) => (
            "transient java.lang.Object[] java.util.ArrayList.elementData",
            "java.util",
        ),
        _ => ("a field of an opaque object", "java.lang"),
    };
    Throwable::with_cause(
        ILLEGAL_STATE_EXCEPTION,
        Some(format!("Unable to access '{field}' to estimate memory usage")),
        Throwable::new(
            "java.lang.reflect.InaccessibleObjectException",
            format!(
                "Unable to make field {field} accessible: module java.base does not \"opens {package}\" to unnamed module @0"
            ),
        ),
    )
}

/// An identity set (`IdentityHashSet`, package-private): values are the
/// same when they share their `Arc`.
#[derive(Default)]
pub(crate) struct IdentityHashSet {
    items: Vec<*const ()>,
}

fn identity(v: &Value) -> Option<*const ()> {
    match v {
        Value::PrimitiveArray(a) => Some(Arc::as_ptr(a) as *const ()),
        Value::Object(o, _) => Some(Arc::as_ptr(o) as *const ()),
        _ => None,
    }
}

impl IdentityHashSet {
    /// `add(e)`.
    pub(crate) fn add(&mut self, v: &Value) -> bool {
        match identity(v) {
            Some(p) if !self.items.contains(&p) => {
                self.items.push(p);
                true
            }
            _ => false,
        }
    }

    /// `contains(e)`: values without identity (boxed primitives, `Object[]`
    /// copies) are never contained.
    pub(crate) fn contains(&self, v: &Value) -> bool {
        identity(v).is_some_and(|p| self.items.contains(&p))
    }
}

/// `Constants` (package-private): facts about the platform.
#[allow(dead_code)] // Java declares them; the estimator reads only JRE_IS_64BIT.
pub(crate) struct Constants;

#[allow(dead_code)] // Java declares them; the estimator reads only JRE_IS_64BIT.
impl Constants {
    /// `JRE_IS_64BIT`.
    pub(crate) const JRE_IS_64BIT: bool = cfg!(target_pointer_width = "64");
}
