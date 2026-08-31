//! The multi-component interval tree ported from
//! `org.apache.lucene.geo.ComponentTree`.

use crate::geo::component2d::{Component2D, WithinRelation};
use crate::index::point_values::Relation;
use std::cmp::Ordering;
use std::sync::Arc;

/// Whether the root level splits on X. Equivalent to
/// `ComponentTree.ROOT_SPLITX`.
const ROOT_SPLITX: bool = false;

/// 2D multi-component geometry represented as an interval tree of components.
///
/// Equivalent to `org.apache.lucene.geo.ComponentTree`, which is package-private
/// in Lucene; Rust has no package-private visibility, so the type is `pub` and
/// is marked internal by this documentation instead.
///
/// Construction takes `O(n log n)` for the partial sorting and tree building.
#[derive(Debug)]
pub struct ComponentTree {
    /// Minimum Y of this geometry's bounding box area.
    min_y: f64,
    /// Maximum Y of this geometry's bounding box area.
    max_y: f64,
    /// Minimum X of this geometry's bounding box area.
    min_x: f64,
    /// Maximum X of this geometry's bounding box area.
    max_x: f64,
    // Child components, if any. Note that internal nodes might not have a
    // consistent bounding box; internal nodes must not be accessed outside of
    // this type.
    left: Option<Box<ComponentTree>>,
    right: Option<Box<ComponentTree>>,
    /// Root node of the edge tree.
    component: Arc<dyn Component2D>,
}

/// Compares two components by minimum then maximum X.
///
/// Equivalent to `ComponentTree.X_COMPARATOR`. Java uses
/// `Comparator.comparingDouble`, which orders with `Double.compare`, i.e. the
/// IEEE 754 total order that [`f64::total_cmp`] implements.
fn x_comparator(a: &Arc<dyn Component2D>, b: &Arc<dyn Component2D>) -> Ordering {
    match a.get_min_x().total_cmp(&b.get_min_x()) {
        Ordering::Equal => a.get_max_x().total_cmp(&b.get_max_x()),
        other => other,
    }
}

/// Compares two components by minimum then maximum Y.
///
/// Equivalent to `ComponentTree.Y_COMPARATOR`.
fn y_comparator(a: &Arc<dyn Component2D>, b: &Arc<dyn Component2D>) -> Ordering {
    match a.get_min_y().total_cmp(&b.get_min_y()) {
        Ordering::Equal => a.get_max_y().total_cmp(&b.get_max_y()),
        other => other,
    }
}

impl ComponentTree {
    fn new(component: Arc<dyn Component2D>) -> Self {
        Self {
            min_y: component.get_min_y(),
            max_y: component.get_max_y(),
            min_x: component.get_min_x(),
            max_x: component.get_max_x(),
            left: None,
            right: None,
            component,
        }
    }

    /// Creates a component tree from the provided components.
    ///
    /// Equivalent to `ComponentTree.create(Component2D[])`. A single component
    /// is returned unchanged.
    ///
    /// # Panics
    ///
    /// Panics when `components` is empty; the two callers,
    /// `LatLonGeometry::create` and `XYGeometry::create`, reject that case
    /// before calling.
    pub fn create(mut components: Vec<Arc<dyn Component2D>>) -> Arc<dyn Component2D> {
        assert!(
            !components.is_empty(),
            "INVARIANT: the geometry factories reject empty component arrays"
        );
        if components.len() == 1 {
            return components.remove(0);
        }
        let high = components.len() as isize - 1;
        let mut root = *Self::create_tree(&mut components, 0, high, ROOT_SPLITX)
            .expect("INVARIANT: a non-empty range always yields a node");
        // pull up min values for the root node so it contains a consistent bounding box
        for component in &components {
            root.min_y = root.min_y.min(component.get_min_y());
            root.min_x = root.min_x.min(component.get_min_x());
        }
        Arc::new(root)
    }

    /// Creates a tree from partially sorted components, with `low` and `high`
    /// inclusive.
    ///
    /// Equivalent to the private
    /// `ComponentTree.createTree(Component2D[], int, int, boolean)`. Java calls
    /// `ArrayUtil.select`, an introselect; Rust's
    /// [`slice::select_nth_unstable_by`] gives the same contract — the element
    /// at `mid` is the one that would be there in sorted order, everything
    /// before it compares less or equal and everything after greater or equal —
    /// which is all the pruning in the query methods relies on.
    fn create_tree(
        components: &mut [Arc<dyn Component2D>],
        low: isize,
        high: isize,
        split_x: bool,
    ) -> Option<Box<ComponentTree>> {
        if low > high {
            return None;
        }
        let mid = (low + high) / 2;
        if low < high {
            let lo = low as usize;
            let hi = high as usize;
            let k = (mid - low) as usize;
            if split_x {
                components[lo..=hi].select_nth_unstable_by(k, x_comparator);
            } else {
                components[lo..=hi].select_nth_unstable_by(k, y_comparator);
            }
        }
        let mut new_node = ComponentTree::new(Arc::clone(&components[mid as usize]));
        // find children
        new_node.left = Self::create_tree(components, low, mid - 1, !split_x);
        new_node.right = Self::create_tree(components, mid + 1, high, !split_x);

        // pull up max values to this node
        if let Some(left) = &new_node.left {
            new_node.max_x = new_node.max_x.max(left.max_x);
            new_node.max_y = new_node.max_y.max(left.max_y);
        }
        if let Some(right) = &new_node.right {
            new_node.max_x = new_node.max_x.max(right.max_x);
            new_node.max_y = new_node.max_y.max(right.max_y);
        }
        Some(Box::new(new_node))
    }

    /// Returns whether the right subtree can be pruned for the given upper
    /// bounds. Factored out of the identical guard Java repeats in every query
    /// method.
    fn descend_right(&self, max_x: f64, max_y: f64, split_x: bool) -> bool {
        (!split_x && max_y >= self.component.get_min_y())
            || (split_x && max_x >= self.component.get_min_x())
    }

    fn contains_split(&self, x: f64, y: f64, split_x: bool) -> bool {
        if y <= self.max_y && x <= self.max_x {
            if self.component.contains(x, y) {
                return true;
            }
            if let Some(left) = &self.left {
                if left.contains_split(x, y, !split_x) {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(x, y, split_x) {
                    return right.contains_split(x, y, !split_x);
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn intersects_line_split(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        split_x: bool,
    ) -> bool {
        if min_y <= self.max_y && min_x <= self.max_x {
            if self
                .component
                .intersects_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y)
            {
                return true;
            }
            if let Some(left) = &self.left {
                if left
                    .intersects_line_split(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, !split_x)
                {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(max_x, max_y, split_x) {
                    return right.intersects_line_split(
                        min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, !split_x,
                    );
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn intersects_triangle_split(
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
        split_x: bool,
    ) -> bool {
        if min_y <= self.max_y && min_x <= self.max_x {
            if self
                .component
                .intersects_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y)
            {
                return true;
            }
            if let Some(left) = &self.left {
                if left.intersects_triangle_split(
                    min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, !split_x,
                ) {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(max_x, max_y, split_x) {
                    return right.intersects_triangle_split(
                        min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, !split_x,
                    );
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn contains_line_split(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        split_x: bool,
    ) -> bool {
        if min_y <= self.max_y && min_x <= self.max_x {
            if self
                .component
                .contains_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y)
            {
                return true;
            }
            if let Some(left) = &self.left {
                if left
                    .contains_line_split(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, !split_x)
                {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(max_x, max_y, split_x) {
                    return right.contains_line_split(
                        min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, !split_x,
                    );
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn contains_triangle_split(
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
        split_x: bool,
    ) -> bool {
        if min_y <= self.max_y && min_x <= self.max_x {
            if self
                .component
                .contains_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y)
            {
                return true;
            }
            if let Some(left) = &self.left {
                if left.contains_triangle_split(
                    min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, !split_x,
                ) {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(max_x, max_y, split_x) {
                    return right.contains_triangle_split(
                        min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, !split_x,
                    );
                }
            }
        }
        false
    }

    fn relate_split(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        split_x: bool,
    ) -> Relation {
        if min_y <= self.max_y && min_x <= self.max_x {
            let relation = self.component.relate(min_x, max_x, min_y, max_y);
            if relation != Relation::CellOutsideQuery {
                return relation;
            }
            if let Some(left) = &self.left {
                let relation = left.relate_split(min_x, max_x, min_y, max_y, !split_x);
                if relation != Relation::CellOutsideQuery {
                    return relation;
                }
            }
            if let Some(right) = &self.right {
                if self.descend_right(max_x, max_y, split_x) {
                    return right.relate_split(min_x, max_x, min_y, max_y, !split_x);
                }
            }
        }
        Relation::CellOutsideQuery
    }

    /// Returns whether this tree has any child node, in which case the `within`
    /// family is not supported.
    fn has_children(&self) -> bool {
        self.left.is_some() || self.right.is_some()
    }
}

impl Component2D for ComponentTree {
    fn get_min_x(&self) -> f64 {
        self.min_x
    }

    fn get_max_x(&self) -> f64 {
        self.max_x
    }

    fn get_min_y(&self) -> f64 {
        self.min_y
    }

    fn get_max_y(&self) -> f64 {
        self.max_y
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        self.contains_split(x, y, ROOT_SPLITX)
    }

    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        self.relate_split(min_x, max_x, min_y, max_y, ROOT_SPLITX)
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
        self.intersects_line_split(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, ROOT_SPLITX)
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
        self.intersects_triangle_split(
            min_x,
            max_x,
            min_y,
            max_y,
            a_x,
            a_y,
            b_x,
            b_y,
            c_x,
            c_y,
            ROOT_SPLITX,
        )
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
        self.contains_line_split(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, ROOT_SPLITX)
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
        self.contains_triangle_split(
            min_x,
            max_x,
            min_y,
            max_y,
            a_x,
            a_y,
            b_x,
            b_y,
            c_x,
            c_y,
            ROOT_SPLITX,
        )
    }

    /// # Panics
    ///
    /// Panics when the tree holds more than one component, which is what Java
    /// signals with `IllegalArgumentException`. The `within` family has no
    /// `Result` in [`Component2D`], so the port keeps the failure fatal rather
    /// than widening the whole interface.
    fn within_point(&self, x: f64, y: f64) -> WithinRelation {
        assert!(
            !self.has_children(),
            "withinPoint is not supported for shapes with more than one component"
        );
        self.component.within_point(x, y)
    }

    /// # Panics
    ///
    /// Panics when the tree holds more than one component; see
    /// [`ComponentTree::within_point`].
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
        assert!(
            !self.has_children(),
            "withinLine is not supported for shapes with more than one component"
        );
        self.component
            .within_line(min_x, max_x, min_y, max_y, a_x, a_y, ab, b_x, b_y)
    }

    /// # Panics
    ///
    /// Panics when the tree holds more than one component; see
    /// [`ComponentTree::within_point`].
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
        assert!(
            !self.has_children(),
            "withinTriangle is not supported for shapes with more than one component"
        );
        self.component.within_triangle(
            min_x, max_x, min_y, max_y, a_x, a_y, ab, b_x, b_y, bc, c_x, c_y, ca,
        )
    }
}
