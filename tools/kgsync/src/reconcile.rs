//! Differential reconciliation of *slices* of the graph: the nodes of one
//! label, or the edges of one (source label, type, target label), that a layer
//! owns. Three phases:
//!
//! 1. **read** — each slice's current state in one labelled statement; all
//!    slices are read concurrently (the server serves parallel readers);
//! 2. **plan** — the difference with the desired state, in parallel;
//! 3. **apply** — only the difference is written, sequentially.
//!
//! Write shapes are the ones measured fastest on rmp 1.21.0 (see
//! `tools/kgsync/README.md`, "Engine measurements"):
//!
//! * node create — `UNWIND rows CREATE (n:L) SET n += r`, large batches;
//! * node update — literal `MATCH (n:L {key: v}) SET n.p = …` chained with
//!   `WITH count(*) AS _`, 200 clauses per statement (index seek per clause;
//!   never `SET n += {…}`, which the planner turns into a label scan);
//! * node delete — the same chain tags the nodes `:KgTomb`, then a bounded
//!   `MATCH (n:KgTomb) … DETACH DELETE n` (a `DELETE` never uses an index);
//! * edge create — literal chain for small sets, `UNWIND … MATCH … MATCH …
//!   CREATE` batches for large ones (one label scan amortised per batch);
//! * edge update — literal chain re-binding the edge with `MATCH`;
//! * edge delete — the chain sets `e.kgTomb` and tags the source `:KgDirty`,
//!   then a bounded `DELETE` over the dirty sources only.

use crate::graph::{ident, lit, lit_s, Graph, MAX_STATEMENT};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{Map, Value};

use std::time::Instant;

pub type Props = Map<String, Value>;

const CHAIN: usize = 200;
const UNWIND_ROWS: usize = 4000;

pub struct Stamp {
    pub commit: String,
    pub date: String,
}

#[derive(Default, Debug, Clone)]
pub struct Report {
    pub slice: String,
    pub desired: usize,
    pub current: usize,
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unresolved: usize,
    pub ms: u128,
}

impl Report {
    pub fn line(&self) -> String {
        format!(
            "{:<48} desired {:>7} current {:>7} | +{:<6} ~{:<6} -{:<6}{} {:>6} ms",
            self.slice,
            self.desired,
            self.current,
            self.created,
            self.updated,
            self.deleted,
            if self.unresolved > 0 {
                format!(" (unresolved endpoints {})", self.unresolved)
            } else {
                String::new()
            },
            self.ms
        )
    }
}

/// Which nodes of a label a layer owns, and which of their properties.
pub struct NodeSlice {
    pub label: &'static str,
    pub key: &'static str,
    /// Properties the layer owns (key excluded). Others are never touched.
    pub owned: Vec<&'static str>,
    /// Cypher boolean over `n` selecting the owned nodes; `true` for all.
    pub filter: String,
}

pub struct EdgeSlice {
    pub rel: &'static str,
    pub src: (&'static str, &'static str),
    pub dst: (&'static str, &'static str),
    pub owned: Vec<&'static str>,
    /// Cypher boolean over `e`, `a`, `b` selecting the owned edges.
    pub filter: String,
}

pub struct Edge {
    pub src: Value,
    pub dst: Value,
    pub props: Props,
}

fn sample() -> Option<usize> {
    std::env::var("KGSYNC_SAMPLE")
        .ok()
        .and_then(|v| v.parse().ok())
}

fn trunc(s: &str) -> String {
    s.chars().take(260).collect()
}

pub fn key_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn props_differ(owned: &[&str], want: &Props, have: &[Value]) -> Option<Props> {
    let mut delta = Props::new();
    for (i, p) in owned.iter().enumerate() {
        let w = want.get(*p).unwrap_or(&Value::Null);
        let h = have.get(i).unwrap_or(&Value::Null);
        let same = match (w, h) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            _ => w == h,
        };
        if !same {
            delta.insert((*p).to_string(), w.clone());
        }
    }
    if delta.is_empty() {
        None
    } else {
        Some(delta)
    }
}

fn stamped(mut p: Props, s: &Stamp) -> Props {
    p.insert("gitCommit".into(), Value::String(s.commit.clone()));
    p.insert("gitDate".into(), Value::String(s.date.clone()));
    p
}

/// Repeats a bounded statement until it changes nothing (keeps every statement
/// inside the server's 5 s budget).
fn drain(g: &Graph, q: &str, counter: &str) -> Result<(), String> {
    if g.dry_run {
        return Ok(());
    }
    loop {
        let c = g.write(q)?;
        if c.get(counter).and_then(|v| v.as_u64()).unwrap_or(0) == 0 {
            return Ok(());
        }
    }
}

/// Packs clauses into statements of at most `CHAIN` clauses and
/// `MAX_STATEMENT` bytes, joined by `WITH count(*) AS _`.
fn run_chains(g: &Graph, clauses: &[String]) -> Result<(), String> {
    let mut stmt = String::new();
    let mut n = 0;
    for c in clauses {
        if n == CHAIN || (n > 0 && stmt.len() + c.len() > MAX_STATEMENT) {
            stmt.push_str(" RETURN 1");
            g.write(&stmt)?;
            stmt.clear();
            n = 0;
        }
        if n > 0 {
            stmt.push_str(" WITH count(*) AS _ ");
        }
        stmt.push_str(c);
        n += 1;
    }
    if n > 0 {
        stmt.push_str(" RETURN 1");
        g.write(&stmt)?;
    }
    Ok(())
}

/// Runs `UNWIND [rows] AS r <body>` in batches bounded by rows and bytes.
fn run_unwind(g: &Graph, rows: &[String], body: &str) -> Result<(), String> {
    let mut batch = String::new();
    let mut n = 0;
    let flush = |batch: &mut String| -> Result<(), String> {
        g.write(&format!("UNWIND [{batch}] AS r {body}"))?;
        batch.clear();
        Ok(())
    };
    for r in rows {
        if n == UNWIND_ROWS || (n > 0 && batch.len() + r.len() > MAX_STATEMENT) {
            flush(&mut batch)?;
            n = 0;
        }
        if n > 0 {
            batch.push(',');
        }
        batch.push_str(r);
        n += 1;
    }
    if n > 0 {
        flush(&mut batch)?;
    }
    Ok(())
}

/// Labels above this many nodes are read in `PARTS` concurrent partitions:
/// the label scan is cheap, the cost is projecting and serialising the rows,
/// which the server spreads over its threads (68k rows: 410 → 183 ms).
const PARTITION_ABOVE: i64 = 20_000;
const PARTS: i64 = 4;

fn label_count(g: &Graph, label: &str) -> i64 {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, i64>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(n) = cache.lock().unwrap().get(label) {
        return *n;
    }
    let n = g
        .rows(&format!("MATCH (n:{label}) RETURN count(n)"))
        .ok()
        .and_then(|r| r.first().and_then(|x| x.first()).and_then(|v| v.as_i64()))
        .unwrap_or(0);
    cache.lock().unwrap().insert(label.to_string(), n);
    n
}

/// Runs `<pattern> WHERE <filter> <ret>`, partitioned on `id(<var>)` when the
/// partitioned label is large, and concatenates the rows.
fn partitioned(
    g: &Graph,
    label: &str,
    var: &str,
    pattern: &str,
    filter: &str,
    ret: &str,
) -> Result<Vec<Vec<Value>>, String> {
    use rayon::prelude::*;
    let base = if filter.is_empty() {
        String::new()
    } else {
        format!("({filter})")
    };
    if label_count(g, label) <= PARTITION_ABOVE {
        let w = if base.is_empty() {
            String::new()
        } else {
            format!(" WHERE {base}")
        };
        return g.rows(&format!("{pattern}{w} {ret}"));
    }
    let parts: Vec<Result<Vec<Vec<Value>>, String>> = (0..PARTS)
        .into_par_iter()
        .map(|k| {
            let cond = format!("id({var}) % {PARTS} = {k}");
            let w = if base.is_empty() {
                cond
            } else {
                format!("{cond} AND {base}")
            };
            g.rows(&format!("{pattern} WHERE {w} {ret}"))
        })
        .collect();
    let mut rows = Vec::new();
    for p in parts {
        rows.extend(p?);
    }
    Ok(rows)
}

// ----------------------------------------------------------------- nodes ---

pub struct NodeCurrent {
    /// Every key of the label (owned or not).
    pub all: HashSet<String>,
    /// Owned nodes: key → (typed key, owned property values).
    pub owned: HashMap<String, (Value, Vec<Value>)>,
    pub ms: u128,
}

pub fn read_nodes(g: &Graph, s: &NodeSlice) -> Result<NodeCurrent, String> {
    let t = Instant::now();
    let mut ret = format!("RETURN n.{}, ({}) AS owned", s.key, s.filter);
    for p in &s.owned {
        ret.push_str(&format!(", n.{p}"));
    }
    let rows = partitioned(g, s.label, "n", &format!("MATCH (n:{})", s.label), "", &ret)?;
    let mut all = HashSet::with_capacity_and_hasher(rows.len(), Default::default());
    let mut owned = HashMap::default();
    for mut r in rows {
        if r.is_empty() || r[0].is_null() {
            continue;
        }
        let k = key_str(&r[0]);
        all.insert(k.clone());
        if r.get(1) == Some(&Value::Bool(true)) {
            let rest = r.split_off(2);
            owned.insert(k, (r.swap_remove(0), rest));
        }
    }
    Ok(NodeCurrent {
        all,
        owned,
        ms: t.elapsed().as_millis(),
    })
}

pub struct NodePlan {
    pub report: Report,
    creates: Vec<String>,
    updates: Vec<String>,
    deletes: Vec<String>,
    /// Keys deleted by this plan (their edges go with them).
    pub deleted: HashSet<String>,
}

pub fn plan_nodes(
    s: &NodeSlice,
    desired: &HashMap<String, (Value, Props)>,
    cur: &NodeCurrent,
    stamp: &Stamp,
) -> NodePlan {
    let mut rep = Report {
        slice: format!("(:{})", s.label),
        desired: desired.len(),
        current: cur.owned.len(),
        ms: cur.ms,
        ..Default::default()
    };
    let mut creates = Vec::new();
    let mut updates = Vec::new();
    for (k, (kv, props)) in desired {
        if !cur.all.contains(k) {
            let mut p = props.clone();
            p.insert(s.key.to_string(), kv.clone());
            p.retain(|_, v| !v.is_null());
            creates.push(lit_s(&Value::Object(stamped(p, stamp))));
            continue;
        }
        let delta = match cur.owned.get(k) {
            Some((_, h)) => props_differ(&s.owned, props, h),
            // Exists outside the slice: adopt it with the owned properties.
            None => Some(props.clone()),
        };
        if let Some(d) = delta {
            // Explicit assignments: `SET n += {…}` makes the planner drop the
            // index seek for a label scan per clause (rmp 1.21.0, measured).
            let mut c = format!("MATCH (n:{} {{", s.label);
            ident(s.key, &mut c);
            c.push(':');
            lit(kv, &mut c);
            c.push_str("}) SET ");
            for (i, (p, v)) in stamped(d, stamp).iter().enumerate() {
                if i > 0 {
                    c.push_str(", ");
                }
                c.push_str("n.");
                ident(p, &mut c);
                c.push_str(" = ");
                lit(v, &mut c);
            }
            updates.push(c);
        }
    }
    let mut deleted = HashSet::default();
    let mut deletes = Vec::new();
    for (k, (kv, _)) in &cur.owned {
        if !desired.contains_key(k) {
            deletes.push(format!(
                "MATCH (n:{} {{{}:{}}}) SET n:KgTomb",
                s.label,
                s.key,
                lit_s(kv)
            ));
            deleted.insert(k.clone());
        }
    }
    rep.created = creates.len();
    rep.updated = updates.len();
    rep.deleted = deletes.len();
    if let Some(n) = sample() {
        for c in creates.iter().take(n) {
            eprintln!("  + {} {}", s.label, trunc(c));
        }
        for c in updates.iter().take(n) {
            eprintln!("  ~ {} {}", s.label, trunc(c));
        }
        for c in deletes.iter().take(n) {
            eprintln!("  - {} {}", s.label, trunc(c));
        }
    }
    NodePlan {
        report: rep,
        creates,
        updates,
        deletes,
        deleted,
    }
}

pub fn apply_nodes(g: &Graph, s: &NodeSlice, p: &mut NodePlan) -> Result<(), String> {
    let t = Instant::now();
    if !p.deletes.is_empty() {
        run_chains(g, &p.deletes)?;
        drain(
            g,
            "MATCH (n:KgTomb) WITH n LIMIT 2000 DETACH DELETE n",
            "nodesDeleted",
        )?;
    }
    if !p.creates.is_empty() {
        run_unwind(g, &p.creates, &format!("CREATE (n:{}) SET n += r", s.label))?;
    }
    run_chains(g, &p.updates)?;
    p.report.ms += t.elapsed().as_millis();
    Ok(())
}

// ----------------------------------------------------------------- edges ---

/// (typed src key, typed dst key, owned property values, number of copies).
pub type EdgeRow = (Value, Value, Vec<Value>, usize);

pub struct EdgeCurrent {
    /// (src key, dst key) → its row.
    pub edges: HashMap<(String, String), EdgeRow>,
    pub ms: u128,
}

pub fn read_edges(g: &Graph, s: &EdgeSlice) -> Result<EdgeCurrent, String> {
    let t = Instant::now();
    let (sl, sk) = s.src;
    let (dl, dk) = s.dst;
    // Grouped by source: the source key is not repeated per edge (25–42 % less
    // to serialise, measured). Duplicated edges stay duplicated in the list.
    let ret = if s.owned.is_empty() {
        format!("RETURN a.{sk}, collect(b.{dk})")
    } else {
        let props: Vec<String> = s.owned.iter().map(|p| format!("e.{p}")).collect();
        format!("RETURN a.{sk}, collect([b.{dk}, {}])", props.join(", "))
    };
    let rows = partitioned(
        g,
        sl,
        "a",
        &format!("MATCH (a:{sl})-[e:{}]->(b:{dl})", s.rel),
        &s.filter,
        &ret,
    )?;
    let mut edges: HashMap<(String, String), EdgeRow> = HashMap::default();
    for r in rows {
        let (Some(a), Some(Value::Array(list))) = (r.first(), r.get(1)) else {
            continue;
        };
        if a.is_null() {
            continue;
        }
        let ak = key_str(a);
        for item in list {
            let (b, rest) = if s.owned.is_empty() {
                (item.clone(), Vec::new())
            } else {
                match item {
                    Value::Array(v) if !v.is_empty() => (v[0].clone(), v[1..].to_vec()),
                    _ => continue,
                }
            };
            if b.is_null() {
                continue;
            }
            let e = edges
                .entry((ak.clone(), key_str(&b)))
                .or_insert((a.clone(), b, rest, 0));
            e.3 += 1;
        }
    }
    Ok(EdgeCurrent {
        edges,
        ms: t.elapsed().as_millis(),
    })
}

/// The keys of a label once the node plans are applied, without copying them:
/// (before − deleted) ∪ desired ∪ extra.
pub struct KeyView<'a> {
    pub before: Option<&'a HashSet<String>>,
    pub gone: Option<&'a HashSet<String>>,
    pub desired: Vec<&'a HashMap<String, (Value, Props)>>,
    pub extra: Option<&'a HashSet<String>>,
}

impl KeyView<'_> {
    pub fn contains(&self, k: &str) -> bool {
        self.desired.iter().any(|d| d.contains_key(k))
            || self.extra.is_some_and(|e| e.contains(k))
            || (self.before.is_some_and(|b| b.contains(k))
                && !self.gone.is_some_and(|g| g.contains(k)))
    }

    pub fn is_gone(&self, k: &str) -> bool {
        self.gone.is_some_and(|g| g.contains(k))
    }
}

pub struct EdgePlan {
    pub report: Report,
    deletes: Vec<String>,
    updates: Vec<String>,
    creates: Vec<Edge>,
}

fn endpoint(label: &str, key: &str, v: &Value, var: &str, out: &mut String) {
    out.push_str(&format!("({var}:{label} {{"));
    ident(key, out);
    out.push(':');
    lit(v, out);
    out.push_str("})");
}

/// `src`/`dst`: the endpoint labels' keys once the node plans are applied (an
/// edge whose node the plans delete goes with it).
pub fn plan_edges(
    s: &EdgeSlice,
    desired: Vec<Edge>,
    cur: EdgeCurrent,
    src: &KeyView,
    dst: &KeyView,
    stamp: &Stamp,
) -> EdgePlan {
    let (sl, sk) = s.src;
    let (dl, dk) = s.dst;
    let mut rep = Report {
        slice: format!("(:{sl})-[:{}]->(:{dl})", s.rel),
        current: cur.edges.values().map(|v| v.3).sum(),
        ms: cur.ms,
        ..Default::default()
    };
    let mut want: HashMap<(String, String), Edge> =
        HashMap::with_capacity_and_hasher(desired.len(), Default::default());
    for e in desired {
        let k = (key_str(&e.src), key_str(&e.dst));
        if !src.contains(&k.0) || !dst.contains(&k.1) {
            rep.unresolved += 1;
            continue;
        }
        want.entry(k).or_insert(e);
    }
    rep.desired = want.len();
    let mark = |a: &Value, b: &Value| {
        let mut c = String::from("MATCH ");
        endpoint(sl, sk, a, "a", &mut c);
        c.push_str(&format!("-[e:{}]->", s.rel));
        endpoint(dl, dk, b, "b", &mut c);
        c.push_str(&format!(
            " WHERE {} SET e.kgTomb = true, a:KgDirty",
            s.filter
        ));
        c
    };
    let mut deletes = Vec::new();
    let mut updates = Vec::new();
    let mut creates = Vec::new();
    // Stale edges: current and not wanted. Edges of deleted nodes go with them.
    for (k, (a, b, _, count)) in &cur.edges {
        if want.contains_key(k) {
            continue;
        }
        rep.deleted += count;
        if !src.is_gone(&k.0) && !dst.is_gone(&k.1) {
            deletes.push(mark(a, b));
        }
    }
    for (k, e) in want {
        match cur.edges.get(&k) {
            None => creates.push(e),
            Some((a, b, have, count)) => {
                if *count > 1 {
                    // Duplicates inside the slice: drop them all, write one.
                    deletes.push(mark(a, b));
                    rep.deleted += count;
                    creates.push(e);
                } else if let Some(d) = props_differ(&s.owned, &e.props, have) {
                    let mut c = String::from("MATCH ");
                    endpoint(sl, sk, a, "a", &mut c);
                    c.push_str(&format!("-[e:{}]->", s.rel));
                    endpoint(dl, dk, b, "b", &mut c);
                    c.push_str(&format!(" WHERE {} SET ", s.filter));
                    for (i, (p, v)) in stamped(d, stamp).iter().enumerate() {
                        if i > 0 {
                            c.push_str(", ");
                        }
                        c.push_str("e.");
                        ident(p, &mut c);
                        c.push_str(" = ");
                        lit(v, &mut c);
                    }
                    updates.push(c);
                }
            }
        }
    }
    rep.created = creates.len();
    rep.updated = updates.len();
    if let Some(n) = sample() {
        for e in creates.iter().take(n) {
            eprintln!("  + {} {} -> {}", s.rel, e.src, e.dst);
        }
        for c in updates.iter().take(n) {
            eprintln!("  ~ {}", trunc(c));
        }
        for c in deletes.iter().take(n) {
            eprintln!("  - {}", trunc(c));
        }
    }
    EdgePlan {
        report: rep,
        deletes,
        updates,
        creates,
    }
}

pub fn apply_edges(
    g: &Graph,
    s: &EdgeSlice,
    p: &mut EdgePlan,
    stamp: &Stamp,
) -> Result<(), String> {
    let t = Instant::now();
    let (sl, sk) = s.src;
    let (dl, dk) = s.dst;
    if !p.deletes.is_empty() {
        run_chains(g, &p.deletes)?;
        drain(
            g,
            &format!("MATCH (a:KgDirty)-[e:{}]->(b:{dl}) WHERE e.kgTomb = true WITH e LIMIT 20000 DELETE e", s.rel),
            "relationshipsDeleted",
        )?;
        drain(
            g,
            "MATCH (a:KgDirty) WITH a LIMIT 20000 REMOVE a:KgDirty",
            "labelsRemoved",
        )?;
    }
    run_chains(g, &p.updates)?;
    let clean = |e: &Edge| -> Props {
        stamped(e.props.clone(), stamp)
            .into_iter()
            .filter(|(_, v)| !v.is_null())
            .collect()
    };
    if p.creates.len() <= 3 * CHAIN {
        let mut chain = Vec::new();
        for e in &p.creates {
            let mut c = String::from("MATCH ");
            endpoint(sl, sk, &e.src, "a", &mut c);
            c.push_str(" MATCH ");
            endpoint(dl, dk, &e.dst, "b", &mut c);
            c.push_str(&format!(" CREATE (a)-[e:{} ", s.rel));
            lit(&Value::Object(clean(e)), &mut c);
            c.push_str("]->(b)");
            chain.push(c);
        }
        run_chains(g, &chain)?;
    } else {
        let mut rows = Vec::with_capacity(p.creates.len());
        for e in &p.creates {
            let mut r = String::from("{a:");
            lit(&e.src, &mut r);
            r.push_str(",b:");
            lit(&e.dst, &mut r);
            r.push_str(",p:");
            lit(&Value::Object(clean(e)), &mut r);
            r.push('}');
            rows.push(r);
        }
        run_unwind(
            g,
            &rows,
            &format!("MATCH (a:{sl} {{{sk}: r.a}}) MATCH (b:{dl} {{{dk}: r.b}}) CREATE (a)-[e:{}]->(b) SET e += r.p", s.rel),
        )?;
    }
    p.report.ms += t.elapsed().as_millis();
    Ok(())
}
