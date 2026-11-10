//! Port of the `lucene/core/src/test/org/apache/lucene/internal/hppc/` test
//! classes of Lucene 10.5.0, each run as a suite by the ported runner:
//! `TestIntIntHashMap`, `TestIntLongHashMap`, `TestLongIntHashMap`, `TestIntFloatHashMap`, `TestIntDoubleHashMap`, `TestLongFloatHashMap`, `TestIntObjectHashMap`, `TestLongObjectHashMap`, `TestCharObjectHashMap`, `TestIntHashSet`, `TestLongHashSet`, `TestCharHashSet`, `TestIntArrayList`, `TestLongArrayList`, `TestFloatArrayList`, `TestMaxSizedIntArrayList`, `TestMaxSizedFloatArrayList`.

mod core_internal_hppc;
mod lucene_core_shims;
mod suite_seeds;

use lucene_core_shims::run;

#[test]
fn test_int_int_hash_map() {
    run(core_internal_hppc::test_int_int_hash_map::class);
}

#[test]
fn test_int_long_hash_map() {
    run(core_internal_hppc::test_int_long_hash_map::class);
}

#[test]
fn test_long_int_hash_map() {
    run(core_internal_hppc::test_long_int_hash_map::class);
}

#[test]
fn test_int_float_hash_map() {
    run(core_internal_hppc::test_int_float_hash_map::class);
}

#[test]
fn test_int_double_hash_map() {
    run(core_internal_hppc::test_int_double_hash_map::class);
}

#[test]
fn test_long_float_hash_map() {
    run(core_internal_hppc::test_long_float_hash_map::class);
}

#[test]
fn test_int_object_hash_map() {
    run(core_internal_hppc::test_int_object_hash_map::class);
}

#[test]
fn test_long_object_hash_map() {
    run(core_internal_hppc::test_long_object_hash_map::class);
}

#[test]
fn test_char_object_hash_map() {
    run(core_internal_hppc::test_char_object_hash_map::class);
}

#[test]
fn test_int_hash_set() {
    run(core_internal_hppc::test_int_hash_set::class);
}

#[test]
fn test_long_hash_set() {
    run(core_internal_hppc::test_long_hash_set::class);
}

#[test]
fn test_char_hash_set() {
    run(core_internal_hppc::test_char_hash_set::class);
}

#[test]
fn test_int_array_list() {
    run(core_internal_hppc::test_int_array_list::class);
}

#[test]
fn test_long_array_list() {
    run(core_internal_hppc::test_long_array_list::class);
}

#[test]
fn test_float_array_list() {
    run(core_internal_hppc::test_float_array_list::class);
}

#[test]
fn test_max_sized_int_array_list() {
    run(core_internal_hppc::test_max_sized_int_array_list::class);
}

#[test]
fn test_max_sized_float_array_list() {
    run(core_internal_hppc::test_max_sized_float_array_list::class);
}
