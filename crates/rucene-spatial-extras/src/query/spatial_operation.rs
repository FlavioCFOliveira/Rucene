/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `SpatialOperation.java`, of the Java package
//! `org.apache.lucene.spatial.query`.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use rucene::error::{LuceneError, Result};
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

/// The eight operations, in Java's declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Op {
    BBoxIntersects,
    BBoxWithin,
    Contains,
    Intersects,
    IsEqualTo,
    IsDisjointTo,
    IsWithin,
    Overlaps,
}

/// Equivalent to `org.apache.lucene.spatial.query.SpatialOperation` (`public
/// abstract class SpatialOperation implements Serializable`): a predicate
/// that compares a stored geometry to a supplied geometry. It's enum-like.
/// For more explanation of each predicate, consider looking at the source
/// implementation of [`evaluate`](Self::evaluate). It's important to be
/// aware that Lucene-spatial makes no distinction of shape boundaries, unlike
/// many standardized definitions. Nor does it make dimensional distinctions
/// (e.g. line vs polygon).
///
/// Java's eight anonymous subclasses are the eight constants; Java compares
/// them by identity, which is equality here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpatialOperation {
    op: Op,
}

#[allow(non_upper_case_globals)]
impl SpatialOperation {
    /// `BBoxIntersects`: bounding box of the indexed shape, then
    /// `Intersects`.
    pub const BBoxIntersects: SpatialOperation = SpatialOperation {
        op: Op::BBoxIntersects,
    };
    /// `BBoxWithin`: bounding box of the indexed shape, then `IsWithin`
    /// (alias `BBoxCoveredBy`).
    pub const BBoxWithin: SpatialOperation = SpatialOperation { op: Op::BBoxWithin };
    /// `Contains`: meets the "Covers" OGC definition (boundary-neutral;
    /// alias `Covers`).
    pub const Contains: SpatialOperation = SpatialOperation { op: Op::Contains };
    /// `Intersects`: meets the "Intersects" OGC definition.
    pub const Intersects: SpatialOperation = SpatialOperation { op: Op::Intersects };
    /// `IsEqualTo` (name `Equals`, alias `IsEqualTo`): meets the "Equals"
    /// OGC definition.
    pub const IsEqualTo: SpatialOperation = SpatialOperation { op: Op::IsEqualTo };
    /// `IsDisjointTo` (name `Disjoint`, alias `IsDisjointTo`): meets the
    /// "Disjoint" OGC definition.
    pub const IsDisjointTo: SpatialOperation = SpatialOperation {
        op: Op::IsDisjointTo,
    };
    /// `IsWithin` (name `Within`, aliases `IsWithin` and `CoveredBy`): meets
    /// the "CoveredBy" OGC definition (boundary-neutral).
    pub const IsWithin: SpatialOperation = SpatialOperation { op: Op::IsWithin };
    /// `Overlaps`: almost meets the "Overlaps" OGC definition, but
    /// boundary-neutral (boundary==interior).
    pub const Overlaps: SpatialOperation = SpatialOperation { op: Op::Overlaps };

    /// `name`, `getName()`.
    pub fn get_name(&self) -> &'static str {
        match self.op {
            Op::BBoxIntersects => "BBoxIntersects",
            Op::BBoxWithin => "BBoxWithin",
            Op::Contains => "Contains",
            Op::Intersects => "Intersects",
            Op::IsEqualTo => "Equals",
            Op::IsDisjointTo => "Disjoint",
            Op::IsWithin => "Within",
            Op::Overlaps => "Overlaps",
        }
    }

    /// The aliases each constant's initializer `register`s.
    fn aliases(&self) -> &'static [&'static str] {
        match self.op {
            Op::BBoxWithin => &["BBoxCoveredBy"], // alias -- the better name
            Op::Contains => &["Covers"],          // alias -- the better name
            Op::IsEqualTo => &["IsEqualTo"],      // alias (deprecated)
            Op::IsDisjointTo => &["IsDisjointTo"], // alias (deprecated)
            Op::IsWithin => &["IsWithin", "CoveredBy"], // alias (deprecated), and the more appropriate name.
            _ => &[],
        }
    }

    /// `values()`: the operations in declaration order.
    pub fn values() -> Vec<SpatialOperation> {
        vec![
            Self::BBoxIntersects,
            Self::BBoxWithin,
            Self::Contains,
            Self::Intersects,
            Self::IsEqualTo,
            Self::IsDisjointTo,
            Self::IsWithin,
            Self::Overlaps,
        ]
    }

    /// `get(String v)`: the operation of the name or alias `v`, matched as
    /// given and then upper-cased.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unknown Operation: ...")`.
    pub fn get(v: &str) -> Result<SpatialOperation> {
        let op = REGISTRY.get(v).or_else(|| REGISTRY.get(&v.to_uppercase()));
        op.copied()
            .ok_or_else(|| LuceneError::illegal_argument(format!("Unknown Operation: {v}")))
    }

    /// `is(SpatialOperation op, SpatialOperation... tst)`.
    pub fn is(op: SpatialOperation, tst: &[SpatialOperation]) -> bool {
        tst.contains(&op)
    }

    /// `evaluate(Shape indexedShape, Shape queryShape)`: returns whether
    /// the relationship between `indexed_shape` and `query_shape` is
    /// satisfied by this operation.
    pub fn evaluate(&self, indexed_shape: &dyn Shape, query_shape: &dyn Shape) -> bool {
        match self.op {
            Op::BBoxIntersects => indexed_shape
                .get_bounding_box()
                .relate(query_shape)
                .intersects(),
            Op::BBoxWithin => {
                let bbox = indexed_shape.get_bounding_box();
                bbox.relate(query_shape) == SpatialRelation::Within || bbox.equals(query_shape)
            }
            Op::Contains => {
                indexed_shape.relate(query_shape) == SpatialRelation::Contains
                    || indexed_shape.equals(query_shape)
            }
            Op::Intersects => indexed_shape.relate(query_shape).intersects(),
            Op::IsEqualTo => indexed_shape.equals(query_shape),
            Op::IsDisjointTo => !indexed_shape.relate(query_shape).intersects(),
            Op::IsWithin => {
                indexed_shape.relate(query_shape) == SpatialRelation::Within
                    || indexed_shape.equals(query_shape)
            }
            // not Contains or Within or Disjoint
            Op::Overlaps => indexed_shape.relate(query_shape) == SpatialRelation::Intersects,
        }
    }

    /// `hashCode()`: the identity hash of the constant (the core's
    /// stand-in over the name of its anonymous class, `SpatialOperation$1`
    /// to `$8`).
    pub fn hash_code(&self) -> i32 {
        let n = Self::values().iter().position(|o| o == self).unwrap_or(0) + 1;
        crate::jdk::identity_hash(&format!(
            "org.apache.lucene.spatial.query.SpatialOperation${n}"
        ))
    }
}

/// `registry`: has aliases (each name as given and upper-cased).
static REGISTRY: LazyLock<HashMap<String, SpatialOperation>> = LazyLock::new(|| {
    let mut registry = HashMap::new();
    for op in SpatialOperation::values() {
        // the constructor registers the name, then the initializer the aliases
        for name in std::iter::once(op.get_name()).chain(op.aliases().iter().copied()) {
            registry.insert(name.to_string(), op);
            registry.insert(name.to_uppercase(), op);
        }
    }
    registry
});

impl fmt::Display for SpatialOperation {
    /// `toString()`: the name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.get_name())
    }
}
