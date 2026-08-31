//! Per-term boosts for multi-term rewrites, ported from
//! `org.apache.lucene.search.BoostAttribute` and
//! `org.apache.lucene.search.BoostAttributeImpl`.

#![deny(unsafe_code)]

use std::any::{Any, TypeId};
use std::sync::OnceLock;

use crate::util::attribute::{Attribute, AttributeImpl, AttributeReflector, AttributeSource};

/// The boost a term carries when no other value was set.
///
/// Equivalent to the `BoostAttribute.DEFAULT_BOOST` constant.
pub const DEFAULT_BOOST: f32 = 1.0;

/// Controls the boost factor of each matching term of a
/// [`MultiTermQuery`](crate::search::MultiTermQuery).
///
/// Equivalent to `org.apache.lucene.search.BoostAttribute`. Add it to a
/// [`TermsEnum`](crate::index::TermsEnum) returned by
/// [`MultiTermQuery::get_terms_enum`](crate::search::MultiTermQuery::get_terms_enum)
/// and update the boost on each returned term. This makes it possible to
/// control the boost factor of every matching term under
/// [`MultiTermQuery::scoring_boolean_rewrite`](crate::search::MultiTermQuery::scoring_boolean_rewrite)
/// or [`TopTermsRewrite`](crate::search::TopTermsRewrite);
/// [`FuzzyQuery`](crate::search::FuzzyQuery) uses it to take the edit distance
/// into account.
///
/// **Please note:** this attribute is intended to be added only by the terms
/// enum to itself, in its constructor, and consumed by the
/// [`RewriteMethod`](crate::search::RewriteMethod).
pub trait BoostAttribute: Attribute {
    /// Sets the boost in this attribute.
    ///
    /// Equivalent to `BoostAttribute.setBoost(float)`.
    fn set_boost(&mut self, boost: f32);

    /// Retrieves the boost; the default is [`DEFAULT_BOOST`].
    ///
    /// Equivalent to `BoostAttribute.getBoost()`.
    fn get_boost(&self) -> f32;
}

/// Implementation class for [`BoostAttribute`].
///
/// Equivalent to `org.apache.lucene.search.BoostAttributeImpl`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoostAttributeImpl {
    boost: f32,
}

impl Default for BoostAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl BoostAttributeImpl {
    /// Creates an attribute whose boost is [`DEFAULT_BOOST`].
    ///
    /// Equivalent to `new BoostAttributeImpl()`.
    pub fn new() -> Self {
        Self {
            boost: DEFAULT_BOOST,
        }
    }
}

impl Attribute for BoostAttributeImpl {}

impl BoostAttribute for BoostAttributeImpl {
    fn set_boost(&mut self, boost: f32) {
        self.boost = boost;
    }

    fn get_boost(&self) -> f32 {
        self.boost
    }
}

impl AttributeImpl for BoostAttributeImpl {
    fn clear(&mut self) {
        self.boost = DEFAULT_BOOST;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target.as_any_mut().downcast_mut::<BoostAttributeImpl>() {
            t.set_boost(self.boost);
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn BoostAttribute>(),
            std::any::type_name::<BoostAttributeImpl>(),
            "boost",
            &self.boost,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(*self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: OnceLock<&'static [TypeId]> = OnceLock::new();
        IDS.get_or_init(|| {
            let ids = vec![
                TypeId::of::<BoostAttributeImpl>(),
                TypeId::of::<dyn BoostAttribute>(),
            ];
            Box::leak(ids.into_boxed_slice())
        })
    }
}

/// Installs a [`BoostAttributeImpl`] in `atts` unless one is already present.
///
/// Equivalent to `attributes().addAttribute(BoostAttribute.class)`.
///
/// **Divergence from Lucene 10.5.0.** Java's `DefaultAttributeFactory` resolves
/// the `*Impl` class reflectively from the interface name; this port's factory
/// is an explicit registry, and an
/// [`AttributeSource`](crate::util::attribute::AttributeSource) built with
/// [`AttributeSource::new`](crate::util::attribute::AttributeSource::new) has
/// an empty one. The implementation is therefore installed directly, which is
/// what the reflective lookup would have produced.
pub fn add_boost_attribute(atts: &mut AttributeSource) {
    if !atts.has_attribute::<BoostAttributeImpl>() {
        atts.add_attribute_impl_instance(Box::new(BoostAttributeImpl::new()));
    }
}

/// Reads the boost of the attribute installed in `atts`, or [`DEFAULT_BOOST`]
/// when there is none.
///
/// Equivalent to `boostAtt.getBoost()` after
/// `attributes().addAttribute(BoostAttribute.class)`; an absent attribute would
/// have been created with the default boost, so the two agree.
pub fn boost_of(atts: &AttributeSource) -> f32 {
    atts.get_attribute::<BoostAttributeImpl>()
        .map_or(DEFAULT_BOOST, |att| att.get_boost())
}

/// Sets the boost of the attribute installed in `atts`, installing one first if
/// necessary.
///
/// Equivalent to `boostAtt.setBoost(float)`.
pub fn set_boost(atts: &mut AttributeSource, boost: f32) {
    add_boost_attribute(atts);
    if let Some(mut att) = atts.get_attribute_mut::<BoostAttributeImpl>() {
        att.set_boost(boost);
    }
}
