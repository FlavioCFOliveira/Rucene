//! The `Component2D` default methods the geo tests call with their
//! six-coordinate (bounding-box-computing) signatures, as
//! `Component2D.java` declares them: `intersectsTriangle(ax, ay, bx, by, cx,
//! cy)`, `intersectsLine`, `containsTriangle`, `containsLine` and
//! `withinTriangle(ax, ay, ab, bx, by, bc, cx, cy, ca)`.

use rucene::geo::{Component2D, WithinRelation};

/// `component.intersectsTriangle(ax, ay, bx, by, cx, cy)`.
pub fn intersects_triangle(
    c: &dyn Component2D,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
) -> bool {
    c.intersects_triangle_bbox(ax, ay, bx, by, cx, cy)
}

/// `component.intersectsLine(ax, ay, bx, by)`.
pub fn intersects_line(c: &dyn Component2D, ax: f64, ay: f64, bx: f64, by: f64) -> bool {
    c.intersects_line_bbox(ax, ay, bx, by)
}

/// `component.containsTriangle(ax, ay, bx, by, cx, cy)`.
pub fn contains_triangle(
    c: &dyn Component2D,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
) -> bool {
    c.contains_triangle_bbox(ax, ay, bx, by, cx, cy)
}

/// `component.containsLine(ax, ay, bx, by)`.
pub fn contains_line(c: &dyn Component2D, ax: f64, ay: f64, bx: f64, by: f64) -> bool {
    c.contains_line_bbox(ax, ay, bx, by)
}

/// `component.withinTriangle(ax, ay, ab, bx, by, bc, cx, cy, ca)`.
#[allow(clippy::too_many_arguments)]
pub fn within_triangle(
    c: &dyn Component2D,
    ax: f64,
    ay: f64,
    ab: bool,
    bx: f64,
    by: f64,
    bc: bool,
    cx: f64,
    cy: f64,
    ca: bool,
) -> WithinRelation {
    c.within_triangle_bbox(ax, ay, ab, bx, by, bc, cx, cy, ca)
}
