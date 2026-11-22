/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `SpatialRelation.java`, of the Java package
//! `org.locationtech.spatial4j.shape`.

use std::fmt;

/// Equivalent to `org.locationtech.spatial4j.shape.SpatialRelation` (`public
/// enum`): the relation of a shape to another one, from the perspective of
/// the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpatialRelation {
    /// `WITHIN`: the shape is within the other one.
    Within,
    /// `CONTAINS`: the shape contains the other one.
    Contains,
    /// `DISJOINT`: the shapes share no point.
    Disjoint,
    /// `INTERSECTS`: the shapes share some points, without containment.
    Intersects,
}

impl SpatialRelation {
    /// `values()`, in declaration order.
    pub const VALUES: [SpatialRelation; 4] = [
        SpatialRelation::Within,
        SpatialRelation::Contains,
        SpatialRelation::Disjoint,
        SpatialRelation::Intersects,
    ];

    /// `transpose()`: the relation from the other shape's perspective.
    pub fn transpose(self) -> SpatialRelation {
        match self {
            SpatialRelation::Contains => SpatialRelation::Within,
            SpatialRelation::Within => SpatialRelation::Contains,
            _ => self,
        }
    }

    /// `combine(SpatialRelation)`; `None` stands for a `null` argument.
    pub fn combine(self, other: Option<SpatialRelation>) -> SpatialRelation {
        let Some(other) = other else {
            return self;
        };
        if other == self {
            return self;
        }
        if (self == SpatialRelation::Disjoint && other == SpatialRelation::Contains)
            || (self == SpatialRelation::Contains && other == SpatialRelation::Disjoint)
        {
            return SpatialRelation::Contains;
        }
        SpatialRelation::Intersects
    }

    /// `intersects()`: not `DISJOINT`.
    pub fn intersects(self) -> bool {
        self != SpatialRelation::Disjoint
    }

    /// `inverse()`: the relation to the complement of the other shape.
    pub fn inverse(self) -> SpatialRelation {
        match self {
            SpatialRelation::Disjoint => SpatialRelation::Contains,
            SpatialRelation::Contains => SpatialRelation::Disjoint,
            SpatialRelation::Within => SpatialRelation::Intersects, //not commutative!
            SpatialRelation::Intersects => SpatialRelation::Intersects,
        }
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            SpatialRelation::Within => "WITHIN",
            SpatialRelation::Contains => "CONTAINS",
            SpatialRelation::Disjoint => "DISJOINT",
            SpatialRelation::Intersects => "INTERSECTS",
        }
    }

    /// `ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            SpatialRelation::Within => 0,
            SpatialRelation::Contains => 1,
            SpatialRelation::Disjoint => 2,
            SpatialRelation::Intersects => 3,
        }
    }
}

impl fmt::Display for SpatialRelation {
    /// `toString()`: the constant name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
