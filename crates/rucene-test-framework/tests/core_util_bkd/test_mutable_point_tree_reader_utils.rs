//! Port of `lucene/core/src/test/org/apache/lucene/util/bkd/TestMutablePointTreeReaderUtils.java`.

use std::borrow::Cow;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::point_values::{IntersectVisitor, MutablePointTree, PointTree};
use rucene::util::bkd::{BKDConfig, MutablePointTreeReaderUtils};
use rucene::util::BytesRef;
use rucene_test_framework::util::test_util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};

/// `TestMutablePointTreeReaderUtils`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        test("testSort", test_sort),
        test(
            "testSortWithIncrementalDocId",
            test_sort_with_incremental_doc_id,
        ),
        test("testSortByDim", test_sort_by_dim),
        test("testPartition", test_partition),
    ];
    class(
        "org.apache.lucene.util.bkd.TestMutablePointTreeReaderUtils",
        methods,
    )
}

fn test_sort() -> Result<()> {
    for _ in 0..10 {
        do_test_sort(false)?;
    }
    Ok(())
}

fn test_sort_with_incremental_doc_id() -> Result<()> {
    for _ in 0..10 {
        do_test_sort(true)?;
    }
    Ok(())
}

/// The active bytes of a `BytesRef`.
fn active(b: &BytesRef) -> &[u8] {
    &b.bytes[b.offset..b.offset + b.length]
}

/// `doTestSort(isDocIdIncremental)`.
fn do_test_sort(is_doc_id_incremental: bool) -> Result<()> {
    let bytes_per_dim = TestUtil::next_int(&*random(), 1, 16);
    let max_doc = TestUtil::next_int(&*random(), 1, 1 << random().next_int_bound(30));
    let config = BKDConfig::new(
        1,
        1,
        bytes_per_dim,
        BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE,
    )?;
    let mut points = create_random_points(&config, max_doc, &mut [0], is_doc_id_incremental);
    let mut reader = DummyPointsReader::new(&points);
    MutablePointTreeReaderUtils::sort(&config, max_doc, &mut reader, 0, points.len())?;
    points.sort_by(|o1, o2| {
        active(&o1.packed_value)
            .cmp(active(&o2.packed_value))
            .then_with(|| o1.doc.cmp(&o2.doc))
    });
    // assertNotSame(points, reader.points): the reader holds its own array.
    Assert::assert_false(std::ptr::eq(points.as_ptr(), reader.points.as_ptr()));
    assert_equals(&points.len(), &reader.points.len());

    // Check doc IDs are in ascending order.
    // If doc IDs are already increasing, StableMSBRadixSorter should keep doc ID's ordering.
    // If doc IDs are not ordered, StableMSBRadixSorter should compare doc ID to guarantee the
    // ordering.
    let mut prev_point: Option<&Point> = None;
    for i in 0..points.len() {
        assert_equals(&*points[i].packed_value, &*reader.points[i].packed_value);
        // assertSame(points[i].packedValue, reader.points[i].packedValue)
        Assert::assert_true(Arc::ptr_eq(
            &points[i].packed_value,
            &reader.points[i].packed_value,
        ));
        if let Some(prev) = prev_point {
            if *reader.points[i].packed_value == *prev.packed_value {
                Assert::assert_true(reader.points[i].doc >= prev.doc);
            }
        }
        prev_point = Some(&reader.points[i]);
    }
    Ok(())
}

fn test_sort_by_dim() -> Result<()> {
    for _ in 0..5 {
        do_test_sort_by_dim()?;
    }
    Ok(())
}

/// `doTestSortByDim()`.
fn do_test_sort_by_dim() -> Result<()> {
    let config = create_random_config()?;
    let max_doc = TestUtil::next_int(&*random(), 1, 1 << random().next_int_bound(30));
    let mut common_prefix_lengths = vec![0i32; config.num_dims as usize];
    let points = create_random_points(&config, max_doc, &mut common_prefix_lengths, false);
    let mut reader = DummyPointsReader::new(&points);
    let sorted_dim = random().next_int_bound(config.num_index_dims) as usize;
    let common: Vec<usize> = common_prefix_lengths.iter().map(|&c| c as usize).collect();
    MutablePointTreeReaderUtils::sort_by_dim(
        &config,
        sorted_dim,
        &common,
        &mut reader,
        0,
        points.len(),
        &mut Vec::new(),
        &mut Vec::new(),
    );
    let bytes_per_dim = config.bytes_per_dim as usize;
    for i in 1..points.len() {
        let offset = sorted_dim * bytes_per_dim;
        let previous_value = active(&reader.points[i - 1].packed_value);
        let current_value = active(&reader.points[i].packed_value);
        let mut cmp = previous_value[offset..offset + bytes_per_dim]
            .cmp(&current_value[offset..offset + bytes_per_dim]) as i32;
        if cmp == 0 {
            let data_dim_offset = config.packed_index_bytes_length() as usize;
            let data_dims_length =
                (config.num_dims - config.num_index_dims) as usize * bytes_per_dim;
            cmp = previous_value[data_dim_offset..data_dim_offset + data_dims_length]
                .cmp(&current_value[data_dim_offset..data_dim_offset + data_dims_length])
                as i32;
            if cmp == 0 {
                cmp = reader.points[i - 1].doc - reader.points[i].doc;
            }
        }
        Assert::assert_true(cmp <= 0);
    }
    Ok(())
}

fn test_partition() -> Result<()> {
    for _ in 0..5 {
        do_test_partition()?;
    }
    Ok(())
}

/// `doTestPartition()`.
fn do_test_partition() -> Result<()> {
    let config = create_random_config()?;
    let mut common_prefix_lengths = vec![0i32; config.num_dims as usize];
    let max_doc = TestUtil::next_int(&*random(), 1, 1 << random().next_int_bound(30));
    let points = create_random_points(&config, max_doc, &mut common_prefix_lengths, false);
    let split_dim = random().next_int_bound(config.num_index_dims) as usize;
    let mut reader = DummyPointsReader::new(&points);
    let pivot = TestUtil::next_int(&*random(), 0, points.len() as i32 - 1) as usize;
    MutablePointTreeReaderUtils::partition(
        &config,
        max_doc,
        split_dim,
        common_prefix_lengths[split_dim] as usize,
        &mut reader,
        0,
        points.len(),
        pivot,
        &mut Vec::new(),
        &mut Vec::new(),
    )?;
    let bytes_per_dim = config.bytes_per_dim as usize;
    let pivot_value = active(&reader.points[pivot].packed_value).to_vec();
    let offset = split_dim * bytes_per_dim;
    for i in 0..points.len() {
        let value = active(&reader.points[i].packed_value);
        let mut cmp = value[offset..offset + bytes_per_dim]
            .cmp(&pivot_value[offset..offset + bytes_per_dim]) as i32;
        if cmp == 0 {
            let data_dim_offset = config.packed_index_bytes_length() as usize;
            let data_dims_length =
                (config.num_dims - config.num_index_dims) as usize * bytes_per_dim;
            cmp = value[data_dim_offset..data_dim_offset + data_dims_length]
                .cmp(&pivot_value[data_dim_offset..data_dim_offset + data_dims_length])
                as i32;
            if cmp == 0 {
                cmp = reader.points[i].doc - reader.points[pivot].doc;
            }
        }
        if i < pivot {
            Assert::assert_true(cmp <= 0);
        } else if i > pivot {
            Assert::assert_true(cmp >= 0);
        } else {
            assert_equals(&0, &cmp);
        }
    }
    Ok(())
}

/// `createRandomConfig()`.
fn create_random_config() -> Result<BKDConfig> {
    let num_index_dims = TestUtil::next_int(&*random(), 1, BKDConfig::MAX_INDEX_DIMS);
    let num_dims = TestUtil::next_int(&*random(), num_index_dims, BKDConfig::MAX_DIMS);
    let bytes_per_dim = TestUtil::next_int(&*random(), 1, 16);
    let max_points_in_leaf_node = TestUtil::next_int(&*random(), 50, 2000);
    BKDConfig::new(
        num_dims,
        num_index_dims,
        bytes_per_dim,
        max_points_in_leaf_node,
    )
}

/// `createRandomPoints(config, maxDoc, commonPrefixLengths,
/// isDocIdIncremental)`.
fn create_random_points(
    config: &BKDConfig,
    max_doc: i32,
    common_prefix_lengths: &mut [i32],
    is_doc_id_incremental: bool,
) -> Vec<Point> {
    Assert::assert_true(common_prefix_lengths.len() == config.num_dims as usize);
    let num_points = TestUtil::next_int(&*random(), 1, 100000) as usize;
    let bytes_per_dim = config.bytes_per_dim as usize;
    let packed_len = config.packed_bytes_length() as usize;
    let index_len = config.packed_index_bytes_length() as usize;
    // The values are built as plain arrays, then wrapped in `Point`s; the
    // prefix copies below write into the points' `packedValue`s, as Java's.
    let mut raw: Vec<(Vec<u8>, i32, u8)> = Vec::with_capacity(num_points);
    if random().next_int_bound(10) != 0 {
        for i in 0..num_points {
            let mut value = vec![0u8; packed_len];
            random().next_bytes(&mut value);
            let doc = if is_doc_id_incremental {
                (i as i32).min(max_doc - 1)
            } else {
                random().next_int_bound(max_doc)
            };
            raw.push(Point::draw(value, doc));
        }
        for c in common_prefix_lengths.iter_mut() {
            *c = TestUtil::next_int(&*random(), 0, config.bytes_per_dim);
        }
        let first_value = raw[0].0.clone();
        for point in raw.iter_mut().skip(1) {
            for (dim, &c) in common_prefix_lengths.iter().enumerate() {
                let offset = dim * bytes_per_dim;
                let c = c as usize;
                point.0[offset..offset + c].copy_from_slice(&first_value[offset..offset + c]);
            }
        }
    } else {
        // index dim are equal, data dims different
        let num_data_dims = (config.num_dims - config.num_index_dims) as usize;
        let mut index_dims = vec![0u8; index_len];
        random().next_bytes(&mut index_dims);
        let mut data_dims = vec![0u8; num_data_dims * bytes_per_dim];
        for i in 0..num_points {
            let mut value = vec![0u8; packed_len];
            value[..index_len].copy_from_slice(&index_dims);
            random().next_bytes(&mut data_dims);
            value[index_len..index_len + num_data_dims * bytes_per_dim].copy_from_slice(&data_dims);
            let doc = if is_doc_id_incremental {
                (i as i32).min(max_doc - 1)
            } else {
                random().next_int_bound(max_doc)
            };
            raw.push(Point::draw(value, doc));
        }
        for c in common_prefix_lengths
            .iter_mut()
            .take(config.num_index_dims as usize)
        {
            *c = config.bytes_per_dim;
        }
        for c in common_prefix_lengths
            .iter_mut()
            .skip(config.num_index_dims as usize)
        {
            *c = TestUtil::next_int(&*random(), 0, config.bytes_per_dim);
        }
        let first_value = raw[0].0.clone();
        for point in raw.iter_mut().skip(1) {
            for dim in config.num_index_dims as usize..config.num_dims as usize {
                let offset = dim * bytes_per_dim;
                let c = common_prefix_lengths[dim] as usize;
                point.0[offset..offset + c].copy_from_slice(&first_value[offset..offset + c]);
            }
        }
    }
    raw.into_iter()
        .map(|(value, doc, first_byte)| Point::new(value, doc, first_byte))
        .collect()
}

/// `Point`: a packed value (a shared `BytesRef`, whose identity
/// `assertSame` checks) and a doc ID.
#[derive(Clone, Debug)]
struct Point {
    packed_value: Arc<BytesRef>,
    doc: i32,
}

impl Point {
    /// The draw of `new Point(packedValue, doc)`: `(byte)
    /// random().nextInt(256)` for the byte before the value.
    fn draw(packed_value: Vec<u8>, doc: i32) -> (Vec<u8>, i32, u8) {
        let first_byte = random().next_int_bound(256) as u8;
        (packed_value, doc, first_byte)
    }

    /// `new Point(packedValue, doc)`: use a non-null offset to make sure
    /// `MutablePointsReaderUtils` does not ignore it.
    fn new(packed_value: Vec<u8>, doc: i32, first_byte: u8) -> Point {
        let mut bytes = vec![0u8; packed_value.len() + 1];
        bytes[0] = first_byte;
        bytes[1..].copy_from_slice(&packed_value);
        Point {
            packed_value: Arc::new(BytesRef {
                bytes,
                offset: 1,
                length: packed_value.len(),
            }),
            doc,
        }
    }
}

impl PartialEq for Point {
    fn eq(&self, that: &Point) -> bool {
        *self.packed_value == *that.packed_value && self.doc == that.doc
    }
}

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "value={} doc={}", self.packed_value, self.doc)
    }
}

/// `DummyPointsReader extends MutablePointTree`.
struct DummyPointsReader {
    points: Vec<Point>,
    temp: Option<Vec<Option<Point>>>,
}

impl DummyPointsReader {
    /// `new DummyPointsReader(points)`: `points.clone()`.
    fn new(points: &[Point]) -> Self {
        DummyPointsReader {
            points: points.to_vec(),
            temp: None,
        }
    }
}

fn unsupported<T>() -> Result<T> {
    Err(LuceneError::UnsupportedOperation(String::new()))
}

impl PointTree for DummyPointsReader {
    /// `clone()` (`final` in `MutablePointTree`): unsupported.
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        unsupported()
    }
    fn move_to_child(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_sibling(&mut self) -> Result<bool> {
        unsupported()
    }
    fn move_to_parent(&mut self) -> Result<bool> {
        unsupported()
    }
    fn min_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn max_packed_value(&self) -> Result<&[u8]> {
        unsupported()
    }
    fn size(&self) -> Result<i64> {
        unsupported()
    }
    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        unsupported()
    }
    fn visit_doc_values(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        unsupported()
    }
    fn as_mutable(&mut self) -> Option<&mut dyn MutablePointTree> {
        Some(self)
    }
}

impl MutablePointTree for DummyPointsReader {
    /// `getValue(i, packedValue)`.
    fn value(&self, i: i32) -> Cow<'_, [u8]> {
        Cow::Borrowed(active(&self.points[i as usize].packed_value))
    }

    fn byte_at(&self, i: i32, k: i32) -> u8 {
        let packed_value = &self.points[i as usize].packed_value;
        packed_value.bytes[packed_value.offset + k as usize]
    }

    fn doc_id(&self, i: i32) -> i32 {
        self.points[i as usize].doc
    }

    fn swap(&mut self, i: i32, j: i32) {
        self.points.swap(i as usize, j as usize);
    }

    fn save(&mut self, i: i32, j: i32) {
        let len = self.points.len();
        let temp = self.temp.get_or_insert_with(|| vec![None; len]);
        temp[j as usize] = Some(self.points[i as usize].clone());
    }

    fn restore(&mut self, i: i32, j: i32) {
        if let Some(temp) = &self.temp {
            for k in i as usize..j as usize {
                self.points[k] = temp[k].clone().expect("saved before restore");
            }
        }
    }
}
