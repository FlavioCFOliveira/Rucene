/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `BaseShape.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use std::sync::Arc;

use crate::context::spatial_context::{SpatialContext, SpatialContextRef};

/// Equivalent to `org.locationtech.spatial4j.shape.BaseShape` (`public
/// abstract class BaseShape<T extends SpatialContext> implements Shape`): the
/// context holder the shape implementations embed. Its `getContext()` is
/// [`get_context`](Self::get_context).
#[derive(Clone, Debug)]
pub struct BaseShape {
    /// `ctx`; `None` stands for a `null` context.
    pub(crate) ctx: Option<SpatialContextRef>,
}

impl BaseShape {
    /// `BaseShape(T ctx)`.
    pub fn new(ctx: Option<SpatialContextRef>) -> Self {
        Self { ctx }
    }

    /// `getContext()`.
    pub fn get_context(&self) -> Option<Arc<SpatialContext>> {
        self.ctx.as_ref().map(SpatialContextRef::get)
    }

    /// The context, which the caller requires to be non-null (Java would
    /// throw `NullPointerException` dereferencing it).
    pub fn ctx(&self) -> Arc<SpatialContext> {
        self.get_context().expect("java.lang.NullPointerException: the shape's context is null")
    }
}
