//! Port of `org.apache.lucene.internal.hppc.LongArrayList`.

use super::macros::define_array_list;

define_array_list! {
    list = LongArrayList,
    element = i64,
    cursor = LongCursor,
    zero = 0,
    bytes_per_element = 8,
    mix = super::bit_mixer::BitMixer::mix_i64,
    eq = super::support::eq_i64,
    sort = super::support::sort_i64,
    size_of_elements = super::support::size_of_long_array,
    base_ram_bytes_used = 24,
    java_class = "LongArrayList",
    java_element = "long",
    element_fmt = "",
}

impl Eq for LongArrayList {}

impl LongArrayList {
    /// Returns an iterator over all the elements contained in this list.
    ///
    /// Equivalent of Java's `stream()`, which returns a `LongStream`; Rust's
    /// iterators fill the same role, so the values are yielded directly rather
    /// than wrapped in a cursor the way [`Self::iter`] does.
    pub fn stream(&self) -> std::iter::Copied<std::slice::Iter<'_, i64>> {
        self.buffer[0..self.size() as usize].iter().copied()
    }
}
