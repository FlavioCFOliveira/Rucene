//! `java.util` behaviour the runner depends on: `Collections.shuffle` and
//! the iteration order of `HashMap` / `HashSet`.

use super::random::Random;

/// `Collections.shuffle(list, rnd)`: from the last position down to the
/// second, swap each element with one at `rnd.nextInt(i + 1)`.
pub fn shuffle<T>(list: &mut [T], rnd: &dyn Random) {
    let mut i = list.len();
    while i > 1 {
        let j = rnd.next_int_bound(i as i32) as usize;
        list.swap(i - 1, j);
        i -= 1;
    }
}

/// `Long.hashCode(v)`.
pub fn long_hash_code(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

/// The iteration order of a `java.util.HashMap` (or `HashSet`) holding
/// keys with the given `hashCode()`s, inserted in order: the indices of
/// the distinct keys (by `eq`) in iteration order.
///
/// A `HashMap` of default capacity 16 and load factor 0.75 doubles while
/// its size exceeds three quarters of its capacity; it iterates its buckets
/// in index order (`(h ^ h >>> 16) & (capacity - 1)`) and the keys of a
/// bucket in insertion order, which resizing preserves. A treeified bucket
/// (eight or more colliding keys) would order differently; the runner's
/// maps never hold that many.
pub fn hash_iteration_order<K>(keys: &[(K, i32)], eq: impl Fn(&K, &K) -> bool) -> Vec<usize> {
    let mut distinct: Vec<usize> = Vec::new();
    for (i, (k, _)) in keys.iter().enumerate() {
        if !distinct.iter().any(|&d| eq(&keys[d].0, k)) {
            distinct.push(i);
        }
    }
    let mut capacity: usize = 16;
    while distinct.len() as f64 > capacity as f64 * 0.75 {
        capacity *= 2;
    }
    let mut order: Vec<(usize, usize, usize)> = distinct
        .iter()
        .enumerate()
        .map(|(rank, &i)| {
            let h = keys[i].1;
            let spread = (h ^ ((h as u32) >> 16) as i32) as u32 as usize;
            (spread & (capacity - 1), rank, i)
        })
        .collect();
    order.sort();
    order.into_iter().map(|(_, _, i)| i).collect()
}
