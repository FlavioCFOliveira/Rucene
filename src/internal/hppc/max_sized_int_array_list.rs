//! Port of `org.apache.lucene.internal.hppc.MaxSizedIntArrayList`.

use super::macros::define_max_sized_array_list;

define_max_sized_array_list! {
    list = MaxSizedIntArrayList,
    base = IntArrayList,
    element = i32,
    cursor = IntCursor,
    bytes_per_element = 4,
    mix = super::bit_mixer::BitMixer::mix_i32,
    size_of_elements = super::support::size_of_int_array,
    base_ram_bytes_used = 24,
    java_class = "MaxSizedIntArrayList",
    java_base = "IntArrayList",
    element_fmt = "",
}

impl Eq for MaxSizedIntArrayList {}
