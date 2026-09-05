//! Attribute system ported from `org.apache.lucene.util`.
//!
//! This module provides the building blocks used by Lucene's analysis pipeline:
//! [`Attribute`], [`AttributeImpl`], [`AttributeFactory`], [`AttributeSource`],
//! [`AttributeReflector`], [`Unwrappable`], and [`CloseableThreadLocal`].
//!
//! Java's reflection-based discovery is replaced by a safe [`TypeId`]-aware
//! registry and explicit factory pattern, which is the idiomatic Rust
//! equivalent.

#![deny(unsafe_code)]

use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::LuceneError;

/// Base marker trait for all Lucene attributes.
///
/// Equivalent to `org.apache.lucene.util.Attribute`.
pub trait Attribute: 'static + Send + Sync + Debug {}

/// Reflector used to introspect the contents of an [`AttributeImpl`] or
/// [`AttributeSource`].
///
/// Equivalent to `org.apache.lucene.util.AttributeReflector`.
pub trait AttributeReflector {
    /// Reports a single attribute property.
    ///
    /// `attribute_type` identifies the attribute interface, `attribute_name`
    /// is its source-level name, `key` is the property name, and `value` is
    /// the current value.
    fn reflect(
        &mut self,
        attribute_type: TypeId,
        attribute_name: &'static str,
        key: &str,
        value: &dyn Debug,
    );
}

impl<F> AttributeReflector for F
where
    F: for<'a, 'b> FnMut(TypeId, &'static str, &'a str, &'b (dyn Debug + 'b)),
{
    fn reflect(
        &mut self,
        attribute_type: TypeId,
        attribute_name: &'static str,
        key: &str,
        value: &dyn Debug,
    ) {
        self(attribute_type, attribute_name, key, value)
    }
}

/// Constructor stored in [`DefaultAttributeFactory`].
pub type AttributeCtor = Box<dyn Fn() -> Box<dyn AttributeImpl> + Send + Sync>;

/// Base trait for attribute implementations stored in an [`AttributeSource`].
///
/// Implementations should also implement [`Clone`] and provide a deep copy
/// through [`Self::clone_box`], because [`AttributeSource::capture_state`]
/// relies on cloning to produce snapshots.
///
/// `Clone` is intentionally not a supertrait: it is not object-safe, and this
/// trait must be usable as `dyn AttributeImpl`. The object-safe [`Clone::clone`]
/// equivalent is [`Self::clone_box`].
///
/// Equivalent to `org.apache.lucene.util.AttributeImpl`.
pub trait AttributeImpl: Attribute + Debug + Send + Sync + 'static {
    /// Resets this attribute to its default value.
    fn clear(&mut self);

    /// Resets this attribute at the end of a field; defaults to [`Self::clear`].
    fn end(&mut self) {
        self.clear();
    }

    /// Copies the values from this attribute into `target`.
    ///
    /// The target must be an implementation of the same concrete type; the
    /// implementation should downcast via [`Any`] and copy its fields.
    fn copy_to(&self, target: &mut dyn AttributeImpl);

    /// Reports this attribute's properties to `reflector`.
    ///
    /// For each invocation the same set of attribute interfaces and keys must
    /// be passed in the same order, but the values may differ.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector);

    /// Returns this attribute's properties as a comma-separated string.
    ///
    /// Equivalent to
    /// `org.apache.lucene.util.AttributeImpl#reflectAsString(boolean)`
    /// (`AttributeImpl.java:58-71`), which drives [`Self::reflect_with`] with a
    /// reflector that appends `key=value` for each property, separated by
    /// commas, optionally prefixing each key with the attribute interface's
    /// name and `#`.
    ///
    /// Java declares the method `final`, so no implementation may change it;
    /// a provided trait method is the closest Rust equivalent — an
    /// implementation *may* override it, but Lucene's contract says it should
    /// not, and overriding [`Self::reflect_with`] is what changes the output.
    ///
    /// This is the per-attribute counterpart of
    /// [`AttributeSource::reflect_as_string`], which concatenates the same
    /// rendering over every implementation a source holds.
    fn reflect_as_string(&self, prepend_att_class: bool) -> String {
        let mut reflector = ReflectAsString {
            buffer: String::new(),
            prepend_att_class,
        };
        self.reflect_with(&mut reflector);
        reflector.buffer
    }

    /// Returns a boxed deep clone of this attribute.
    fn clone_box(&self) -> Box<dyn AttributeImpl>;

    /// Returns the [`TypeId`] of the concrete implementation.
    fn impl_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    /// Returns the [`TypeId`]s of all [`Attribute`] interfaces this
    /// implementation supports.
    fn attribute_interfaces(&self) -> &'static [TypeId];

    /// Returns this attribute as [`Any`] for immutable downcasting.
    ///
    /// Implementations typically return `self`.
    fn as_any(&self) -> &dyn Any;

    /// Returns this attribute as [`Any`] for mutable downcasting.
    ///
    /// Implementations typically return `self`.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Reports whether this attribute equals `other`.
    ///
    /// `org.apache.lucene.util.AttributeImpl` does not override
    /// `java.lang.Object#equals`, so the default here is Java's: identity.
    /// Implementations that Lucene gives value equality — `CharTermAttributeImpl`
    /// and its siblings — override this, exactly as they override `equals` in
    /// Java.
    ///
    /// Used by [`AttributeSource`]'s [`PartialEq`], which is the port of
    /// `AttributeSource#equals`.
    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        std::ptr::eq(
            self.as_any() as *const dyn Any as *const (),
            other.as_any() as *const dyn Any as *const (),
        )
    }

    /// Returns this attribute's hash code.
    ///
    /// `org.apache.lucene.util.AttributeImpl` does not override
    /// `java.lang.Object#hashCode`, so the default here is Java's: an identity
    /// hash, derived from the address the implementation lives at.
    /// Implementations that override [`Self::equals`] must override this too,
    /// exactly as they override `hashCode` in Java.
    ///
    /// Used by [`AttributeSource::hash_code`], which is the port of
    /// `AttributeSource#hashCode`.
    fn hash_code(&self) -> i32 {
        (self.as_any() as *const dyn Any as *const () as usize) as i32
    }
}

/// Factory that creates [`AttributeImpl`] instances on demand.
///
/// Equivalent to `org.apache.lucene.util.AttributeFactory`.
pub trait AttributeFactory: Send + Sync + Debug {
    /// Creates a new attribute implementation for the requested interface
    /// `attribute_type`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the factory does not know how
    /// to create the requested attribute.
    fn create_attribute_instance(
        &self,
        attribute_type: TypeId,
    ) -> Result<Box<dyn AttributeImpl>, LuceneError>;
}

/// Returns a correctly typed constructor for the no-argument constructor of
/// `I`.
///
/// Equivalent to the package-private
/// `AttributeFactory.findAttributeImplCtor(Class<? extends AttributeImpl>)`,
/// which resolves the constructor through `MethodHandles.publicLookup()` and
/// adapts the handle to one returning `AttributeImpl`.
///
/// Divergence: Lucene performs the lookup reflectively at run time and raises
/// `IllegalArgumentException` ("Cannot lookup accessible no-arg constructor
/// for: ...") when the class has no accessible no-argument constructor
/// (`AttributeFactory.java` lines 38-45). Rust has no reflection, so the lookup
/// is expressed as the `Default` bound — the language's own no-argument
/// constructor — which the compiler checks. The failure Lucene reports at run
/// time therefore cannot arise, and this function is infallible.
pub fn find_attribute_impl_ctor<I: AttributeImpl + Default>() -> AttributeCtor {
    Box::new(|| Box::new(I::default()))
}

/// Default attribute factory backed by a [`TypeId`] registry.
///
/// Users register constructors for each attribute interface their application
/// uses. This replaces Java's reflection-based `DefaultAttributeFactory`, which
/// discovered the `*Impl` class from the interface name.
#[derive(Default)]
pub struct DefaultAttributeFactory {
    registry: HashMap<TypeId, AttributeCtor>,
}

impl DefaultAttributeFactory {
    /// Creates an empty factory.
    pub fn new() -> Self {
        Self {
            registry: HashMap::new(),
        }
    }

    /// Registers a constructor for `attribute_type`.
    ///
    /// The constructor must produce an implementation that lists
    /// `attribute_type` in its [`AttributeImpl::attribute_interfaces`].
    pub fn register<I: AttributeImpl + 'static>(
        &mut self,
        attribute_type: TypeId,
        ctor: impl Fn() -> I + Send + Sync + 'static,
    ) {
        self.registry
            .insert(attribute_type, Box::new(move || Box::new(ctor())));
    }

    /// Convenience registration using the implementation type itself as the
    /// attribute interface.
    ///
    /// This is useful for tests and for attributes whose public interface is
    /// the concrete implementation type.
    pub fn register_self<I: AttributeImpl + Default + 'static>(&mut self) {
        self.register(TypeId::of::<I>(), I::default);
    }

    /// Registers `I` under every interface it advertises, including its own
    /// concrete [`TypeId`].
    ///
    /// This is the Rust equivalent of Lucene's
    /// `AttributeFactory.getStaticImplementation(factory, implClass)`, which
    /// installs a single implementation as the provider for all of the
    /// attribute interfaces it implements.
    pub fn register_attribute_impl<I: AttributeImpl + Default + Send + Sync + 'static>(&mut self) {
        let instance = I::default();
        let interfaces = instance.attribute_interfaces().to_vec();
        let ctor = || I::default();
        for interface_id in interfaces {
            self.register(interface_id, ctor);
        }
    }
}

impl Debug for DefaultAttributeFactory {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultAttributeFactory")
            .field("registered_count", &self.registry.len())
            .finish()
    }
}

impl AttributeFactory for DefaultAttributeFactory {
    fn create_attribute_instance(
        &self,
        attribute_type: TypeId,
    ) -> Result<Box<dyn AttributeImpl>, LuceneError> {
        self.registry
            .get(&attribute_type)
            .map(|ctor| ctor())
            .ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "no AttributeImpl registered for TypeId {:?}",
                    attribute_type
                ))
            })
    }
}

/// **Expert**: an [`AttributeFactory`] returning an instance of one
/// implementation for the attributes that implementation covers, and calling a
/// delegate factory for every other attribute.
///
/// This lets a single [`AttributeImpl`] combining several attributes be
/// preferred over separate implementations.
///
/// Equivalent to `org.apache.lucene.util.AttributeFactory.StaticImplementationAttributeFactory`.
///
/// Divergence: Lucene's class is `abstract` with a `protected abstract A
/// createInstance()` that `getStaticImplementation` overrides in an anonymous
/// subclass invoking a `MethodHandle` (`AttributeFactory.java` lines 97-158).
/// Rust has no inheritance, so the single abstract method is held as the
/// constructor field the anonymous subclass would have captured, and the class
/// is concrete.
pub struct StaticImplementationAttributeFactory {
    delegate: Arc<dyn AttributeFactory>,
    clazz: TypeId,
    interfaces: &'static [TypeId],
    ctor: AttributeCtor,
}

impl StaticImplementationAttributeFactory {
    /// Creates an instance of the implementation this factory is fixed to.
    ///
    /// Equivalent to
    /// `AttributeFactory.StaticImplementationAttributeFactory#createInstance()`.
    pub fn create_instance(&self) -> Box<dyn AttributeImpl> {
        (self.ctor)()
    }
}

impl Debug for StaticImplementationAttributeFactory {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticImplementationAttributeFactory")
            .field("clazz", &self.clazz)
            .field("interfaces", &self.interfaces)
            .finish_non_exhaustive()
    }
}

impl AttributeFactory for StaticImplementationAttributeFactory {
    fn create_attribute_instance(
        &self,
        attribute_type: TypeId,
    ) -> Result<Box<dyn AttributeImpl>, LuceneError> {
        // `attClass.isAssignableFrom(clazz)`: the requested interface is one
        // the fixed implementation covers, or the implementation itself.
        if attribute_type == self.clazz || self.interfaces.contains(&attribute_type) {
            Ok(self.create_instance())
        } else {
            self.delegate.create_attribute_instance(attribute_type)
        }
    }
}

impl PartialEq for StaticImplementationAttributeFactory {
    fn eq(&self, other: &Self) -> bool {
        // Lucene compares `this.delegate.equals(af.delegate)`; `AttributeFactory`
        // does not override `equals`, so that is reference identity.
        Arc::ptr_eq(&self.delegate, &other.delegate) && self.clazz == other.clazz
    }
}

impl Eq for StaticImplementationAttributeFactory {}

impl Hash for StaticImplementationAttributeFactory {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Lucene computes `31 * delegate.hashCode() + clazz.hashCode()`, and
        // `AttributeFactory` inherits `Object.hashCode`, i.e. identity.
        (Arc::as_ptr(&self.delegate) as *const () as usize).hash(state);
        self.clazz.hash(state);
    }
}

/// Returns an [`AttributeFactory`] returning an instance of `A` for the
/// attributes it implements, and calling `delegate` for all other attributes.
///
/// This can be used to prefer a specific [`AttributeImpl`] that combines
/// several attributes over separate implementations.
///
/// Equivalent to `org.apache.lucene.util.AttributeFactory#getStaticImplementation`.
///
/// Divergence: Lucene takes the implementation as a `Class<A>` argument and
/// looks its constructor up reflectively, which is why its documentation asks
/// callers to cache the result in a static field. Rust takes the
/// implementation as the type parameter `A` and its `Default` bound as the
/// constructor, so nothing is resolved at run time and no caching is needed.
pub fn get_static_implementation<A: AttributeImpl + Default>(
    delegate: Arc<dyn AttributeFactory>,
) -> StaticImplementationAttributeFactory {
    let constr = find_attribute_impl_ctor::<A>();
    StaticImplementationAttributeFactory {
        delegate,
        clazz: TypeId::of::<A>(),
        interfaces: A::default().attribute_interfaces(),
        ctor: constr,
    }
}

/// Internal linked-list node used by [`AttributeSource`] to represent the
/// current attribute state.
#[derive(Clone, Debug)]
struct State {
    attribute: Rc<RefCell<Box<dyn AttributeImpl>>>,
    next: Option<Box<State>>,
}

/// Captured state of an [`AttributeSource`], suitable for later restoration.
///
/// This is the Rust equivalent of Lucene's `AttributeSource.State`.
#[derive(Debug)]
pub struct CapturedState {
    attributes: Vec<(TypeId, Box<dyn AttributeImpl>)>,
}

impl Clone for CapturedState {
    fn clone(&self) -> Self {
        Self {
            attributes: self
                .attributes
                .iter()
                .map(|(id, att)| (*id, att.clone_box()))
                .collect(),
        }
    }
}

/// Port of `org.apache.lucene.util.AttributeSource`.
///
/// Container that stores a set of attribute implementations and provides
/// lifecycle operations matching Lucene's `AttributeSource`.
///
/// Each attribute interface can appear at most once in a source. Adding an
/// attribute that is already present returns the existing instance.
#[derive(Clone, Debug)]
pub struct AttributeSource {
    /// Maps attribute interface TypeId -> shared implementation.
    attributes: HashMap<TypeId, Rc<RefCell<Box<dyn AttributeImpl>>>>,
    /// Maps concrete implementation TypeId -> shared implementation.
    attribute_impls: HashMap<TypeId, Rc<RefCell<Box<dyn AttributeImpl>>>>,
    /// Lazily-built linked list of the current implementations.
    current_state: RefCell<Option<Box<State>>>,
    /// Factory used to create missing attributes.
    factory: Arc<dyn AttributeFactory>,
}

impl AttributeSource {
    /// Creates an empty source using an empty default factory.
    pub fn new() -> Self {
        Self::new_with_factory(Arc::new(DefaultAttributeFactory::new()))
    }

    /// Creates an empty source using the supplied factory.
    pub fn new_with_factory(factory: Arc<dyn AttributeFactory>) -> Self {
        Self {
            attributes: HashMap::new(),
            attribute_impls: HashMap::new(),
            current_state: RefCell::new(None),
            factory,
        }
    }

    /// Creates a source that shares its attribute maps with `input`.
    ///
    /// Changes made through either source are visible to the other, matching
    /// Lucene's `AttributeSource(AttributeSource input)` constructor.
    pub fn new_from(input: &Self) -> Self {
        Self {
            attributes: input.attributes.clone(),
            attribute_impls: input.attribute_impls.clone(),
            current_state: RefCell::new(None),
            factory: Arc::clone(&input.factory),
        }
    }

    /// Returns the factory used by this source.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeFactory`.
    pub fn factory(&self) -> &dyn AttributeFactory {
        self.factory.as_ref()
    }

    fn add_attribute_impl(&mut self, att: Box<dyn AttributeImpl>) {
        let type_id = att.impl_type_id();
        if self.attribute_impls.contains_key(&type_id) {
            return;
        }

        let interfaces = att.attribute_interfaces().to_vec();
        let rc = Rc::new(RefCell::new(att));

        for interface_id in interfaces {
            if !self.attributes.contains_key(&interface_id) {
                *self.current_state.borrow_mut() = None;
                self.attributes.insert(interface_id, rc.clone());
            }
        }
        self.attribute_impls.insert(type_id, rc);
    }

    /// Adds an attribute of type `A`, creating it through the factory if
    /// necessary, and returns a borrowed reference to it.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the factory cannot create the
    /// attribute.
    pub fn add_attribute<A: Attribute>(&mut self) -> Result<Ref<'_, A>, LuceneError> {
        let id = TypeId::of::<A>();
        if !self.attributes.contains_key(&id) {
            let new_impl = self.factory.create_attribute_instance(id)?;
            self.add_attribute_impl(new_impl);
        }
        let rc = self.attributes.get(&id).unwrap();
        Ok(Ref::map(rc.borrow(), |att| {
            att.as_any()
                .downcast_ref::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// Adds an attribute by its concrete [`TypeId`] and returns the shared
    /// implementation handle.
    pub fn add_attribute_by_id(
        &mut self,
        attribute_type: TypeId,
    ) -> Result<Rc<RefCell<Box<dyn AttributeImpl>>>, LuceneError> {
        if !self.attributes.contains_key(&attribute_type) {
            let new_impl = self.factory.create_attribute_instance(attribute_type)?;
            self.add_attribute_impl(new_impl);
        }
        Ok(Rc::clone(self.attributes.get(&attribute_type).unwrap()))
    }

    /// Adds a pre-built [`AttributeImpl`] instance to this source.
    pub fn add_attribute_impl_instance(&mut self, att: Box<dyn AttributeImpl>) {
        self.add_attribute_impl(att);
    }

    /// Returns true if this source contains attribute `A`.
    pub fn has_attribute<A: Attribute>(&self) -> bool {
        self.attributes.contains_key(&TypeId::of::<A>())
    }

    /// Returns true if this source contains the attribute with the given
    /// [`TypeId`].
    pub fn has_attribute_by_id(&self, attribute_type: TypeId) -> bool {
        self.attributes.contains_key(&attribute_type)
    }

    /// Returns true if this source has any attributes.
    pub fn has_attributes(&self) -> bool {
        !self.attributes.is_empty()
    }

    /// Returns a borrowed reference to attribute `A`, if present.
    pub fn get_attribute<A: Attribute>(&self) -> Option<Ref<'_, A>> {
        let rc = self.attributes.get(&TypeId::of::<A>())?;
        Some(Ref::map(rc.borrow(), |att| {
            att.as_any()
                .downcast_ref::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// Returns a mutable borrowed reference to attribute `A`, if present.
    pub fn get_attribute_mut<A: Attribute>(&self) -> Option<RefMut<'_, A>> {
        let rc = self.attributes.get(&TypeId::of::<A>())?;
        Some(RefMut::map(rc.borrow_mut(), |att| {
            att.as_any_mut()
                .downcast_mut::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// Clears every attribute in this source.
    pub fn clear_attributes(&self) {
        if let Some(state) = self.get_current_state() {
            let mut cur = Some(state);
            while let Some(s) = cur {
                s.attribute.borrow_mut().clear();
                cur = s.next;
            }
        }
    }

    /// Calls [`AttributeImpl::end`] on every attribute.
    pub fn end_attributes(&self) {
        if let Some(state) = self.get_current_state() {
            let mut cur = Some(state);
            while let Some(s) = cur {
                s.attribute.borrow_mut().end();
                cur = s.next;
            }
        }
    }

    /// Removes every attribute and implementation from this source.
    pub fn remove_all_attributes(&mut self) {
        self.attributes.clear();
        self.attribute_impls.clear();
        *self.current_state.borrow_mut() = None;
    }

    /// Returns an iterator over the attribute interface [`TypeId`]s in
    /// insertion order.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeClassesIterator`.
    pub fn attribute_classes(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.attributes.keys().copied()
    }

    /// Returns an iterator over the unique attribute implementations in this
    /// source.
    ///
    /// This may yield fewer entries than [`Self::attribute_classes`], when one
    /// implementation covers more than one attribute interface.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeImplsIterator`.
    pub fn attribute_impls_iter(
        &self,
    ) -> impl Iterator<Item = Rc<RefCell<Box<dyn AttributeImpl>>>> + '_ {
        self.attribute_impls.values().cloned()
    }

    /// Captures the current state of all attributes as a deep snapshot.
    ///
    /// Returns `None` when this source has no attributes.
    pub fn capture_state(&self) -> Option<CapturedState> {
        let current = self.get_current_state()?;

        let mut attributes = Vec::new();
        let mut cur = Some(&current);
        while let Some(state) = cur {
            let att = state.attribute.borrow();
            attributes.push((att.impl_type_id(), att.clone_box()));
            cur = state.next.as_ref();
        }

        Some(CapturedState { attributes })
    }

    /// Restores all attributes in this source from `state`.
    ///
    /// Attributes in `state` that are not present in this source cause an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `state` contains an
    /// implementation not present in this source.
    pub fn restore_state(&self, state: &CapturedState) -> Result<(), LuceneError> {
        for (type_id, captured) in &state.attributes {
            let target_rc = self.attribute_impls.get(type_id).ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "state contains AttributeImpl with TypeId {:?} not present in target",
                    type_id
                ))
            })?;
            captured.copy_to(&mut **target_rc.borrow_mut());
        }
        Ok(())
    }

    /// Copies all attributes from this source to `target`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `target` lacks a matching
    /// implementation.
    pub fn copy_to(&self, target: &AttributeSource) -> Result<(), LuceneError> {
        if let Some(state) = self.get_current_state() {
            let mut cur = Some(state);
            while let Some(s) = cur {
                let src = s.attribute.borrow();
                let type_id = src.impl_type_id();
                let target_rc = target.attribute_impls.get(&type_id).ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "source contains AttributeImpl with TypeId {:?} not in target",
                        type_id
                    ))
                })?;
                src.copy_to(&mut **target_rc.borrow_mut());
                cur = s.next;
            }
        }
        Ok(())
    }

    /// Returns a new source containing clones of all current attribute
    /// implementations.
    pub fn clone_attributes(&self) -> AttributeSource {
        let mut clone = AttributeSource::new_with_factory(Arc::clone(&self.factory));
        if let Some(state) = self.get_current_state() {
            let mut cur = Some(state);
            while let Some(s) = cur {
                let att = s.attribute.borrow();
                clone.add_attribute_impl(att.clone_box());
                cur = s.next;
            }
        }
        clone
    }

    /// Reflects every attribute implementation through `reflector`.
    pub fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        if let Some(state) = self.get_current_state() {
            let mut cur = Some(state);
            while let Some(s) = cur {
                s.attribute.borrow().reflect_with(reflector);
                cur = s.next;
            }
        }
    }

    /// Returns a hash code combining the hash codes of every attribute
    /// implementation this source holds.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#hashCode`.
    ///
    /// Divergence: Lucene folds the implementations with `code = code * 31 +
    /// state.attribute.hashCode()` while walking the state list, whose order is
    /// the insertion order of the `LinkedHashMap` backing `attributeImpls`
    /// (`AttributeSource.java` lines 335-341). This port's `attribute_impls` is
    /// a [`HashMap`], which has no insertion order and whose iteration order
    /// differs between two instances holding the same entries, so a positional
    /// fold could not agree with [`PartialEq`]. The implementations' hash codes
    /// are therefore summed, which is order-independent; the resulting value
    /// differs from Java's, but it is not part of any index format.
    pub fn hash_code(&self) -> i32 {
        let mut code: i32 = 0;
        for att in self.attribute_impls.values() {
            code = code.wrapping_add(att.borrow().hash_code());
        }
        code
    }

    /// Returns a comma-separated string of all reflected attribute values.
    ///
    /// If `prepend_att_class` is true, each key is prefixed with the attribute
    /// source-level name and `#`.
    pub fn reflect_as_string(&self, prepend_att_class: bool) -> String {
        let mut reflector = ReflectAsString {
            buffer: String::new(),
            prepend_att_class,
        };
        self.reflect_with(&mut reflector);
        reflector.buffer
    }
}

/// Equivalent to `org.apache.lucene.util.AttributeSource#equals`.
///
/// Two sources are equal when neither holds any attribute, or when both hold
/// the same number of implementations and every implementation is of the same
/// concrete type and equal to its counterpart.
///
/// Divergence: Lucene walks both state lists in step and requires the
/// implementations to match pairwise *in insertion order*
/// (`AttributeSource.java` lines 343-377). This port's `attribute_impls` is a
/// [`HashMap`], which has no insertion order and whose iteration order differs
/// between two instances holding the same entries, so a pairwise walk would not
/// even be deterministic. The counterpart is therefore looked up by concrete
/// implementation type — the very key Lucene's `attributeImpls` map uses, so
/// the class check is subsumed by the lookup. The two answers agree except when
/// the same implementations were added in different orders, which Lucene
/// reports unequal and this reports equal.
impl PartialEq for AttributeSource {
    fn eq(&self, other: &Self) -> bool {
        if !self.has_attributes() {
            return !other.has_attributes();
        }
        if !other.has_attributes() {
            return false;
        }
        if self.attribute_impls.len() != other.attribute_impls.len() {
            return false;
        }
        // it is only equal if all attribute impls are the same
        for (impl_type, att) in &self.attribute_impls {
            let Some(other_att) = other.attribute_impls.get(impl_type) else {
                return false;
            };
            let this_att = att.borrow();
            let that_att = other_att.borrow();
            if !that_att.equals(&**this_att) {
                return false;
            }
        }
        true
    }
}

struct ReflectAsString {
    buffer: String,
    prepend_att_class: bool,
}

impl AttributeReflector for ReflectAsString {
    fn reflect(
        &mut self,
        _attribute_type: TypeId,
        attribute_name: &'static str,
        key: &str,
        value: &dyn Debug,
    ) {
        if !self.buffer.is_empty() {
            self.buffer.push(',');
        }
        if self.prepend_att_class {
            self.buffer.push_str(attribute_name);
            self.buffer.push('#');
        }
        self.buffer.push_str(key);
        self.buffer.push('=');
        self.buffer.push_str(&format!("{:?}", value));
    }
}

impl AttributeSource {
    fn get_current_state(&self) -> Option<Box<State>> {
        if self.current_state.borrow().is_none() && self.has_attributes() {
            let mut head: Option<Box<State>> = None;
            let mut tail: Option<&mut Box<State>> = None;

            for rc in self.attribute_impls.values() {
                let node = Box::new(State {
                    attribute: rc.clone(),
                    next: None,
                });

                match tail {
                    None => {
                        head = Some(node);
                        tail = head.as_mut();
                    }
                    Some(t) => {
                        t.next = Some(node);
                        tail = t.next.as_mut();
                    }
                }
            }

            *self.current_state.borrow_mut() = head;
        }

        self.current_state.borrow().clone()
    }
}

impl Default for AttributeSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Port of `org.apache.lucene.util.Unwrappable`.
///
/// Trait for wrappers that expose their underlying delegate.
///
/// Equivalent to Lucene's `Unwrappable<T>`.
pub trait Unwrappable<T: ?Sized> {
    /// Returns the wrapped object.
    fn unwrap(&self) -> &T;
}

/// Helper trait used by [`unwrap_all`] to attempt to treat a value as a
/// wrapper.
pub trait AsUnwrappable<T: ?Sized> {
    /// Returns `Some(wrapper)` if this value is a wrapper around `T`.
    fn as_unwrappable(&self) -> Option<&dyn Unwrappable<T>>;
}

/// Unwraps all nested `Unwrappable`s around `value`.
///
/// This is the Rust equivalent of `Unwrappable.unwrapAll(T)`. The base type
/// `T` must implement [`AsUnwrappable<T>`] so that the unwrapper can detect
/// whether a value is wrapped.
pub fn unwrap_all<T>(mut value: &T) -> &T
where
    T: ?Sized + AsUnwrappable<T>,
{
    while let Some(wrapper) = value.as_unwrappable() {
        value = wrapper.unwrap();
    }
    value
}

// Per-instance thread-local identifier.
static NEXT_THREAD_LOCAL_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CLOSEABLE_THREAD_LOCALS: RefCell<HashMap<u64, Box<dyn Any>>> =
        RefCell::new(HashMap::new());
}

/// Port of `org.apache.lucene.util.CloseableThreadLocal`.
///
/// Per-thread object reuse with explicit cleanup.
///
/// Equivalent to Lucene's `CloseableThreadLocal`. Each instance owns an entry
/// in a thread-local map keyed by a unique id. Calling [`Self::close`]
/// removes the entry for the current thread, allowing the value to be
/// reclaimed.
pub struct CloseableThreadLocal<T: 'static> {
    id: u64,
    initial_value: Box<dyn Fn() -> Rc<RefCell<T>> + Send + Sync>,
}

impl<T: 'static> CloseableThreadLocal<T> {
    /// Creates a thread local whose initial value is produced by `initial_value`.
    pub fn new(initial_value: impl Fn() -> T + Send + Sync + 'static) -> Self {
        let producer = move || Rc::new(RefCell::new(initial_value()));
        Self {
            id: NEXT_THREAD_LOCAL_ID.fetch_add(1, Ordering::Relaxed),
            initial_value: Box::new(producer),
        }
    }

    /// Returns the value to install for a thread that does not have one yet.
    ///
    /// Equivalent to `org.apache.lucene.util.CloseableThreadLocal#initialValue`.
    ///
    /// Divergence: Lucene declares the method `protected` and returns `null`
    /// from it, so that a subclass supplies the value by overriding it
    /// (`CloseableThreadLocal.java` lines 62-64), and `get` returns `null` when
    /// it is not overridden. Rust has no inheritance, so the override is the
    /// closure handed to [`Self::new`] and this method invokes it; because a
    /// closure is always supplied, the `null` case does not arise and the
    /// return type is not an [`Option`].
    pub fn initial_value(&self) -> Rc<RefCell<T>> {
        (self.initial_value)()
    }

    /// Returns the thread-local value, creating it if necessary.
    pub fn get(&self) -> Rc<RefCell<T>> {
        CLOSEABLE_THREAD_LOCALS.with(|map| {
            let mut map = map.borrow_mut();
            let boxed = map.entry(self.id).or_insert_with(|| {
                let value = self.initial_value();
                Box::new(value)
            });
            boxed
                .downcast_ref::<Rc<RefCell<T>>>()
                .expect("CloseableThreadLocal type mismatch")
                .clone()
        })
    }

    /// Replaces the thread-local value.
    pub fn set(&self, value: T) {
        CLOSEABLE_THREAD_LOCALS.with(|map| {
            map.borrow_mut()
                .insert(self.id, Box::new(Rc::new(RefCell::new(value))));
        });
    }

    /// Removes the thread-local entry for the current thread.
    pub fn close(&self) {
        CLOSEABLE_THREAD_LOCALS.with(|map| {
            map.borrow_mut().remove(&self.id);
        });
    }
}

impl<T: 'static> Debug for CloseableThreadLocal<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloseableThreadLocal")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    #[derive(Clone, Debug, Default)]
    struct TestAttribute {
        value: i32,
    }

    impl Attribute for TestAttribute {}

    impl AttributeImpl for TestAttribute {
        fn clear(&mut self) {
            self.value = 0;
        }

        fn copy_to(&self, target: &mut dyn AttributeImpl) {
            if let Some(t) = target.as_any_mut().downcast_mut::<TestAttribute>() {
                t.value = self.value;
            }
        }

        fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
            reflector.reflect(
                TypeId::of::<TestAttribute>(),
                std::any::type_name::<TestAttribute>(),
                "value",
                &self.value,
            );
        }

        fn clone_box(&self) -> Box<dyn AttributeImpl> {
            Box::new(self.clone())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }

        fn attribute_interfaces(&self) -> &'static [TypeId] {
            static INTERFACES: [TypeId; 1] = [TypeId::of::<TestAttribute>()];
            &INTERFACES
        }
    }

    #[derive(Clone, Debug, Default)]
    struct OtherAttribute {
        text: String,
    }

    impl Attribute for OtherAttribute {}

    impl AttributeImpl for OtherAttribute {
        fn clear(&mut self) {
            self.text.clear();
        }

        fn copy_to(&self, target: &mut dyn AttributeImpl) {
            if let Some(t) = target.as_any_mut().downcast_mut::<OtherAttribute>() {
                t.text.clone_from(&self.text);
            }
        }

        fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
            reflector.reflect(
                TypeId::of::<OtherAttribute>(),
                std::any::type_name::<OtherAttribute>(),
                "text",
                &self.text,
            );
        }

        fn clone_box(&self) -> Box<dyn AttributeImpl> {
            Box::new(self.clone())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }

        fn attribute_interfaces(&self) -> &'static [TypeId] {
            static INTERFACES: [TypeId; 1] = [TypeId::of::<OtherAttribute>()];
            &INTERFACES
        }
    }

    fn test_factory() -> Arc<dyn AttributeFactory> {
        let mut factory = DefaultAttributeFactory::new();
        factory.register_self::<TestAttribute>();
        factory.register_self::<OtherAttribute>();
        Arc::new(factory)
    }

    #[test]
    fn add_get_has_attribute() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        assert!(!source.has_attribute::<TestAttribute>());

        {
            let att = source.add_attribute::<TestAttribute>().unwrap();
            assert_eq!(att.value, 0);
        }

        assert!(source.has_attribute::<TestAttribute>());
        assert!(!source.has_attribute::<OtherAttribute>());

        let att = source.get_attribute::<TestAttribute>().unwrap();
        assert_eq!(att.value, 0);
    }

    #[test]
    fn add_attribute_returns_existing_instance() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        let id = TypeId::of::<TestAttribute>();

        let first = source.add_attribute_by_id(id).unwrap();
        let second = source.add_attribute_by_id(id).unwrap();

        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn multiple_attributes_in_one_source() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.add_attribute::<OtherAttribute>().unwrap();

        source.get_attribute_mut::<TestAttribute>().unwrap().value = 42;
        source.get_attribute_mut::<OtherAttribute>().unwrap().text = "hello".to_string();

        assert_eq!(source.get_attribute::<TestAttribute>().unwrap().value, 42);
        assert_eq!(
            source.get_attribute::<OtherAttribute>().unwrap().text,
            "hello"
        );

        let classes: Vec<TypeId> = source.attribute_classes().collect();
        assert_eq!(classes.len(), 2);
        assert!(source.has_attribute::<TestAttribute>());
        assert!(source.has_attribute::<OtherAttribute>());
    }

    #[test]
    fn clear_attributes_resets_all() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.add_attribute::<OtherAttribute>().unwrap();

        source.get_attribute_mut::<TestAttribute>().unwrap().value = 10;
        source.get_attribute_mut::<OtherAttribute>().unwrap().text = "x".to_string();

        source.clear_attributes();

        assert_eq!(source.get_attribute::<TestAttribute>().unwrap().value, 0);
        assert!(source
            .get_attribute::<OtherAttribute>()
            .unwrap()
            .text
            .is_empty());
    }

    #[test]
    fn capture_and_restore_state_across_attributes() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.add_attribute::<OtherAttribute>().unwrap();

        source.get_attribute_mut::<TestAttribute>().unwrap().value = 5;
        source.get_attribute_mut::<OtherAttribute>().unwrap().text = "state".to_string();

        let state = source.capture_state().unwrap();

        source.clear_attributes();
        assert_eq!(source.get_attribute::<TestAttribute>().unwrap().value, 0);
        assert!(source
            .get_attribute::<OtherAttribute>()
            .unwrap()
            .text
            .is_empty());

        source.restore_state(&state).unwrap();
        assert_eq!(source.get_attribute::<TestAttribute>().unwrap().value, 5);
        assert_eq!(
            source.get_attribute::<OtherAttribute>().unwrap().text,
            "state"
        );

        // Captured state is independent: mutating source does not mutate state.
        source.get_attribute_mut::<TestAttribute>().unwrap().value = 99;
        let captured_test = state
            .attributes
            .iter()
            .find(|(id, _)| *id == TypeId::of::<TestAttribute>())
            .map(|(_, att)| att)
            .unwrap();
        assert_eq!(
            captured_test
                .as_any()
                .downcast_ref::<TestAttribute>()
                .unwrap()
                .value,
            5
        );
    }

    #[test]
    fn copy_to_matching_source() {
        let factory = test_factory();
        let mut source1 = AttributeSource::new_with_factory(Arc::clone(&factory));
        let mut source2 = AttributeSource::new_with_factory(factory);

        source1.add_attribute::<TestAttribute>().unwrap();
        source1.add_attribute::<OtherAttribute>().unwrap();
        source2.add_attribute::<TestAttribute>().unwrap();
        source2.add_attribute::<OtherAttribute>().unwrap();

        source1.get_attribute_mut::<TestAttribute>().unwrap().value = 7;
        source1.get_attribute_mut::<OtherAttribute>().unwrap().text = "copied".to_string();

        source1.copy_to(&source2).unwrap();

        assert_eq!(source2.get_attribute::<TestAttribute>().unwrap().value, 7);
        assert_eq!(
            source2.get_attribute::<OtherAttribute>().unwrap().text,
            "copied"
        );
    }

    #[test]
    fn copy_to_missing_impl_fails() {
        let factory = test_factory();
        let mut source1 = AttributeSource::new_with_factory(Arc::clone(&factory));
        let mut source2 = AttributeSource::new_with_factory(factory);

        source1.add_attribute::<TestAttribute>().unwrap();
        source2.add_attribute::<OtherAttribute>().unwrap();

        assert!(source1.copy_to(&source2).is_err());
    }

    #[test]
    fn clone_attributes_is_independent() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.get_attribute_mut::<TestAttribute>().unwrap().value = 123;

        let clone = source.clone_attributes();
        assert_eq!(clone.get_attribute::<TestAttribute>().unwrap().value, 123);

        source.get_attribute_mut::<TestAttribute>().unwrap().value = 456;
        assert_eq!(clone.get_attribute::<TestAttribute>().unwrap().value, 123);
    }

    #[test]
    fn new_from_shares_implementations() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();

        let shared = AttributeSource::new_from(&source);
        shared.get_attribute_mut::<TestAttribute>().unwrap().value = 111;
        assert_eq!(source.get_attribute::<TestAttribute>().unwrap().value, 111);
    }

    #[test]
    fn reflect_as_string_contains_keys_and_values() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.get_attribute_mut::<TestAttribute>().unwrap().value = 77;

        let s = source.reflect_as_string(false);
        assert!(s.contains("value=77"));
    }

    #[test]
    fn closeable_thread_local_reuse_and_cleanup() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let tl = CloseableThreadLocal::new(|| COUNTER.fetch_add(1, Ordering::SeqCst));

        let first = tl.get();
        let second = tl.get();
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(*first.borrow(), 0);

        tl.set(100);
        let after_set = tl.get();
        assert_eq!(*after_set.borrow(), 100);

        tl.close();
        let after_close = tl.get();
        assert!(!Rc::ptr_eq(&first, &after_close));
        assert_eq!(*after_close.borrow(), 1);
    }

    #[test]
    fn closeable_thread_local_is_per_thread() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let tl = CloseableThreadLocal::new(|| COUNTER.fetch_add(1, Ordering::SeqCst));
        let main_value = *tl.get().borrow();

        let child_value = thread::scope(|s| {
            s.spawn(|| *tl.get().borrow())
                .join()
                .expect("child thread panicked")
        });

        assert_ne!(main_value, child_value);
    }

    #[test]
    fn captured_state_clones_independently() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.get_attribute_mut::<TestAttribute>().unwrap().value = 3;

        let state1 = source.capture_state().unwrap();
        let state2 = state1.clone();

        source.get_attribute_mut::<TestAttribute>().unwrap().value = 4;
        assert_eq!(
            state1.attributes[0]
                .1
                .as_any()
                .downcast_ref::<TestAttribute>()
                .unwrap()
                .value,
            3
        );
        assert_eq!(
            state2.attributes[0]
                .1
                .as_any()
                .downcast_ref::<TestAttribute>()
                .unwrap()
                .value,
            3
        );
    }

    /// `AttributeImpl.reflectAsString(boolean)` renders one attribute's own
    /// properties, unlike `AttributeSource.reflectAsString`, which concatenates
    /// every implementation a source holds.
    #[test]
    fn attribute_impl_reflect_as_string_renders_its_own_properties() {
        let att = TestAttribute { value: 77 };
        assert_eq!(att.reflect_as_string(false), "value=77");

        // With the class prefix, Java writes `attClass.getName() + '#' + key`.
        let with_class = att.reflect_as_string(true);
        assert!(
            with_class.ends_with("#value=77"),
            "expected a `<class>#value=77` rendering, got {with_class}"
        );
        assert!(with_class.contains("TestAttribute"), "got {with_class}");
    }

    /// An attribute that reflects several properties separates them with a
    /// comma and no space, and never leads with one.
    #[test]
    fn attribute_impl_reflect_as_string_separates_properties_with_commas() {
        #[derive(Clone, Debug, Default)]
        struct TwoValues {
            a: i32,
            b: i32,
        }
        impl Attribute for TwoValues {}
        impl AttributeImpl for TwoValues {
            fn clear(&mut self) {
                self.a = 0;
                self.b = 0;
            }
            fn copy_to(&self, target: &mut dyn AttributeImpl) {
                if let Some(t) = target.as_any_mut().downcast_mut::<TwoValues>() {
                    t.a = self.a;
                    t.b = self.b;
                }
            }
            fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
                reflector.reflect(TypeId::of::<TwoValues>(), "TwoValues", "a", &self.a);
                reflector.reflect(TypeId::of::<TwoValues>(), "TwoValues", "b", &self.b);
            }
            fn clone_box(&self) -> Box<dyn AttributeImpl> {
                Box::new(self.clone())
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
            fn attribute_interfaces(&self) -> &'static [TypeId] {
                static INTERFACES: [TypeId; 1] = [TypeId::of::<TwoValues>()];
                &INTERFACES
            }
        }

        let att = TwoValues { a: 1, b: 2 };
        assert_eq!(att.reflect_as_string(false), "a=1,b=2");
        assert_eq!(att.reflect_as_string(true), "TwoValues#a=1,TwoValues#b=2");
    }

    /// An attribute that reflects nothing renders the empty string, which is
    /// what Java's empty `StringBuilder` produces.
    #[test]
    fn attribute_impl_reflect_as_string_of_a_silent_attribute_is_empty() {
        #[derive(Clone, Debug, Default)]
        struct Silent;
        impl Attribute for Silent {}
        impl AttributeImpl for Silent {
            fn clear(&mut self) {}
            fn copy_to(&self, _target: &mut dyn AttributeImpl) {}
            fn reflect_with(&self, _reflector: &mut dyn AttributeReflector) {}
            fn clone_box(&self) -> Box<dyn AttributeImpl> {
                Box::new(self.clone())
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
            fn attribute_interfaces(&self) -> &'static [TypeId] {
                static INTERFACES: [TypeId; 1] = [TypeId::of::<Silent>()];
                &INTERFACES
            }
        }
        assert_eq!(Silent.reflect_as_string(false), "");
        assert_eq!(Silent.reflect_as_string(true), "");
    }
}

impl std::fmt::Display for AttributeSource {
    /// Port of `AttributeSource.toString()`, which is
    /// `getClass().getSimpleName() + '@' + Integer.toHexString(System.identityHashCode(this))
    /// + " " + reflectAsString(false)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Rust has no identity hash; the
    /// address of the value stands in for it, the convention this port already
    /// uses for `Object.hashCode()` (see `IndexCommit`). The digits differ from
    /// a JVM's, exactly as two JVM runs differ from each other. Everything after
    /// the space — the reflected attribute rendering, which is what the string
    /// is read for — is reproduced exactly.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AttributeSource@{} {}",
            crate::util::java_fmt::to_hex_string(self as *const Self as usize as i32),
            self.reflect_as_string(false)
        )
    }
}
