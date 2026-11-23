// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CacheValue`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use rucene::error::Result;

use crate::util::icu_exception::ICUException;

/// `CacheValue.Strength`: whether new cache values hold their objects
/// strongly or softly.
///
/// Equivalent to the enum `com.ibm.icu.impl.CacheValue.Strength`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strength {
    /// `STRONG`: subsequent cache values hold their objects strongly.
    Strong,
    /// `SOFT`: subsequent cache values hold their objects softly (the
    /// default).
    Soft,
}

/// `strength` (private static volatile): `0` for `STRONG`, `1` for `SOFT`.
static STRENGTH: AtomicU8 = AtomicU8::new(1);

/// Value type for cache items: a null value, a strongly held value or a
/// softly held one.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.CacheValue<V>` with
/// its private subclasses `NullValue`, `StrongValue` and `SoftValue`.
///
/// A `SoftValue` holds its object through a `SoftReference`, which the JVM
/// may clear when memory runs low; `resetIfCleared` then installs a new
/// object. A Rust process has no collector to clear it, so the soft value
/// keeps its object until it is dropped. That is one of the behaviours the
/// `SoftReference` contract allows (a softly reachable object may be kept
/// as long as memory permits), so `get` never answers `null` for a non-null
/// value, and `reset_if_cleared` returns the held object.
#[derive(Debug)]
pub enum CacheValue<V> {
    /// `NullValue`: the cached value is `null`.
    Null,
    /// `StrongValue`.
    Strong(V),
    /// `SoftValue`: the referent, `None` once cleared.
    Soft(Mutex<Option<V>>),
}

impl<V: Clone> CacheValue<V> {
    /// `setStrength(Strength strength)`: the strength of future values.
    pub fn set_strength(strength: Strength) {
        STRENGTH.store(
            match strength {
                Strength::Strong => 0,
                Strength::Soft => 1,
            },
            Ordering::SeqCst,
        );
    }

    /// `futureInstancesWillBeStrong()`.
    pub fn future_instances_will_be_strong() -> bool {
        STRENGTH.load(Ordering::SeqCst) == 0
    }

    /// `getInstance(V value)`: the null value for `None`, else a strong or
    /// soft value per the current strength.
    pub fn get_instance(value: Option<V>) -> CacheValue<V> {
        match value {
            None => CacheValue::Null,
            Some(v) => {
                if Self::future_instances_will_be_strong() {
                    CacheValue::Strong(v)
                } else {
                    CacheValue::Soft(Mutex::new(Some(v)))
                }
            }
        }
    }

    /// `isNull()`: whether this is the null value.
    pub fn is_null(&self) -> bool {
        matches!(self, CacheValue::Null)
    }

    /// `get()`: the value, `None` for the null value or a cleared soft
    /// value.
    pub fn get(&self) -> Option<V> {
        match self {
            CacheValue::Null => None,
            CacheValue::Strong(v) => Some(v.clone()),
            CacheValue::Soft(r) => r.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }

    /// `resetIfCleared(V value)`: if the value was cleared, sets it to
    /// `value` and returns `value`; otherwise returns the held value.
    ///
    /// # Errors
    ///
    /// `ICUException("resetting a null value to a non-null value")` when a
    /// null value is reset to a non-null one.
    pub fn reset_if_cleared(&self, value: Option<V>) -> Result<Option<V>> {
        match self {
            CacheValue::Null => {
                if value.is_some() {
                    return Err(ICUException::with_message(
                        "resetting a null value to a non-null value",
                    )
                    .error());
                }
                Ok(None)
            }
            // `value` is ignored: a strong value is never cleared.
            CacheValue::Strong(v) => Ok(Some(v.clone())),
            CacheValue::Soft(r) => {
                let mut r = r.lock().unwrap_or_else(|e| e.into_inner());
                match &*r {
                    None => {
                        *r = value.clone();
                        Ok(value)
                    }
                    // The caller lost a race to set the value; return the
                    // existing one.
                    Some(old) => Ok(Some(old.clone())),
                }
            }
        }
    }
}
