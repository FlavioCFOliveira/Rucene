//! Port of `org.apache.lucene.internal.hppc.LongFloatHashMap`.

use super::macros::define_primitive_hash_map;

define_primitive_hash_map! {
    map = LongFloatHashMap,
    cursor = LongFloatCursor,
    key = i64,
    value = f32,
    key_cursor = LongCursor,
    value_cursor = FloatCursor,
    key_zero = 0,
    value_zero = 0.0,
    hash_key = BitMixer::mix_phi_i64,
    mix_key = BitMixer::mix_i64,
    mix_value = BitMixer::mix_f32,
    eq_value = super::support::eq_f32_bits,
    add_value = super::support::add_f32,
    size_of_keys = super::support::size_of_long_array,
    size_of_values = super::support::size_of_float_array,
    base_ram_bytes_used = 48,
    java_class = "LongFloatHashMap",
    java_key = "long",
    java_value = "float",
    value_fmt = ":?",
}
