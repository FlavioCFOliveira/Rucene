//! Port of `generators/StringGenerator.java` (randomizedtesting 2.8.1).

use crate::jdk::random::Random;

/// Generates random strings of characters whose choice and distribution
/// depend on the implementation: `abstract class StringGenerator`.
///
/// Lengths are counted in UTF-16 code units ("code units", Java's `char`)
/// or in code points, as in Java; the returned `String` holds the same
/// characters.
pub trait StringGenerator: Send + Sync {
    /// `ofStringLength(r, min, max)`: an alias for
    /// [`of_code_units_length`](Self::of_code_units_length).
    fn of_string_length(&self, r: &dyn Random, min_code_units: i32, max_code_units: i32) -> String {
        self.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `ofCodeUnitsLength(r, min, max)`: a string of `[min, max]` UTF-16
    /// code units.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the generator cannot produce the
    /// length (for example an odd length from surrogate pairs only).
    fn of_code_units_length(
        &self,
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String;

    /// `ofCodePointsLength(r, min, max)`: a string of `[min, max]` code
    /// points (its UTF-16 length may exceed `max`).
    fn of_code_points_length(
        &self,
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String;
}
