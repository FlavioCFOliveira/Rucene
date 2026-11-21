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

#[cfg(test)]
mod tests {
    use super::IntArrayList;
    use crate::internal::hppc::max_sized_int_array_list::MaxSizedIntArrayList;

    /// `toString()` is `Arrays.toString(this.toArray())`
    /// (`IntArrayList.java:384-386`), which `MaxSizedIntArrayList` inherits;
    /// that is `Display`. `Debug` prefixes the type name exactly once.
    #[test]
    fn display_is_java_to_string_and_debug_names_the_type_once() {
        let mut list = IntArrayList::new();
        list.add(1);
        list.add(2);
        assert_eq!(list.to_string(), "[1, 2]");
        assert_eq!(format!("{list:?}"), "IntArrayList[1, 2]");

        let mut bounded = MaxSizedIntArrayList::new(5);
        bounded.add(3);
        bounded.add(4);
        assert_eq!(bounded.to_string(), "[3, 4]");
        assert_eq!(
            format!("{bounded:?}"),
            "MaxSizedIntArrayList[3, 4], maxSize: 5"
        );
    }
}
