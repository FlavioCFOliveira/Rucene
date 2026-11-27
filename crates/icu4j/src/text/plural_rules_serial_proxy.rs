// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2008, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.text.PluralRulesSerialProxy`.

use std::sync::Arc;

use crate::text::plural_rules::PluralRules;

/// Serialization proxy for PluralRules: the rules text, from which the
/// rules are recreated on deserialization.
///
/// Equivalent to `com.ibm.icu.text.PluralRulesSerialProxy`
/// (package-private, `serialVersionUID` 42).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluralRulesSerialProxy {
    data: String,
}

impl PluralRulesSerialProxy {
    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = 42;

    /// Equivalent to `PluralRulesSerialProxy(String)`.
    pub(crate) fn new(rules: String) -> Self {
        Self { data: rules }
    }

    /// The serialized rules text (the `data` field).
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Equivalent to `readResolve()`: `PluralRules.createRules(data)`.
    pub fn read_resolve(&self) -> Option<Arc<PluralRules>> {
        PluralRules::create_rules(&self.data)
    }
}
