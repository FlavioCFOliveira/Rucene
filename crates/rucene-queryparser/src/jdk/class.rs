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

//! `java.lang.Class`: the part of the reflection the flexible query parser
//! uses on its node classes and on the `Number` classes of `PointsConfig`
//! (`getName`, `getSimpleName`, `getCanonicalName`, `getSuperclass`,
//! `getInterfaces`, `isAssignableFrom`, `toString`, `hashCode`).
//!
//! **Divergence.** An emulation of JDK 25 (`Class` read as specification),
//! not a port of a Lucene file. Rust has no runtime class objects: a class is
//! a static [`JavaClass`] descriptor that names its superclass and the
//! interfaces it implements directly, in declaration order, as
//! `getSuperclass()` and `getInterfaces()` return them. Two descriptors are
//! the same class when their binary names are equal, as two `Class` objects
//! of one class loader are. `hashCode()` is the identity hash, which the JVM
//! draws at random; the port uses the stable stand-in of
//! [`java_identity_hash_code_stand_in`].

use std::fmt;

use rucene::util::java_collections::java_identity_hash_code_stand_in;

/// A Java class or interface.
///
/// Equivalent to a `java.lang.Class` object.
#[derive(Debug)]
pub struct JavaClass {
    /// `getName()`: the binary name (`a.b.Outer$Inner`).
    name: &'static str,
    /// `isInterface()`.
    interface: bool,
    /// `getSuperclass()`: `None` for `java.lang.Object` and interfaces.
    superclass: Option<&'static JavaClass>,
    /// `getInterfaces()`: the interfaces the class implements (or the
    /// interface extends) directly, in declaration order.
    interfaces: &'static [&'static JavaClass],
}

impl JavaClass {
    /// A class: its binary name, its superclass (`None` only for
    /// `java.lang.Object`) and the interfaces it implements directly.
    pub const fn class(
        name: &'static str,
        superclass: Option<&'static JavaClass>,
        interfaces: &'static [&'static JavaClass],
    ) -> Self {
        Self {
            name,
            interface: false,
            superclass,
            interfaces,
        }
    }

    /// An interface: its binary name and the interfaces it extends directly.
    pub const fn interface(name: &'static str, interfaces: &'static [&'static JavaClass]) -> Self {
        Self {
            name,
            interface: true,
            superclass: None,
            interfaces,
        }
    }

    /// `getName()`.
    pub fn get_name(&self) -> &'static str {
        self.name
    }

    /// `getSimpleName()`: the name after the last `.` and `$`.
    pub fn get_simple_name(&self) -> &'static str {
        let after_dot = self.name.rsplit('.').next().unwrap_or(self.name);
        after_dot.rsplit('$').next().unwrap_or(after_dot)
    }

    /// `getCanonicalName()`: the binary name with `.` for `$`, as for a
    /// top-level or member class.
    pub fn get_canonical_name(&self) -> String {
        self.name.replace('$', ".")
    }

    /// `isInterface()`.
    pub fn is_interface(&self) -> bool {
        self.interface
    }

    /// `getSuperclass()`.
    pub fn get_superclass(&self) -> Option<&'static JavaClass> {
        self.superclass
    }

    /// `getInterfaces()`.
    pub fn get_interfaces(&self) -> &'static [&'static JavaClass] {
        self.interfaces
    }

    /// `isAssignableFrom(Class<?> cls)`: whether `cls` is this class, or a
    /// subclass or implementation of it, through its superclasses and all
    /// the interfaces they implement or extend.
    pub fn is_assignable_from(&self, cls: &JavaClass) -> bool {
        if self == cls {
            return true;
        }
        if cls.interfaces.iter().any(|i| self.is_assignable_from(i)) {
            return true;
        }
        match cls.superclass {
            Some(superclass) => self.is_assignable_from(superclass),
            // Every class and interface is assignable to `Object`.
            None => self.name == OBJECT.name,
        }
    }

    /// `toString()`: `"interface " + name` for an interface, `"class " +
    /// name` otherwise.
    pub fn to_java_string(&self) -> String {
        if self.interface {
            format!("interface {}", self.name)
        } else {
            format!("class {}", self.name)
        }
    }

    /// `hashCode()`, the identity hash (see the module documentation).
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in(self.name)
    }
}

/// The module and class loader of a class as JDK 25's `ClassCastException`
/// message names them (`Klass::class_in_module_of_loader`), for the
/// deployment Lucene runs in: the JDK classes the port names are in
/// `java.base` or `java.xml` of the bootstrap loader, every other class (the
/// Lucene jars on the class path) in the unnamed module of the application
/// loader.
fn module_of_loader(class: &str) -> &'static str {
    if class.starts_with("java.") {
        "module java.base of loader 'bootstrap'"
    } else if class.starts_with("org.w3c.dom.") || class.starts_with("javax.xml.") {
        "module java.xml of loader 'bootstrap'"
    } else {
        "unnamed module of loader 'app'"
    }
}

/// The message of the `ClassCastException` of the reference cast of an
/// object of class `from` to class `to`, as JDK 25 words it
/// (`SharedRuntime::generate_class_cast_message`): `class <from> cannot be
/// cast to class <to> (<from> and <to> are in <module>)`, or `(<from> is in
/// <module>; <to> is in <module>)` when their modules differ (probed on JDK
/// 25.0.5).
pub fn class_cast_message(from: &str, to: &str) -> String {
    let (m_from, m_to) = (module_of_loader(from), module_of_loader(to));
    if m_from == m_to {
        format!("class {from} cannot be cast to class {to} ({from} and {to} are in {m_from})")
    } else {
        format!(
            "class {from} cannot be cast to class {to} ({from} is in {m_from}; {to} is in {m_to})"
        )
    }
}

impl PartialEq for JavaClass {
    /// `==` on two `Class` objects of one class loader.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for JavaClass {}

impl fmt::Display for JavaClass {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// `java.lang.Object`.
pub static OBJECT: JavaClass = JavaClass::class("java.lang.Object", None, &[]);

/// `java.lang.Cloneable`.
pub static CLONEABLE: JavaClass = JavaClass::interface("java.lang.Cloneable", &[]);

/// `java.io.Serializable`.
pub static SERIALIZABLE: JavaClass = JavaClass::interface("java.io.Serializable", &[]);

/// `java.lang.Comparable`.
pub static COMPARABLE: JavaClass = JavaClass::interface("java.lang.Comparable", &[]);

/// `java.lang.constant.Constable`.
pub static CONSTABLE: JavaClass = JavaClass::interface("java.lang.constant.Constable", &[]);

/// `java.lang.constant.ConstantDesc`.
pub static CONSTANT_DESC: JavaClass = JavaClass::interface("java.lang.constant.ConstantDesc", &[]);

/// `java.lang.Number`.
pub static NUMBER: JavaClass =
    JavaClass::class("java.lang.Number", Some(&OBJECT), &[&SERIALIZABLE]);

/// `java.lang.Byte`.
pub static BYTE: JavaClass =
    JavaClass::class("java.lang.Byte", Some(&NUMBER), &[&COMPARABLE, &CONSTABLE]);

/// `java.lang.Short`.
pub static SHORT: JavaClass =
    JavaClass::class("java.lang.Short", Some(&NUMBER), &[&COMPARABLE, &CONSTABLE]);

/// `java.lang.Integer`.
pub static INTEGER: JavaClass = JavaClass::class(
    "java.lang.Integer",
    Some(&NUMBER),
    &[&COMPARABLE, &CONSTABLE, &CONSTANT_DESC],
);

/// `java.lang.Long`.
pub static LONG: JavaClass = JavaClass::class(
    "java.lang.Long",
    Some(&NUMBER),
    &[&COMPARABLE, &CONSTABLE, &CONSTANT_DESC],
);

/// `java.lang.Float`.
pub static FLOAT: JavaClass = JavaClass::class(
    "java.lang.Float",
    Some(&NUMBER),
    &[&COMPARABLE, &CONSTABLE, &CONSTANT_DESC],
);

/// `java.lang.Double`.
pub static DOUBLE: JavaClass = JavaClass::class(
    "java.lang.Double",
    Some(&NUMBER),
    &[&COMPARABLE, &CONSTABLE, &CONSTANT_DESC],
);

/// `java.math.BigInteger`.
pub static BIG_INTEGER: JavaClass =
    JavaClass::class("java.math.BigInteger", Some(&NUMBER), &[&COMPARABLE]);

/// `java.math.BigDecimal`.
pub static BIG_DECIMAL: JavaClass =
    JavaClass::class("java.math.BigDecimal", Some(&NUMBER), &[&COMPARABLE]);

/// `java.text.Format`.
pub static FORMAT: JavaClass = JavaClass::class(
    "java.text.Format",
    Some(&OBJECT),
    &[&SERIALIZABLE, &CLONEABLE],
);

/// `java.text.NumberFormat`.
pub static NUMBER_FORMAT: JavaClass =
    JavaClass::class("java.text.NumberFormat", Some(&FORMAT), &[]);

/// `java.text.DecimalFormat`.
pub static DECIMAL_FORMAT: JavaClass =
    JavaClass::class("java.text.DecimalFormat", Some(&NUMBER_FORMAT), &[]);
