//! Port of `org.apache.lucene.internal.hppc.TestIntArrayList`: tests for
//! `IntArrayList`, "mostly forked and trimmed from
//! com.carrotsearch.hppc.IntArrayListTest" (HPPC 0.9.0).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{Assert, Before};
use rucene::internal::hppc::IntArrayList;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use super::support::{
    assert_array_equals, assert_equals_obj, assert_not_equals, assert_sorted_list_equals,
};
use crate::lucene_core_shims::ti;

/// The instance state of `TestIntArrayList`.
pub struct TestIntArrayList {
    key0: i32,
    key1: i32,
    key2: i32,
    key3: i32,
    key4: i32,
    key5: i32,
    key6: i32,
    key7: i32,

    /// Per-test fresh initialized instance.
    list: IntArrayList,
}

impl TestIntArrayList {
    pub(crate) fn new() -> TestIntArrayList {
        TestIntArrayList {
            key0: cast(0),
            key1: cast(1),
            key2: cast(2),
            key3: cast(3),
            key4: cast(4),
            key5: cast(5),
            key6: cast(6),
            key7: cast(7),
            // Assigned by `initialize()` before each test.
            list: IntArrayList::new(),
        }
    }
}

/// Convert to target type from an integer used to test stuff.
fn cast(v: i32) -> i32 {
    v
}

/// `initialize()` (`@Before`).
pub(crate) fn initialize(this: &mut T) {
    this.list = IntArrayList::new();
}

/// `class Sub extends IntArrayList {}`: a class of its own, so that
/// `IntArrayList.equals` (`getClass() == obj.getClass()`) tells it from an
/// `IntArrayList` with the same elements.
#[derive(Debug, PartialEq)]
struct Sub(IntArrayList);

type T = TestIntArrayList;

impl TestIntArrayList {
    fn test_initially_empty(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals(0, self.list.size());
        Ok(())
    }

    fn test_add(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&[self.key1, self.key2]);
        assert_list_equals(self.list.to_array(), &[1, 2]);
        Ok(())
    }

    fn test_add_two_args(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&[self.key1, self.key2]);
        self.list.add_array(&[self.key3, self.key4]);
        assert_list_equals(self.list.to_array(), &[1, 2, 3, 4]);
        Ok(())
    }

    fn test_add_array(&mut self) -> rucene::error::Result<()> {
        self.list.add_range(&as_array(&[0, 1, 2, 3]), 1, 2);
        assert_list_equals(self.list.to_array(), &[1, 2]);
        Ok(())
    }

    fn test_add_var_arg(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3]));
        self.list
            .add_array(&[self.key4, self.key5, self.key6, self.key7]);
        assert_list_equals(self.list.to_array(), &[0, 1, 2, 3, 4, 5, 6, 7]);
        Ok(())
    }

    fn test_add_all(&mut self) -> rucene::error::Result<()> {
        let mut list2 = IntArrayList::new();
        list2.add_array(&as_array(&[0, 1, 2]));

        self.list.add_all(&list2);
        self.list.add_all(&list2);

        assert_list_equals(self.list.to_array(), &[0, 1, 2, 0, 1, 2]);
        Ok(())
    }

    fn test_insert(&mut self) -> rucene::error::Result<()> {
        self.list.insert(0, self.key1);
        self.list.insert(0, self.key2);
        self.list.insert(2, self.key3);
        self.list.insert(1, self.key4);

        assert_list_equals(self.list.to_array(), &[2, 4, 1, 3]);
        Ok(())
    }

    fn test_set(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2]));

        Assert::assert_equals(0, self.list.set(0, self.key3));
        Assert::assert_equals(1, self.list.set(1, self.key4));
        Assert::assert_equals(2, self.list.set(2, self.key5));

        assert_list_equals(self.list.to_array(), &[3, 4, 5]);
        Ok(())
    }

    fn test_remove_at(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3, 4]));

        self.list.remove_at(0);
        self.list.remove_at(2);
        self.list.remove_at(1);

        assert_list_equals(self.list.to_array(), &[1, 4]);
        Ok(())
    }

    fn test_remove_last(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3, 4]));

        Assert::assert_equals(4, self.list.remove_last());
        Assert::assert_equals(4, self.list.size());
        assert_list_equals(self.list.to_array(), &[0, 1, 2, 3]);
        Assert::assert_equals(3, self.list.remove_last());
        Assert::assert_equals(3, self.list.size());
        assert_list_equals(self.list.to_array(), &[0, 1, 2]);
        Assert::assert_equals(2, self.list.remove_last());
        Assert::assert_equals(1, self.list.remove_last());
        Assert::assert_equals(0, self.list.remove_last());
        Assert::assert_true(self.list.is_empty());
        Ok(())
    }

    fn test_remove_element(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3, 3, 4]));

        Assert::assert_true(self.list.remove_element(3));
        Assert::assert_true(self.list.remove_element(2));
        Assert::assert_false(self.list.remove_element(5));

        assert_list_equals(self.list.to_array(), &[0, 1, 3, 4]);
        Ok(())
    }

    fn test_remove_range(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3, 4]));

        self.list.remove_range(0, 2);
        assert_list_equals(self.list.to_array(), &[2, 3, 4]);

        self.list.remove_range(2, 3);
        assert_list_equals(self.list.to_array(), &[2, 3]);

        self.list.remove_range(1, 1);
        assert_list_equals(self.list.to_array(), &[2, 3]);

        self.list.remove_range(0, 1);
        assert_list_equals(self.list.to_array(), &[3]);
        Ok(())
    }

    fn test_remove_first_last(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 1, 0]));

        Assert::assert_equals(-1, self.list.remove_first(self.key5));
        Assert::assert_equals(-1, self.list.remove_last_element(self.key5));
        assert_list_equals(self.list.to_array(), &[0, 1, 2, 1, 0]);

        Assert::assert_equals(1, self.list.remove_first(self.key1));
        assert_list_equals(self.list.to_array(), &[0, 2, 1, 0]);

        Assert::assert_equals(3, self.list.remove_last_element(self.key0));
        assert_list_equals(self.list.to_array(), &[0, 2, 1]);

        Assert::assert_equals(0, self.list.remove_last_element(self.key0));
        assert_list_equals(self.list.to_array(), &[2, 1]);

        Assert::assert_equals(-1, self.list.remove_last_element(self.key0));
        Ok(())
    }

    fn test_remove_all(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 0, 1, 0]));

        Assert::assert_equals(0, self.list.remove_all(self.key2));
        Assert::assert_equals(3, self.list.remove_all(self.key0));
        assert_list_equals(self.list.to_array(), &[1, 1]);

        Assert::assert_equals(2, self.list.remove_all(self.key1));
        Assert::assert_true(self.list.is_empty());
        Ok(())
    }

    fn test_index_of(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 1, 0]));

        Assert::assert_equals(0, self.list.index_of(self.key0));
        Assert::assert_equals(-1, self.list.index_of(self.key3));
        Assert::assert_equals(2, self.list.index_of(self.key2));
        Ok(())
    }

    fn test_last_index_of(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 1, 0]));

        Assert::assert_equals(4, self.list.last_index_of(self.key0));
        Assert::assert_equals(-1, self.list.last_index_of(self.key3));
        Assert::assert_equals(2, self.list.last_index_of(self.key2));
        Ok(())
    }

    fn test_ensure_capacity(&mut self) -> rucene::error::Result<()> {
        let mut list = IntArrayList::with_expected_elements(0);
        Assert::assert_equals(list.size() as usize, list.buffer.len());
        let buffer1 = list.buffer.as_ptr();
        list.ensure_capacity(100);
        // `assertNotSame(buffer1, list.buffer)`
        Assert::assert_true(!std::ptr::eq(buffer1, list.buffer.as_ptr()));
        Ok(())
    }

    fn test_resize_and_clean_buffer(&mut self) -> rucene::error::Result<()> {
        self.list.ensure_capacity(20);
        self.list.buffer.fill(self.key1);

        self.list.resize(10);
        Assert::assert_equals(10, self.list.size());
        for i in 0..self.list.size() {
            Assert::assert_equals(0, self.list.get(i));
        }

        self.list.buffer.fill(0);
        for i in 5..self.list.size() {
            self.list.set(i, self.key1);
        }
        self.list.resize(5);
        Assert::assert_equals(5, self.list.size());
        for i in self.list.size() as usize..self.list.buffer.len() {
            Assert::assert_equals(0, self.list.buffer[i]);
        }
        Ok(())
    }

    fn test_trim_to_size(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[1, 2]));
        self.list.trim_to_size();
        Assert::assert_equals(2, self.list.buffer.len());
        Ok(())
    }

    fn test_release(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[1, 2]));
        self.list.release();
        Assert::assert_equals(0, self.list.size());
        self.list.add_array(&as_array(&[1, 2]));
        Assert::assert_equals(2, self.list.size());
        Ok(())
    }

    fn test_iterable(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3]));
        let mut count = 0;
        for cursor in &self.list {
            count += 1;
            Assert::assert_equals(self.list.get(cursor.index), cursor.value);
            Assert::assert_equals(self.list.buffer[cursor.index as usize], cursor.value);
        }
        Assert::assert_equals(count, self.list.size());

        count = 0;
        self.list.resize(0);
        for _cursor in &self.list {
            count += 1;
        }
        Assert::assert_equals(0, count);
        Ok(())
    }

    /// `Iterator.hasNext()` is the iterator's `peek()`.
    fn test_iterator(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[0, 1, 2, 3]));
        let mut iterator = self.list.iter().peekable();
        let mut count = 0;
        while iterator.peek().is_some() {
            iterator.peek();
            iterator.peek();
            iterator.peek();
            iterator.next();
            count += 1;
        }
        Assert::assert_equals(count, self.list.size());

        self.list.resize(0);
        Assert::assert_false(self.list.iter().peekable().peek().is_some());
        Ok(())
    }

    fn test_clear(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[1, 2, 3]));
        self.list.clear();
        Assert::assert_true(self.list.is_empty());
        Assert::assert_equals(-1, self.list.index_of(cast(1)));
        Ok(())
    }

    fn test_from(&mut self) -> rucene::error::Result<()> {
        self.list = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        Assert::assert_equals(3, self.list.size());
        assert_list_equals(self.list.to_array(), &[1, 2, 3]);
        Assert::assert_equals(self.list.size() as usize, self.list.buffer.len());
        Ok(())
    }

    fn test_copy_list(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&as_array(&[1, 2, 3]));
        let copy = IntArrayList::from_list(&self.list);
        Assert::assert_equals(3, copy.size());
        assert_list_equals(copy.to_array(), &[1, 2, 3]);
        Assert::assert_equals(copy.size() as usize, copy.buffer.len());
        Ok(())
    }

    fn test_hash_code_equals(&mut self) -> rucene::error::Result<()> {
        let l0 = IntArrayList::from(&[]);
        Assert::assert_equals(1, l0.hash_code());
        assert_equals_obj(&l0, &IntArrayList::from(&[]));

        let l1 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        let mut l2 = IntArrayList::from(&[self.key1, self.key2]);
        l2.add(self.key3);

        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        assert_equals_obj(&l1, &l2);
        Ok(())
    }

    fn test_equal_elements(&mut self) -> rucene::error::Result<()> {
        let l1 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        let mut l2 = IntArrayList::from(&[self.key1, self.key2]);
        l2.add(self.key3);

        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        Assert::assert_true(l2.equal_elements(&l1));
        Ok(())
    }

    fn test_to_array(&mut self) -> rucene::error::Result<()> {
        let mut l1 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        l1.ensure_capacity(100);
        let result = l1.to_array();
        assert_array_equals(&[self.key1, self.key2, self.key3], &result);
        Ok(())
    }

    fn test_clone(&mut self) -> rucene::error::Result<()> {
        self.list.add_array(&[self.key1, self.key2, self.key3]);

        let mut cloned = self.list.clone();
        let i = cloned.index_of(self.key1);
        cloned.remove_at(i);

        assert_sorted_list_equals(self.list.to_array(), &[self.key1, self.key2, self.key3]);
        assert_sorted_list_equals(cloned.to_array(), &[self.key2, self.key3]);
        Ok(())
    }

    fn test_to_string(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals_str(
            None,
            Some(&format!("[{}, {}, {}]", self.key1, self.key2, self.key3)),
            Some(&IntArrayList::from(&[self.key1, self.key2, self.key3]).to_string()),
        );
        Ok(())
    }

    fn test_equals_same_class(&mut self) -> rucene::error::Result<()> {
        let l1 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        let l2 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        let l3 = IntArrayList::from(&[self.key1, self.key3, self.key2]);

        assert_equals_obj(&l1, &l2);
        Assert::assert_equals(l1.hash_code(), l2.hash_code());
        assert_not_equals(&l1, &l3);
        Ok(())
    }

    fn test_equals_sub_class(&mut self) -> rucene::error::Result<()> {
        let l1 = IntArrayList::from(&[self.key1, self.key2, self.key3]);
        let mut l2 = Sub(IntArrayList::new());
        let mut l3 = Sub(IntArrayList::new());
        l2.0.add_all(&l1);
        l3.0.add_all(&l1);

        assert_equals_obj(&l2, &l3);
        // `assertNotEquals(l1, l3)`: `IntArrayList.equals` requires the same
        // class.
        if super::support::java_equals(&l1, &l3) {
            Assert::fail(&format!("Values should be different. Actual: {l3:?}"));
        }
        Ok(())
    }

    fn test_stream(&mut self) -> rucene::error::Result<()> {
        Assert::assert_equals(
            self.key1,
            IntArrayList::from(&[self.key1, self.key2, self.key3])
                .stream()
                .min()
                .expect("java.util.NoSuchElementException"),
        );
        Assert::assert_equals(
            self.key3,
            IntArrayList::from(&[self.key2, self.key1, self.key3])
                .stream()
                .max()
                .expect("java.util.NoSuchElementException"),
        );
        Assert::assert_equals(
            0,
            IntArrayList::from(&[self.key1, self.key2, -self.key3])
                .stream()
                .fold(0i32, i32::wrapping_add),
        );
        expect_throws("java.util.NoSuchElementException", || {
            // `IntArrayList.from().stream().min().orElseThrow()`
            match IntArrayList::from(&[]).stream().min() {
                Some(min) => Ok(min),
                None => Err(rucene::error::LuceneError::unchecked(
                    "java.util.NoSuchElementException",
                    "No value present",
                )),
            }
        });
        Ok(())
    }

    fn test_sort(&mut self) -> rucene::error::Result<()> {
        self.list
            .add_array(&[self.key3, self.key1, self.key3, self.key2]);
        let mut list2 = IntArrayList::new();
        list2.ensure_capacity(100);
        list2.add_all(&self.list);
        let list2_ptr: *const IntArrayList = &list2;
        // `assertSame(list2, list2.sort())`
        Assert::assert_true(std::ptr::eq(list2_ptr, list2.sort()));
        assert_equals_obj(
            &IntArrayList::from(&[self.key1, self.key2, self.key3, self.key3]),
            &list2,
        );
        Ok(())
    }

    fn test_reverse(&mut self) -> rucene::error::Result<()> {
        for i in 0..10 {
            let mut elements = vec![0; i];
            for j in 0..i {
                elements[j] = cast(j as i32);
            }
            let mut list = IntArrayList::new();
            list.ensure_capacity(30);
            list.add_array(&elements);
            let list_ptr: *const IntArrayList = &list;
            // `assertSame(list, list.reverse())`
            Assert::assert_true(std::ptr::eq(list_ptr, list.reverse()));
            Assert::assert_equals(elements.len() as i32, list.size());
            let mut reverse_index = elements.len() as i32 - 1;
            for cursor in &list {
                Assert::assert_equals(elements[reverse_index as usize], cursor.value);
                reverse_index -= 1;
            }
        }
        Ok(())
    }
}

/// Check if the array's content is identical to a given sequence of elements.
fn assert_list_equals(array: Vec<i32>, elements: &[i32]) {
    Assert::assert_equals(elements.len(), array.len());
    assert_array_equals(elements, &array);
}

/// `asArray(int... elements)` (`private static`).
fn as_array(elements: &[i32]) -> Vec<i32> {
    elements.to_vec()
}

/// The class `org.apache.lucene.internal.hppc.TestIntArrayList`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.internal.hppc.TestIntArrayList")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(T::new))
        .method(Method::new("initialize").annotate(Before).body(initialize))
        .method(ti("testInitiallyEmpty", T::test_initially_empty))
        .method(ti("testAdd", T::test_add))
        .method(ti("testAddTwoArgs", T::test_add_two_args))
        .method(ti("testAddArray", T::test_add_array))
        .method(ti("testAddVarArg", T::test_add_var_arg))
        .method(ti("testAddAll", T::test_add_all))
        .method(ti("testInsert", T::test_insert))
        .method(ti("testSet", T::test_set))
        .method(ti("testRemoveAt", T::test_remove_at))
        .method(ti("testRemoveLast", T::test_remove_last))
        .method(ti("testRemoveElement", T::test_remove_element))
        .method(ti("testRemoveRange", T::test_remove_range))
        .method(ti("testRemoveFirstLast", T::test_remove_first_last))
        .method(ti("testRemoveAll", T::test_remove_all))
        .method(ti("testIndexOf", T::test_index_of))
        .method(ti("testLastIndexOf", T::test_last_index_of))
        .method(ti("testEnsureCapacity", T::test_ensure_capacity))
        .method(ti(
            "testResizeAndCleanBuffer",
            T::test_resize_and_clean_buffer,
        ))
        .method(ti("testTrimToSize", T::test_trim_to_size))
        .method(ti("testRelease", T::test_release))
        .method(ti("testIterable", T::test_iterable))
        .method(ti("testIterator", T::test_iterator))
        .method(ti("testClear", T::test_clear))
        .method(ti("testFrom", T::test_from))
        .method(ti("testCopyList", T::test_copy_list))
        .method(ti("testHashCodeEquals", T::test_hash_code_equals))
        .method(ti("testEqualElements", T::test_equal_elements))
        .method(ti("testToArray", T::test_to_array))
        .method(ti("testClone", T::test_clone))
        .method(ti("testToString", T::test_to_string))
        .method(ti("testEqualsSameClass", T::test_equals_same_class))
        .method(ti("testEqualsSubClass", T::test_equals_sub_class))
        .method(ti("testStream", T::test_stream))
        .method(ti("testSort", T::test_sort))
        .method(ti("testReverse", T::test_reverse))
        .build()
}
