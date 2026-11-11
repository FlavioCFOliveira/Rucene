//! The ported `org.apache.lucene.internal.hppc` test classes, one module
//! per class, and their shared helpers.
//!
//! The ports keep Java's index loops and its declare-then-assign locals,
//! line by line.
#![allow(clippy::needless_range_loop, clippy::needless_late_init)]

pub mod support;

pub mod test_char_hash_set;
pub mod test_char_object_hash_map;
pub mod test_float_array_list;
pub mod test_int_array_list;
pub mod test_int_double_hash_map;
pub mod test_int_float_hash_map;
pub mod test_int_hash_set;
pub mod test_int_int_hash_map;
pub mod test_int_long_hash_map;
pub mod test_int_object_hash_map;
pub mod test_long_array_list;
pub mod test_long_float_hash_map;
pub mod test_long_hash_set;
pub mod test_long_int_hash_map;
pub mod test_long_object_hash_map;
pub mod test_max_sized_float_array_list;
pub mod test_max_sized_int_array_list;
