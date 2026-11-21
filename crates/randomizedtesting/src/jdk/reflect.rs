//! `java.lang.Class`, `java.lang.reflect.Method`, `Field` and
//! `Constructor`, and runtime-visible annotations, as Rust declarations.
//!
//! randomizedtesting's runner is driven by reflection: it reads a suite
//! class's annotations, walks its superclass chain, lists its declared
//! methods and fields, sorts them by `toGenericString()`, checks their
//! modifiers and parameter counts, invokes them and reads rule fields. Rust
//! has neither reflection nor runtime annotations, so a suite *declares*
//! what Java would reflect: a [`Class`] built with [`ClassBuilder`] holds its
//! binary name, modifiers, superclass, annotations, constructor, methods and
//! fields, each method with the modifiers, return type and parameter types
//! Java would report, its annotations and a Rust body. The runner reads this
//! declaration exactly where Java calls reflection, so a ported suite reads
//! close to its Java source: one builder call per Java member, one
//! annotation value per Java annotation.
//!
//! **Divergences.**
//! * A declaration replaces reflection: what the runner observes is what the
//!   suite declares, so a declaration must state the Java modifiers and
//!   types for the runner's validation, ordering and messages to match
//!   Java's.
//! * An annotation is a Rust value implementing [`JavaAnnotation`]; its
//!   type is an [`AnnotationType`] constant, and `Class.getAnnotation`
//!   is [`Class::get_annotation`] (which includes `@Inherited` annotations
//!   of superclasses, as Java does).
//! * An instance of a suite class is a boxed Rust value with its runtime
//!   class ([`Instance`]); instance method bodies receive the value as
//!   `&mut T`. Invoking a body holds the instance's lock for the call.
//!   [`Method::invoke`] selects the body as the JVM selects it for
//!   `Method.invoke` (the receiver's most-derived override); an override is
//!   matched by name and parameter types, the Java-language rule that the
//!   JVM implements through the bridge methods `javac` emits for covariant
//!   returns, which a declaration does not declare.
//! * Constructor and factory arguments are [`Value`]s, the `Object`s a Java
//!   parameterized suite passes; `String.valueOf` of a value is its
//!   [`Display`](std::fmt::Display).

use std::any::Any;
use std::fmt;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use super::exception::{self, Throwable};
pub use crate::annotations::AnnotationType;

/// `java.lang.reflect.Modifier` bits.
pub mod modifier {
    /// `Modifier.PUBLIC`.
    pub const PUBLIC: i32 = 0x0001;
    /// `Modifier.PRIVATE`.
    pub const PRIVATE: i32 = 0x0002;
    /// `Modifier.PROTECTED`.
    pub const PROTECTED: i32 = 0x0004;
    /// `Modifier.STATIC`.
    pub const STATIC: i32 = 0x0008;
    /// `Modifier.FINAL`.
    pub const FINAL: i32 = 0x0010;
    /// `Modifier.SYNCHRONIZED`.
    pub const SYNCHRONIZED: i32 = 0x0020;
    /// `Modifier.VOLATILE`.
    pub const VOLATILE: i32 = 0x0040;
    /// `Modifier.TRANSIENT`.
    pub const TRANSIENT: i32 = 0x0080;
    /// `Modifier.NATIVE`.
    pub const NATIVE: i32 = 0x0100;
    /// `Modifier.INTERFACE`.
    pub const INTERFACE: i32 = 0x0200;
    /// `Modifier.ABSTRACT`.
    pub const ABSTRACT: i32 = 0x0400;
    /// `Modifier.STRICT`.
    pub const STRICT: i32 = 0x0800;

    /// `Modifier.toString(mod)`: the keywords in the order the JLS
    /// recommends.
    pub fn to_string(m: i32) -> String {
        let names = [
            (PUBLIC, "public"),
            (PROTECTED, "protected"),
            (PRIVATE, "private"),
            (ABSTRACT, "abstract"),
            (STATIC, "static"),
            (FINAL, "final"),
            (TRANSIENT, "transient"),
            (VOLATILE, "volatile"),
            (SYNCHRONIZED, "synchronized"),
            (NATIVE, "native"),
            (STRICT, "strictfp"),
            (INTERFACE, "interface"),
        ];
        names
            .iter()
            .filter(|(bit, _)| m & bit != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A runtime-visible annotation (`java.lang.annotation.Annotation`).
pub trait JavaAnnotation: Any + Send + Sync + fmt::Debug {
    /// `annotationType()`.
    fn annotation_type(&self) -> &'static AnnotationType;

    /// `toString()` of JDK 25: `@` and the canonical type name, then the
    /// members in parentheses (`value` unnamed when it is the only one).
    /// The default suits an annotation without members.
    fn to_java_string(&self) -> String {
        format!("@{}()", self.annotation_type().canonical_name())
    }

    /// `self` as `Any`, for [`Class::get_annotation`]'s downcast.
    fn as_any(&self) -> &dyn Any;
}

/// A shared annotation value.
pub type AnnotationRef = Arc<dyn JavaAnnotation>;

/// An annotation without members of a given type (for example a user test
/// group, `@MyGroup`).
#[derive(Debug, Clone, Copy)]
pub struct Marker(pub &'static AnnotationType);

impl JavaAnnotation for Marker {
    fn annotation_type(&self) -> &'static AnnotationType {
        self.0
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Quotes a string member as JDK 25's `Annotation.toString()` does
/// (`AnnotationInvocationHandler.toSourceString(String)`): per UTF-16 code
/// unit, `"` and `\` escaped with a backslash, `\b`, `\f`, `\n`, `\r` and
/// `\t` as such, printable ASCII (`' '..='~'`, the apostrophe included) as
/// is, and every other unit as `\u` and four lower-case hexadecimal digits
/// (a supplementary character as its two surrogates).
///
/// Observed on JDK 25.0.5 (`@A("q'\b\f\r\u0001\u001f ~\u007f\u00e9\ud83d\ude00")`
/// prints every escape back as written, the apostrophe bare).
pub fn quote_member(s: &str) -> String {
    let mut b = String::from("\"");
    for unit in s.encode_utf16() {
        match unit {
            0x22 => b.push_str("\\\""),
            0x5c => b.push_str("\\\\"),
            0x08 => b.push_str("\\b"),
            0x0c => b.push_str("\\f"),
            0x0a => b.push_str("\\n"),
            0x0d => b.push_str("\\r"),
            0x09 => b.push_str("\\t"),
            0x20..=0x7e => b.push(char::from(unit as u8)),
            unit => {
                let _ = write!(b, "\\u{unit:04x}");
            }
        }
    }
    b.push('"');
    b
}

/// Finds the first annotation of type `A` in `annotations`.
pub fn find_annotation<A: JavaAnnotation>(annotations: &[AnnotationRef]) -> Option<&A> {
    annotations
        .iter()
        .find_map(|a| a.as_any().downcast_ref::<A>())
}

/// Whether `annotations` holds one of type `ty`.
pub fn has_annotation_type(annotations: &[AnnotationRef], ty: &AnnotationType) -> bool {
    annotations
        .iter()
        .any(|a| a.annotation_type().name == ty.name)
}

/// A Java object passed as a constructor or factory argument, or returned
/// by a method: `java.lang.Object` as the runner observes it.
#[derive(Clone)]
pub enum Value {
    /// `null`.
    Null,
    /// A `void` method's result.
    Void,
    /// `Boolean`.
    Bool(bool),
    /// `Integer`.
    Int(i32),
    /// `Long`.
    Long(i64),
    /// `Double`.
    Double(f64),
    /// `String`.
    Str(String),
    /// `Object[]`.
    Array(Vec<Value>),
    /// An `Iterable` (a `List`).
    List(Vec<Value>),
    /// Any other object, with its `toString()`.
    Object(Arc<dyn Any + Send + Sync>, String),
    /// A primitive array (`byte[]`, `long[]`, …): its component type and
    /// length; the `Arc` is its identity.
    PrimitiveArray(Arc<PrimitiveArray>),
    /// A `TestRule` (a `@Rule` or `@ClassRule` field's value).
    TestRule(Arc<dyn crate::junit::TestRule>),
    /// A `MethodRule` (a `@Rule` field's value).
    MethodRule(Arc<dyn crate::junit::MethodRule>),
}

/// A primitive array's shape: what the memory estimator measures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimitiveArray {
    /// The component type: `boolean`, `byte`, `char`, `short`, `int`,
    /// `float`, `long` or `double`.
    pub component: &'static str,
    /// The length.
    pub len: usize,
}

impl Value {
    /// `new <component>[len]`.
    pub fn primitive_array(component: &'static str, len: usize) -> Value {
        Value::PrimitiveArray(Arc::new(PrimitiveArray { component, len }))
    }

    /// An object value with its `toString()`.
    pub fn object<T: Any + Send + Sync>(v: T, to_string: impl Into<String>) -> Value {
        Value::Object(Arc::new(v), to_string.into())
    }

    /// The `Object[]` elements of an array value.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(v) => Some(v),
            _ => None,
        }
    }

    /// The object of an [`Value::Object`] value, downcast.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        match self {
            Value::Object(o, _) => o.downcast_ref::<T>(),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    /// `String.valueOf(object)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null | Value::Void => f.write_str("null"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Long(l) => write!(f, "{l}"),
            Value::Double(d) => f.write_str(&super::lang::double_to_string(*d)),
            Value::Str(s) => f.write_str(s),
            Value::Array(_) => f.write_str("[Ljava.lang.Object;"),
            Value::List(v) => {
                f.write_str("[")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{x}")?;
                }
                f.write_str("]")
            }
            Value::Object(_, s) => f.write_str(s),
            Value::PrimitiveArray(a) => write!(f, "[{}", a.component),
            Value::TestRule(_) => f.write_str("TestRule"),
            Value::MethodRule(_) => f.write_str("MethodRule"),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// An instance of a declared class: the Rust value its constructor built,
/// and its runtime class (`getClass()`). Cloning shares the object (a Java
/// reference copy).
#[derive(Clone)]
pub struct Instance {
    object: Arc<Mutex<Box<dyn Any + Send>>>,
    class: Arc<Class>,
}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "instance of {}", self.class.name)
    }
}

impl Instance {
    /// `getClass()`.
    pub fn get_class(&self) -> &Arc<Class> {
        &self.class
    }

    /// Whether `a` and `b` are the same object (`a == b`).
    pub fn ptr_eq(a: &Instance, b: &Instance) -> bool {
        Arc::ptr_eq(&a.object, &b.object)
    }
}

/// Locks an instance (a poisoned lock is still usable: the panic that
/// poisoned it is the test's failure, reported elsewhere).
pub fn lock_instance(instance: &Instance) -> MutexGuard<'_, Box<dyn Any + Send>> {
    instance.object.lock().unwrap_or_else(|e| e.into_inner())
}

type StaticBody = Arc<dyn Fn(&[Value]) -> Value + Send + Sync>;
type InstanceBody = Arc<dyn Fn(&mut (dyn Any + Send), &[Value]) -> Value + Send + Sync>;

/// A method body.
#[derive(Clone)]
pub enum MethodBody {
    /// A `static` method.
    Static(StaticBody),
    /// An instance method, receiving the instance.
    Instance(InstanceBody),
    /// An `abstract` method.
    Abstract,
}

/// `java.lang.reflect.Method`, declared.
pub struct Method {
    declaring_class: String,
    name: String,
    modifiers: i32,
    return_type: String,
    parameter_types: Vec<String>,
    exception_types: Vec<String>,
    annotations: Vec<AnnotationRef>,
    body: MethodBody,
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_generic_string())
    }
}

impl Method {
    /// A method named `name`, `public void`, no parameters, no annotations,
    /// abstract until a body is given.
    pub fn new(name: impl Into<String>) -> Method {
        Method {
            declaring_class: String::new(),
            name: name.into(),
            modifiers: modifier::PUBLIC,
            return_type: "void".to_string(),
            parameter_types: Vec::new(),
            exception_types: Vec::new(),
            annotations: Vec::new(),
            body: MethodBody::Abstract,
        }
    }

    /// Sets the modifiers (`modifier::*` bits).
    pub fn modifiers(mut self, modifiers: i32) -> Method {
        self.modifiers = modifiers;
        self
    }

    /// Sets the generic return type (`getGenericReturnType().getTypeName()`).
    pub fn returns(mut self, type_name: impl Into<String>) -> Method {
        self.return_type = type_name.into();
        self
    }

    /// Sets the parameter types (`getParameterTypes()` names).
    pub fn params(mut self, types: &[&str]) -> Method {
        self.parameter_types = types.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Sets the `throws` clause.
    pub fn throws(mut self, types: &[&str]) -> Method {
        self.exception_types = types.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Adds an annotation.
    pub fn annotate(mut self, a: impl JavaAnnotation) -> Method {
        self.annotations.push(Arc::new(a));
        self
    }

    /// Adds a shared annotation.
    pub fn annotate_ref(mut self, a: AnnotationRef) -> Method {
        self.annotations.push(a);
        self
    }

    /// A static body; marks the method `static`.
    pub fn static_body(mut self, body: impl Fn() + Send + Sync + 'static) -> Method {
        self.modifiers |= modifier::STATIC;
        self.body = MethodBody::Static(Arc::new(move |_| {
            body();
            Value::Void
        }));
        self
    }

    /// A static body returning a value; marks the method `static`.
    pub fn static_value(
        mut self,
        body: impl Fn(&[Value]) -> Value + Send + Sync + 'static,
    ) -> Method {
        self.modifiers |= modifier::STATIC;
        self.body = MethodBody::Static(Arc::new(body));
        self
    }

    /// An instance body over the instance type `T`.
    pub fn body<T: Any + Send>(mut self, body: impl Fn(&mut T) + Send + Sync + 'static) -> Method {
        self.modifiers &= !modifier::STATIC;
        self.body = MethodBody::Instance(Arc::new(move |instance, _| {
            match instance.downcast_mut::<T>() {
                Some(t) => body(t),
                None => exception::illegal_argument("object is not an instance of declaring class"),
            }
            Value::Void
        }));
        self
    }

    /// An instance body that ignores the instance.
    pub fn body_any(mut self, body: impl Fn() + Send + Sync + 'static) -> Method {
        self.modifiers &= !modifier::STATIC;
        self.body = MethodBody::Instance(Arc::new(move |_, _| {
            body();
            Value::Void
        }));
        self
    }

    /// `getDeclaringClass().getName()`.
    pub fn declaring_class(&self) -> &str {
        &self.declaring_class
    }

    /// `getName()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getModifiers()`.
    pub fn get_modifiers(&self) -> i32 {
        self.modifiers
    }

    /// `getParameterTypes()` (names).
    pub fn parameter_types(&self) -> &[String] {
        &self.parameter_types
    }

    /// The erasure of the return type (`getReturnType().getName()`).
    pub fn raw_return_type(&self) -> &str {
        self.return_type
            .split('<')
            .next()
            .unwrap_or(&self.return_type)
    }

    /// `getAnnotations()` (declared; method annotations are not inherited).
    pub fn annotations(&self) -> &[AnnotationRef] {
        &self.annotations
    }

    /// `getAnnotation(A.class)`.
    pub fn get_annotation<A: JavaAnnotation>(&self) -> Option<&A> {
        find_annotation(&self.annotations)
    }

    /// `isAnnotationPresent(type)`.
    pub fn is_annotation_present(&self, ty: &AnnotationType) -> bool {
        has_annotation_type(&self.annotations, ty)
    }

    /// `isStatic`.
    pub fn is_static(&self) -> bool {
        self.modifiers & modifier::STATIC != 0
    }

    /// `toGenericString()`: modifiers, return type, `Class.method(params)`
    /// and the `throws` clause.
    pub fn to_generic_string(&self) -> String {
        self.signature_string()
    }

    /// `toString()`: as [`to_generic_string`](Self::to_generic_string) for
    /// a declaration without type variables, with the erased return type.
    pub fn to_java_string(&self) -> String {
        let mut b = String::new();
        let m = modifier::to_string(self.modifiers & METHOD_MODIFIERS);
        if !m.is_empty() {
            b.push_str(&m);
            b.push(' ');
        }
        b.push_str(self.raw_return_type());
        b.push(' ');
        b.push_str(&self.declaring_class);
        b.push('.');
        b.push_str(&self.name);
        b.push('(');
        b.push_str(&self.parameter_types.join(","));
        b.push(')');
        if !self.exception_types.is_empty() {
            b.push_str(" throws ");
            b.push_str(&self.exception_types.join(","));
        }
        b
    }

    fn signature_string(&self) -> String {
        let mut b = String::new();
        let m = modifier::to_string(self.modifiers & METHOD_MODIFIERS);
        if !m.is_empty() {
            b.push_str(&m);
            b.push(' ');
        }
        b.push_str(&self.return_type);
        b.push(' ');
        b.push_str(&self.declaring_class);
        b.push('.');
        b.push_str(&self.name);
        b.push('(');
        b.push_str(&self.parameter_types.join(","));
        b.push(')');
        if !self.exception_types.is_empty() {
            b.push_str(" throws ");
            b.push_str(&self.exception_types.join(","));
        }
        b
    }

    /// `invoke(instance, args)`: runs the body the JVM selects; a throwable
    /// it throws propagates unchanged (Java wraps it in
    /// `InvocationTargetException`, which the runner unwraps).
    ///
    /// As JDK 25's `DirectMethodHandleAccessor`, which invokes a handle
    /// from `findStatic` or `findVirtual`: a `static` method ignores the
    /// instance and runs its own body; an instance method checks the
    /// receiver (`checkReceiver`) and then, unless it is `private` (invoked
    /// directly), runs the receiver's most-derived override (JVMS 5.4.6
    /// method selection, with JVMS 5.4.5's overriding rule).
    ///
    /// # Panics
    ///
    /// `NullPointerException` for an instance method without an instance;
    /// `IllegalArgumentException` (`object of type C is not an instance of
    /// D`) for a receiver outside the declaring class;
    /// `AbstractMethodError` when the selected method is abstract.
    pub fn invoke(&self, instance: Option<&Instance>, args: &[Value]) -> Value {
        if let MethodBody::Static(f) = &self.body {
            return f(args);
        }
        let Some(receiver) = instance else {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        };
        // `checkReceiver`: the receiver's class chain, up to the declaring
        // class.
        let mut chain: Vec<&Class> = Vec::new();
        let mut c: Option<&Class> = Some(&receiver.class);
        while let Some(k) = c {
            chain.push(k);
            if k.name == self.declaring_class {
                break;
            }
            c = k.superclass.as_deref();
        }
        if chain.last().map(|k| k.name.as_str()) != Some(self.declaring_class.as_str()) {
            exception::illegal_argument(format!(
                "object of type {} is not an instance of {}",
                receiver.class.name, self.declaring_class
            ));
        }
        // A `private` method is never overridden (JVMS 5.4.5), so it
        // selects itself, as its direct handle invokes it.
        let selected = select_override(&chain, self);
        match &selected.body {
            MethodBody::Instance(f) => {
                let mut guard = lock_instance(receiver);
                f(&mut **guard, args)
            }
            MethodBody::Static(_) | MethodBody::Abstract => exception::throw(
                "java.lang.AbstractMethodError",
                format!("{}.{}", selected.declaring_class, selected.name),
            ),
        }
    }

    /// Whether `self` has the name and parameter types of `other` and is an
    /// instance method that is not `private`: a candidate override.
    fn may_override(&self, other: &Method) -> bool {
        self.modifiers & (modifier::STATIC | modifier::PRIVATE) == 0
            && self.name == other.name
            && self.parameter_types == other.parameter_types
    }
}

/// JVMS 5.4.6 method selection for a resolved instance method `resolved` of `chain`'s last class and a receiver of `chain`'s
/// first class (`chain` runs from the receiver's class up to the declaring
/// class): the declaration in the most-derived class that overrides
/// `resolved`, else `resolved` itself.
fn select_override<'a>(chain: &[&'a Class], resolved: &'a Method) -> &'a Method {
    let top = chain.len() - 1;
    for (ci, class) in chain.iter().enumerate().take(top) {
        for m in &class.methods {
            if m.may_override(resolved) && overrides(chain, ci, m, top, resolved) {
                return m;
            }
        }
    }
    resolved
}

/// JVMS 5.4.5: whether `mc`, declared by `chain[ci]`, overrides `ma`,
/// declared by its superclass `chain[ai]`: same name and parameter types,
/// `mc` not `private`, and `ma` `public` or `protected`, or package access
/// with `chain[ci]` in `ma`'s package, or `mc` overriding a method `m'` of
/// an intermediate class that overrides `ma`.
fn overrides(chain: &[&Class], ci: usize, mc: &Method, ai: usize, ma: &Method) -> bool {
    if !mc.may_override(ma) || ma.modifiers & (modifier::PRIVATE | modifier::STATIC) != 0 {
        return false;
    }
    if ma.modifiers & (modifier::PUBLIC | modifier::PROTECTED) != 0
        || chain[ci].package_name() == chain[ai].package_name()
    {
        return true;
    }
    (ci + 1..ai).any(|bi| {
        chain[bi]
            .methods
            .iter()
            .any(|mb| overrides(chain, ci, mc, bi, mb) && overrides(chain, bi, mb, ai, ma))
    })
}

/// `Modifier.methodModifiers()`.
const METHOD_MODIFIERS: i32 = modifier::PUBLIC
    | modifier::PROTECTED
    | modifier::PRIVATE
    | modifier::ABSTRACT
    | modifier::STATIC
    | modifier::FINAL
    | modifier::SYNCHRONIZED
    | modifier::NATIVE
    | modifier::STRICT;

type StaticGetter = Arc<dyn Fn() -> Value + Send + Sync>;
type InstanceGetter = Arc<dyn Fn(&(dyn Any + Send)) -> Value + Send + Sync>;

/// How a field's value is read.
#[derive(Clone)]
pub enum FieldAccess {
    /// A `static` field.
    Static(StaticGetter),
    /// An instance field.
    Instance(InstanceGetter),
}

/// `java.lang.reflect.Field`, declared.
pub struct Field {
    declaring_class: String,
    name: String,
    modifiers: i32,
    type_name: String,
    annotations: Vec<AnnotationRef>,
    access: FieldAccess,
}

impl fmt::Debug for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_generic_string())
    }
}

impl Field {
    /// A static field of type `type_name` read by `getter`.
    pub fn new_static(
        name: impl Into<String>,
        type_name: impl Into<String>,
        getter: impl Fn() -> Value + Send + Sync + 'static,
    ) -> Field {
        Field {
            declaring_class: String::new(),
            name: name.into(),
            modifiers: modifier::PUBLIC | modifier::STATIC,
            type_name: type_name.into(),
            annotations: Vec::new(),
            access: FieldAccess::Static(Arc::new(getter)),
        }
    }

    /// An instance field of type `type_name`, read from the instance `T`.
    pub fn new_instance<T: Any>(
        name: impl Into<String>,
        type_name: impl Into<String>,
        getter: impl Fn(&T) -> Value + Send + Sync + 'static,
    ) -> Field {
        Field {
            declaring_class: String::new(),
            name: name.into(),
            modifiers: modifier::PUBLIC,
            type_name: type_name.into(),
            annotations: Vec::new(),
            access: FieldAccess::Instance(Arc::new(move |i| match i.downcast_ref::<T>() {
                Some(t) => getter(t),
                None => Value::Null,
            })),
        }
    }

    /// An instance field of type `type_name` whose value `getter` gives for
    /// any instance: a field a superclass declares, which reads per-instance
    /// state the superclass keeps itself, whatever the subclass's instance
    /// type.
    pub fn new_instance_any(
        name: impl Into<String>,
        type_name: impl Into<String>,
        getter: impl Fn() -> Value + Send + Sync + 'static,
    ) -> Field {
        Field {
            declaring_class: String::new(),
            name: name.into(),
            modifiers: modifier::PUBLIC,
            type_name: type_name.into(),
            annotations: Vec::new(),
            access: FieldAccess::Instance(Arc::new(move |_| getter())),
        }
    }

    /// Sets the modifiers; `STATIC` follows the accessor.
    pub fn modifiers(mut self, modifiers: i32) -> Field {
        let is_static = matches!(self.access, FieldAccess::Static(_));
        self.modifiers =
            (modifiers & !modifier::STATIC) | if is_static { modifier::STATIC } else { 0 };
        self
    }

    /// Adds an annotation.
    pub fn annotate(mut self, a: impl JavaAnnotation) -> Field {
        self.annotations.push(Arc::new(a));
        self
    }

    /// `getName()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getDeclaringClass().getName()`.
    pub fn declaring_class(&self) -> &str {
        &self.declaring_class
    }

    /// `getModifiers()`.
    pub fn get_modifiers(&self) -> i32 {
        self.modifiers
    }

    /// `getType().getName()` (erased).
    pub fn type_name(&self) -> &str {
        self.type_name.split('<').next().unwrap_or(&self.type_name)
    }

    /// `getType().isPrimitive()`.
    pub fn is_primitive(&self) -> bool {
        matches!(
            self.type_name.as_str(),
            "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double"
        )
    }

    /// `getAnnotations()`.
    pub fn annotations(&self) -> &[AnnotationRef] {
        &self.annotations
    }

    /// `isAnnotationPresent(type)`.
    pub fn is_annotation_present(&self, ty: &AnnotationType) -> bool {
        has_annotation_type(&self.annotations, ty)
    }

    /// `Field.get(instance)`.
    ///
    /// # Panics
    ///
    /// `NullPointerException` for an instance field without an instance.
    pub fn get(&self, instance: Option<&Instance>) -> Value {
        match &self.access {
            FieldAccess::Static(g) => g(),
            FieldAccess::Instance(g) => match instance {
                Some(i) => {
                    let guard = lock_instance(i);
                    g(&**guard)
                }
                None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
            },
        }
    }

    /// `toGenericString()`: modifiers, type and `Class.field`.
    pub fn to_generic_string(&self) -> String {
        let m = modifier::to_string(self.modifiers & FIELD_MODIFIERS);
        let prefix = if m.is_empty() {
            String::new()
        } else {
            format!("{m} ")
        };
        format!(
            "{prefix}{} {}.{}",
            self.type_name, self.declaring_class, self.name
        )
    }

    /// `toString()`: as `toGenericString()` with the erased type.
    pub fn to_java_string(&self) -> String {
        let m = modifier::to_string(self.modifiers & FIELD_MODIFIERS);
        let prefix = if m.is_empty() {
            String::new()
        } else {
            format!("{m} ")
        };
        format!(
            "{prefix}{} {}.{}",
            self.type_name(),
            self.declaring_class,
            self.name
        )
    }
}

/// `Modifier.fieldModifiers()`.
const FIELD_MODIFIERS: i32 = modifier::PUBLIC
    | modifier::PROTECTED
    | modifier::PRIVATE
    | modifier::STATIC
    | modifier::FINAL
    | modifier::TRANSIENT
    | modifier::VOLATILE;

type ConstructorBody = Arc<dyn Fn(&[Value]) -> Box<dyn Any + Send> + Send + Sync>;

/// `java.lang.reflect.Constructor`, declared.
#[derive(Clone)]
pub struct Constructor {
    /// The declaring class, set when the class is built.
    class: Weak<Class>,
    modifiers: i32,
    parameter_types: Vec<String>,
    /// The `@Name` of each parameter, if any.
    parameter_names: Vec<Option<String>>,
    body: ConstructorBody,
}

impl Constructor {
    /// A public constructor without parameters.
    pub fn no_args<T: Any + Send>(make: impl Fn() -> T + Send + Sync + 'static) -> Constructor {
        Constructor {
            class: Weak::new(),
            modifiers: modifier::PUBLIC,
            parameter_types: Vec::new(),
            parameter_names: Vec::new(),
            body: Arc::new(move |_| Box::new(make())),
        }
    }

    /// A public constructor with parameters of the given types and `@Name`s.
    pub fn with_params<T: Any + Send>(
        params: &[(&str, Option<&str>)],
        make: impl Fn(&[Value]) -> T + Send + Sync + 'static,
    ) -> Constructor {
        Constructor {
            class: Weak::new(),
            modifiers: modifier::PUBLIC,
            parameter_types: params.iter().map(|(t, _)| t.to_string()).collect(),
            parameter_names: params.iter().map(|(_, n)| n.map(str::to_string)).collect(),
            body: Arc::new(move |args| Box::new(make(args))),
        }
    }

    /// Sets the modifiers.
    pub fn modifiers(mut self, modifiers: i32) -> Constructor {
        self.modifiers = modifiers;
        self
    }

    /// `getModifiers()`.
    pub fn get_modifiers(&self) -> i32 {
        self.modifiers
    }

    /// `getParameterTypes()` (names).
    pub fn parameter_types(&self) -> &[String] {
        &self.parameter_types
    }

    /// The `@Name` value of parameter `i` (`getParameterAnnotations()`).
    pub fn parameter_name(&self, i: usize) -> Option<&str> {
        self.parameter_names.get(i).and_then(|n| n.as_deref())
    }

    /// `newInstance(args)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`wrong number of arguments`) when the
    /// argument count differs from the parameter count; a throwable the
    /// body throws propagates. Panics as well for a constructor that was
    /// never declared by a built class (a declaration error).
    pub fn new_instance(&self, args: &[Value]) -> Instance {
        if args.len() != self.parameter_types.len() {
            exception::illegal_argument(format!(
                "wrong number of arguments: {} expected: {}",
                args.len(),
                self.parameter_types.len()
            ));
        }
        let class = self
            .class
            .upgrade()
            .expect("a constructor is invoked through the built class that declares it");
        Instance {
            object: Arc::new(Mutex::new((self.body)(args))),
            class,
        }
    }
}

/// `java.lang.Class`, declared with [`ClassBuilder`].
pub struct Class {
    name: String,
    modifiers: i32,
    superclass: Option<Arc<Class>>,
    annotations: Vec<AnnotationRef>,
    methods: Vec<Arc<Method>>,
    fields: Vec<Arc<Field>>,
    constructors: Vec<Constructor>,
    initializer: Option<Arc<dyn Fn() + Send + Sync>>,
    /// The outcome of the static initializer (`Class.forName(name, true, …)`).
    initialized: OnceLock<Result<(), Throwable>>,
}

impl fmt::Debug for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.name)
    }
}

impl fmt::Display for Class {
    /// `Class.toString()`: `class <name>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "class {}", self.name)
    }
}

impl Class {
    /// `getName()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `getSimpleName()`: the name after the last `.` and `$`.
    pub fn simple_name(&self) -> &str {
        let after_dot = self.name.rsplit('.').next().unwrap_or(&self.name);
        after_dot.rsplit('$').next().unwrap_or(after_dot)
    }

    /// `getModifiers()`.
    pub fn get_modifiers(&self) -> i32 {
        self.modifiers
    }

    /// `getSuperclass()`: `None` for a class extending `Object`.
    pub fn superclass(&self) -> Option<&Arc<Class>> {
        self.superclass.as_ref()
    }

    /// `getPackage().getName()`.
    pub fn package_name(&self) -> &str {
        match self.name.rfind('.') {
            Some(i) => &self.name[..i],
            None => "",
        }
    }

    /// `getDeclaredAnnotations()`.
    pub fn declared_annotations(&self) -> &[AnnotationRef] {
        &self.annotations
    }

    /// `getAnnotations()`: the declared annotations, then the `@Inherited`
    /// annotations of the superclasses whose type is not yet present.
    pub fn annotations(&self) -> Vec<AnnotationRef> {
        let mut all: Vec<AnnotationRef> = self.annotations.clone();
        let mut c = self.superclass.clone();
        while let Some(sup) = c {
            for a in &sup.annotations {
                let ty = a.annotation_type();
                if ty.inherited && !has_annotation_type(&all, ty) {
                    all.push(Arc::clone(a));
                }
            }
            c = sup.superclass.clone();
        }
        all
    }

    /// `getAnnotation(A.class)`, inherited annotations included.
    pub fn get_annotation<A: JavaAnnotation + Clone>(&self) -> Option<A> {
        find_annotation::<A>(&self.annotations()).cloned()
    }

    /// `getAnnotation(type)` by type, inherited annotations included.
    pub fn get_annotation_ref(&self, ty: &AnnotationType) -> Option<AnnotationRef> {
        self.annotations()
            .into_iter()
            .find(|a| a.annotation_type().name == ty.name)
    }

    /// `getDeclaredAnnotation(A.class)` (not inherited).
    pub fn get_declared_annotation<A: JavaAnnotation>(&self) -> Option<&A> {
        find_annotation(&self.annotations)
    }

    /// `isAnnotationPresent(type)`, inherited annotations included.
    pub fn is_annotation_present(&self, ty: &AnnotationType) -> bool {
        self.get_annotation_ref(ty).is_some()
    }

    /// `getDeclaredMethods()`.
    pub fn declared_methods(&self) -> &[Arc<Method>] {
        &self.methods
    }

    /// `getDeclaredFields()`.
    pub fn declared_fields(&self) -> &[Arc<Field>] {
        &self.fields
    }

    /// `getConstructors()`: the public constructors.
    pub fn constructors(&self) -> Vec<&Constructor> {
        self.constructors
            .iter()
            .filter(|c| c.modifiers & modifier::PUBLIC != 0)
            .collect()
    }

    /// The declared constructors (`getDeclaredConstructors()`).
    pub fn declared_constructors(&self) -> &[Constructor] {
        &self.constructors
    }

    /// Whether `other` is this class or one of its superclasses
    /// (`other.isAssignableFrom(this)` for classes).
    pub fn is_subclass_of(&self, other: &Class) -> bool {
        let mut c: Option<&Class> = Some(self);
        while let Some(x) = c {
            if x.name == other.name {
                return true;
            }
            c = x.superclass.as_deref();
        }
        false
    }

    /// `Class.forName(name, true, loader)`: runs the static initializers of
    /// the superclasses, then this class's, once. A throwable an initializer
    /// throws is returned, every time (Java throws
    /// `ExceptionInInitializerError` the first time and
    /// `NoClassDefFoundError` after; the runner unwraps the first).
    pub fn initialize(&self) -> Result<(), Throwable> {
        if let Some(sup) = &self.superclass {
            sup.initialize()?;
        }
        self.initialized
            .get_or_init(|| match &self.initializer {
                None => Ok(()),
                Some(init) => {
                    let init = Arc::clone(init);
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || init()))
                        .map_err(Throwable::from_panic)
                }
            })
            .clone()
    }
}

/// Declares a [`Class`].
pub struct ClassBuilder {
    class: Class,
}

impl Class {
    /// Starts the declaration of the class with binary name `name`:
    /// `public`, extending `Object`, with no members.
    pub fn builder(name: impl Into<String>) -> ClassBuilder {
        ClassBuilder {
            class: Class {
                name: name.into(),
                modifiers: modifier::PUBLIC,
                superclass: None,
                annotations: Vec::new(),
                methods: Vec::new(),
                fields: Vec::new(),
                constructors: Vec::new(),
                initializer: None,
                initialized: OnceLock::new(),
            },
        }
    }
}

impl ClassBuilder {
    /// Sets the class modifiers.
    pub fn modifiers(mut self, modifiers: i32) -> ClassBuilder {
        self.class.modifiers = modifiers;
        self
    }

    /// `extends superclass`.
    pub fn extends(mut self, superclass: &Arc<Class>) -> ClassBuilder {
        self.class.superclass = Some(Arc::clone(superclass));
        self
    }

    /// Adds a class annotation.
    pub fn annotate(mut self, a: impl JavaAnnotation) -> ClassBuilder {
        self.class.annotations.push(Arc::new(a));
        self
    }

    /// Adds a shared class annotation.
    pub fn annotate_ref(mut self, a: AnnotationRef) -> ClassBuilder {
        self.class.annotations.push(a);
        self
    }

    /// Declares a method.
    pub fn method(mut self, mut m: Method) -> ClassBuilder {
        m.declaring_class = self.class.name.clone();
        self.class.methods.push(Arc::new(m));
        self
    }

    /// Declares a field.
    pub fn field(mut self, mut f: Field) -> ClassBuilder {
        f.declaring_class = self.class.name.clone();
        self.class.fields.push(Arc::new(f));
        self
    }

    /// Declares a constructor.
    pub fn constructor(mut self, c: Constructor) -> ClassBuilder {
        self.class.constructors.push(c);
        self
    }

    /// Declares the static initializer (`static { … }`).
    pub fn static_init(mut self, init: impl Fn() + Send + Sync + 'static) -> ClassBuilder {
        self.class.initializer = Some(Arc::new(init));
        self
    }

    /// Ends the declaration.
    pub fn build(self) -> Arc<Class> {
        let mut class = self.class;
        Arc::new_cyclic(|me| {
            for c in &mut class.constructors {
                c.class = Weak::clone(me);
            }
            class
        })
    }
}

#[cfg(test)]
mod quote_member_tests {
    use super::quote_member;

    /// JDK 25.0.5 prints `@A("q'\b\f\r\u0001\u001f ~\u007fé😀x")`
    /// for that member value (observed with a single-file probe): the
    /// escapes per UTF-16 unit, the apostrophe bare.
    #[test]
    fn quotes_as_jdk_25() {
        assert_eq!(
            quote_member("q'\u{8}\u{c}\r\u{1}\u{1f} ~\u{7f}\u{e9}\u{1f600}x"),
            r#""q'\b\f\r\u0001\u001f ~\u007f\u00e9\ud83d\ude00x""#
        );
        assert_eq!(quote_member("a\"b\\c\n\t"), r#""a\"b\\c\n\t""#);
        assert_eq!(quote_member(""), r#""""#);
    }
}
