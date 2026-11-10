//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/TestRamUsageEstimator.java`.
//!
//! Java's `ramUsed` reads any object by reflection; the objects measured here
//! describe their Java layout through `RamObject` (a declared list of the
//! fields reflection finds), directly or as a `ModelBuilder` graph.

use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::{
    BooleanQueryBuilder, DisjunctionMaxQuery, Occur, PhraseQueryBuilder, Query, TermQuery,
};
use rucene::util::byte_block_pool::DirectAllocator;
use rucene::util::{
    Accountable, ByteBlockPool, BytesRef, BytesRefHash, Constants, RamUsageEstimator,
    SizedCollection, SizedMap, SizedObject,
};
use rucene_test_framework::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use rucene_test_framework::util::ram_usage_tester::{
    aligned_shallow_instance_size, JavaNode, JavaObject, JavaObjectArray, ModelBuilder, NodeChild,
    NodeShape, RamObject, RamUsageTester, ARRAY_LIST_SHALLOW_SIZE, HASH_MAP_SHALLOW_SIZE,
};

use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, class, random, test,
};

/// `TestRamUsageEstimator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestRamUsageEstimator",
        vec![
            test("testSanity", test_sanity),
            test("testStaticOverloads", test_static_overloads),
            test("testStrings", test_strings),
            test("testBytesRefHash", test_bytes_ref_hash),
            test("testMap", test_map),
            test("testCollection", test_collection),
            test("testQuery", test_query),
            test("testReferenceSize", test_reference_size),
            test("testHotspotBean", test_hotspot_bean),
            test("testPrintValues", test_print_values),
        ],
    )
}

/// `static final String[] strings`.
const STRINGS: [&str; 3] = ["test string", "hollow", "catchmaster"];

fn ram_used(o: &dyn RamObject) -> i64 {
    RamUsageTester::ram_used(o)
}

const REF: i64 = RamUsageEstimator::NUM_BYTES_OBJECT_REF;

/// `shallowSizeOfInstance(String.class)`: JDK 25's `byte[] value`, `byte
/// coder`, `int hash` and `boolean hashIsZero`.
fn shallow_size_of_instance_string() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[REF, 1, 4, 1])
}

/// `shallowSizeOfInstance(Holder.class)`: `long field1`, `String name`,
/// `Holder holder`, `long field2, field3, field4`.
const HOLDER_FIELDS: [i64; 6] = [8, REF, REF, 8, 8, 8];

fn shallow_size_of_instance_holder() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&HOLDER_FIELDS)
}

/// `shallowSizeOfInstance(HolderSubclass.class)`: `Holder`'s fields plus
/// `byte foo` and `int bar`.
fn shallow_size_of_instance_holder_subclass() -> i64 {
    let mut fields = HOLDER_FIELDS.to_vec();
    fields.extend([1, 4]);
    RamUsageEstimator::shallow_size_of_declared_instance(&fields)
}

/// `shallowSizeOfInstance(HolderSubclass2.class)`: only `Holder`'s fields.
fn shallow_size_of_instance_holder_subclass2() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&HOLDER_FIELDS)
}

fn test_sanity() -> Result<()> {
    assert!(ram_used(&"test string".to_string()) > shallow_size_of_instance_string());

    let mut holder = Holder::new();
    holder.holder = Some(Box::new(Holder::with("string2", 5000)));
    assert!(ram_used(&holder) > shallow_size_of_instance_holder());
    assert!(ram_used(&holder) > ram_used(holder.holder.as_deref().unwrap()));

    assert!(shallow_size_of_instance_holder_subclass() >= shallow_size_of_instance_holder());
    assert!(shallow_size_of_instance_holder() == shallow_size_of_instance_holder_subclass2());

    let owned: Vec<String> = STRINGS.iter().map(|s| s.to_string()).collect();
    let strings = JavaObjectArray(owned.iter().map(|s| Some(s as &dyn RamObject)).collect());
    assert!(ram_used(&strings) > RamUsageEstimator::shallow_size_of(&STRINGS));
    Ok(())
}

fn test_static_overloads() -> Result<()> {
    let rnd = random();
    {
        let array = vec![0u8; rnd.next_int_bound(1024) as usize];
        assert_equals(&RamUsageEstimator::size_of(&array), &ram_used(&array));
        // `shallowSizeOf(Object)` over a primitive array.
        assert_equals(
            &RamUsageEstimator::shallow_size_of_array::<u8>(array.len()),
            &ram_used(&array),
        );
    }

    {
        let array = vec![false; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_bool(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<bool>(array.len()),
        );
    }

    {
        let array = vec![0u16; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_char(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<u16>(array.len()),
        );
    }

    {
        let array = vec![0i16; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_short(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i16>(array.len()),
        );
    }

    {
        let array = vec![0i32; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_int(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i32>(array.len()),
        );
    }

    {
        let array = vec![0f32; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_float(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<f32>(array.len()),
        );
    }

    {
        let array = vec![0i64; rnd.next_int_bound(1024) as usize];
        assert_equals(&ram_used(&array), &RamUsageEstimator::size_of_long(&array));
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<i64>(array.len()),
        );
    }

    {
        let array = vec![0f64; rnd.next_int_bound(1024) as usize];
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::size_of_double(&array),
        );
        assert_equals(
            &ram_used(&array),
            &RamUsageEstimator::shallow_size_of_array::<f64>(array.len()),
        );
    }
    Ok(())
}

fn test_strings() -> Result<()> {
    let owned: Vec<String> = STRINGS.iter().map(|s| s.to_string()).collect();
    let array = JavaObjectArray(owned.iter().map(|s| Some(s as &dyn RamObject)).collect());
    let actual = ram_used(&array);
    let estimated = RamUsageEstimator::size_of_string_array(
        &STRINGS.iter().map(|s| Some(*s)).collect::<Vec<_>>(),
    );
    assert_equals(&actual, &estimated);
    Ok(())
}

fn test_bytes_ref_hash() -> Result<()> {
    // `new BytesRefHash()`: its own `ByteBlockPool(new DirectAllocator())`
    // and a `DirectBytesStartArray(DEFAULT_CAPACITY)` with a fresh counter.
    let mut pool = ByteBlockPool::with_allocator(Box::new(DirectAllocator::new()));
    let mut bytes =
        BytesRefHash::with_direct_bytes_start(16, Arc::new(std::sync::atomic::AtomicI64::new(0)));
    for i in 0..100 {
        bytes.add(
            &mut pool,
            BytesRef::from_char_sequence(&format!("foo bar {i}")).slice(),
        )?;
        bytes.add(
            &mut pool,
            BytesRef::from_char_sequence(&format!("baz bam {i}")).slice(),
        )?;
    }
    let actual = ram_used(&*ModelBuilder::new().bytes_ref_hash(&bytes, &pool));
    let estimated = bytes.ram_bytes_used(&pool);
    assert_equals_delta(actual as f64, estimated as f64, actual as f64 * 0.1);
    Ok(())
}

/// A value of the `Map<String, Object>` and `List<Object>` of `testMap` and
/// `testCollection`.
enum JavaValue {
    /// A boxed `Long`.
    Long(i64),
    /// A `String`.
    Str(String),
    /// A `Term`.
    Term(Term),
    /// The container itself (`map.put("self", map)`, `list.add(list)`).
    This,
}

impl JavaValue {
    fn sized<'a>(&'a self, this: SizedObject<'a>) -> SizedObject<'a> {
        match self {
            JavaValue::Long(v) => SizedObject::Long(*v),
            JavaValue::Str(s) => SizedObject::String(s),
            JavaValue::Term(t) => SizedObject::Accountable(t as &dyn Accountable),
            JavaValue::This => this,
        }
    }

    fn node(&self, b: &mut ModelBuilder) -> NodeChild {
        match self {
            JavaValue::Long(v) => NodeChild::Node(b.long(*v)),
            JavaValue::Str(s) => NodeChild::Node(b.string(s)),
            JavaValue::Term(t) => NodeChild::Node(b.term(t)),
            JavaValue::This => NodeChild::This,
        }
    }
}

/// `new HashMap<String, Object>()`; `put` replaces the value of a present key.
struct JavaHashMap(Vec<(String, JavaValue)>);

impl JavaHashMap {
    fn put(&mut self, key: &str, value: JavaValue) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    /// The map as `RamUsageTester` walks it.
    fn model(&self) -> Rc<JavaNode> {
        let mut b = ModelBuilder::new();
        let entries = self
            .0
            .iter()
            .map(|(k, v)| (NodeChild::Node(b.string(k)), v.node(&mut b)))
            .collect();
        JavaNode::new(
            "java.util.HashMap",
            NodeShape::JavaMap {
                shallow_size: HASH_MAP_SHALLOW_SIZE,
                entries,
            },
        )
    }
}

impl SizedMap for JavaHashMap {
    fn shallow_size(&self) -> i64 {
        HASH_MAP_SHALLOW_SIZE
    }

    /// `HashMap.Node`: `int hash`, `K key`, `V value`, `Node next`.
    fn entry_shallow_size(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_declared_instance(&[4, REF, REF, REF])
    }

    fn entries(&self) -> Vec<(Option<SizedObject<'_>>, Option<SizedObject<'_>>)> {
        self.0
            .iter()
            .map(|(k, v)| {
                (
                    Some(SizedObject::String(k)),
                    Some(v.sized(SizedObject::Map(self))),
                )
            })
            .collect()
    }
}

/// `new ArrayList<Object>()`.
struct JavaArrayList(Vec<JavaValue>);

impl JavaArrayList {
    /// The list as `RamUsageTester` walks it.
    fn model(&self) -> Rc<JavaNode> {
        let mut b = ModelBuilder::new();
        let elements = self.0.iter().map(|v| v.node(&mut b)).collect();
        JavaNode::new(
            "java.util.ArrayList",
            NodeShape::JavaIterable {
                shallow_size: ARRAY_LIST_SHALLOW_SIZE,
                elements,
            },
        )
    }
}

impl SizedCollection for JavaArrayList {
    fn shallow_size(&self) -> i64 {
        ARRAY_LIST_SHALLOW_SIZE
    }

    fn elements(&self) -> Vec<Option<SizedObject<'_>>> {
        self.0
            .iter()
            .map(|v| Some(v.sized(SizedObject::Collection(self))))
            .collect()
    }
}

fn test_map() -> Result<()> {
    let mut map = JavaHashMap(Vec::new());
    map.put("primitive", JavaValue::Long(1234));
    map.put("string", JavaValue::Str("string".to_string()));
    for i in 0..100 {
        map.put(
            &format!("complex {i}"),
            JavaValue::Term(Term::from_text(format!("foo {i}"), &format!("bar {i}"))),
        );
    }
    let error_factor = if RamUsageEstimator::COMPRESSED_REFS_ENABLED {
        0.2
    } else {
        0.3
    };
    let mut actual = ram_used(&*map.model());
    let mut estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Map(&map)));
    assert_equals_delta(
        actual as f64,
        estimated as f64,
        actual as f64 * error_factor,
    );

    // test recursion
    map.put("self", JavaValue::This);
    actual = ram_used(&*map.model());
    estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Map(&map)));
    assert_equals_delta(
        actual as f64,
        estimated as f64,
        actual as f64 * error_factor,
    );
    Ok(())
}

fn test_collection() -> Result<()> {
    let mut list = JavaArrayList(Vec::new());
    list.0.push(JavaValue::Long(1234));
    list.0.push(JavaValue::Str("string".to_string()));
    for i in 0..100 {
        list.0.push(JavaValue::Term(Term::from_text(
            format!("foo {i}"),
            &format!("term {i}"),
        )));
    }
    let mut actual = ram_used(&*list.model());
    let mut estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Collection(&list)));
    assert_equals_delta(actual as f64, estimated as f64, actual as f64 * 0.1);

    // test recursion
    list.0.push(JavaValue::This);
    actual = ram_used(&*list.model());
    estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Collection(&list)));
    assert_equals_delta(actual as f64, estimated as f64, actual as f64 * 0.1);
    Ok(())
}

fn test_query() -> Result<()> {
    let dismax: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(
        vec![
            Arc::new(TermQuery::new(Term::from_text("foo1", "bar1"))) as Arc<dyn Query>,
            Arc::new(TermQuery::new(Term::from_text("baz1", "bam1"))),
        ],
        1.0,
    )?);
    let mut builder = BooleanQueryBuilder::new();
    builder.add(
        Arc::new(TermQuery::new(Term::from_text("foo2", "bar2"))),
        Occur::SHOULD,
    )?;
    builder.add(
        Arc::new(
            PhraseQueryBuilder::new()
                .add(Term::from_text("foo3", "baz3"))?
                .build()?,
        ),
        Occur::MUST_NOT,
    )?;
    builder.add(dismax, Occur::MUST)?;
    let bq = builder.build();
    let actual = ram_used(&*ModelBuilder::new().query(&bq));
    let estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Query(&bq)));
    // sizeOfObject uses much lower default size estimate than we normally use
    // but the query-specific default is so large that the comparison becomes meaningless.
    assert_equals_delta(actual as f64, estimated as f64, actual as f64 * 0.5);
    Ok(())
}

fn test_reference_size() -> Result<()> {
    let num_bytes_object_ref = RamUsageEstimator::NUM_BYTES_OBJECT_REF;
    assert!(num_bytes_object_ref == 4 || num_bytes_object_ref == 8);
    if Constants::jre_is_64bit() {
        assert_equals_msg(
            Some(
                "For 64 bit JVMs, reference size must be 8, unless compressed references are enabled",
            ),
            &if RamUsageEstimator::COMPRESSED_REFS_ENABLED {
                4
            } else {
                8
            },
            &num_bytes_object_ref,
        );
    } else {
        assert_equals_msg(
            Some("For 32bit JVMs, reference size must always be 4"),
            &4,
            &num_bytes_object_ref,
        );
        assert!(
            !RamUsageEstimator::COMPRESSED_REFS_ENABLED,
            "For 32bit JVMs, compressed references can never be enabled"
        );
    }
    Ok(())
}

/// `ManagementFactory.getCompilationMXBean()`.
///
/// **Divergence (declared).** The port runs on no JVM, so there is no
/// compilation system; the JDK documents `null` for a virtual machine that
/// has none, which is what this returns.
fn get_compilation_mx_bean_name() -> Option<String> {
    None
}

fn test_hotspot_bean() -> Result<()> {
    LuceneTestCase::assume_true(
        "testHotspotBean only works on 64bit JVMs.",
        Constants::jre_is_64bit(),
    );
    LuceneTestCase::assume_true(
        "testHotspotBean only works on Hotspot (OpenJDK, Oracle) virtual machines.",
        get_compilation_mx_bean_name()
            .unwrap_or_else(|| "Unknown".to_string())
            .starts_with("HotSpot"),
    );
    assert!(
        RamUsageEstimator::JVM_IS_HOTSPOT_64BIT,
        "We should have been able to detect Hotspot's internal settings from the management bean."
    );
    Ok(())
}

/// Helper to print out current settings for debugging `-Dtests.verbose=true`
fn test_print_values() -> Result<()> {
    LuceneTestCase::assume_true(
        "Specify -Dtests.verbose=true to print constants of RamUsageEstimator.",
        *VERBOSE,
    );
    println!(
        "JVM_IS_HOTSPOT_64BIT = {}",
        RamUsageEstimator::JVM_IS_HOTSPOT_64BIT
    );
    println!(
        "COMPRESSED_REFS_ENABLED = {}",
        RamUsageEstimator::COMPRESSED_REFS_ENABLED
    );
    println!(
        "NUM_BYTES_OBJECT_ALIGNMENT = {}",
        RamUsageEstimator::NUM_BYTES_OBJECT_ALIGNMENT
    );
    println!(
        "NUM_BYTES_OBJECT_REF = {}",
        RamUsageEstimator::NUM_BYTES_OBJECT_REF
    );
    println!(
        "NUM_BYTES_OBJECT_HEADER = {}",
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
    );
    println!(
        "NUM_BYTES_ARRAY_HEADER = {}",
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
    );
    println!("LONG_SIZE = {}", RamUsageEstimator::LONG_SIZE);
    Ok(())
}

/// `private static class Holder`.
#[allow(dead_code)]
struct Holder {
    field1: i64,
    name: String,
    holder: Option<Box<Holder>>,
    field2: i64,
    field3: i64,
    field4: i64,
}

impl Holder {
    /// `Holder()`.
    fn new() -> Self {
        Self {
            field1: 5000,
            name: "name".to_string(),
            holder: None,
            field2: 0,
            field3: 0,
            field4: 0,
        }
    }

    /// `Holder(String name, long field1)`.
    fn with(name: &str, field1: i64) -> Self {
        Self {
            field1,
            name: name.to_string(),
            ..Self::new()
        }
    }
}

/// What reflection reads of a `Holder`: the fields of [`HOLDER_FIELDS`],
/// following `name` and `holder`.
impl RamObject for Holder {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.util.TestRamUsageEstimator$Holder"
    }

    fn java_object(&self) -> JavaObject<'_> {
        JavaObject::Object {
            aligned_shallow_size: aligned_shallow_instance_size(&HOLDER_FIELDS),
            fields: vec![
                ("name", Some(&self.name as &dyn RamObject)),
                (
                    "holder",
                    self.holder.as_deref().map(|h| h as &dyn RamObject),
                ),
            ],
        }
    }
}

// `private static class HolderSubclass extends Holder { byte foo; int bar; }`
// and `private static class HolderSubclass2 extends Holder {}` are read only
// through `shallowSizeOfInstance`, whose declared field lists are above.
