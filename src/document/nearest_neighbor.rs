//! K-nearest-neighbour search over 2D latitude/longitude indexed points,
//! ported from `org.apache.lucene.document.NearestNeighbor`.
//!
//! The BKD cells of every segment are explored in order of their approximate
//! distance to the query point, and the closest `n` documents are kept in a
//! bounded priority queue whose worst hit continuously tightens a bounding box
//! that prunes the remaining cells.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::error::Result;
use crate::geo::encoding::GeoEncodingUtils;
use crate::geo::geometry::Rectangle;
use crate::index::point_values::{IntersectVisitor, PointTree, Relation};
use crate::index::PointValues;
use crate::util::extra::{PriorityQueue, PriorityQueueComparator};
use crate::util::sloppy_math::SloppyMath;
use crate::util::Bits;

/// How many bytes one encoded coordinate occupies.
const COORDINATE_BYTES: usize = 4;

/// One hit of [`nearest`].
///
/// Equivalent to the static nested class `NearestNeighbor.NearestHit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearestHit {
    /// The document, already re-based onto the top-level reader.
    pub doc_id: i32,
    /// The distance from the hit to the query point, as the sort key
    /// [`SloppyMath::haversin_sort_key`] computes.
    pub distance_sort_key: f64,
}

impl std::fmt::Display for NearestHit {
    /// Equivalent to `NearestHit.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NearestHit(docID={} distanceSortKey={})",
            self.doc_id, self.distance_sort_key
        )
    }
}

/// Orders the hit queue so the *worst* hit sits on top and is evicted first.
///
/// Equivalent to `NearestNeighbor.NearestHitQueue.lessThan(NearestHit, NearestHit)`.
struct NearestHitQueueComparator;

impl PriorityQueueComparator<NearestHit> for NearestHitQueueComparator {
    fn less_than(&self, a: &NearestHit, b: &NearestHit) -> bool {
        // Keep the worst hit — the highest distance — at the top, so it can be
        // evicted when a better hit arrives.
        let cmp = a.distance_sort_key.total_cmp(&b.distance_sort_key);
        if cmp != Ordering::Equal {
            return cmp == Ordering::Greater;
        }
        // Tie-break by higher document id, which is the worse one.
        a.doc_id > b.doc_id
    }
}

/// A BKD cell waiting to be explored.
///
/// Equivalent to the record `NearestNeighbor.Cell`.
struct Cell {
    index: Box<dyn PointTree>,
    reader_index: usize,
    min_packed: Vec<u8>,
    max_packed: Vec<u8>,
    /// The closest distance from a point in this cell to the query point, as
    /// the sort key [`SloppyMath::haversin_sort_key`] computes. It is an
    /// approximation: the cell may hold a closer point.
    distance_sort_key: f64,
}

impl Cell {
    fn new(
        index: Box<dyn PointTree>,
        reader_index: usize,
        min_packed: &[u8],
        max_packed: &[u8],
        distance_sort_key: f64,
    ) -> Self {
        Self {
            index,
            reader_index,
            // Java's compact constructor clones both arrays, because the tree
            // cursor rewrites them as it moves.
            min_packed: min_packed.to_vec(),
            max_packed: max_packed.to_vec(),
            distance_sort_key,
        }
    }
}

impl std::fmt::Debug for Cell {
    /// Equivalent to `Cell.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let min_lat = GeoEncodingUtils::decode_latitude_bytes(&self.min_packed, 0);
        let min_lon = GeoEncodingUtils::decode_longitude_bytes(&self.min_packed, COORDINATE_BYTES);
        let max_lat = GeoEncodingUtils::decode_latitude_bytes(&self.max_packed, 0);
        let max_lon = GeoEncodingUtils::decode_longitude_bytes(&self.max_packed, COORDINATE_BYTES);
        write!(
            f,
            "Cell(readerIndex={} lat={min_lat} TO {max_lat}, lon={min_lon} TO {max_lon}; \
             distanceSortKey={})",
            self.reader_index, self.distance_sort_key
        )
    }
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.distance_sort_key.total_cmp(&other.distance_sort_key) == Ordering::Equal
    }
}

impl Eq for Cell {}

impl Ord for Cell {
    /// Reverses `Cell.compareTo(Cell)`, because Java uses a `java.util.PriorityQueue`
    /// — a *min*-heap by natural order — and Rust's [`BinaryHeap`] is a max-heap.
    /// The cell explored first is therefore the same one in both.
    fn cmp(&self, other: &Self) -> Ordering {
        other.distance_sort_key.total_cmp(&self.distance_sort_key)
    }
}

impl PartialOrd for Cell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Visits the points of a leaf cell and keeps the closest ones.
///
/// Equivalent to the private static class `NearestNeighbor.NearestVisitor`.
struct NearestVisitor<'a> {
    cur_doc_base: i32,
    cur_live_docs: Option<&'a dyn Bits>,
    top_n: usize,
    hit_queue: PriorityQueue<NearestHit, NearestHitQueueComparator>,
    point_lat: f64,
    point_lon: f64,
    set_bottom_counter: i32,

    min_lon: f64,
    max_lon: f64,
    min_lat: f64,
    max_lat: f64,
    /// A second longitude range, for the cross-dateline case.
    min_lon2: f64,
}

impl<'a> NearestVisitor<'a> {
    fn new(
        hit_queue: PriorityQueue<NearestHit, NearestHitQueueComparator>,
        top_n: usize,
        point_lat: f64,
        point_lon: f64,
    ) -> Self {
        Self {
            cur_doc_base: 0,
            cur_live_docs: None,
            top_n,
            hit_queue,
            point_lat,
            point_lon,
            set_bottom_counter: 0,
            min_lon: f64::NEG_INFINITY,
            max_lon: f64::INFINITY,
            min_lat: f64::NEG_INFINITY,
            max_lat: f64::INFINITY,
            min_lon2: f64::INFINITY,
        }
    }

    /// Equivalent to the private `NearestVisitor.maybeUpdateBBox()`.
    fn maybe_update_bbox(&mut self) -> Result<()> {
        if self.set_bottom_counter < 1024 || (self.set_bottom_counter & 0x3F) == 0x3F {
            if let Some(hit) = self.hit_queue.top().copied() {
                let box_ = Rectangle::from_point_distance(
                    self.point_lat,
                    self.point_lon,
                    SloppyMath::haversin_meters_from_sort_key(hit.distance_sort_key),
                )?;
                self.min_lat = box_.min_lat();
                self.max_lat = box_.max_lat();
                if box_.crosses_dateline() {
                    // Box one.
                    self.min_lon = f64::NEG_INFINITY;
                    self.max_lon = box_.max_lon();
                    // Box two.
                    self.min_lon2 = box_.min_lon();
                } else {
                    self.min_lon = box_.min_lon();
                    self.max_lon = box_.max_lon();
                    // Disable box two.
                    self.min_lon2 = f64::INFINITY;
                }
            }
        }
        self.set_bottom_counter += 1;
        Ok(())
    }
}

impl IntersectVisitor for NearestVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        // Java throws `AssertionError`: this visitor is only ever driven
        // through `visitDocValues`, which always supplies the packed value.
        Err(crate::error::LuceneError::IllegalState(
            "NearestVisitor is only driven through visit_doc_values".to_string(),
        ))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if let Some(live_docs) = self.cur_live_docs {
            if !live_docs.get(doc_id as usize) {
                return Ok(());
            }
        }

        let doc_latitude = GeoEncodingUtils::decode_latitude_bytes(packed_value, 0);
        let doc_longitude =
            GeoEncodingUtils::decode_longitude_bytes(packed_value, COORDINATE_BYTES);

        // Test the bounding box.
        if doc_latitude < self.min_lat || doc_latitude > self.max_lat {
            return Ok(());
        }
        if (doc_longitude < self.min_lon || doc_longitude > self.max_lon)
            && doc_longitude < self.min_lon2
        {
            return Ok(());
        }

        // Use the haversine sort key when comparing hits: it is faster to
        // compute than the true distance.
        let distance_sort_key = SloppyMath::haversin_sort_key(
            self.point_lat,
            self.point_lon,
            doc_latitude,
            doc_longitude,
        );

        let full_doc_id = self.cur_doc_base + doc_id;

        if self.hit_queue.size() == self.top_n {
            // The queue is already full.
            let Some(hit) = self.hit_queue.top().copied() else {
                return Ok(());
            };
            // Documents are not collected in order here, so the tie-break case
            // must be tested explicitly.
            if distance_sort_key < hit.distance_sort_key
                || (distance_sort_key == hit.distance_sort_key && full_doc_id < hit.doc_id)
            {
                self.hit_queue.update_top_with(NearestHit {
                    doc_id: full_doc_id,
                    distance_sort_key,
                });
                self.maybe_update_bbox()?;
            }
        } else {
            self.hit_queue.add(NearestHit {
                doc_id: full_doc_id,
                distance_sort_key,
            });
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let cell_min_lat = GeoEncodingUtils::decode_latitude_bytes(min_packed_value, 0);
        let cell_min_lon =
            GeoEncodingUtils::decode_longitude_bytes(min_packed_value, COORDINATE_BYTES);
        let cell_max_lat = GeoEncodingUtils::decode_latitude_bytes(max_packed_value, 0);
        let cell_max_lon =
            GeoEncodingUtils::decode_longitude_bytes(max_packed_value, COORDINATE_BYTES);

        if cell_max_lat < self.min_lat
            || self.max_lat < cell_min_lat
            || ((cell_max_lon < self.min_lon || self.max_lon < cell_min_lon)
                && cell_max_lon < self.min_lon2)
        {
            // This cell falls outside the search bounding box, so there is no
            // point exploring it any further.
            return Relation::CellOutsideQuery;
        }
        Relation::CellCrossesQuery
    }
}

/// Returns the `n` documents closest to `(point_lat, point_lon)`, ordered from
/// nearest to farthest.
///
/// Equivalent to the static
/// `NearestNeighbor.nearest(double, double, List<PointValues>, List<Bits>, IntArrayList, int)`.
///
/// * `readers` — the point values of each leaf, in leaf order;
/// * `live_docs` — the live-docs bits of each leaf, `None` where a leaf has no
///   deletions;
/// * `doc_bases` — the document base of each leaf.
///
/// # Errors
///
/// Propagates whatever the point trees raise, and returns
/// [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
/// when `n` is too large for the hit queue.
pub fn nearest(
    point_lat: f64,
    point_lon: f64,
    readers: &[Box<dyn PointValues>],
    live_docs: &[Option<Box<dyn Bits>>],
    doc_bases: &[i32],
    n: usize,
) -> Result<Vec<NearestHit>> {
    // Holds the closest collected points seen so far.
    let hit_queue = PriorityQueue::new(n, NearestHitQueueComparator)?;

    // Holds all cells, sorted by proximity to the point.
    let mut cell_queue: BinaryHeap<Cell> = BinaryHeap::new();

    let mut visitor = NearestVisitor::new(hit_queue, n, point_lat, point_lon);

    // Add the root cell of every reader to the queue.
    for (i, reader) in readers.iter().enumerate() {
        let (Some(min_packed_value), Some(max_packed_value)) =
            (reader.min_packed_value()?, reader.max_packed_value()?)
        else {
            continue;
        };
        let index_tree = reader.point_tree()?;
        let distance =
            approx_best_distance_packed(&min_packed_value, &max_packed_value, point_lat, point_lon);
        cell_queue.push(Cell::new(
            index_tree,
            i,
            &min_packed_value,
            &max_packed_value,
            distance,
        ));
    }

    while let Some(mut cell) = cell_queue.pop() {
        if visitor.compare(&cell.min_packed, &cell.max_packed) == Relation::CellOutsideQuery {
            continue;
        }

        if !cell.index.move_to_child()? {
            // Leaf block: visit every point and possibly collect it.
            visitor.cur_doc_base = doc_bases[cell.reader_index];
            visitor.cur_live_docs = live_docs[cell.reader_index].as_deref();
            cell.index.visit_doc_values(&mut visitor)?;
        } else {
            // Non-leaf block: split into two cells and put them back in the
            // queue. The cursor must be cloned so the two branches can be
            // explored "concurrently".
            let new_index = cell.index.clone_tree();
            let distance = approx_best_distance_packed(
                new_index.min_packed_value(),
                new_index.max_packed_value(),
                point_lat,
                point_lon,
            );
            let (min_packed, max_packed) = (
                new_index.min_packed_value().to_vec(),
                new_index.max_packed_value().to_vec(),
            );
            cell_queue.push(Cell::new(
                new_index,
                cell.reader_index,
                &min_packed,
                &max_packed,
                distance,
            ));

            // A binary tree is assumed here, as in Lucene.
            if cell.index.move_to_sibling()? {
                let distance = approx_best_distance_packed(
                    cell.index.min_packed_value(),
                    cell.index.max_packed_value(),
                    point_lat,
                    point_lon,
                );
                let (min_packed, max_packed) = (
                    cell.index.min_packed_value().to_vec(),
                    cell.index.max_packed_value().to_vec(),
                );
                let reader_index = cell.reader_index;
                cell_queue.push(Cell::new(
                    cell.index,
                    reader_index,
                    &min_packed,
                    &max_packed,
                    distance,
                ));
            }
        }
    }

    let mut hits = vec![
        NearestHit {
            doc_id: 0,
            distance_sort_key: 0.0,
        };
        visitor.hit_queue.size()
    ];
    let mut down_to = visitor.hit_queue.size();
    while visitor.hit_queue.size() != 0 {
        down_to -= 1;
        hits[down_to] = visitor
            .hit_queue
            .pop()
            .expect("INVARIANT: the queue was just observed to be non-empty");
    }

    Ok(hits)
}

/// Equivalent to the private static
/// `NearestNeighbor.approxBestDistance(byte[], byte[], double, double)`.
///
/// The incoming bounds never cross the dateline, because they come from a BKD
/// cell.
fn approx_best_distance_packed(
    min_packed_value: &[u8],
    max_packed_value: &[u8],
    point_lat: f64,
    point_lon: f64,
) -> f64 {
    let min_lat = GeoEncodingUtils::decode_latitude_bytes(min_packed_value, 0);
    let min_lon = GeoEncodingUtils::decode_longitude_bytes(min_packed_value, COORDINATE_BYTES);
    let max_lat = GeoEncodingUtils::decode_latitude_bytes(max_packed_value, 0);
    let max_lon = GeoEncodingUtils::decode_longitude_bytes(max_packed_value, COORDINATE_BYTES);
    approx_best_distance(min_lat, max_lat, min_lon, max_lon, point_lat, point_lon)
}

/// Equivalent to the private static
/// `NearestNeighbor.approxBestDistance(double, double, double, double, double, double)`.
fn approx_best_distance(
    min_lat: f64,
    max_lat: f64,
    min_lon: f64,
    max_lon: f64,
    point_lat: f64,
    point_lon: f64,
) -> f64 {
    if point_lat >= min_lat && point_lat <= max_lat && point_lon >= min_lon && point_lon <= max_lon
    {
        // The point is inside the cell.
        return 0.0;
    }

    let d1 = SloppyMath::haversin_sort_key(point_lat, point_lon, min_lat, min_lon);
    let d2 = SloppyMath::haversin_sort_key(point_lat, point_lon, min_lat, max_lon);
    let d3 = SloppyMath::haversin_sort_key(point_lat, point_lon, max_lat, max_lon);
    let d4 = SloppyMath::haversin_sort_key(point_lat, point_lon, max_lat, min_lon);
    d1.min(d2).min(d3.min(d4))
}

/// The namespace `nearest` belongs to.
///
/// Equivalent to the package-private class
/// `org.apache.lucene.document.NearestNeighbor`, which holds only static
/// members.
pub struct NearestNeighbor;

impl NearestNeighbor {
    /// Returns the `n` documents closest to `(point_lat, point_lon)`.
    ///
    /// Equivalent to the static `NearestNeighbor.nearest(...)`; see the free
    /// function [`nearest`].
    ///
    /// # Errors
    ///
    /// As [`nearest`].
    pub fn nearest(
        point_lat: f64,
        point_lon: f64,
        readers: &[Box<dyn PointValues>],
        live_docs: &[Option<Box<dyn Bits>>],
        doc_bases: &[i32],
        n: usize,
    ) -> Result<Vec<NearestHit>> {
        nearest(point_lat, point_lon, readers, live_docs, doc_bases, n)
    }
}
