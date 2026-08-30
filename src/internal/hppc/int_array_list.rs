//! Port of `org.apache.lucene.internal.hppc.IntArrayList`.

use super::macros::define_array_list;

define_array_list! {
    list = IntArrayList,
    element = i32,
    cursor = IntCursor,
    zero = 0,
    bytes_per_element = 4,
    mix = super::bit_mixer::BitMixer::mix_i32,
    eq = super::support::eq_i32,
    sort = super::support::sort_i32,
    size_of_elements = super::support::size_of_int_array,
    base_ram_bytes_used = 24,
    java_class = "IntArrayList",
    java_element = "int",
    element_fmt = "",
}

impl Eq for IntArrayList {}

impl IntArrayList {
    /// Returns an iterator over all the elements contained in this list.
    ///
    /// Equivalent of Java's `stream()`, which returns an `IntStream`; Rust's
    /// iterators fill the same role, so the values are yielded directly rather
    /// than wrapped in a cursor the way [`Self::iter`] does.
    pub fn stream(&self) -> std::iter::Copied<std::slice::Iter<'_, i32>> {
        self.buffer[0..self.size() as usize].iter().copied()
    }
}
