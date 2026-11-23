//! This crate ports, from LocationTech spatial4j 0.8
//! (`org.locationtech.spatial4j:spatial4j:0.8`, tag `spatial4j-0.8`,
//! Apache-2.0), the types Apache Lucene 10.5.0 `lucene/spatial-extras` and
//! `lucene/benchmark` import and their transitive closure over the library's
//! main code, plus the test helpers `lucene/spatial-extras`' tests import
//! (`TestLog`, `shape::RandomizedShapeTest`,
//! `shape::RectIntersectionTestHelper`). The rest of the library is out of scope
//! (user decision, sprint 17). Each file keeps its upstream copyright and
//! license notice.
//!
//! **Divergence.** Java's shapes are mutable objects shared by reference:
//! `Point.reset`, `Rectangle.reset` and `Circle.reset` change a shape in place,
//! and `getCenter()` / `getBoundingBox()` may return `this`. Here a shape is
//! shared as `Arc<dyn Shape>` and keeps its mutable state in atomics, so
//! `reset` takes `&self` and every holder of the `Arc` sees the change, as in
//! Java. A method that returns `this` in Java returns an `Arc` of the same
//! object where the shape can name itself (it keeps a weak self reference);
//! that is the case for every `Point`, `Rectangle` and `Circle` of this crate.
//!
//! **Divergence.** Java's unchecked exceptions surface as
//! [`rucene::error::LuceneError`] values in a `Result` on every method whose
//! Java contract throws them on bad input (the shape factory, the readers,
//! the codecs, `getBuffered`). Methods whose Java body throws only on a
//! broken invariant (an `assert`, a `ClassCastException` in `equals`, an
//! `InvalidShapeException` from a factory call on values it built itself)
//! panic with the Java exception text instead, so that `relate`,
//! `getBoundingBox` and `getCenter` keep their infallible Java signatures.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.locationtech.spatial4j`: `org.locationtech.spatial4j.shape.impl` is the
//! module `shape::r#impl`. Each Java source file is one module, named in
//! snake_case (`SpatialContext.java` is [`context::spatial_context`]); a Java
//! package name that is not snake_case becomes a snake_case module, and one that
//! is a Rust keyword a raw identifier. A nested Java type is declared in the
//! module of its source file under its own name; the name of its enclosing type
//! is prefixed only when its own name is a name of the Rust prelude or the name
//! of another item of that module, and the Java kind is appended when the
//! prefixed name is itself taken.

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

/// Reproductions of the JDK behaviours the port relies on.
pub(crate) mod jdk {
    use std::sync::atomic::{AtomicU64, Ordering};

    use rucene::error::{LuceneError, LuceneErrorKind, Result};

    /// A `double` field that can be reassigned through a shared reference,
    /// as Java's mutable shape fields are. Plain (relaxed) loads and stores:
    /// Java's fields are not `volatile` either.
    #[derive(Debug, Default)]
    pub(crate) struct AtomicF64(AtomicU64);

    impl AtomicF64 {
        pub(crate) fn new(v: f64) -> Self {
            Self(AtomicU64::new(v.to_bits()))
        }

        pub(crate) fn get(&self) -> f64 {
            f64::from_bits(self.0.load(Ordering::Relaxed))
        }

        pub(crate) fn set(&self, v: f64) {
            self.0.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    impl Clone for AtomicF64 {
        fn clone(&self) -> Self {
            Self::new(self.get())
        }
    }

    /// `Double.doubleToLongBits(double)`.
    pub(crate) fn double_to_long_bits(v: f64) -> i64 {
        if v.is_nan() {
            0x7ff8000000000000
        } else {
            v.to_bits() as i64
        }
    }

    /// The `temp = v != +0.0d ? Double.doubleToLongBits(v) : 0L; (int)
    /// (temp ^ (temp >>> 32))` term of the generated `hashCode()` methods.
    pub(crate) fn hash_double(v: f64) -> i32 {
        let temp = if v != 0.0 { double_to_long_bits(v) } else { 0 };
        (temp ^ ((temp as u64) >> 32) as i64) as i32
    }

    /// `Double.compare(a, b) != 0`.
    pub(crate) fn double_compare_ne(a: f64, b: f64) -> bool {
        rucene::util::java_double_compare(a, b) != std::cmp::Ordering::Equal
    }

    /// `Math.round(double)`.
    pub(crate) fn java_round(a: f64) -> i64 {
        if a.is_nan() {
            return 0;
        }
        let f = a.floor();
        let r = if a - f >= 0.5 { f + 1.0 } else { f };
        r as i64
    }

    /// `Double.parseDouble(String)`, with Java's `NumberFormatException`.
    pub(crate) fn parse_double(s: &str) -> Result<f64> {
        if s.trim_matches(|c: char| c <= ' ').is_empty() {
            return Err(LuceneError::number_format("empty String"));
        }
        rucene::util::java_fmt::parse_double(s)
            .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
    }

    /// `new <class>(message, cause)` for an unchecked Java exception.
    pub(crate) fn unchecked_with_cause(
        class: &'static str,
        message: Option<String>,
        cause: Option<LuceneError>,
    ) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Unchecked {
            class,
            message,
            cause: cause.map(std::sync::Arc::new),
        })
    }

    /// `Character.isDigit(char)`: a decimal digit (`Nd`).
    ///
    /// `Character.isLetter`, `Character.isWhitespace` and
    /// `Character.isJavaIdentifierPart` are the exact JDK 25 tables of
    /// [`rucene_analysis_common::jdk::character`], which the readers call
    /// directly.
    pub(crate) fn is_digit(c: char) -> bool {
        rucene::util::java_character_digit(c, 10).is_some()
    }

    /// `java.text.NumberFormat` as `LegacyShapeWriter.makeNumberFormat`
    /// builds it: `NumberFormat.getInstance(Locale.ROOT)` (`#,##0.###`)
    /// without grouping, at most `fraction_digits` fraction digits.
    pub(crate) fn make_number_format(
        fraction_digits: usize,
    ) -> rucene::util::java_fmt::DecimalFormat {
        let pattern = if fraction_digits == 0 {
            "0".to_string()
        } else {
            format!("0.{}", "#".repeat(fraction_digits))
        };
        rucene::util::java_fmt::DecimalFormat::new(&pattern)
            .expect("a literal DecimalFormat pattern is valid")
    }
}

pub mod context;
pub mod distance;
pub mod exception;
pub mod io;
pub mod java_map;
pub mod shape;
#[cfg(feature = "test-helpers")]
pub mod test_log;
