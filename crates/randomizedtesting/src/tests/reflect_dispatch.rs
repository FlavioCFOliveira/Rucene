//! `Method.invoke` selection (JDK 25 `DirectMethodHandleAccessor` over a
//! `findStatic`/`findVirtual` handle; JVMS 5.4.5 and 5.4.6): the cases the
//! runner oracle cannot reach from one package (package access across
//! packages, the transitive overriding rule) and the receiver checks.

use std::sync::{Arc, Mutex};

use crate::jdk::exception::Throwable;
use crate::jdk::reflect::{modifier, Class, Constructor, Instance, Method};

static LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Serialises the tests of this module, which share [`LOG`].
static SERIAL: Mutex<()> = Mutex::new(());

fn logger(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || LOG.lock().unwrap_or_else(|e| e.into_inner()).push(s)
}

fn take_log() -> Vec<&'static str> {
    std::mem::take(&mut *LOG.lock().unwrap_or_else(|e| e.into_inner()))
}

fn method(class: &Class, name: &str) -> Arc<Method> {
    class
        .declared_methods()
        .iter()
        .find(|m| m.name() == name)
        .map(Arc::clone)
        .unwrap_or_else(|| panic!("{name} declared"))
}

fn instance(class: &Class) -> Instance {
    class.declared_constructors()[0].new_instance(&[])
}

fn invoke(m: &Method, receiver: Option<&Instance>) -> Result<Vec<&'static str>, Throwable> {
    take_log();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| m.invoke(receiver, &[])))
        .map(|_| take_log())
        .map_err(Throwable::from_panic)
}

/// `a.A` declares `m` (public), `p` (package access), `q` (protected),
/// `r` (private) and the static `s`; `a.B extends A` and `b.C extends B`
/// redeclare them; `b.C` also redeclares the package-access `p` of another
/// package and the private `r` as public, which override neither (JVMS
/// 5.4.5).
fn hierarchy() -> (Arc<Class>, Arc<Class>, Arc<Class>) {
    let a = Class::builder("a.A")
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("m").body_any(logger("A.m")))
        .method(Method::new("p").modifiers(0).body_any(logger("A.p")))
        .method(
            Method::new("q")
                .modifiers(modifier::PROTECTED)
                .body_any(logger("A.q")),
        )
        .method(
            Method::new("r")
                .modifiers(modifier::PRIVATE)
                .body_any(logger("A.r")),
        )
        .method(Method::new("s").static_body(logger("A.s")))
        .method(Method::new("abs").modifiers(modifier::PUBLIC | modifier::ABSTRACT))
        .build();
    let b = Class::builder("a.B")
        .extends(&a)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("q").body_any(logger("B.q")))
        .method(
            Method::new("r")
                .modifiers(modifier::PRIVATE)
                .body_any(logger("B.r")),
        )
        .method(Method::new("s").static_body(logger("B.s")))
        .build();
    let c = Class::builder("b.C")
        .extends(&b)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("m").body_any(logger("C.m")))
        .method(Method::new("p").modifiers(0).body_any(logger("C.p")))
        .method(Method::new("abs").body_any(logger("C.abs")))
        .method(Method::new("r").body_any(logger("C.r")))
        .build();
    (a, b, c)
}

#[test]
fn invoke_selects_the_receivers_most_derived_override() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b, c) = hierarchy();
    let (ia, ib, ic) = (instance(&a), instance(&b), instance(&c));
    let m = method(&a, "m");
    assert_eq!(invoke(&m, Some(&ia)).unwrap(), ["A.m"]);
    // B inherits A.m.
    assert_eq!(invoke(&m, Some(&ib)).unwrap(), ["A.m"]);
    assert_eq!(invoke(&m, Some(&ic)).unwrap(), ["C.m"]);
    // A protected method overridden by a public one.
    assert_eq!(invoke(&method(&a, "q"), Some(&ic)).unwrap(), ["B.q"]);
    // Invoking the override's own Method dispatches the same way.
    assert_eq!(invoke(&method(&b, "q"), Some(&ic)).unwrap(), ["B.q"]);
    // An abstract method runs the receiver's implementation.
    assert_eq!(invoke(&method(&a, "abs"), Some(&ic)).unwrap(), ["C.abs"]);
}

#[test]
fn package_access_is_not_overridden_from_another_package() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (a, _b, c) = hierarchy();
    let ic = instance(&c);
    assert_eq!(invoke(&method(&a, "p"), Some(&ic)).unwrap(), ["A.p"]);
    // C.p is invoked through its own Method.
    assert_eq!(invoke(&method(&c, "p"), Some(&ic)).unwrap(), ["C.p"]);
}

/// JVMS 5.4.5's transitive case: `x.D.p` (package access) is overridden by
/// `x.E.p` made `public`, which `y.F.p` overrides from another package.
#[test]
fn transitive_override_across_packages() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let d = Class::builder("x.D")
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("p").modifiers(0).body_any(logger("D.p")))
        .build();
    let e = Class::builder("x.E")
        .extends(&d)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("p").body_any(logger("E.p")))
        .build();
    let f = Class::builder("y.F")
        .extends(&e)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("p").body_any(logger("F.p")))
        .build();
    let g = Class::builder("y.G")
        .extends(&d)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("p").body_any(logger("G.p")))
        .build();
    assert_eq!(
        invoke(&method(&d, "p"), Some(&instance(&f))).unwrap(),
        ["F.p"]
    );
    // Without the public intermediate, y.G.p does not override x.D.p.
    assert_eq!(
        invoke(&method(&d, "p"), Some(&instance(&g))).unwrap(),
        ["D.p"]
    );
}

#[test]
fn private_and_static_methods_run_their_own_body() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b, c) = hierarchy();
    let ic = instance(&c);
    assert_eq!(invoke(&method(&a, "r"), Some(&ic)).unwrap(), ["A.r"]);
    assert_eq!(invoke(&method(&b, "r"), Some(&ic)).unwrap(), ["B.r"]);
    // A static method ignores the receiver, null included.
    assert_eq!(invoke(&method(&a, "s"), Some(&ic)).unwrap(), ["A.s"]);
    assert_eq!(invoke(&method(&a, "s"), None).unwrap(), ["A.s"]);
}

#[test]
fn receiver_checks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b, c) = hierarchy();
    let t = invoke(&method(&a, "m"), None).unwrap_err();
    assert_eq!(t.class_name(), "java.lang.NullPointerException");
    // checkReceiver: a superclass instance is not an instance of B.
    let t = invoke(&method(&b, "q"), Some(&instance(&a))).unwrap_err();
    assert_eq!(t.class_name(), "java.lang.IllegalArgumentException");
    assert_eq!(
        t.message(),
        Some("object of type a.A is not an instance of a.B")
    );
    // The abstract method itself, on a receiver without an implementation.
    let t = invoke(&method(&a, "abs"), Some(&instance(&b))).unwrap_err();
    assert_eq!(t.class_name(), "java.lang.AbstractMethodError");
    // The receiver's runtime class.
    assert_eq!(instance(&c).get_class().name(), "b.C");
}

/// Within one package: `k.H`'s public `r` does not override `k.K`'s
/// private `r`, and a private `u` in `k.H` (a separately compiled class
/// file; `javac` refuses the source) does not override `k.K`'s public `u`.
/// JDK 25 prints `K.r` and `K.u`.
#[test]
fn private_declarations_never_take_part_in_overriding() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let k = Class::builder("k.K")
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("r")
                .modifiers(modifier::PRIVATE)
                .body_any(logger("K.r")),
        )
        .method(Method::new("u").body_any(logger("K.u")))
        .build();
    let h = Class::builder("k.H")
        .extends(&k)
        .constructor(Constructor::no_args(|| ()))
        .method(Method::new("r").body_any(logger("H.r")))
        .method(
            Method::new("u")
                .modifiers(modifier::PRIVATE)
                .body_any(logger("H.u")),
        )
        .build();
    let ih = instance(&h);
    assert_eq!(invoke(&method(&k, "r"), Some(&ih)).unwrap(), ["K.r"]);
    assert_eq!(invoke(&method(&k, "u"), Some(&ih)).unwrap(), ["K.u"]);
}
