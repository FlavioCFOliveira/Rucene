//! Plumbing shared by the classes of the `core_codecs` target: Java test
//! classes whose instance fields live in a fixture struct the runner
//! constructs.

#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

/// A public `test*` method of a class whose instance is the fixture `F`.
pub fn ftest<F: Any + Send>(name: &str, body: fn(&mut F) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |f: &mut F| {
            if let Err(e) = body(f) {
                rethrow_java(e)
            }
        })
}

/// `public void setUp() throws Exception { super.setUp(); body }`.
pub fn set_up<F: Any + Send>(body: fn(&mut F) -> Result<()>) -> Method {
    Method::new("setUp")
        .throws(&["java.lang.Exception"])
        .body(move |f: &mut F| {
            LuceneTestCase::super_set_up();
            if let Err(e) = body(f) {
                rethrow_java(e)
            }
        })
}

/// `public class name extends LuceneTestCase` whose instance is `F`
/// (`F::default()`, Java's field initialisers), with `methods`.
pub fn fixture_class<F: Any + Send + Default>(name: &str, methods: Vec<Method>) -> Arc<Class> {
    fixture_class_annotated::<F>(name, methods, |b| b)
}

/// [`fixture_class`] with class-level annotations.
pub fn fixture_class_annotated<F: Any + Send + Default>(
    name: &str,
    methods: Vec<Method>,
    annotate: impl FnOnce(ClassBuilder) -> ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(
        Class::builder(name)
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(F::default)),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}
