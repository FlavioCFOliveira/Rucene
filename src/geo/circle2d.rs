//! The 2D circle component ported from `org.apache.lucene.geo.Circle2D`.

use crate::error::Result;
use crate::geo::component2d::{
    contains_point, disjoint, point_in_triangle, within, Component2D, WithinRelation,
};
use crate::geo::encoding::GeoUtils;
use crate::geo::geometry::{Circle, Rectangle, XYCircle, XYRectangle};
use crate::index::point_values::Relation;
use crate::util::SloppyMath;
use std::fmt;
use std::sync::Arc;

/// Distance model a [`Circle2D`] delegates to.
///
/// Equivalent to the private interface `Circle2D.DistanceCalculator`.
trait DistanceCalculator: fmt::Debug + Send + Sync {
    /// Returns whether the point is within the distance.
    fn contains(&self, x: f64, y: f64) -> bool;

    /// Returns whether the line is within the distance.
    fn intersects_line(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool;

    /// Relates this calculator to the provided bounding box.
    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation;

    /// Returns whether the bounding box is disjoint with this calculator's
    /// bounding box.
    fn disjoint(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool;

    /// Returns whether the bounding box contains this calculator's bounding
    /// box.
    fn within(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool;

    /// Returns the minimum X of this calculator.
    fn get_min_x(&self) -> f64;

    /// Returns the maximum X of this calculator.
    fn get_max_x(&self) -> f64;

    /// Returns the minimum Y of this calculator.
    fn get_min_y(&self) -> f64;

    /// Returns the maximum Y of this calculator.
    fn get_max_y(&self) -> f64;

    /// Returns the centre X.
    fn get_x(&self) -> f64;

    /// Returns the centre Y.
    fn get_y(&self) -> f64;
}

/// 2D circle implementation containing spatial logic.
///
/// Equivalent to `org.apache.lucene.geo.Circle2D`, which is package-private in
/// Lucene; Rust has no package-private visibility, so the type is `pub` and is
/// marked internal by this documentation instead.
#[derive(Debug)]
pub struct Circle2D {
    calculator: Box<dyn DistanceCalculator>,
}

/// Returns whether the segment `a`-`b` passes within the calculator's distance.
///
/// Equivalent to the private static `Circle2D.intersectsLine(...)`. Algorithm
/// based on <https://stackoverflow.com/questions/3120357/get-closest-point-to-a-line>.
#[allow(clippy::too_many_arguments)]
fn intersects_line(
    center_x: f64,
    center_y: f64,
    a_x: f64,
    a_y: f64,
    b_x: f64,
    b_y: f64,
    calculator: &dyn DistanceCalculator,
) -> bool {
    let vector_apx = center_x - a_x;
    let vector_apy = center_y - a_y;

    let vector_abx = b_x - a_x;
    let vector_aby = b_y - a_y;

    let magnitude_ab = vector_abx * vector_abx + vector_aby * vector_aby;
    let dot_product = vector_apx * vector_abx + vector_apy * vector_aby;

    let distance = dot_product / magnitude_ab;

    if distance < 0.0 || distance > 1.0 {
        return false;
    }

    let p_x = a_x + vector_abx * distance;
    let p_y = a_y + vector_aby * distance;

    let min_x = a_x.min(b_x);
    let min_y = a_y.min(b_y);
    let max_x = a_x.max(b_x);
    let max_y = a_y.max(b_y);

    if p_x >= min_x && p_x <= max_x && p_y >= min_y && p_y <= max_y {
        return calculator.contains(p_x, p_y);
    }
    false
}

/// Cartesian distance model.
///
/// Equivalent to the private static class `Circle2D.CartesianDistance`.
#[derive(Debug)]
struct CartesianDistance {
    center_x: f64,
    center_y: f64,
    radius_squared: f64,
    rectangle: XYRectangle,
}

impl CartesianDistance {
    fn new(center_x: f32, center_y: f32, radius: f32) -> Result<Self> {
        Ok(Self {
            center_x: f64::from(center_x),
            center_y: f64::from(center_y),
            rectangle: XYRectangle::from_point_distance(center_x, center_y, radius)?,
            // product performed with doubles
            radius_squared: f64::from(radius) * f64::from(radius),
        })
    }
}

impl DistanceCalculator for CartesianDistance {
    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        if contains_point(self.center_x, self.center_y, min_x, max_x, min_y, max_y) {
            if self.contains(min_x, min_y)
                && self.contains(max_x, min_y)
                && self.contains(max_x, max_y)
                && self.contains(min_x, max_y)
            {
                // we are fully enclosed, collect everything within this subtree
                return Relation::CellInsideQuery;
            }
        } else {
            // circle not fully inside, compute closest distance
            let mut sum_of_squared_diffs = 0.0f64;
            if self.center_x < min_x {
                let diff = min_x - self.center_x;
                sum_of_squared_diffs += diff * diff;
            } else if self.center_x > max_x {
                let diff = max_x - self.center_x;
                sum_of_squared_diffs += diff * diff;
            }
            if self.center_y < min_y {
                let diff = min_y - self.center_y;
                sum_of_squared_diffs += diff * diff;
            } else if self.center_y > max_y {
                let diff = max_y - self.center_y;
                sum_of_squared_diffs += diff * diff;
            }
            if sum_of_squared_diffs > self.radius_squared {
                // disjoint
                return Relation::CellOutsideQuery;
            }
        }
        Relation::CellCrossesQuery
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        if contains_point(
            x,
            y,
            f64::from(self.rectangle.min_x()),
            f64::from(self.rectangle.max_x()),
            f64::from(self.rectangle.min_y()),
            f64::from(self.rectangle.max_y()),
        ) {
            let diff_x = x - self.center_x;
            let diff_y = y - self.center_y;
            return diff_x * diff_x + diff_y * diff_y <= self.radius_squared;
        }
        false
    }

    fn intersects_line(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool {
        intersects_line(self.center_x, self.center_y, a_x, a_y, b_x, b_y, self)
    }

    fn disjoint(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
        disjoint(
            f64::from(self.rectangle.min_x()),
            f64::from(self.rectangle.max_x()),
            f64::from(self.rectangle.min_y()),
            f64::from(self.rectangle.max_y()),
            min_x,
            max_x,
            min_y,
            max_y,
        )
    }

    fn within(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
        within(
            f64::from(self.rectangle.min_x()),
            f64::from(self.rectangle.max_x()),
            f64::from(self.rectangle.min_y()),
            f64::from(self.rectangle.max_y()),
            min_x,
            max_x,
            min_y,
            max_y,
        )
    }

    fn get_min_x(&self) -> f64 {
        f64::from(self.rectangle.min_x())
    }

    fn get_max_x(&self) -> f64 {
        f64::from(self.rectangle.max_x())
    }

    fn get_min_y(&self) -> f64 {
        f64::from(self.rectangle.min_y())
    }

    fn get_max_y(&self) -> f64 {
        f64::from(self.rectangle.max_y())
    }

    fn get_x(&self) -> f64 {
        self.center_x
    }

    fn get_y(&self) -> f64 {
        self.center_y
    }
}

/// Haversine distance model.
///
/// Equivalent to the private static class `Circle2D.HaversinDistance`.
#[derive(Debug)]
struct HaversinDistance {
    center_lat: f64,
    center_lon: f64,
    sort_key: f64,
    axis_lat: f64,
    rectangle: Rectangle,
    crosses_dateline: bool,
}

impl HaversinDistance {
    fn new(center_lon: f64, center_lat: f64, radius: f64) -> Result<Self> {
        let rectangle = Rectangle::from_point_distance(center_lat, center_lon, radius)?;
        Ok(Self {
            center_lat,
            center_lon,
            sort_key: GeoUtils::distance_query_sort_key(radius),
            axis_lat: Rectangle::axis_lat(center_lat, radius),
            crosses_dateline: rectangle.min_lon() > rectangle.max_lon(),
            rectangle,
        })
    }
}

impl DistanceCalculator for HaversinDistance {
    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        // Java propagates the `IllegalArgumentException` that `GeoUtils.relate`
        // raises for a dateline-crossing box; `Component2D.relate` has no
        // `Result`, and `Circle2D.relate` only reaches here after the
        // calculator's own `disjoint`/`within` checks, so the box is never a
        // dateline-crossing one. Treat the impossible case the way Java's
        // unchecked exception would: as a fatal programming error.
        GeoUtils::relate(
            min_y,
            max_y,
            min_x,
            max_x,
            self.center_lat,
            self.center_lon,
            self.sort_key,
            self.axis_lat,
        )
        .expect("INVARIANT: Component2D never relates against a dateline-crossing box")
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        if self.crosses_dateline {
            if contains_point(
                x,
                y,
                self.rectangle.min_lon(),
                GeoUtils::MAX_LON_INCL,
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
            ) || contains_point(
                x,
                y,
                GeoUtils::MIN_LON_INCL,
                self.rectangle.max_lon(),
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
            ) {
                return SloppyMath::haversin_sort_key(y, x, self.center_lat, self.center_lon)
                    <= self.sort_key;
            }
        } else if contains_point(
            x,
            y,
            self.rectangle.min_lon(),
            self.rectangle.max_lon(),
            self.rectangle.min_lat(),
            self.rectangle.max_lat(),
        ) {
            return SloppyMath::haversin_sort_key(y, x, self.center_lat, self.center_lon)
                <= self.sort_key;
        }
        false
    }

    fn intersects_line(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool {
        if intersects_line(self.center_lon, self.center_lat, a_x, a_y, b_x, b_y, self) {
            return true;
        }
        if self.crosses_dateline {
            let new_center_lon = if self.center_lon > 0.0 {
                self.center_lon - 360.0
            } else {
                self.center_lon + 360.0
            };
            return intersects_line(new_center_lon, self.center_lat, a_x, a_y, b_x, b_y, self);
        }
        false
    }

    fn disjoint(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
        if self.crosses_dateline {
            disjoint(
                self.rectangle.min_lon(),
                GeoUtils::MAX_LON_INCL,
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            ) && disjoint(
                GeoUtils::MIN_LON_INCL,
                self.rectangle.max_lon(),
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            )
        } else {
            disjoint(
                self.rectangle.min_lon(),
                self.rectangle.max_lon(),
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            )
        }
    }

    fn within(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
        if self.crosses_dateline {
            within(
                self.rectangle.min_lon(),
                GeoUtils::MAX_LON_INCL,
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            ) || within(
                GeoUtils::MIN_LON_INCL,
                self.rectangle.max_lon(),
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            )
        } else {
            within(
                self.rectangle.min_lon(),
                self.rectangle.max_lon(),
                self.rectangle.min_lat(),
                self.rectangle.max_lat(),
                min_x,
                max_x,
                min_y,
                max_y,
            )
        }
    }

    fn get_min_x(&self) -> f64 {
        if self.crosses_dateline {
            // Component2D does not support boxes that cross the dateline
            return GeoUtils::MIN_LON_INCL;
        }
        self.rectangle.min_lon()
    }

    fn get_max_x(&self) -> f64 {
        if self.crosses_dateline {
            // Component2D does not support boxes that cross the dateline
            return GeoUtils::MAX_LON_INCL;
        }
        self.rectangle.max_lon()
    }

    fn get_min_y(&self) -> f64 {
        self.rectangle.min_lat()
    }

    fn get_max_y(&self) -> f64 {
        self.rectangle.max_lat()
    }

    fn get_x(&self) -> f64 {
        self.center_lon
    }

    fn get_y(&self) -> f64 {
        self.center_lat
    }
}

impl Circle2D {
    /// Builds a component from a cartesian circle; distances are computed with
    /// cartesian distance.
    ///
    /// Equivalent to `Circle2D.create(XYCircle)`.
    ///
    /// # Panics
    ///
    /// Panics only if the circle's bounding box cannot be built, which the
    /// [`XYCircle`] constructor already rules out by rejecting a non-finite or
    /// non-positive radius.
    pub fn create_from_xy_circle(circle: &XYCircle) -> Arc<dyn Component2D> {
        let calculator =
            CartesianDistance::new(circle.get_x(), circle.get_y(), circle.get_radius())
                .expect("INVARIANT: XYCircle validated its centre and radius on construction");
        Arc::new(Circle2D {
            calculator: Box::new(calculator),
        })
    }

    /// Builds a component from a geographic circle; distances are computed with
    /// haversine distance.
    ///
    /// Equivalent to `Circle2D.create(Circle)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the circle's
    /// bounding box is not a valid rectangle.
    pub fn create_from_circle(circle: &Circle) -> Result<Arc<dyn Component2D>> {
        let calculator =
            HaversinDistance::new(circle.get_lon(), circle.get_lat(), circle.get_radius())?;
        Ok(Arc::new(Circle2D {
            calculator: Box::new(calculator),
        }))
    }
}

impl Component2D for Circle2D {
    fn get_min_x(&self) -> f64 {
        self.calculator.get_min_x()
    }

    fn get_max_x(&self) -> f64 {
        self.calculator.get_max_x()
    }

    fn get_min_y(&self) -> f64 {
        self.calculator.get_min_y()
    }

    fn get_max_y(&self) -> f64 {
        self.calculator.get_max_y()
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        self.calculator.contains(x, y)
    }

    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return Relation::CellOutsideQuery;
        }
        if self.calculator.within(min_x, max_x, min_y, max_y) {
            return Relation::CellCrossesQuery;
        }
        self.calculator.relate(min_x, max_x, min_y, max_y)
    }

    fn intersects_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
    ) -> bool {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return false;
        }
        self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self.calculator.intersects_line(a_x, a_y, b_x, b_y)
    }

    fn intersects_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return false;
        }
        self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self.contains(c_x, c_y)
            || point_in_triangle(
                min_x,
                max_x,
                min_y,
                max_y,
                self.calculator.get_x(),
                self.calculator.get_y(),
                a_x,
                a_y,
                b_x,
                b_y,
                c_x,
                c_y,
            )
            || self.calculator.intersects_line(a_x, a_y, b_x, b_y)
            || self.calculator.intersects_line(b_x, b_y, c_x, c_y)
            || self.calculator.intersects_line(c_x, c_y, a_x, a_y)
    }

    fn contains_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
    ) -> bool {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return false;
        }
        self.contains(a_x, a_y) && self.contains(b_x, b_y)
    }

    fn contains_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return false;
        }
        self.contains(a_x, a_y) && self.contains(b_x, b_y) && self.contains(c_x, c_y)
    }

    fn within_point(&self, x: f64, y: f64) -> WithinRelation {
        if self.contains(x, y) {
            WithinRelation::NotWithin
        } else {
            WithinRelation::Disjoint
        }
    }

    fn within_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        ab: bool,
        b_x: f64,
        b_y: f64,
    ) -> WithinRelation {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return WithinRelation::Disjoint;
        }
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        if ab && self.calculator.intersects_line(a_x, a_y, b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        WithinRelation::Disjoint
    }

    fn within_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        ab: bool,
        b_x: f64,
        b_y: f64,
        bc: bool,
        c_x: f64,
        c_y: f64,
        ca: bool,
    ) -> WithinRelation {
        if self.calculator.disjoint(min_x, max_x, min_y, max_y) {
            return WithinRelation::Disjoint;
        }

        // if any of the points is inside the circle then we cannot be within this indexed shape
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) || self.contains(c_x, c_y) {
            return WithinRelation::NotWithin;
        }

        // we only check edges that belong to the original polygon; if we intersect any of them,
        // then we are not within.
        if ab && self.calculator.intersects_line(a_x, a_y, b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        if bc && self.calculator.intersects_line(b_x, b_y, c_x, c_y) {
            return WithinRelation::NotWithin;
        }
        if ca && self.calculator.intersects_line(c_x, c_y, a_x, a_y) {
            return WithinRelation::NotWithin;
        }

        // check if the centre is within the triangle: this is the only check that returns this
        // circle as a candidate, but that is fine as the centre must be inside to be one of the
        // triangles.
        if point_in_triangle(
            min_x,
            max_x,
            min_y,
            max_y,
            self.calculator.get_x(),
            self.calculator.get_y(),
            a_x,
            a_y,
            b_x,
            b_y,
            c_x,
            c_y,
        ) {
            return WithinRelation::Candidate;
        }
        WithinRelation::Disjoint
    }
}
