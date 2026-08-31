//! Port of `org.apache.lucene.internal.hppc.HashContainers`.
//!
//! Constants and sizing arithmetic shared by every open-addressing container in
//! this module. Lucene's class is package-private; the items here are `pub`
//! because the containers live in sibling modules, but they are not re-exported
//! from [`crate::internal::hppc`] and are not part of the crate's public
//! surface in spirit.

use std::sync::atomic::{AtomicI32, Ordering};

use super::buffer_allocation_exception::BufferAllocationException;
use crate::util::BitUtil;

/// Default number of elements a container is sized for.
pub const DEFAULT_EXPECTED_ELEMENTS: i32 = 4;

/// Default load factor of the hash containers.
pub const DEFAULT_LOAD_FACTOR: f32 = 0.75;

/// Minimal sane load factor (99 empty slots per 100).
pub const MIN_LOAD_FACTOR: f32 = 1.0 / 100.0;

/// Maximum sane load factor (1 empty slot per 100).
pub const MAX_LOAD_FACTOR: f32 = 99.0 / 100.0;

/// Minimum hash buffer size.
pub const MIN_HASH_ARRAY_LENGTH: i32 = 4;

/// Maximum array size for hash containers.
///
/// A power of two that is still allocable in Java without becoming a negative
/// `int`; the port keeps the same bound so that the containers accept and
/// reject exactly the same capacities as Lucene.
pub const MAX_HASH_ARRAY_LENGTH: i32 = (0x8000_0000_u32 >> 1) as i32;

/// Process-wide counter handing every new container a distinct iteration seed.
///
/// Equivalent of Java's `static final AtomicInteger ITERATION_SEED`.
static ITERATION_SEED: AtomicI32 = AtomicI32::new(0);

/// Equivalent of Java's `ITERATION_SEED.incrementAndGet()`.
///
/// Relaxed ordering is enough for the same reason Lucene gives for
/// `nextIterationSeed`: nothing depends on the value beyond each thread getting
/// a sequence of varying seeds.
#[inline]
pub fn next_iteration_seed() -> i32 {
    ITERATION_SEED
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
}

/// Returns the small odd stride used to walk the slots during iteration.
#[inline]
pub fn iteration_increment(seed: i32) -> i32 {
    29 + ((seed & 7) << 1)
}

/// Returns the size of the buffer that should replace one of `array_size`.
///
/// # Panics
///
/// Panics with a [`BufferAllocationException`] when the buffer already has the
/// maximum size, exactly where Lucene throws it.
pub fn next_buffer_size(array_size: i32, elements: i32, load_factor: f64) -> i32 {
    debug_assert!(check_power_of_two(array_size));
    if array_size == MAX_HASH_ARRAY_LENGTH {
        BufferAllocationException::new(format!(
            "Maximum array size exceeded for this load factor (elements: {elements}, load factor: {load_factor:.6})"
        ))
        .throw();
    }

    array_size << 1
}

/// Returns the number of assigned slots at which a buffer of `array_size` must
/// be rehashed.
pub fn expand_at_count(array_size: i32, load_factor: f64) -> i32 {
    debug_assert!(check_power_of_two(array_size));
    // Take care of the hash container invariant (there has to be at least one
    // empty slot to ensure the lookup loop finds either the element or an empty
    // slot).
    std::cmp::min(
        array_size - 1,
        (array_size as f64 * load_factor).ceil() as i32,
    )
}

/// Returns `true` if `array_size` is a power of two greater than one.
///
/// Mirrors Lucene's assertion helper: it only ever returns `true`, and reports
/// a violated invariant by failing its own debug assertions.
pub fn check_power_of_two(array_size: i32) -> bool {
    // These are internals, we can just assert without retrying.
    debug_assert!(array_size > 1);
    debug_assert!(BitUtil::next_highest_power_of_two(array_size) == array_size);
    true
}

/// Returns the smallest buffer size able to hold `elements` at `load_factor`.
///
/// # Panics
///
/// Panics when `elements` is negative (Java throws `IllegalArgumentException`),
/// and with a [`BufferAllocationException`] when the required size exceeds
/// [`MAX_HASH_ARRAY_LENGTH`].
pub fn min_buffer_size(elements: i32, load_factor: f64) -> i32 {
    if elements < 0 {
        panic!("Number of elements must be >= 0: {elements}");
    }

    let mut length = (elements as f64 / load_factor).ceil() as i64;
    if length == elements as i64 {
        length += 1;
    }
    length = std::cmp::max(
        MIN_HASH_ARRAY_LENGTH as i64,
        BitUtil::next_highest_power_of_two_long(length),
    );

    if length > MAX_HASH_ARRAY_LENGTH as i64 {
        BufferAllocationException::new(format!(
            "Maximum array size exceeded for this load factor (elements: {elements}, load factor: {load_factor:.6})"
        ))
        .throw();
    }

    length as i32
}

/// Validates that `load_factor` lies within the given inclusive range.
///
/// # Panics
///
/// Panics with a [`BufferAllocationException`] when it does not, exactly where
/// Lucene throws it.
pub fn check_load_factor(load_factor: f64, min_allowed_inclusive: f64, max_allowed_inclusive: f64) {
    if load_factor < min_allowed_inclusive || load_factor > max_allowed_inclusive {
        BufferAllocationException::new(format!(
            "The load factor should be in range [{min_allowed_inclusive:.2}, {max_allowed_inclusive:.2}]: {load_factor:.6}"
        ))
        .throw();
    }
}
