/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! Skeletons of the types declared in `SpatialRelation.java`, of the Java
//! package `org.locationtech.spatial4j.shape`.

/// Java: `public enum`.
///
/// Skeleton of `org.locationtech.spatial4j.shape.SpatialRelation`; ported by rmp task #621.
pub enum SpatialRelation {
    /// Java constant `WITHIN`.
    Within,
    /// Java constant `CONTAINS`.
    Contains,
    /// Java constant `DISJOINT`.
    Disjoint,
    /// Java constant `INTERSECTS`.
    Intersects,
}
