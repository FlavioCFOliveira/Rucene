/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestSerializableObjectClassFilter.java`.
//!
//! Java's test classes `CustomShape` and `NotASerializableObject` sit on the
//! class path `SerializableObject.readObject` searches by name; the port puts
//! them on its class path with
//! [`register_class`](rucene_spatial3d::geom::standard_objects::register_class),
//! once per process, before any test reads them. The flags Java's
//! `NotASerializableObject` sets on class initialization and construction are
//! kept and asserted; the port has no code path that initializes or
//! constructs a registered non-`SerializableObject` class.

use std::any::Any;
use std::fmt;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use rucene_spatial3d::geom::standard_objects::{register_class, JavaClass};

use crate::spatial3d_support::prelude::*;

static NON_SERIALIZABLE_CONSTRUCTED: AtomicBool = AtomicBool::new(false);
static NON_SERIALIZABLE_CLASS_INITIALIZED: AtomicBool = AtomicBool::new(false);

const CUSTOM_SHAPE: &str =
    "org.apache.lucene.spatial3d.geom.TestSerializableObjectClassFilter$CustomShape";
const NOT_A_SERIALIZABLE_OBJECT: &str =
    "org.apache.lucene.spatial3d.geom.TestSerializableObjectClassFilter$NotASerializableObject";

/// A SerializableObject that is not registered in `StandardObjects`.
pub struct CustomShape {
    value: i32,
}

impl CustomShape {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    /// `CustomShape(InputStream)`.
    pub fn from_stream(input: &mut dyn Read) -> Result<Self> {
        Ok(Self {
            value: serializable_object::read_int(input)?,
        })
    }
}

impl fmt::Display for CustomShape {
    /// `Object.toString()`: the class name, `@`, the identity hash in hex.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{:x}", CUSTOM_SHAPE, self.java_hash_code())
    }
}

impl SerializableObject for CustomShape {
    fn write(&self, output: &mut dyn Write) -> Result<()> {
        serializable_object::write_int(output, self.value)
    }
    fn class_name(&self) -> &'static str {
        CUSTOM_SHAPE
    }
    /// `Object.equals`: identity.
    fn java_equals(&self, o: &dyn Any) -> bool {
        o.downcast_ref::<CustomShape>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }
    /// `Object.hashCode`: an identity hash.
    fn java_hash_code(&self) -> i32 {
        (self as *const Self as usize) as i32
    }
}

/// Puts the test's classes on the port's class path.
fn register_classes() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        register_class(
            CUSTOM_SHAPE,
            JavaClass::Serializable {
                from_stream: Some(Arc::new(|input: &mut dyn Read| {
                    Ok(Arc::new(CustomShape::from_stream(input)?) as Arc<dyn SerializableObject>)
                })),
                from_stream_with_model: None,
            },
        );
        // Not a SerializableObject, but instantiable from an InputStream in
        // Java (its static initializer and constructor set the flags). A
        // registered non-`SerializableObject` class carries no constructor:
        // the flags stay unset unless `readObject` instantiated the class by
        // some other path.
        register_class(NOT_A_SERIALIZABLE_OBJECT, JavaClass::NotSerializable);
    });
}

/// `TestSerializableObjectClassFilter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestSerializableObjectClassFilter",
        vec![
            test("testCustomClassRoundTrips", test_custom_class_round_trips),
            test(
                "testRejectsNonSerializableObjectClass",
                test_rejects_non_serializable_object_class,
            ),
        ],
    )
}

fn test_custom_class_round_trips() -> Result<()> {
    register_classes();
    let shape = CustomShape::new(42);
    let mut out: Vec<u8> = Vec::new();
    serializable_object::write_object(&mut out, &shape)?;
    let mut input: &[u8] = &out;
    let copy = serializable_object::read_object(&mut input)?;
    Assert::assert_true(copy.as_any().is::<CustomShape>());
    Assert::assert_equals(42, class_cast::<CustomShape>(&*copy, CUSTOM_SHAPE)?.value);
    Ok(())
}

fn test_rejects_non_serializable_object_class() -> Result<()> {
    register_classes();
    let mut out: Vec<u8> = Vec::new();
    serializable_object::write_boolean(&mut out, false)?;
    serializable_object::write_string(&mut out, NOT_A_SERIALIZABLE_OBJECT)?;
    let mut input: &[u8] = &out;
    let e = expect_throws("java.io.IOException", || {
        serializable_object::read_object(&mut input)
    });
    Assert::assert_true(
        e.cause()
            .is_some_and(|c| c.is_instance("java.lang.ClassCastException")),
    );
    Assert::assert_true(
        e.message()
            .unwrap_or_default()
            .contains(NOT_A_SERIALIZABLE_OBJECT),
    );
    Assert::assert_false(NON_SERIALIZABLE_CLASS_INITIALIZED.load(Ordering::SeqCst));
    Assert::assert_false(NON_SERIALIZABLE_CONSTRUCTED.load(Ordering::SeqCst));
    Ok(())
}
