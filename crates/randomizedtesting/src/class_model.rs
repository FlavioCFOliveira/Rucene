//! Port of `com.carrotsearch.randomizedtesting.ClassModel`
//! (randomizedtesting 2.8.1, `ClassModel.java`).

use std::sync::Arc;

use crate::annotations::AnnotationType;
use crate::jdk::reflect::{find_annotation, modifier, Class, Field, JavaAnnotation, Method};

/// Access scope of a class member (`ClassModel.Scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `PUBLIC`.
    Public,
    /// `PROTECTED`.
    Protected,
    /// `PACKAGE`.
    Package,
    /// `PRIVATE`.
    Private,
}

fn access_scope(modifiers: i32) -> Scope {
    if modifiers & modifier::PUBLIC != 0 {
        Scope::Public
    } else if modifiers & modifier::PROTECTED != 0 {
        Scope::Protected
    } else if modifiers & modifier::PRIVATE != 0 {
        Scope::Private
    } else {
        Scope::Package
    }
}

fn package_of(class_name: &str) -> &str {
    match class_name.rfind('.') {
        Some(i) => &class_name[..i],
        None => "",
    }
}

/// A method in the class hierarchy and its override/shadow links
/// (`ClassModel.MethodModel`): `up` is the method it overrides or shadows in
/// a superclass, `down` the one that overrides or shadows it in a subclass.
#[derive(Debug, Clone)]
pub struct MethodModel {
    /// `element`.
    pub element: Arc<Method>,
    scope: Scope,
    up: Option<usize>,
    down: Option<usize>,
}

/// A field in the class hierarchy and its shadow links
/// (`ClassModel.FieldModel`).
#[derive(Debug, Clone)]
pub struct FieldModel {
    /// `element`.
    pub element: Arc<Field>,
    scope: Scope,
    up: Option<usize>,
    down: Option<usize>,
}

/// The methods and fields of a class and its superclasses with their
/// override relationships: `ClassModel`.
#[derive(Debug, Clone)]
pub struct ClassModel {
    methods: Vec<MethodModel>,
    fields: Vec<FieldModel>,
}

impl MethodModel {
    /// `getAccessScope()`.
    pub fn access_scope(&self) -> Scope {
        self.scope
    }

    /// `MethodModel.toString()`: `SimpleClass.method`.
    pub fn to_java_string(&self) -> String {
        let c = self.element.declaring_class();
        let simple = c.rsplit(['.', '$']).next().unwrap_or(c);
        format!("{}.{}", simple, self.element.name())
    }

    /// `equals`: same name and parameter types.
    fn same_signature(&self, other: &MethodModel) -> bool {
        self.element.name() == other.element.name()
            && self.element.parameter_types() == other.element.parameter_types()
    }

    /// `overridesOrShadows(sub)`.
    fn overrides_or_shadows(&self, sub: &MethodModel) -> bool {
        if !self.same_signature(sub) {
            return false;
        }
        if self.scope == Scope::Package {
            package_of(self.element.declaring_class()) == package_of(sub.element.declaring_class())
        } else {
            true
        }
    }
}

impl std::hash::Hash for MethodModel {
    /// `MethodModel.hashCode()` (`ClassModel.java:148-155`): the name and the
    /// parameter types, the fields `equals` compares
    /// (`same_signature`).
    ///
    /// **Divergence from randomizedtesting 2.8.1.** Java sums
    /// `element.getName().hashCode()` and `31 * c.hashCode()` per parameter
    /// class, where `Class.hashCode()` is the JVM identity hash, so the value
    /// changes from run to run and no port can reproduce it. The port hashes
    /// the name and the parameter type names instead, which keeps the `Hash`
    /// contract with that equality.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.element.name().hash(state);
        self.element.parameter_types().hash(state);
    }
}

impl std::fmt::Display for FieldModel {
    /// `FieldModel.toString()` (`ClassModel.java:171-174`):
    /// `getDeclaringClass().getSimpleName() + "." + getName()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c = self.element.declaring_class();
        let simple = c.rsplit(['.', '$']).next().unwrap_or(c);
        write!(f, "{}.{}", simple, self.element.name())
    }
}

impl std::hash::Hash for FieldModel {
    /// `FieldModel.hashCode()` (`ClassModel.java:194-197`): Java's value,
    /// [`hash_code`](FieldModel::hash_code), consistent with `equals`, which
    /// compares the field names.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

impl FieldModel {
    /// `FieldModel.hashCode()` (`ClassModel.java:194-197`):
    /// `element.getName().hashCode()`.
    pub fn hash_code(&self) -> i32 {
        crate::jdk::lang::string_hash_code(self.element.name())
    }

    /// `getAccessScope()`.
    pub fn access_scope(&self) -> Scope {
        self.scope
    }

    /// `overridesOrShadows(sub)`.
    ///
    /// Java compares the package of this field's class with itself (both
    /// sides read `f1`), so a package-private field always shadows; the
    /// port keeps that.
    fn overrides_or_shadows(&self, sub: &FieldModel) -> bool {
        self.element.name() == sub.element.name()
    }
}

impl ClassModel {
    /// `new ClassModel(clazz)`: the members of `clazz` and its superclasses
    /// (leaf first), each class's members sorted by `toGenericString()`.
    pub fn new(clazz: &Class) -> ClassModel {
        // Methods.
        let mut methods: Vec<MethodModel> = Vec::new();
        let mut tops: Vec<Vec<usize>> = Vec::new();
        let mut c: Option<&Class> = Some(clazz);
        while let Some(cls) = c {
            let mut declared: Vec<Arc<Method>> = cls.declared_methods().to_vec();
            declared.sort_by(|a, b| {
                crate::annotations::jdk_string_compare(
                    &a.to_generic_string(),
                    &b.to_generic_string(),
                )
            });
            for m in declared {
                let idx = methods.len();
                let scope = access_scope(m.get_modifiers());
                methods.push(MethodModel {
                    element: m,
                    scope,
                    up: None,
                    down: None,
                });
                if scope != Scope::Private {
                    let group = tops
                        .iter()
                        .position(|g| methods[g[0]].same_signature(&methods[idx]));
                    let group = match group {
                        Some(g) => g,
                        None => {
                            tops.push(Vec::new());
                            tops.len() - 1
                        }
                    };
                    let mut kept = Vec::new();
                    for &sub in &tops[group] {
                        if methods[idx].overrides_or_shadows(&methods[sub]) {
                            methods[sub].up = Some(idx);
                            methods[idx].down = Some(sub);
                        } else {
                            kept.push(sub);
                        }
                    }
                    kept.push(idx);
                    // An empty group has no representative; keep the new one.
                    tops[group] = kept;
                }
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        // Fields.
        let mut fields: Vec<FieldModel> = Vec::new();
        let mut ftops: Vec<Vec<usize>> = Vec::new();
        let mut c: Option<&Class> = Some(clazz);
        while let Some(cls) = c {
            let mut declared: Vec<Arc<Field>> = cls.declared_fields().to_vec();
            declared.sort_by(|a, b| {
                crate::annotations::jdk_string_compare(
                    &a.to_generic_string(),
                    &b.to_generic_string(),
                )
            });
            for f in declared {
                let idx = fields.len();
                let scope = access_scope(f.get_modifiers());
                fields.push(FieldModel {
                    element: f,
                    scope,
                    up: None,
                    down: None,
                });
                if scope != Scope::Private {
                    let group = ftops
                        .iter()
                        .position(|g| fields[g[0]].element.name() == fields[idx].element.name());
                    let group = match group {
                        Some(g) => g,
                        None => {
                            ftops.push(Vec::new());
                            ftops.len() - 1
                        }
                    };
                    let mut kept = Vec::new();
                    for &sub in &ftops[group] {
                        if fields[idx].overrides_or_shadows(&fields[sub]) {
                            fields[sub].up = Some(idx);
                            fields[idx].down = Some(sub);
                        } else {
                            kept.push(sub);
                        }
                    }
                    kept.push(idx);
                    ftops[group] = kept;
                }
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        ClassModel { methods, fields }
    }

    /// `getMethods()`, in model order.
    pub fn methods(&self) -> &[MethodModel] {
        &self.methods
    }

    /// `getFields()`, in model order.
    pub fn fields(&self) -> &[FieldModel] {
        &self.fields
    }

    /// `MethodModel.getDown()`.
    pub fn down(&self, mm: &MethodModel) -> Option<&MethodModel> {
        mm.down.map(|i| &self.methods[i])
    }

    /// `MethodModel.getUp()`.
    pub fn up(&self, mm: &MethodModel) -> Option<&MethodModel> {
        mm.up.map(|i| &self.methods[i])
    }

    /// `FieldModel.getDown()`.
    pub fn field_down(&self, fm: &FieldModel) -> Option<&FieldModel> {
        fm.down.map(|i| &self.fields[i])
    }

    /// `FieldModel.getUp()`.
    pub fn field_up(&self, fm: &FieldModel) -> Option<&FieldModel> {
        fm.up.map(|i| &self.fields[i])
    }

    /// The model of `method` (`methods.get(method)`).
    pub fn model_of(&self, method: &Arc<Method>) -> Option<&MethodModel> {
        self.methods
            .iter()
            .find(|m| Arc::ptr_eq(&m.element, method))
    }

    /// `getAnnotatedLeafMethods(annotation)`: the methods carrying
    /// `annotation` that no overriding method carries again, in model order.
    pub fn get_annotated_leaf_methods(&self, annotation: &AnnotationType) -> Vec<&MethodModel> {
        let mut result = Vec::new();
        'outer: for mm in &self.methods {
            if mm.element.is_annotation_present(annotation) {
                let mut next = self.down(mm);
                while let Some(n) = next {
                    if n.element.is_annotation_present(annotation) {
                        continue 'outer;
                    }
                    next = self.down(n);
                }
                result.push(mm);
            }
        }
        result
    }

    /// `getAnnotation(method, annClass, inherited)`: the method's
    /// annotation, or (`inherited`) the first one up the override chain.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`No model for method: null`) for a method
    /// outside the model.
    pub fn get_annotation<'a, A: JavaAnnotation>(
        &'a self,
        method: &'a Arc<Method>,
        inherited: bool,
    ) -> Option<&'a A> {
        let Some(mut mm) = self.model_of(method) else {
            crate::jdk::exception::illegal_argument("No model for method: null")
        };
        if inherited {
            loop {
                if let Some(a) = find_annotation::<A>(mm.element.annotations()) {
                    return Some(a);
                }
                mm = self.up(mm)?;
            }
        } else {
            method.get_annotation::<A>()
        }
    }

    /// `isAnnotationPresent(method, annClass, inherited)`.
    pub fn is_annotation_present<A: JavaAnnotation>(
        &self,
        method: &Arc<Method>,
        inherited: bool,
    ) -> bool {
        self.get_annotation::<A>(method, inherited).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jdk::reflect::Value;
    use std::hash::BuildHasher;

    fn model() -> ClassModel {
        let class = Class::builder("com.example.Outer$Inner")
            .field(Field::new_static("count", "int", || Value::Null))
            .field(Field::new_static(
                "aVeryLongFieldNameForOverflow",
                "int",
                || Value::Null,
            ))
            .method(Method::new("run").params(&["int", "java.lang.String"]))
            .method(Method::new("run").params(&["int"]))
            .build();
        ClassModel::new(&class)
    }

    #[test]
    fn field_model_to_string_is_simple_class_dot_name() {
        let m = model();
        let count = m
            .fields()
            .iter()
            .find(|f| f.element.name() == "count")
            .unwrap();
        // `getDeclaringClass().getSimpleName() + "." + getName()`.
        assert_eq!(count.to_string(), "Inner.count");
    }

    #[test]
    fn field_model_hash_code_is_java_string_hash_of_name() {
        let m = model();
        let by_name = |n: &str| m.fields().iter().find(|f| f.element.name() == n).unwrap();
        // `"count".hashCode()` and a name whose hash wraps in `int` arithmetic.
        assert_eq!(by_name("count").hash_code(), 94851343);
        assert_eq!(
            by_name("aVeryLongFieldNameForOverflow").hash_code(),
            258827513
        );
        let other = Class::builder("com.example.Other")
            .field(Field::new_static("count", "long", || Value::Null))
            .build();
        let other = ClassModel::new(&other);
        // `equals` compares names only, so equal names hash alike.
        let s = std::collections::hash_map::RandomState::new();
        assert_eq!(s.hash_one(by_name("count")), s.hash_one(&other.fields()[0]));
    }

    #[test]
    fn method_model_hash_follows_name_and_parameter_types() {
        let m = model();
        let two = m
            .methods()
            .iter()
            .find(|x| x.element.parameter_types().len() == 2)
            .unwrap();
        let one = m
            .methods()
            .iter()
            .find(|x| x.element.parameter_types().len() == 1)
            .unwrap();
        let again = Class::builder("com.example.Sub")
            .method(Method::new("run").params(&["int", "java.lang.String"]))
            .build();
        let again = ClassModel::new(&again);
        let s = std::collections::hash_map::RandomState::new();
        assert!(two.same_signature(&again.methods()[0]));
        assert_eq!(s.hash_one(two), s.hash_one(&again.methods()[0]));
        assert_ne!(s.hash_one(two), s.hash_one(one));
    }
}
