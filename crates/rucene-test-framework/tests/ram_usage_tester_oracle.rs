//! Oracle parity of `RamUsageTester` (rmp #342) against Apache Lucene 10.5.0:
//! `testutil_ramusage.tsv` (`TestUtilOracle.ramUsage`) holds `ramUsed` and
//! `humanSizeOf` of primitive arrays, strings, `BytesRef`s, reference arrays,
//! an `ArrayList` and a `HashMap` of strings, and `ramUsed` with an
//! `Accumulator` that does not follow array elements, measured by a 64-bit
//! JDK 25 with compressed references.

mod oracle_support;

use rucene::util::{BytesRef, RamUsageEstimator};
use rucene_test_framework::util::ram_usage_tester::{
    aligned_shallow_instance_size, Accumulator, JavaObject, JavaObjectArray, Queue, RamObject,
    RamUsageTester,
};

/// A `java.util.ArrayList`: `AbstractList.modCount`, `elementData` and
/// `size` (24 bytes), walked as a JDK `Iterable`.
struct ArrayListModel<'a>(Vec<&'a String>);

impl RamObject for ArrayListModel<'_> {
    fn java_class_name(&self) -> &'static str {
        "java.util.ArrayList"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::JavaIterable {
            shallow_size: aligned_shallow_instance_size(&[4, 4, 4]),
            elements: self.0.iter().map(|s| Some(*s as &dyn RamObject)).collect(),
        }
    }
}

/// A `java.util.HashMap`: `AbstractMap.keySet` and `values`, `table`,
/// `entrySet`, `size`, `modCount`, `threshold` and `loadFactor` (48 bytes),
/// walked as a JDK `Map`.
struct HashMapModel<'a>(Vec<(&'a String, &'a String)>);

impl RamObject for HashMapModel<'_> {
    fn java_class_name(&self) -> &'static str {
        "java.util.HashMap"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::JavaMap {
            shallow_size: aligned_shallow_instance_size(&[4; 8]),
            entries: self
                .0
                .iter()
                .map(|(k, v)| (Some(*k as &dyn RamObject), Some(*v as &dyn RamObject)))
                .collect(),
        }
    }
}

/// The oracle's accumulator that returns an array's shallow size without
/// following its elements.
struct ShallowArrays;

impl Accumulator for ShallowArrays {
    fn accumulate_array<'a>(
        &mut self,
        _array: &'a dyn RamObject,
        shallow_size: i64,
        _values: &[Option<&'a dyn RamObject>],
        _queue: &mut Queue<'a>,
    ) -> i64 {
        shallow_size
    }
}

/// `ramUsed` and `humanSizeOf` of every object of the oracle are Java's.
#[test]
fn ram_usage_tester_is_lucenes() {
    let rows = oracle_support::rows("testutil_ramusage.tsv");
    assert_eq!(rows.len(), 30);
    let a = "a".to_string();
    let b = "b".to_string();
    let (one, two, three) = ("one".to_string(), "two".to_string(), "three".to_string());
    let (k1, v1, k2, v2) = (
        "k1".to_string(),
        "v1".to_string(),
        "key2".to_string(),
        "value2".to_string(),
    );
    let (i2, i1) = (vec![1i32, 2], vec![3i32]);
    let shared = JavaObjectArray(vec![Some(&a), Some(&a), Some(&b), None]);
    let hello = BytesRef::from_char_sequence("hello");
    for row in &rows {
        let (case, used, human) = (&row[0], &row[1], &row[2]);
        let object: Box<dyn RamObject + '_> = match case.as_str() {
            c if c.starts_with("byte[") => {
                let n: usize = c[5..c.len() - 1].parse().unwrap();
                Box::new(vec![0u8; n])
            }
            "boolean[9]" => Box::new(vec![false; 9]),
            "char[5]" => Box::new(vec![0u16; 5]),
            "short[3]" => Box::new(vec![0i16; 3]),
            "int[3]" => Box::new(vec![0i32; 3]),
            "float[2]" => Box::new(vec![0f32; 2]),
            "long[5]" => Box::new(vec![0i64; 5]),
            "double[3]" => Box::new(vec![0f64; 3]),
            "int[100000]" => Box::new(vec![0i32; 100_000]),
            "string()" => Box::new(String::new()),
            "string(a)" => Box::new("a".to_string()),
            "string(abcdefgh)" => Box::new("abcdefgh".to_string()),
            "string(x*100)" => Box::new("x".repeat(100)),
            "string(U+10000)" => Box::new("\u{10000}".to_string()),
            "bytesref()" => Box::new(BytesRef::default()),
            "bytesref(byte[10])" => Box::new(BytesRef::new(vec![0; 10])),
            "bytesref(hello)" => Box::new(BytesRef::from_char_sequence("hello")),
            "object[](a,a,b,null)" => {
                Box::new(JavaObjectArray(vec![Some(&a), Some(&a), Some(&b), None]))
            }
            "object[0]" => Box::new(JavaObjectArray(Vec::new())),
            "int[][](2,1)" => Box::new(JavaObjectArray(vec![Some(&i2), Some(&i1)])),
            "arraylist(one,two,three)" => Box::new(ArrayListModel(vec![&one, &two, &three])),
            "hashmap(k1=v1,key2=value2)" => Box::new(HashMapModel(vec![(&k1, &v1), (&k2, &v2)])),
            "shallowArrays:object[](a,a,b,null)" => {
                let size = RamUsageTester::ram_used_with(Some(&shared), &mut ShallowArrays);
                assert_eq!(size.to_string(), *used, "{case}");
                continue;
            }
            "shallowArrays:bytesref(hello)" => {
                let size = RamUsageTester::ram_used_with(Some(&hello), &mut ShallowArrays);
                assert_eq!(size.to_string(), *used, "{case}");
                continue;
            }
            other => panic!("unknown case {other}"),
        };
        assert_eq!(
            RamUsageTester::ram_used(&*object).to_string(),
            *used,
            "{case}"
        );
        assert_eq!(RamUsageTester::human_size_of(&*object), *human, "{case}");
    }
    // `null` measures 0.
    assert_eq!(
        RamUsageTester::ram_used_with(
            None,
            &mut rucene_test_framework::util::ram_usage_tester::DefaultAccumulator
        ),
        0
    );
    // The shallow size of a `String` is the estimator's constant.
    assert_eq!(RamUsageEstimator::STRING_SIZE, 24);
}
