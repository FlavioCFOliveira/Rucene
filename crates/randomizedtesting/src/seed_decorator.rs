//! Port of `com.carrotsearch.randomizedtesting.SeedDecorator`
//! (randomizedtesting 2.8.1, `SeedDecorator.java`).

/// Modifies the initial seed of a `Randomness` (see the `SeedDecorators`
/// annotation): `SeedDecorator`.
///
/// **Divergence.** Java's `initialize` takes the suite's
/// `Class<?>`; the port takes the suite's binary class name (what
/// `Class.getName()` returns), the only part of the class the shipped
/// decorators read. A Rust suite that passes the Java suite's name derives
/// the same seeds as Java.
pub trait SeedDecorator: Send + Sync {
    /// `initialize(Class<?> suiteClass)`: called once after instantiation.
    fn initialize(&mut self, suite_class_name: &str);

    /// `decorate(long seed)`.
    fn decorate(&self, seed: i64) -> i64;
}
