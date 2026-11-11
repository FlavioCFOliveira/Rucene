//! Port of `com.carrotsearch.randomizedtesting.rules.RamUsageEstimator`
//! (randomizedtesting 2.8.1, `rules/RamUsageEstimator.java`), with the
//! package-private `IdentityHashSet`, `MurmurHash3` and `Constants` it
//! declares.
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
//!   string, a list, a JUnit `RuleChain`, which holds an `ArrayList`, a
//!   path, an opaque Rust object) fails as an inaccessible JDK object does.

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
        // A `List`, or a JUnit `RuleChain`, whose rule list is an
        // `ArrayList` (`RuleChain.rulesStartingWithInnerMost`).
        Value::List(_) => (
            "transient java.lang.Object[] java.util.ArrayList.elementData",
            "java.util",
        ),
        Value::TestRule(r) if r.java_class() == Some("org.junit.rules.RuleChain") => (
            "transient java.lang.Object[] java.util.ArrayList.elementData",
            "java.util",
        ),
        // A `java.nio.file.Path` (`RandomizedTest.globalTempDir`): the
        // platform's `sun.nio.fs` path, whose first reference field is `fs`.
        Value::Object(o, _) if o.downcast_ref::<std::path::PathBuf>().is_some() => {
            if cfg!(windows) {
                (
                    "private final sun.nio.fs.WindowsFileSystem sun.nio.fs.WindowsPath.fs",
                    "sun.nio.fs",
                )
            } else {
                (
                    "private final sun.nio.fs.UnixFileSystem sun.nio.fs.UnixPath.fs",
                    "sun.nio.fs",
                )
            }
        }
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

/// Port of `com.carrotsearch.randomizedtesting.rules.IdentityHashSet`.
///
/// An identity hash set implemented with open addressing and linear probing
/// (package-private `IdentityHashSet<KType>`): two values are the same when
/// they share their `Arc`. Values without an identity of their own (`null`,
/// boxed primitives, which the port stores by value) are never added nor
/// contained.
///
/// **Divergence.** Java hashes `System.identityHashCode(o)`
/// (`rules/RamUsageEstimator.java:787`), a JVM-assigned number. Rust has no
/// such number; the port hashes the folded address of the shared allocation,
/// which identifies the value for as long as the set holds it. Only the slot
/// order (the iteration order) depends on it, and it is no more stable in
/// Java across runs.
pub(crate) struct IdentityHashSet {
    /// `keys`: every slot, always a power of two of them.
    keys: Vec<Option<*const ()>>,
    /// `assigned`: the number of occupied slots.
    assigned: i32,
    /// `loadFactor`.
    #[allow(dead_code)] // Java stores it; resizing reads DEFAULT_LOAD_FACTOR.
    load_factor: f32,
    /// `resizeThreshold`.
    resize_threshold: i32,
}

fn identity(v: &Value) -> Option<*const ()> {
    match v {
        Value::PrimitiveArray(a) => Some(Arc::as_ptr(a) as *const ()),
        Value::Object(o, _) => Some(Arc::as_ptr(o) as *const ()),
        _ => None,
    }
}

impl Default for IdentityHashSet {
    /// `new IdentityHashSet()`.
    fn default() -> Self {
        IdentityHashSet::new()
    }
}

#[allow(dead_code)] // Java declares the full set API; the estimator uses add and contains.
impl IdentityHashSet {
    /// `DEFAULT_LOAD_FACTOR`.
    pub(crate) const DEFAULT_LOAD_FACTOR: f32 = 0.75;
    /// `MIN_CAPACITY`.
    pub(crate) const MIN_CAPACITY: i32 = 4;

    /// `new IdentityHashSet()`: capacity 16, the default load factor.
    pub(crate) fn new() -> Self {
        Self::with_capacity_and_load_factor(16, Self::DEFAULT_LOAD_FACTOR)
    }

    /// `new IdentityHashSet(initialCapacity)`.
    pub(crate) fn with_capacity(initial_capacity: i32) -> Self {
        Self::with_capacity_and_load_factor(initial_capacity, Self::DEFAULT_LOAD_FACTOR)
    }

    /// `new IdentityHashSet(initialCapacity, loadFactor)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` (Java's `assert` under `-ea`) when `load_factor` is
    /// outside `(0, 1)`.
    pub(crate) fn with_capacity_and_load_factor(initial_capacity: i32, load_factor: f32) -> Self {
        let initial_capacity = initial_capacity.max(Self::MIN_CAPACITY);
        crate::jdk::exception::java_assert(initial_capacity > 0, || {
            format!("Initial capacity must be between (0, {}].", i32::MAX)
        });
        crate::jdk::exception::java_assert(load_factor > 0.0 && load_factor < 1.0, || {
            "Load factor must be between (0, 1).".to_string()
        });
        let mut set = IdentityHashSet {
            keys: Vec::new(),
            assigned: 0,
            load_factor,
            resize_threshold: 0,
        };
        set.allocate_buffers(Self::round_capacity(initial_capacity));
        set
    }

    /// `add(e)`: whether `e` was absent.
    pub(crate) fn add(&mut self, v: &Value) -> bool {
        let Some(e) = identity(v) else {
            return false;
        };
        if self.assigned >= self.resize_threshold {
            self.expand_and_rehash();
        }
        let mask = self.keys.len() - 1;
        let mut slot = Self::rehash(e) as usize & mask;
        while let Some(existing) = self.keys[slot] {
            if existing == e {
                return false;
            }
            slot = (slot + 1) & mask;
        }
        self.assigned += 1;
        self.keys[slot] = Some(e);
        true
    }

    /// `contains(e)`.
    pub(crate) fn contains(&self, v: &Value) -> bool {
        let Some(e) = identity(v) else {
            return false;
        };
        let mask = self.keys.len() - 1;
        let mut slot = Self::rehash(e) as usize & mask;
        while let Some(existing) = self.keys[slot] {
            if existing == e {
                return true;
            }
            slot = (slot + 1) & mask;
        }
        false
    }

    /// `rehash(o)`: `MurmurHash3.hash(System.identityHashCode(o))` (see the
    /// divergence above).
    fn rehash(e: *const ()) -> i32 {
        let address = e as usize as u64;
        MurmurHash3::hash_int((address ^ (address >> 32)) as i32)
    }

    /// `expandAndRehash()`: doubles the slots and reinserts every key.
    fn expand_and_rehash(&mut self) {
        let old_keys = std::mem::take(&mut self.keys);
        crate::jdk::exception::java_assert_no_message(self.assigned >= self.resize_threshold);
        self.allocate_buffers(Self::next_capacity(old_keys.len() as i32));
        let mask = self.keys.len() - 1;
        for key in old_keys.into_iter().flatten() {
            let mut slot = Self::rehash(key) as usize & mask;
            while self.keys[slot].is_some() {
                slot = (slot + 1) & mask;
            }
            self.keys[slot] = Some(key);
        }
    }

    /// `allocateBuffers(capacity)`.
    fn allocate_buffers(&mut self, capacity: i32) {
        self.keys = vec![None; capacity as usize];
        self.resize_threshold = (capacity as f32 * Self::DEFAULT_LOAD_FACTOR) as i32;
    }

    /// `nextCapacity(current)`.
    fn next_capacity(current: i32) -> i32 {
        crate::jdk::exception::java_assert(current > 0 && current.count_ones() == 1, || {
            "Capacity must be a power of two.".to_string()
        });
        crate::jdk::exception::java_assert(current.wrapping_shl(1) > 0, || {
            format!("Maximum capacity exceeded ({}).", 0x8000_0000u32 >> 1)
        });
        let current = if current < Self::MIN_CAPACITY / 2 {
            Self::MIN_CAPACITY / 2
        } else {
            current
        };
        current << 1
    }

    /// `roundCapacity(requestedCapacity)`.
    fn round_capacity(requested_capacity: i32) -> i32 {
        const MAX: i32 = (0x8000_0000u32 >> 1) as i32;
        if requested_capacity > MAX {
            return MAX;
        }
        let mut capacity = Self::MIN_CAPACITY;
        while capacity < requested_capacity {
            capacity <<= 1;
        }
        capacity
    }

    /// `clear()`.
    pub(crate) fn clear(&mut self) {
        self.assigned = 0;
        self.keys.iter_mut().for_each(|k| *k = None);
    }

    /// `size()`.
    pub(crate) fn size(&self) -> i32 {
        self.assigned
    }

    /// `isEmpty()`.
    pub(crate) fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// `iterator()`: the identities of the keys, in slot order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = *const ()> + '_ {
        self.keys.iter().flatten().copied()
    }
}

/// Port of `com.carrotsearch.randomizedtesting.rules.MurmurHash3`.
///
/// Hash routines for primitive types, the finalization step of Austin
/// Appleby's MurmurHash3: the package-private `MurmurHash3` of the `rules`
/// package (`rules/RamUsageEstimator.java:914`), the same code as the root
/// package's `MurmurHash3`, which it runs.
pub(crate) struct MurmurHash3;

impl MurmurHash3 {
    /// `hash(int)`: hashes a 4-byte sequence.
    pub(crate) fn hash_int(k: i32) -> i32 {
        crate::murmur_hash3::MurmurHash3::hash_int(k)
    }

    /// `hash(long)`: hashes an 8-byte sequence.
    #[allow(dead_code)] // Java declares it; IdentityHashSet hashes ints.
    pub(crate) fn hash_long(k: i64) -> i64 {
        crate::murmur_hash3::MurmurHash3::hash_long(k)
    }
}

/// Port of `com.carrotsearch.randomizedtesting.rules.Constants`.
///
/// Some useful constants (package-private `Constants`): the platform's
/// pointer width and the JVM's system properties, read once.
#[allow(dead_code)] // Java declares them; the estimator reads only JRE_IS_64BIT.
pub(crate) struct Constants;

#[allow(dead_code)] // Java declares them; the estimator reads only JRE_IS_64BIT.
impl Constants {
    /// `JRE_IS_64BIT`: whether references are 64-bit (Java asks
    /// `sun.misc.Unsafe.addressSize()`; the port asks the target).
    pub(crate) const JRE_IS_64BIT: bool = cfg!(target_pointer_width = "64");

    fn property(
        cell: &'static std::sync::OnceLock<Option<String>>,
        key: &str,
    ) -> Option<&'static str> {
        cell.get_or_init(|| crate::jdk::system::get_property(key))
            .as_deref()
    }

    /// `JAVA_VERSION`: `System.getProperty("java.version")`.
    pub(crate) fn java_version() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "java.version")
    }

    /// `JAVA_VENDOR`: `System.getProperty("java.vendor")`.
    pub(crate) fn java_vendor() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "java.vendor")
    }

    /// `JVM_VENDOR`: `System.getProperty("java.vm.vendor")`.
    pub(crate) fn jvm_vendor() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "java.vm.vendor")
    }

    /// `JVM_VERSION`: `System.getProperty("java.vm.version")`.
    pub(crate) fn jvm_version() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "java.vm.version")
    }

    /// `JVM_NAME`: `System.getProperty("java.vm.name")`.
    pub(crate) fn jvm_name() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "java.vm.name")
    }

    /// `OS_ARCH`: `System.getProperty("os.arch")`.
    pub(crate) fn os_arch() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "os.arch")
    }

    /// `OS_VERSION`: `System.getProperty("os.version")`.
    pub(crate) fn os_version() -> Option<&'static str> {
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        Self::property(&V, "os.version")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `IdentityHashSet`: identity membership, growth past the resize
    /// threshold with every key rehashed, `clear`, and Java's capacity rules.
    #[test]
    fn identity_hash_set_open_addressing() {
        let values: Vec<Value> = (0..100)
            .map(|i| Value::primitive_array("byte", i))
            .collect();
        let mut set = IdentityHashSet::new();
        assert_eq!(set.keys.len(), 16);
        assert!(set.is_empty());
        for (i, v) in values.iter().enumerate() {
            assert!(set.add(v), "first add of {i}");
            assert!(!set.add(v), "second add of {i}");
            assert_eq!(set.size(), i as i32 + 1);
        }
        // 100 keys at load factor 0.75: 256 slots.
        assert_eq!(set.keys.len(), 256);
        for v in &values {
            assert!(set.contains(v));
            // An equal value with another identity is not contained.
            let Value::PrimitiveArray(a) = v else {
                unreachable!()
            };
            assert!(!set.contains(&Value::primitive_array(a.component, a.len)));
        }
        assert_eq!(set.iter().count(), 100);
        // Values without an identity of their own.
        assert!(!set.add(&Value::Int(1)));
        assert!(!set.contains(&Value::Null));
        set.clear();
        assert!(set.is_empty());
        assert!(values.iter().all(|v| !set.contains(v)));
        assert_eq!(IdentityHashSet::round_capacity(0), 4);
        assert_eq!(IdentityHashSet::round_capacity(17), 32);
        assert_eq!(IdentityHashSet::round_capacity(i32::MAX), 1 << 30);
        assert_eq!(IdentityHashSet::next_capacity(1), 4);
        assert_eq!(IdentityHashSet::next_capacity(16), 32);
        assert_eq!(IdentityHashSet::with_capacity(3).keys.len(), 4);
    }

    /// A JUnit `RuleChain` fails as on JDK 25, through its `ArrayList`: the
    /// messages a Java run of `TestStaticFieldsInvariantRule` reports when
    /// `Base.classRules` is the first unmeasurable field.
    #[test]
    fn rule_chain_is_unmeasurable_through_its_array_list() {
        let chain = Value::TestRule(Arc::new(crate::junit::RuleChain::empty_rule_chain()));
        let e = RamUsageEstimator::size_of(&chain).expect_err("a RuleChain is unmeasurable");
        assert_eq!(
            e.to_string(),
            "java.lang.IllegalStateException: Unable to access 'transient java.lang.Object[] \
             java.util.ArrayList.elementData' to estimate memory usage"
        );
        assert_eq!(
            e.cause().map(|c| c.to_string()).unwrap_or_default(),
            "java.lang.reflect.InaccessibleObjectException: Unable to make field transient \
             java.lang.Object[] java.util.ArrayList.elementData accessible: module java.base does \
             not \"opens java.util\" to unnamed module @0"
        );
    }

    /// `rules.MurmurHash3` is the root package's code.
    #[test]
    fn rules_murmur_hash3_equals_root() {
        for k in [0, 1, -1, i32::MIN, i32::MAX, 0x1234_5678] {
            assert_eq!(
                MurmurHash3::hash_int(k),
                crate::murmur_hash3::MurmurHash3::hash_int(k)
            );
            assert_eq!(
                MurmurHash3::hash_long(i64::from(k) << 17),
                crate::murmur_hash3::MurmurHash3::hash_long(i64::from(k) << 17)
            );
        }
        // fmix32 of 1 (Austin Appleby's MurmurHash3 finalizer).
        assert_eq!(MurmurHash3::hash_int(1) as u32, 0x514E_28B7);
    }

    /// `Constants`: unset JVM properties read `null`, the pointer width
    /// follows the target.
    #[test]
    fn constants_read_properties_once() {
        assert_eq!(Constants::JRE_IS_64BIT, cfg!(target_pointer_width = "64"));
        assert_eq!(
            Constants::java_version(),
            crate::jdk::system::get_property("java.version").as_deref()
        );
    }
}
