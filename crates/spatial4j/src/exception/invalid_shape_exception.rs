/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `InvalidShapeException.java`, of the Java package
//! `org.locationtech.spatial4j.exception`.

use rucene::error::{LuceneError, LuceneErrorKind};

/// Equivalent to `org.locationtech.spatial4j.exception.InvalidShapeException`
/// (`public class InvalidShapeException extends RuntimeException`): a shape
/// whose definition is invalid.
///
/// The exception travels as a [`LuceneError`] of the unchecked kind carrying
/// this class name; the associated functions build it and recognise it.
pub struct InvalidShapeException;

impl InvalidShapeException {
    /// The Java binary class name.
    pub const CLASS: &'static str = "org.locationtech.spatial4j.exception.InvalidShapeException";

    /// `new InvalidShapeException(String reason)`; `None` stands for a
    /// `null` reason.
    pub fn new(reason: Option<String>) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Unchecked {
            class: Self::CLASS,
            message: reason,
            cause: None,
        })
    }

    /// `new InvalidShapeException(String reason)` with a non-null reason.
    pub fn error(reason: impl Into<String>) -> LuceneError {
        Self::new(Some(reason.into()))
    }

    /// `new InvalidShapeException(String reason, Throwable cause)`.
    pub fn with_cause(reason: Option<String>, cause: LuceneError) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Unchecked {
            class: Self::CLASS,
            message: reason,
            cause: Some(std::sync::Arc::new(cause)),
        })
    }

    /// `e instanceof InvalidShapeException`.
    pub fn is(error: &LuceneError) -> bool {
        matches!(error.kind(), LuceneErrorKind::Unchecked { class, .. } if *class == Self::CLASS)
    }
}
