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
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::analysis::tokenattributes::{
    BytesTermAttribute, CharTermAttribute, FlagsAttribute, KeywordAttribute, OffsetAttribute,
    PayloadAttribute, PositionIncrementAttribute, PositionLengthAttribute, TermFrequencyAttribute,
    TermToBytesRefAttribute, TypeAttribute,
};
use crate::error::LuceneError;
use crate::util::atomic_ref_cell::{AtomicRef, AtomicRefCell, AtomicRefMut};

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

    // -- Interface casts -----------------------------------------------------
    //
    // Java holds an attribute by its interface (`CharTermAttribute termAtt =
    // addAttribute(CharTermAttribute.class)`), whatever class the factory
    // chose to implement it with. Rust cannot cast a `dyn AttributeImpl` to
    // another trait object, so every implementation answers, for each token
    // attribute interface it implements, the cast to it; see
    // [`AttributeInterface`] and [`AttributeSource::get_attribute_interface`].

    /// This implementation as a `CharTermAttribute`, Java's cast `(CharTermAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_char_term_attribute(&self) -> Option<&dyn CharTermAttribute> {
        None
    }

    /// This implementation mutably as a `CharTermAttribute`; see
    /// [`as_char_term_attribute`](Self::as_char_term_attribute).
    fn as_char_term_attribute_mut(&mut self) -> Option<&mut dyn CharTermAttribute> {
        None
    }

    /// This implementation as a `TermToBytesRefAttribute`, Java's cast `(TermToBytesRefAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_term_to_bytes_ref_attribute(&self) -> Option<&dyn TermToBytesRefAttribute> {
        None
    }

    /// This implementation mutably as a `TermToBytesRefAttribute`; see
    /// [`as_term_to_bytes_ref_attribute`](Self::as_term_to_bytes_ref_attribute).
    fn as_term_to_bytes_ref_attribute_mut(&mut self) -> Option<&mut dyn TermToBytesRefAttribute> {
        None
    }

    /// This implementation as a `BytesTermAttribute`, Java's cast `(BytesTermAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_bytes_term_attribute(&self) -> Option<&dyn BytesTermAttribute> {
        None
    }

    /// This implementation mutably as a `BytesTermAttribute`; see
    /// [`as_bytes_term_attribute`](Self::as_bytes_term_attribute).
    fn as_bytes_term_attribute_mut(&mut self) -> Option<&mut dyn BytesTermAttribute> {
        None
    }

    /// This implementation as a `OffsetAttribute`, Java's cast `(OffsetAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_offset_attribute(&self) -> Option<&dyn OffsetAttribute> {
        None
    }

    /// This implementation mutably as a `OffsetAttribute`; see
    /// [`as_offset_attribute`](Self::as_offset_attribute).
    fn as_offset_attribute_mut(&mut self) -> Option<&mut dyn OffsetAttribute> {
        None
    }

    /// This implementation as a `PositionIncrementAttribute`, Java's cast `(PositionIncrementAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_position_increment_attribute(&self) -> Option<&dyn PositionIncrementAttribute> {
        None
    }

    /// This implementation mutably as a `PositionIncrementAttribute`; see
    /// [`as_position_increment_attribute`](Self::as_position_increment_attribute).
    fn as_position_increment_attribute_mut(
        &mut self,
    ) -> Option<&mut dyn PositionIncrementAttribute> {
        None
    }

    /// This implementation as a `PositionLengthAttribute`, Java's cast `(PositionLengthAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_position_length_attribute(&self) -> Option<&dyn PositionLengthAttribute> {
        None
    }

    /// This implementation mutably as a `PositionLengthAttribute`; see
    /// [`as_position_length_attribute`](Self::as_position_length_attribute).
    fn as_position_length_attribute_mut(&mut self) -> Option<&mut dyn PositionLengthAttribute> {
        None
    }

    /// This implementation as a `TypeAttribute`, Java's cast `(TypeAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_type_attribute(&self) -> Option<&dyn TypeAttribute> {
        None
    }

    /// This implementation mutably as a `TypeAttribute`; see
    /// [`as_type_attribute`](Self::as_type_attribute).
    fn as_type_attribute_mut(&mut self) -> Option<&mut dyn TypeAttribute> {
        None
    }

    /// This implementation as a `TermFrequencyAttribute`, Java's cast `(TermFrequencyAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_term_frequency_attribute(&self) -> Option<&dyn TermFrequencyAttribute> {
        None
    }

    /// This implementation mutably as a `TermFrequencyAttribute`; see
    /// [`as_term_frequency_attribute`](Self::as_term_frequency_attribute).
    fn as_term_frequency_attribute_mut(&mut self) -> Option<&mut dyn TermFrequencyAttribute> {
        None
    }

    /// This implementation as a `PayloadAttribute`, Java's cast `(PayloadAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_payload_attribute(&self) -> Option<&dyn PayloadAttribute> {
        None
    }

    /// This implementation mutably as a `PayloadAttribute`; see
    /// [`as_payload_attribute`](Self::as_payload_attribute).
    fn as_payload_attribute_mut(&mut self) -> Option<&mut dyn PayloadAttribute> {
        None
    }

    /// This implementation as a `FlagsAttribute`, Java's cast `(FlagsAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_flags_attribute(&self) -> Option<&dyn FlagsAttribute> {
        None
    }

    /// This implementation mutably as a `FlagsAttribute`; see
    /// [`as_flags_attribute`](Self::as_flags_attribute).
    fn as_flags_attribute_mut(&mut self) -> Option<&mut dyn FlagsAttribute> {
        None
    }

    /// This implementation as a `KeywordAttribute`, Java's cast `(KeywordAttribute) impl`;
    /// `None` when it does not implement the interface.
    fn as_keyword_attribute(&self) -> Option<&dyn KeywordAttribute> {
        None
    }

    /// This implementation mutably as a `KeywordAttribute`; see
    /// [`as_keyword_attribute`](Self::as_keyword_attribute).
    fn as_keyword_attribute_mut(&mut self) -> Option<&mut dyn KeywordAttribute> {
        None
    }

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
    /// Returns `LuceneErrorKind::IllegalArgument` if the factory does not know how
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
                LuceneError::illegal_argument(format!(
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

/// A token attribute interface, `dyn CharTermAttribute` and its siblings:
/// what Java names with `XAttribute.class`.
///
/// An [`AttributeSource`] keys its attributes by the interface's [`TypeId`]
/// (`TypeId::of::<dyn CharTermAttribute>()`); [`cast`](Self::cast) turns the
/// implementation found there into the interface, through the casts every
/// [`AttributeImpl`] answers.
pub trait AttributeInterface: 'static {
    /// Casts `att` to this interface.
    fn cast(att: &dyn AttributeImpl) -> Option<&Self>;

    /// Casts `att` mutably to this interface.
    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self>;
}

impl AttributeInterface for dyn CharTermAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_char_term_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_char_term_attribute_mut()
    }
}

impl AttributeInterface for dyn TermToBytesRefAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_term_to_bytes_ref_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_term_to_bytes_ref_attribute_mut()
    }
}

impl AttributeInterface for dyn BytesTermAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_bytes_term_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_bytes_term_attribute_mut()
    }
}

impl AttributeInterface for dyn OffsetAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_offset_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_offset_attribute_mut()
    }
}

impl AttributeInterface for dyn PositionIncrementAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_position_increment_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_position_increment_attribute_mut()
    }
}

impl AttributeInterface for dyn PositionLengthAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_position_length_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_position_length_attribute_mut()
    }
}

impl AttributeInterface for dyn TypeAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_type_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_type_attribute_mut()
    }
}

impl AttributeInterface for dyn TermFrequencyAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_term_frequency_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_term_frequency_attribute_mut()
    }
}

impl AttributeInterface for dyn PayloadAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_payload_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_payload_attribute_mut()
    }
}

impl AttributeInterface for dyn FlagsAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_flags_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_flags_attribute_mut()
    }
}

impl AttributeInterface for dyn KeywordAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_keyword_attribute()
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_keyword_attribute_mut()
    }
}

/// A shared, mutable attribute implementation held by an [`AttributeSource`].
pub type AttributeCell = Arc<AtomicRefCell<Box<dyn AttributeImpl>>>;

/// The attribute maps every [`AttributeSource`] of one chain shares.
///
/// Java's `AttributeSource(AttributeSource input)` constructor assigns the
/// very `attributes` and `attributeImpls` maps (and the `currentState` array)
/// of `input` to the new source (`AttributeSource.java:105-114`), so an
/// attribute any source of a token stream chain adds is seen by every other:
/// a consumer that adds an attribute to the outermost filter has it cleared by
/// the tokenizer's `clearAttributes()`. This structure is those shared maps;
/// `impls` keeps the insertion order of Java's `LinkedHashMap`, which is the
/// order `getCurrentState()` walks.
#[derive(Debug, Default)]
struct SharedAttributes {
    /// Attribute interface (or implementation) `TypeId` -> implementation.
    attributes: HashMap<TypeId, AttributeCell>,
    /// The keys of `attributes`, in insertion order.
    interface_order: Vec<TypeId>,
    /// Concrete implementation `TypeId` -> implementation, in insertion order.
    impls: Vec<(TypeId, AttributeCell)>,
}

impl SharedAttributes {
    fn impl_of(&self, impl_type: TypeId) -> Option<&AttributeCell> {
        self.impls
            .iter()
            .find(|(id, _)| *id == impl_type)
            .map(|(_, cell)| cell)
    }
}

/// Port of `org.apache.lucene.util.AttributeSource`.
///
/// Container that stores a set of attribute implementations and provides
/// lifecycle operations matching Lucene's `AttributeSource`.
///
/// Each attribute interface can appear at most once in a source. Adding an
/// attribute that is already present returns the existing instance.
///
/// A source built with [`AttributeSource::new_from`] (or cloned) shares its
/// attribute maps with its input, exactly as Java's sources share theirs: an
/// attribute either one adds is seen by the other's
/// [`clear_attributes`](Self::clear_attributes),
/// [`end_attributes`](Self::end_attributes), state capture and restoration,
/// copying, reflection, [`has_attribute`](Self::has_attribute) and
/// [`get_attribute_by_id`](Self::get_attribute_by_id), and is the instance
/// [`add_attribute`](Self::add_attribute) returns to the other.
///
/// **Divergence from Lucene 10.5.0.** [`get_attribute`](Self::get_attribute)
/// and [`get_attribute_mut`](Self::get_attribute_mut) return a guard borrowed
/// from the source itself, which a map shared behind an `AtomicRefCell` cannot lend
/// without `unsafe`; they read the source's own view of the shared map, which
/// is brought up to date by every `add_attribute*` call on that source. An
/// attribute another source of the chain added afterwards is therefore found
/// by [`get_attribute_by_id`](Self::get_attribute_by_id) and `has_attribute`,
/// but not by those two until this source adds any attribute.
#[derive(Clone, Debug)]
pub struct AttributeSource {
    /// This source's view of `shared.attributes`, for the borrowed getters.
    attributes: HashMap<TypeId, AttributeCell>,
    /// The maps shared with every source of the chain.
    shared: Arc<AtomicRefCell<SharedAttributes>>,
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
            shared: Arc::new(AtomicRefCell::new(SharedAttributes::default())),
            factory,
        }
    }

    /// Creates a source that shares its attribute maps with `input`.
    ///
    /// Changes made through either source are visible to the other, matching
    /// Lucene's `AttributeSource(AttributeSource input)` constructor.
    pub fn new_from(input: &Self) -> Self {
        Self {
            attributes: input.shared.borrow().attributes.clone(),
            shared: Arc::clone(&input.shared),
            factory: Arc::clone(&input.factory),
        }
    }

    /// Returns the factory used by this source.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeFactory`.
    pub fn factory(&self) -> &dyn AttributeFactory {
        self.factory.as_ref()
    }

    /// Brings this source's view up to date with the shared maps.
    fn sync_view(&mut self) {
        let shared = self.shared.borrow();
        if self.attributes.len() != shared.attributes.len() {
            for (id, cell) in &shared.attributes {
                self.attributes
                    .entry(*id)
                    .or_insert_with(|| Arc::clone(cell));
            }
        }
    }

    /// Equivalent to `AttributeSource.addAttributeImpl(AttributeImpl)`
    /// (`AttributeSource.java:188-202`): the implementation takes every one of
    /// its `Attribute` interfaces that no other implementation provides yet,
    /// and is registered only when it takes at least one — an implementation
    /// whose interfaces are all provided already is dropped.
    ///
    /// The port also lists an implementation's own type among its
    /// [`AttributeImpl::attribute_interfaces`], so that it can be looked up by
    /// it; that key is not an `Attribute` interface of Java's
    /// `getAttributeInterfaces`, so it never decides whether the
    /// implementation is registered.
    fn add_attribute_impl(&mut self, att: Box<dyn AttributeImpl>) {
        {
            let mut shared = self.shared.borrow_mut();
            let type_id = att.impl_type_id();
            if shared.impl_of(type_id).is_none() {
                let interfaces = att.attribute_interfaces().to_vec();
                let attribute_interfaces: Vec<TypeId> = interfaces
                    .iter()
                    .copied()
                    .filter(|interface_id| *interface_id != type_id)
                    .collect();
                let deciding = if attribute_interfaces.is_empty() {
                    &interfaces
                } else {
                    &attribute_interfaces
                };
                if !deciding
                    .iter()
                    .all(|interface_id| shared.attributes.contains_key(interface_id))
                {
                    let rc: AttributeCell = Arc::new(AtomicRefCell::new(att));
                    for interface_id in interfaces {
                        if let std::collections::hash_map::Entry::Vacant(slot) =
                            shared.attributes.entry(interface_id)
                        {
                            slot.insert(Arc::clone(&rc));
                            shared.interface_order.push(interface_id);
                        }
                    }
                    shared.impls.push((type_id, rc));
                }
            }
        }
        self.sync_view();
    }

    /// Adds an attribute of type `A`, creating it through the factory if
    /// necessary, and returns a borrowed reference to it.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the factory cannot create the
    /// attribute.
    pub fn add_attribute<A: Attribute>(&mut self) -> Result<AtomicRef<'_, A>, LuceneError> {
        let id = TypeId::of::<A>();
        self.sync_view();
        if !self.attributes.contains_key(&id) {
            let new_impl = self.factory.create_attribute_instance(id)?;
            self.add_attribute_impl(new_impl);
        }
        let rc = self.attributes.get(&id).unwrap();
        Ok(AtomicRef::map(rc.borrow(), |att| {
            att.as_any()
                .downcast_ref::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// Adds an attribute by its [`TypeId`] (an attribute interface, such as
    /// `TypeId::of::<dyn CharTermAttribute>()`, or an implementation) and
    /// returns the shared implementation handle.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#addAttribute(Class)`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the factory cannot create the
    /// attribute.
    pub fn add_attribute_by_id(
        &mut self,
        attribute_type: TypeId,
    ) -> Result<AttributeCell, LuceneError> {
        self.sync_view();
        if !self.attributes.contains_key(&attribute_type) {
            let new_impl = self.factory.create_attribute_instance(attribute_type)?;
            self.add_attribute_impl(new_impl);
        }
        Ok(Arc::clone(
            self.attributes.get(&attribute_type).ok_or_else(|| {
                LuceneError::illegal_argument(format!(
                    "the factory's implementation does not provide TypeId {:?}",
                    attribute_type
                ))
            })?,
        ))
    }

    /// Adds a pre-built [`AttributeImpl`] instance to this source.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#addAttributeImpl`.
    pub fn add_attribute_impl_instance(&mut self, att: Box<dyn AttributeImpl>) {
        self.add_attribute_impl(att);
    }

    /// Returns true if this source contains attribute `A`.
    pub fn has_attribute<A: Attribute>(&self) -> bool {
        self.has_attribute_by_id(TypeId::of::<A>())
    }

    /// Returns true if this source contains the attribute with the given
    /// [`TypeId`].
    pub fn has_attribute_by_id(&self, attribute_type: TypeId) -> bool {
        self.shared
            .borrow()
            .attributes
            .contains_key(&attribute_type)
    }

    /// Returns true if this source has any attributes.
    pub fn has_attributes(&self) -> bool {
        !self.shared.borrow().attributes.is_empty()
    }

    /// Returns the implementation of the attribute with the given [`TypeId`]
    /// (an attribute interface or an implementation), if present.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttribute(Class)`,
    /// which returns `null` for an absent attribute.
    pub fn get_attribute_by_id(&self, attribute_type: TypeId) -> Option<AttributeCell> {
        self.shared
            .borrow()
            .attributes
            .get(&attribute_type)
            .map(Arc::clone)
    }

    /// Returns a borrowed reference to attribute `A`, if present.
    pub fn get_attribute<A: Attribute>(&self) -> Option<AtomicRef<'_, A>> {
        let rc = self.attributes.get(&TypeId::of::<A>())?;
        Some(AtomicRef::map(rc.borrow(), |att| {
            att.as_any()
                .downcast_ref::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// Returns the instance of the attribute interface `I` this source holds,
    /// whatever implementation provides it, or `None`.
    ///
    /// Equivalent to `getAttribute(XAttribute.class)` (`AttributeSource.java:
    /// 323-325`), which returns the instance as the interface.
    ///
    /// # Panics
    ///
    /// When the implementation is borrowed mutably, or does not answer the
    /// cast to `I` it is registered for.
    pub fn get_attribute_interface<I: AttributeInterface + ?Sized>(
        &self,
    ) -> Option<AtomicRef<'_, I>> {
        let rc = self.attributes.get(&TypeId::of::<I>())?;
        Some(AtomicRef::map(rc.borrow(), |att| {
            I::cast(&**att).expect("INVARIANT: an implementation answers the casts it is keyed by")
        }))
    }

    /// Returns the instance of the attribute interface `I` mutably; see
    /// [`get_attribute_interface`](Self::get_attribute_interface).
    ///
    /// # Panics
    ///
    /// When the implementation is borrowed, or does not answer the cast to
    /// `I` it is registered for.
    pub fn get_attribute_interface_mut<I: AttributeInterface + ?Sized>(
        &self,
    ) -> Option<AtomicRefMut<'_, I>> {
        let rc = self.attributes.get(&TypeId::of::<I>())?;
        Some(AtomicRefMut::map(rc.borrow_mut(), |att| {
            I::cast_mut(&mut **att)
                .expect("INVARIANT: an implementation answers the casts it is keyed by")
        }))
    }

    /// Returns whether this source holds the attribute interface `I`.
    ///
    /// Equivalent to `hasAttribute(XAttribute.class)`.
    pub fn has_attribute_interface<I: AttributeInterface + ?Sized>(&self) -> bool {
        self.has_attribute_by_id(TypeId::of::<I>())
    }

    /// Adds the attribute interface `I`, creating its implementation with
    /// this source's factory when the source does not hold it yet.
    ///
    /// Equivalent to `addAttribute(XAttribute.class)` (`AttributeSource.java:
    /// 214-234`).
    ///
    /// # Errors
    ///
    /// What the factory raises for an interface it cannot implement.
    pub fn add_attribute_interface<I: AttributeInterface + ?Sized>(
        &mut self,
    ) -> Result<(), LuceneError> {
        self.add_attribute_by_id(TypeId::of::<I>()).map(|_| ())
    }

    /// Returns a mutable borrowed reference to attribute `A`, if present.
    pub fn get_attribute_mut<A: Attribute>(&self) -> Option<AtomicRefMut<'_, A>> {
        let rc = self.attributes.get(&TypeId::of::<A>())?;
        Some(AtomicRefMut::map(rc.borrow_mut(), |att| {
            att.as_any_mut()
                .downcast_mut::<A>()
                .expect("attribute type mismatch")
        }))
    }

    /// The implementations, in insertion order (Java's `getCurrentState()`
    /// walk).
    fn impl_cells(&self) -> Vec<AttributeCell> {
        self.shared
            .borrow()
            .impls
            .iter()
            .map(|(_, cell)| Arc::clone(cell))
            .collect()
    }

    /// Clears every attribute in this source.
    pub fn clear_attributes(&self) {
        for cell in self.impl_cells() {
            cell.borrow_mut().clear();
        }
    }

    /// Calls [`AttributeImpl::end`] on every attribute.
    pub fn end_attributes(&self) {
        for cell in self.impl_cells() {
            cell.borrow_mut().end();
        }
    }

    /// Removes every attribute and implementation from this source, and from
    /// every source sharing its maps.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#removeAllAttributes`.
    pub fn remove_all_attributes(&mut self) {
        {
            let mut shared = self.shared.borrow_mut();
            shared.attributes.clear();
            shared.interface_order.clear();
            shared.impls.clear();
        }
        self.attributes.clear();
    }

    /// Returns an iterator over the attribute interface [`TypeId`]s in
    /// insertion order.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeClassesIterator`.
    pub fn attribute_classes(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.shared.borrow().interface_order.clone().into_iter()
    }

    /// Returns an iterator over the unique attribute implementations in this
    /// source, in insertion order.
    ///
    /// This may yield fewer entries than [`Self::attribute_classes`], when one
    /// implementation covers more than one attribute interface.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#getAttributeImplsIterator`.
    pub fn attribute_impls_iter(&self) -> impl Iterator<Item = AttributeCell> + '_ {
        self.impl_cells().into_iter()
    }

    /// Captures the current state of all attributes as a deep snapshot.
    ///
    /// Returns `None` when this source has no attributes.
    pub fn capture_state(&self) -> Option<CapturedState> {
        let cells = self.impl_cells();
        if cells.is_empty() {
            return None;
        }
        let attributes = cells
            .iter()
            .map(|cell| {
                let att = cell.borrow();
                (att.impl_type_id(), att.clone_box())
            })
            .collect();
        Some(CapturedState { attributes })
    }

    /// Restores all attributes in this source from `state`.
    ///
    /// Attributes in `state` that are not present in this source cause an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if `state` contains an
    /// implementation not present in this source.
    pub fn restore_state(&self, state: &CapturedState) -> Result<(), LuceneError> {
        for (type_id, captured) in &state.attributes {
            let target_rc = self
                .shared
                .borrow()
                .impl_of(*type_id)
                .map(Arc::clone)
                .ok_or_else(|| {
                    LuceneError::illegal_argument(format!(
                        "State contains AttributeImpl of type {:?} that is not in in this AttributeSource",
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
    /// Returns `LuceneErrorKind::IllegalArgument` if `target` lacks a matching
    /// implementation.
    pub fn copy_to(&self, target: &AttributeSource) -> Result<(), LuceneError> {
        for cell in self.impl_cells() {
            let type_id = cell.borrow().impl_type_id();
            let target_rc = target
                .shared
                .borrow()
                .impl_of(type_id)
                .map(Arc::clone)
                .ok_or_else(|| {
                    LuceneError::illegal_argument(format!(
                        "This AttributeSource contains AttributeImpl of type {:?} that is not in the target",
                        type_id
                    ))
                })?;
            if Arc::ptr_eq(&cell, &target_rc) {
                continue;
            }
            cell.borrow().copy_to(&mut **target_rc.borrow_mut());
        }
        Ok(())
    }

    /// Returns a new source containing clones of all current attribute
    /// implementations.
    pub fn clone_attributes(&self) -> AttributeSource {
        let mut clone = AttributeSource::new_with_factory(Arc::clone(&self.factory));
        for cell in self.impl_cells() {
            let att = cell.borrow().clone_box();
            clone.add_attribute_impl(att);
        }
        clone
    }

    /// Reflects every attribute implementation through `reflector`, in
    /// insertion order.
    pub fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        for cell in self.impl_cells() {
            cell.borrow().reflect_with(reflector);
        }
    }

    /// Returns a hash code combining the hash codes of every attribute
    /// implementation this source holds.
    ///
    /// Equivalent to `org.apache.lucene.util.AttributeSource#hashCode`
    /// (`AttributeSource.java:335-341`): `code = code * 31 +
    /// attribute.hashCode()` over the implementations in insertion order.
    pub fn hash_code(&self) -> i32 {
        let mut code: i32 = 0;
        for cell in self.impl_cells() {
            code = code
                .wrapping_mul(31)
                .wrapping_add(cell.borrow().hash_code());
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

/// Equivalent to `org.apache.lucene.util.AttributeSource#equals`
/// (`AttributeSource.java:343-377`).
///
/// Two sources are equal when neither holds any attribute, or when both hold
/// the same number of implementations and, walking both in insertion order,
/// each pair is of the same concrete type and equal.
impl PartialEq for AttributeSource {
    fn eq(&self, other: &Self) -> bool {
        if !self.has_attributes() {
            return !other.has_attributes();
        }
        if !other.has_attributes() {
            return false;
        }
        let this_cells = self.impl_cells();
        let other_cells = other.impl_cells();
        if this_cells.len() != other_cells.len() {
            return false;
        }
        // it is only equal if all attribute impls are the same in the same order
        for (this_cell, other_cell) in this_cells.iter().zip(other_cells.iter()) {
            let this_att = this_cell.borrow();
            let that_att = other_cell.borrow();
            if that_att.impl_type_id() != this_att.impl_type_id() || !that_att.equals(&**this_att) {
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

    /// `addAttributeImpl` registers an implementation only for interfaces no
    /// other implementation provides (`AttributeSource.java:188-202`): a
    /// `TermFrequencyAttributeImpl` added after the packed implementation
    /// already provides `TermFrequencyAttribute` is dropped. Measured against
    /// `lucene-core-10.5.0.jar` (`oracle/fix/LastPass.java`, case C10):
    /// `impls=[PackedTokenAttributeImpl] termFreq=PackedTokenAttributeImpl`.
    #[test]
    fn an_implementation_whose_interfaces_are_all_provided_is_not_registered() {
        use crate::analysis::tokenattributes::{
            PackedTokenAttributeImpl, TermFrequencyAttributeImpl,
        };
        let mut source =
            AttributeSource::new_with_factory(crate::analysis::default_token_attribute_factory());
        source.add_attribute::<PackedTokenAttributeImpl>().unwrap();
        source.add_attribute_impl_instance(Box::new(TermFrequencyAttributeImpl::new()));
        assert!(!source.has_attribute::<TermFrequencyAttributeImpl>());
        assert_eq!(source.attribute_impls_iter().count(), 1);
    }

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

        assert!(Arc::ptr_eq(&first, &second));
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

    /// `AttributeSource(AttributeSource input)` shares the very maps of its
    /// input (`AttributeSource.java:105-114`): an attribute added through the
    /// derived source is cleared by the input's `clearAttributes()` and is the
    /// instance `addAttribute` returns on the input. Lucene's
    /// `BaseTokenStreamTestCase` relies on it: it adds
    /// `CheckClearAttributesAttribute` to the outermost filter and expects the
    /// tokenizer to clear it (oracle row `core 0 shared.clear` of
    /// `analysis_mocks.tsv`: the flags set on the filter read `0` after the
    /// tokenizer's `clearAttributes()`).
    #[test]
    fn an_attribute_added_through_a_derived_source_is_shared_with_the_input() {
        let mut input = AttributeSource::new_with_factory(test_factory());
        input.add_attribute::<TestAttribute>().unwrap();
        let mut derived = AttributeSource::new_from(&input);
        drop(derived.add_attribute::<OtherAttribute>().unwrap());
        derived.get_attribute_mut::<OtherAttribute>().unwrap().text = "x".to_string();

        assert!(input.has_attribute::<OtherAttribute>());
        input.clear_attributes();
        assert_eq!(derived.get_attribute::<OtherAttribute>().unwrap().text, "");

        let from_input = input
            .add_attribute_by_id(TypeId::of::<OtherAttribute>())
            .unwrap();
        let from_derived = derived
            .get_attribute_by_id(TypeId::of::<OtherAttribute>())
            .unwrap();
        assert!(Arc::ptr_eq(&from_input, &from_derived));
        assert_eq!(input.attribute_impls_iter().count(), 2);
    }

    /// `copyTo` onto a source sharing the same implementations copies each
    /// implementation onto itself, which changes nothing.
    #[test]
    fn copy_to_a_source_sharing_the_implementations_changes_nothing() {
        let mut source = AttributeSource::new_with_factory(test_factory());
        source.add_attribute::<TestAttribute>().unwrap();
        source.get_attribute_mut::<TestAttribute>().unwrap().value = 5;
        let shared = AttributeSource::new_from(&source);
        source.copy_to(&shared).unwrap();
        assert_eq!(shared.get_attribute::<TestAttribute>().unwrap().value, 5);
    }

    /// `equals` walks both sources in insertion order
    /// (`AttributeSource.java:343-377`): the same implementations added in a
    /// different order are not equal.
    #[test]
    fn equality_walks_the_implementations_in_insertion_order() {
        use crate::analysis::{FlagsAttributeImpl, KeywordAttributeImpl};
        let factory: Arc<dyn AttributeFactory> = {
            let mut f = DefaultAttributeFactory::new();
            f.register_self::<FlagsAttributeImpl>();
            f.register_self::<KeywordAttributeImpl>();
            Arc::new(f)
        };
        let source = |first_flags: bool| {
            let mut s = AttributeSource::new_with_factory(Arc::clone(&factory));
            if first_flags {
                drop(s.add_attribute::<FlagsAttributeImpl>().unwrap());
                drop(s.add_attribute::<KeywordAttributeImpl>().unwrap());
            } else {
                drop(s.add_attribute::<KeywordAttributeImpl>().unwrap());
                drop(s.add_attribute::<FlagsAttributeImpl>().unwrap());
            }
            s
        };
        assert!(source(true) == source(true));
        assert!(source(true) != source(false));
        assert_ne!(source(true).hash_code(), source(false).hash_code());
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
