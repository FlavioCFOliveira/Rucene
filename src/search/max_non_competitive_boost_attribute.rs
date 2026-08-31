//! The competitive-boost cut-off shared by the segment enums of a multi-term
//! rewrite, ported from
//! `org.apache.lucene.search.MaxNonCompetitiveBoostAttribute` and
//! `org.apache.lucene.search.MaxNonCompetitiveBoostAttributeImpl`.

#![deny(unsafe_code)]

use std::any::{Any, TypeId};
use std::sync::OnceLock;

use crate::util::attribute::{Attribute, AttributeImpl, AttributeReflector, AttributeSource};
use crate::util::BytesRef;

/// Lets a [`RewriteMethod`](crate::search::RewriteMethod) tell every segment
/// enum which boosts are no longer competitive.
///
/// Equivalent to `org.apache.lucene.search.MaxNonCompetitiveBoostAttribute`.
/// Add it to a fresh [`AttributeSource`](crate::util::attribute::AttributeSource)
/// before calling
/// [`MultiTermQuery::get_terms_enum`](crate::search::MultiTermQuery::get_terms_enum);
/// [`FuzzyQuery`](crate::search::FuzzyQuery) uses it to control its internal
/// behaviour so that only competitive terms are returned.
///
/// **Please note:** this attribute is intended to be added by the
/// [`RewriteMethod`](crate::search::RewriteMethod) to an empty attribute source
/// that is shared by all segments during query rewrite. That attribute source
/// is passed to every segment enum by
/// [`MultiTermQuery::get_terms_enum`](crate::search::MultiTermQuery::get_terms_enum).
/// [`TopTermsRewrite`](crate::search::TopTermsRewrite) uses this attribute to
/// inform all enums about the current boost that is not competitive.
pub trait MaxNonCompetitiveBoostAttribute: Attribute {
    /// Sets the maximum boost that would not be competitive.
    ///
    /// Equivalent to
    /// `MaxNonCompetitiveBoostAttribute.setMaxNonCompetitiveBoost(float)`.
    fn set_max_non_competitive_boost(&mut self, max_non_competitive_boost: f32);

    /// Returns the maximum boost that would not be competitive. The default is
    /// [`f32::NEG_INFINITY`], which means every term is competitive.
    ///
    /// Equivalent to
    /// `MaxNonCompetitiveBoostAttribute.getMaxNonCompetitiveBoost()`.
    fn get_max_non_competitive_boost(&self) -> f32;

    /// Sets the term that triggered the boost change, or `None`.
    ///
    /// Equivalent to
    /// `MaxNonCompetitiveBoostAttribute.setCompetitiveTerm(BytesRef)`.
    fn set_competitive_term(&mut self, competitive_term: Option<BytesRef>);

    /// Returns the term that triggered the boost change, or `None`. The default
    /// is `None`, which means every term is competitive.
    ///
    /// Equivalent to `MaxNonCompetitiveBoostAttribute.getCompetitiveTerm()`.
    fn get_competitive_term(&self) -> Option<&BytesRef>;
}

/// Implementation class for [`MaxNonCompetitiveBoostAttribute`].
///
/// Equivalent to
/// `org.apache.lucene.search.MaxNonCompetitiveBoostAttributeImpl`.
#[derive(Clone, Debug, PartialEq)]
pub struct MaxNonCompetitiveBoostAttributeImpl {
    max_non_competitive_boost: f32,
    competitive_term: Option<BytesRef>,
}

impl Default for MaxNonCompetitiveBoostAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl MaxNonCompetitiveBoostAttributeImpl {
    /// Creates an attribute whose boost is [`f32::NEG_INFINITY`] and whose
    /// competitive term is `None`.
    ///
    /// Equivalent to `new MaxNonCompetitiveBoostAttributeImpl()`.
    pub fn new() -> Self {
        Self {
            max_non_competitive_boost: f32::NEG_INFINITY,
            competitive_term: None,
        }
    }
}

impl Attribute for MaxNonCompetitiveBoostAttributeImpl {}

impl MaxNonCompetitiveBoostAttribute for MaxNonCompetitiveBoostAttributeImpl {
    fn set_max_non_competitive_boost(&mut self, max_non_competitive_boost: f32) {
        self.max_non_competitive_boost = max_non_competitive_boost;
    }

    fn get_max_non_competitive_boost(&self) -> f32 {
        self.max_non_competitive_boost
    }

    fn set_competitive_term(&mut self, competitive_term: Option<BytesRef>) {
        self.competitive_term = competitive_term;
    }

    fn get_competitive_term(&self) -> Option<&BytesRef> {
        self.competitive_term.as_ref()
    }
}

impl AttributeImpl for MaxNonCompetitiveBoostAttributeImpl {
    fn clear(&mut self) {
        self.max_non_competitive_boost = f32::NEG_INFINITY;
        self.competitive_term = None;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        if let Some(t) = target
            .as_any_mut()
            .downcast_mut::<MaxNonCompetitiveBoostAttributeImpl>()
        {
            t.set_max_non_competitive_boost(self.max_non_competitive_boost);
            t.set_competitive_term(self.competitive_term.clone());
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn MaxNonCompetitiveBoostAttribute>(),
            std::any::type_name::<MaxNonCompetitiveBoostAttributeImpl>(),
            "maxNonCompetitiveBoost",
            &self.max_non_competitive_boost,
        );
        reflector.reflect(
            TypeId::of::<dyn MaxNonCompetitiveBoostAttribute>(),
            std::any::type_name::<MaxNonCompetitiveBoostAttributeImpl>(),
            "competitiveTerm",
            &self.competitive_term,
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
        static IDS: OnceLock<&'static [TypeId]> = OnceLock::new();
        IDS.get_or_init(|| {
            let ids = vec![
                TypeId::of::<MaxNonCompetitiveBoostAttributeImpl>(),
                TypeId::of::<dyn MaxNonCompetitiveBoostAttribute>(),
            ];
            Box::leak(ids.into_boxed_slice())
        })
    }
}

/// Installs a [`MaxNonCompetitiveBoostAttributeImpl`] in `atts` unless one is
/// already present.
///
/// Equivalent to
/// `attributes.addAttribute(MaxNonCompetitiveBoostAttribute.class)`; see
/// [`crate::search::boost_attribute::add_boost_attribute`] for why the
/// implementation is installed directly.
pub fn add_max_non_competitive_boost_attribute(atts: &mut AttributeSource) {
    if !atts.has_attribute::<MaxNonCompetitiveBoostAttributeImpl>() {
        atts.add_attribute_impl_instance(Box::new(MaxNonCompetitiveBoostAttributeImpl::new()));
    }
}

/// Records the maximum non-competitive boost and the term that triggered the
/// change, installing the attribute first if necessary.
///
/// Equivalent to the pair of calls
/// `maxBoostAtt.setMaxNonCompetitiveBoost(float)` and
/// `maxBoostAtt.setCompetitiveTerm(BytesRef)` that
/// [`TopTermsRewrite`](crate::search::TopTermsRewrite) makes together.
pub fn set_max_non_competitive_boost(
    atts: &mut AttributeSource,
    max_non_competitive_boost: f32,
    competitive_term: Option<BytesRef>,
) {
    add_max_non_competitive_boost_attribute(atts);
    if let Some(mut att) = atts.get_attribute_mut::<MaxNonCompetitiveBoostAttributeImpl>() {
        att.set_max_non_competitive_boost(max_non_competitive_boost);
        att.set_competitive_term(competitive_term);
    }
}

/// Reads the maximum non-competitive boost recorded in `atts`, or
/// [`f32::NEG_INFINITY`] when the attribute is absent.
///
/// Equivalent to `maxBoostAtt.getMaxNonCompetitiveBoost()`.
pub fn max_non_competitive_boost_of(atts: &AttributeSource) -> f32 {
    atts.get_attribute::<MaxNonCompetitiveBoostAttributeImpl>()
        .map_or(f32::NEG_INFINITY, |att| att.get_max_non_competitive_boost())
}

/// Reads the competitive term recorded in `atts`, or `None` when the attribute
/// is absent.
///
/// Equivalent to `maxBoostAtt.getCompetitiveTerm()`.
pub fn competitive_term_of(atts: &AttributeSource) -> Option<BytesRef> {
    atts.get_attribute::<MaxNonCompetitiveBoostAttributeImpl>()
        .and_then(|att| att.get_competitive_term().cloned())
}
