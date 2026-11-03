//! `java.util.Random` (JDK 25), `Xoroshiro128PlusRandom` and
//! `AssertingRandom`: 20 seeds × 10 000 draws of every method and parameter
//! (`random_draws.tsv`), their errors (`random_errors.tsv`) and
//! `StrictMath.log` (`strict_math_log.tsv`).

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{assert_same_lines, catch, java_error, mask_after, rows, seeds, Rec};
use crate::jdk::random::{JdkRandom, Random};
use crate::jdk::strict_math;
use crate::jdk::thread::Thread;
use crate::{AssertingRandom, Xoroshiro128PlusRandom};

const DRAWS: usize = 10_000;

type Draw = Box<dyn Fn(&dyn Random, &mut Rec)>;

fn hex_f32(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

fn hex_f64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

/// The oracle's `randomMethods()`, label for label.
fn random_methods() -> Vec<(String, Draw)> {
    let mut m: Vec<(String, Draw)> = Vec::new();
    for bits in [1, 16, 31, 32] {
        m.push((
            format!("next({bits})"),
            Box::new(move |r, rec| rec.i(r.next(bits))),
        ));
    }
    m.push(("nextInt()".into(), Box::new(|r, rec| rec.i(r.next_int()))));
    for b in [
        1,
        2,
        3,
        10,
        16,
        100,
        1 << 30,
        (1 << 30) + 1,
        1_000_000_007,
        i32::MAX,
    ] {
        m.push((
            format!("nextInt({b})"),
            Box::new(move |r, rec| rec.i(r.next_int_bound(b))),
        ));
    }
    let int_ranges = [
        (0, 1),
        (-5, 5),
        (10, 26),
        (0, 1 << 20),
        (-3, -1),
        (i32::MIN, i32::MAX),
        (i32::MIN, 0),
        (-(1 << 30), 1 << 30),
        (i32::MIN, i32::MIN + 1),
        (i32::MAX - 1, i32::MAX),
        (-1, i32::MAX),
    ];
    for (o, b) in int_ranges {
        m.push((
            format!("nextInt({o},{b})"),
            Box::new(move |r, rec| rec.i(r.next_int_range(o, b))),
        ));
    }
    m.push(("nextLong()".into(), Box::new(|r, rec| rec.l(r.next_long()))));
    for b in [
        1,
        2,
        3,
        1000,
        1i64 << 40,
        1i64 << 62,
        (1i64 << 62) + 1,
        i64::MAX,
    ] {
        m.push((
            format!("nextLong({b})"),
            Box::new(move |r, rec| rec.l(r.next_long_bound(b))),
        ));
    }
    let long_ranges = [
        (0, 1),
        (-1, 1),
        (0, 1i64 << 33),
        (i64::MIN, i64::MAX),
        (i64::MIN, 0),
        (-10, 1i64 << 62),
        (i64::MIN, i64::MIN + 1),
        (-(1i64 << 62), 1i64 << 62),
    ];
    for (o, b) in long_ranges {
        m.push((
            format!("nextLong({o},{b})"),
            Box::new(move |r, rec| rec.l(r.next_long_range(o, b))),
        ));
    }
    m.push((
        "nextBoolean()".into(),
        Box::new(|r, rec| rec.z(r.next_boolean())),
    ));
    m.push((
        "nextFloat()".into(),
        Box::new(|r, rec| rec.f(r.next_float())),
    ));
    for b in [1f32, 0.5, 3.0, 1e30, f32::from_bits(1), f32::MAX] {
        m.push((
            format!("nextFloat({})", hex_f32(b)),
            Box::new(move |r, rec| rec.f(r.next_float_bound(b))),
        ));
    }
    let float_ranges = [
        (0f32, 1f32),
        (-1.0, 1.0),
        (-f32::MAX, f32::MAX),
        (1e-30, 2e-30),
        (-5.0, -4.5),
        (0.0, f32::from_bits(1)),
    ];
    for (o, b) in float_ranges {
        m.push((
            format!("nextFloat({},{})", hex_f32(o), hex_f32(b)),
            Box::new(move |r, rec| rec.f(r.next_float_range(o, b))),
        ));
    }
    m.push((
        "nextDouble()".into(),
        Box::new(|r, rec| rec.d(r.next_double())),
    ));
    for b in [1f64, 0.5, 3.0, 1e300, f64::from_bits(1), f64::MAX] {
        m.push((
            format!("nextDouble({})", hex_f64(b)),
            Box::new(move |r, rec| rec.d(r.next_double_bound(b))),
        ));
    }
    let double_ranges = [
        (0f64, 1f64),
        (-1.0, 1.0),
        (-f64::MAX, f64::MAX),
        (1e-300, 2e-300),
        (-5.0, -4.5),
        (0.0, f64::from_bits(1)),
        (-1e308, 1e308),
    ];
    for (o, b) in double_ranges {
        m.push((
            format!("nextDouble({},{})", hex_f64(o), hex_f64(b)),
            Box::new(move |r, rec| rec.d(r.next_double_range(o, b))),
        ));
    }
    m.push((
        "nextGaussian()".into(),
        Box::new(|r, rec| rec.d(r.next_gaussian())),
    ));
    for len in [0usize, 1, 3, 4, 7, 8, 9, 17] {
        m.push((
            format!("nextBytes({len})"),
            Box::new(move |r, rec| {
                let mut a = vec![0u8; len];
                r.next_bytes(&mut a);
                rec.bytes(&a);
            }),
        ));
    }
    m.push((
        "isDeprecated()".into(),
        Box::new(|r, rec| rec.z(r.is_deprecated())),
    ));
    m
}

/// The oracle's `streamMethods()`: one call consumes the whole stream.
fn stream_methods() -> Vec<(String, Draw)> {
    let n = DRAWS as i64;
    let mut m: Vec<(String, Draw)> = Vec::new();
    m.push((
        "ints(10000)".into(),
        Box::new(move |r, rec| (&r).ints_sized(n).for_each(|v| rec.i(v))),
    ));
    m.push((
        "ints()".into(),
        Box::new(|r, rec| (&r).ints().take(DRAWS).for_each(|v| rec.i(v))),
    ));
    for (o, b) in [(0, 10), (i32::MIN, i32::MAX), (-5, 5), (0, 1 << 20)] {
        m.push((
            format!("ints(10000,{o},{b})"),
            Box::new(move |r, rec| (&r).ints_sized_range(n, o, b).for_each(|v| rec.i(v))),
        ));
        m.push((
            format!("ints({o},{b})"),
            Box::new(move |r, rec| (&r).ints_range(o, b).take(DRAWS).for_each(|v| rec.i(v))),
        ));
    }
    m.push((
        "longs(10000)".into(),
        Box::new(move |r, rec| (&r).longs_sized(n).for_each(|v| rec.l(v))),
    ));
    m.push((
        "longs()".into(),
        Box::new(|r, rec| (&r).longs().take(DRAWS).for_each(|v| rec.l(v))),
    ));
    for (o, b) in [(0, 10), (i64::MIN, i64::MAX), (-5, 1i64 << 40)] {
        m.push((
            format!("longs(10000,{o},{b})"),
            Box::new(move |r, rec| (&r).longs_sized_range(n, o, b).for_each(|v| rec.l(v))),
        ));
        m.push((
            format!("longs({o},{b})"),
            Box::new(move |r, rec| (&r).longs_range(o, b).take(DRAWS).for_each(|v| rec.l(v))),
        ));
    }
    m.push((
        "doubles(10000)".into(),
        Box::new(move |r, rec| (&r).doubles_sized(n).for_each(|v| rec.d(v))),
    ));
    m.push((
        "doubles()".into(),
        Box::new(|r, rec| (&r).doubles().take(DRAWS).for_each(|v| rec.d(v))),
    ));
    for (o, b) in [(0f64, 1f64), (-f64::MAX, f64::MAX), (-5.0, -4.5)] {
        let lbl = format!("{},{}", hex_f64(o), hex_f64(b));
        m.push((
            format!("doubles(10000,{lbl})"),
            Box::new(move |r, rec| (&r).doubles_sized_range(n, o, b).for_each(|v| rec.d(v))),
        ));
        m.push((
            format!("doubles({lbl})"),
            Box::new(move |r, rec| (&r).doubles_range(o, b).take(DRAWS).for_each(|v| rec.d(v))),
        ));
    }
    m
}

fn generator(kind: &str, seed: i64) -> Arc<dyn Random> {
    match kind {
        "jdk" => Arc::new(JdkRandom::new(seed)),
        "xoro" => Arc::new(Xoroshiro128PlusRandom::new(seed)),
        "asserting" => Arc::new(AssertingRandom::new(
            &Thread::current(),
            Arc::new(Xoroshiro128PlusRandom::new(seed)),
        )),
        other => panic!("{other}"),
    }
}

fn interleaved(r: &dyn Random, rec: &mut Rec) {
    for _ in 0..DRAWS {
        rec.i(r.next_int());
        rec.i(r.next_int_bound(7));
        rec.l(r.next_long());
        rec.d(r.next_gaussian());
        rec.f(r.next_float());
        rec.z(r.next_boolean());
        rec.d(r.next_double());
        rec.i(r.next_int_range(-3, 9));
        let mut a = [0u8; 5];
        r.next_bytes(&mut a);
        rec.bytes(&a);
        rec.d(r.next_gaussian());
    }
}

fn check_generator(kind: &str) {
    let expected: BTreeMap<String, String> = rows("random_draws.tsv")
        .into_iter()
        .filter(|r| r[0] == kind)
        .map(|r| (format!("{}\t{}", r[1], r[2]), format!("{}\t{}", r[3], r[4])))
        .collect();
    let methods = random_methods();
    let streams = stream_methods();
    let mut actual = BTreeMap::new();
    for seed in seeds() {
        let seed_hex = format!("{:x}", seed);
        for (label, draw) in &methods {
            if kind == "asserting" && label.starts_with("next(") {
                continue;
            }
            let r = generator(kind, seed);
            let mut rec = Rec::default();
            for _ in 0..DRAWS {
                draw(&*r, &mut rec);
            }
            actual.insert(format!("{seed_hex}\t{label}"), rec.line());
        }
        for (label, draw) in &streams {
            let r = generator(kind, seed);
            let mut rec = Rec::default();
            draw(&*r, &mut rec);
            actual.insert(format!("{seed_hex}\t{label}"), rec.line());
        }
        let r = generator(kind, seed);
        let mut rec = Rec::default();
        interleaved(&*r, &mut rec);
        actual.insert(format!("{seed_hex}\tinterleaved"), rec.line());
    }
    assert_same_lines(&format!("{kind} draws"), &expected, &actual);
}

#[test]
fn jdk_random_draws_equal_jdk25() {
    check_generator("jdk");
}

#[test]
fn xoroshiro_draws_equal_randomizedtesting() {
    check_generator("xoro");
}

#[test]
fn asserting_random_draws_equal_randomizedtesting() {
    check_generator("asserting");
}

type Call = Box<dyn Fn(&dyn Random)>;

fn error_calls() -> Vec<(&'static str, Call)> {
    let inf32 = f32::INFINITY;
    let inf = f64::INFINITY;
    vec![
        (
            "nextInt(0)",
            Box::new(|r| {
                r.next_int_bound(0);
            }),
        ),
        (
            "nextInt(-1)",
            Box::new(|r| {
                r.next_int_bound(-1);
            }),
        ),
        (
            "nextInt(5,5)",
            Box::new(|r| {
                r.next_int_range(5, 5);
            }),
        ),
        (
            "nextInt(5,4)",
            Box::new(|r| {
                r.next_int_range(5, 4);
            }),
        ),
        (
            "nextLong(0)",
            Box::new(|r| {
                r.next_long_bound(0);
            }),
        ),
        (
            "nextLong(-1)",
            Box::new(|r| {
                r.next_long_bound(-1);
            }),
        ),
        (
            "nextLong(3,3)",
            Box::new(|r| {
                r.next_long_range(3, 3);
            }),
        ),
        (
            "nextLong(3,2)",
            Box::new(|r| {
                r.next_long_range(3, 2);
            }),
        ),
        (
            "nextFloat(0)",
            Box::new(|r| {
                r.next_float_bound(0.0);
            }),
        ),
        (
            "nextFloat(-1)",
            Box::new(|r| {
                r.next_float_bound(-1.0);
            }),
        ),
        (
            "nextFloat(NaN)",
            Box::new(|r| {
                r.next_float_bound(f32::NAN);
            }),
        ),
        (
            "nextFloat(inf)",
            Box::new(move |r| {
                r.next_float_bound(inf32);
            }),
        ),
        (
            "nextFloat(1,1)",
            Box::new(|r| {
                r.next_float_range(1.0, 1.0);
            }),
        ),
        (
            "nextFloat(0,inf)",
            Box::new(move |r| {
                r.next_float_range(0.0, inf32);
            }),
        ),
        (
            "nextFloat(-inf,0)",
            Box::new(move |r| {
                r.next_float_range(-inf32, 0.0);
            }),
        ),
        (
            "nextFloat(NaN,1)",
            Box::new(|r| {
                r.next_float_range(f32::NAN, 1.0);
            }),
        ),
        (
            "nextDouble(0)",
            Box::new(|r| {
                r.next_double_bound(0.0);
            }),
        ),
        (
            "nextDouble(-1)",
            Box::new(|r| {
                r.next_double_bound(-1.0);
            }),
        ),
        (
            "nextDouble(NaN)",
            Box::new(|r| {
                r.next_double_bound(f64::NAN);
            }),
        ),
        (
            "nextDouble(inf)",
            Box::new(move |r| {
                r.next_double_bound(inf);
            }),
        ),
        (
            "nextDouble(1,1)",
            Box::new(|r| {
                r.next_double_range(1.0, 1.0);
            }),
        ),
        (
            "nextDouble(0,inf)",
            Box::new(move |r| {
                r.next_double_range(0.0, inf);
            }),
        ),
        (
            "nextDouble(-inf,0)",
            Box::new(move |r| {
                r.next_double_range(-inf, 0.0);
            }),
        ),
        (
            "nextDouble(NaN,1)",
            Box::new(|r| {
                r.next_double_range(f64::NAN, 1.0);
            }),
        ),
        (
            "ints(-1)",
            Box::new(|r| {
                (&r).ints_sized(-1);
            }),
        ),
        (
            "ints(5,5)",
            Box::new(|r| {
                (&r).ints_range(5, 5);
            }),
        ),
        (
            "ints(10,5,5)",
            Box::new(|r| {
                (&r).ints_sized_range(10, 5, 5);
            }),
        ),
        (
            "ints(-1,0,5)",
            Box::new(|r| {
                (&r).ints_sized_range(-1, 0, 5);
            }),
        ),
        (
            "longs(-1)",
            Box::new(|r| {
                (&r).longs_sized(-1);
            }),
        ),
        (
            "longs(5,5)",
            Box::new(|r| {
                (&r).longs_range(5, 5);
            }),
        ),
        (
            "longs(10,5,5)",
            Box::new(|r| {
                (&r).longs_sized_range(10, 5, 5);
            }),
        ),
        (
            "doubles(-1)",
            Box::new(|r| {
                (&r).doubles_sized(-1);
            }),
        ),
        (
            "doubles(1,1)",
            Box::new(|r| {
                (&r).doubles_range(1.0, 1.0);
            }),
        ),
        (
            "doubles(10,0,inf)",
            Box::new(move |r| {
                (&r).doubles_sized_range(10, 0.0, inf);
            }),
        ),
        (
            "doubles(10,NaN,1)",
            Box::new(|r| {
                (&r).doubles_sized_range(10, f64::NAN, 1.0);
            }),
        ),
        ("setSeed(1)", Box::new(|r| r.set_seed(1))),
        ("setSeed(0)", Box::new(|r| r.set_seed(0))),
    ]
}

fn result_of(f: impl FnOnce()) -> String {
    match catch(f) {
        Ok(()) => "OK\t-".to_string(),
        Err(p) => java_error(&p),
    }
}

#[test]
fn random_errors_equal_jdk25() {
    let expected: BTreeMap<String, String> = rows("random_errors.tsv")
        .into_iter()
        .filter(|r| {
            ["jdk", "xoro", "asserting"].contains(&r[0].as_str())
                && !r[1].starts_with("foreign")
                && !r[1].starts_with("destroyed")
                && r[1] != "next(32)"
        })
        .map(|r| (format!("{}\t{}", r[0], r[1]), format!("{}\t{}", r[2], r[3])))
        .collect();
    let mut actual = BTreeMap::new();
    for kind in ["jdk", "xoro", "asserting"] {
        for (label, call) in error_calls() {
            let r = generator(kind, 7);
            actual.insert(format!("{kind}\t{label}"), result_of(|| call(&*r)));
        }
    }
    assert_same_lines("random errors", &expected, &actual);
}

fn error_row(call: &str) -> String {
    rows("random_errors.tsv")
        .into_iter()
        .find(|r| r[1] == call)
        .map(|r| r[2..].join("\t"))
        .unwrap_or_else(|| panic!("no row {call}"))
}

#[test]
fn asserting_random_misuse_messages_equal_randomizedtesting() {
    let a = Arc::new(AssertingRandom::new(
        &Thread::current(),
        Arc::new(Xoroshiro128PlusRandom::new(1)),
    ));
    assert_eq!(
        result_of(|| {
            a.next(32);
        }),
        error_row("next(32)")
    );

    // Foreign thread: the oracle's owner was `main`, the user `foreign-thread`.
    let owner_box: Arc<std::sync::Mutex<Option<Arc<AssertingRandom>>>> = Arc::default();
    let result: Arc<std::sync::Mutex<String>> = Arc::default();
    let (ob, res) = (owner_box.clone(), result.clone());
    let owner = Thread::new("main", move || {
        let a = Arc::new(AssertingRandom::new(
            &Thread::current(),
            Arc::new(Xoroshiro128PlusRandom::new(1)),
        ));
        *ob.lock().unwrap() = Some(a.clone());
        let res2 = res.clone();
        let foreign = Thread::new("foreign-thread", move || {
            *res2.lock().unwrap() = match catch(|| a.next_int()) {
                Ok(_) => "OK".into(),
                Err(p) => p,
            };
        });
        foreign.start();
        foreign.join().unwrap();
    });
    owner.start();
    owner.join().unwrap();
    let message = result.lock().unwrap().clone();
    let mut lines = message.lines();
    let head = java_error(lines.next().unwrap());
    let cause = java_error(lines.next().unwrap().trim_start_matches("Caused by: "));
    assert_eq!(
        mask_after(&head, "#"),
        mask_after(&error_row("foreign-thread"), "#")
    );
    assert_eq!(
        mask_after(&cause, "#"),
        mask_after(&error_row("foreign-thread-cause"), "#")
    );

    // Destroyed.
    let a = owner_box.lock().unwrap().clone().unwrap();
    a.destroy();
    let destroyed = Arc::new(AssertingRandom::new(
        &Thread::current(),
        Arc::new(Xoroshiro128PlusRandom::new(1)),
    ));
    destroyed.destroy();
    assert_eq!(
        result_of(|| {
            destroyed.next_int();
        }),
        error_row("destroyed-nextInt")
    );
    assert!(AssertingRandom::is_verifying());
}

#[test]
fn thread_formatting_equals_jdk25() {
    let expect = |k: &str| mask_after(&error_row(k), "id=");
    let expect_hash = |k: &str| mask_after(&error_row(k), "#");
    let t = Thread::new("unstarted", || {});
    let got_name = crate::threads::Threads::thread_name(&t);
    assert_eq!(
        mask_after(&got_name, "id="),
        expect("Threads.threadName(new)")
    );
    assert_eq!(
        mask_after(&t.to_string(), "#"),
        expect_hash("Thread.toString(new)")
    );
    t.start();
    t.join().unwrap();
    let got_name = crate::threads::Threads::thread_name(&t);
    assert_eq!(
        mask_after(&got_name, "id="),
        expect("Threads.threadName(terminated)")
    );
    assert_eq!(
        mask_after(&t.to_string(), "#"),
        expect_hash("Thread.toString(terminated)")
    );
    let main_like = Thread::new("main", || {
        let me = Thread::current();
        assert_eq!(
            mask_after(&crate::threads::Threads::thread_name(&me), "id="),
            mask_after(&error_row("Threads.threadName"), "id=")
        );
        assert_eq!(
            mask_after(&me.to_string(), "#"),
            mask_after(&error_row("Thread.toString"), "#")
        );
    });
    main_like.start();
    main_like.join().unwrap();
}

#[test]
fn strict_math_log_equals_jdk25() {
    for r in rows("strict_math_log.tsv") {
        match r[0].as_str() {
            "edge" => {
                let x = f64::from_bits(u64::from_str_radix(&r[1], 16).unwrap());
                let want = u64::from_str_radix(&r[2], 16).unwrap();
                let got = strict_math::log(x).to_bits();
                assert_eq!(got, want, "log({x:e}) bits {got:x} != {want:x}");
            }
            "digest" => {
                let seed = u64::from_str_radix(&r[1], 16).unwrap() as i64;
                let rnd = Xoroshiro128PlusRandom::new(seed);
                let mut rec = Rec::default();
                for _ in 0..DRAWS {
                    let s = rnd.next_double();
                    rec.d(strict_math::log(s));
                    rec.d(strict_math::log(1.0 + (s - 0.5) * 1e-6));
                    rec.d(strict_math::log(f64::from_bits(
                        (rnd.next_long() as u64) >> 1,
                    )));
                }
                assert_eq!(
                    rec.line(),
                    format!("{}\t{}", r[2], r[3]),
                    "log digest seed {seed:x}"
                );
            }
            other => panic!("{other}"),
        }
    }
}
