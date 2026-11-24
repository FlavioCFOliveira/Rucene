//! `kgsync` — fast, differential synchronisation of the Rucene knowledge graph.
//!
//! Every layer computes the *desired* state of the slices it owns from primary
//! sources (the Rust AST, the Java AST, the doc comments, `git`, `rmp`), all in
//! parallel; the reconciler reads each slice's *current* state in one labelled
//! statement and writes only the difference. Nothing is rebuilt; an unchanged
//! tree writes nothing. See `tools/kgsync/README.md`.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod gitlayer;
mod graph;
mod java;
mod layers;
mod mapping;
mod reconcile;
mod rust_ast;

use graph::Graph;
use reconcile::Stamp;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

/// Bumped whenever an extractor's output changes for the same inputs, so that
/// the next run re-derives every layer.
pub const FINGERPRINT_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "/1");

pub struct Opts {
    pub roadmap: String,
    pub repo: PathBuf,
    pub layers: Vec<String>,
    pub dry_run: bool,
    pub schema: bool,
    pub allow_dirty: bool,
    pub verbose: bool,
    pub full: bool,
}

fn git(repo: &PathBuf, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn usage() -> ! {
    eprintln!(
        "usage: kgsync [--roadmap NAME] [--repo DIR] [--layers rust,java,map,git] \
         [--dry-run] [--full] [--schema] [--allow-dirty] [--verbose]\n\
         default layers: rust,java,map,git; a layer whose inputs are unchanged since the\n\
         last sync is skipped (--full re-derives and verifies every layer)"
    );
    std::process::exit(2)
}

fn main() {
    let mut o = Opts {
        roadmap: "rucene".into(),
        repo: PathBuf::from("."),
        layers: vec!["rust".into(), "java".into(), "map".into(), "git".into()],
        dry_run: false,
        schema: false,
        allow_dirty: false,
        verbose: false,
        full: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--roadmap" => o.roadmap = args.next().unwrap_or_else(|| usage()),
            "--repo" => o.repo = PathBuf::from(args.next().unwrap_or_else(|| usage())),
            "--layers" => {
                o.layers = args
                    .next()
                    .unwrap_or_else(|| usage())
                    .split(',')
                    .map(String::from)
                    .collect()
            }
            "--dry-run" => o.dry_run = true,
            "--schema" => o.schema = true,
            "--allow-dirty" => o.allow_dirty = true,
            "--verbose" | "-v" => o.verbose = true,
            "--full" => o.full = true,
            _ => usage(),
        }
    }
    let t0 = Instant::now();
    let repo = std::fs::canonicalize(&o.repo).expect("repo");
    o.repo = repo.clone();
    let dirty = git(&repo, &["status", "--porcelain", "--untracked-files=no"]);
    if !dirty.is_empty() && !o.allow_dirty && !o.dry_run {
        eprintln!("kgsync: the working tree has uncommitted changes; commit first (or pass --allow-dirty)");
        std::process::exit(1);
    }
    let stamp = Stamp {
        commit: git(&repo, &["rev-parse", "HEAD"]),
        date: git(&repo, &["show", "-s", "--format=%cs", "HEAD"]),
    };
    let g = Graph::new(&o.roadmap, o.dry_run);
    if let Err(e) = g.rows("RETURN 1") {
        eprintln!(
            "kgsync: graph server not reachable ({e}); start it: rmp graph serve -r {}",
            o.roadmap
        );
        std::process::exit(1);
    }
    if o.schema {
        if let Err(e) = layers::schema(&g) {
            eprintln!("kgsync: schema: {e}");
            std::process::exit(1);
        }
    }
    match layers::run(&g, &o, &stamp) {
        Ok(reports) => {
            let (mut c, mut u, mut d) = (0, 0, 0);
            for r in &reports {
                c += r.created;
                u += r.updated;
                d += r.deleted;
                if o.verbose || r.created + r.updated + r.deleted + r.unresolved > 0 {
                    println!("{}", r.line());
                }
            }
            println!(
                "kgsync {} @ {}: {} slices, +{c} ~{u} -{d}{}; {} statements (read {} ms, write {} ms); total {:.2} s",
                o.layers.join(","),
                &stamp.commit[..12.min(stamp.commit.len())],
                reports.len(),
                if o.dry_run { " (dry run: nothing written)" } else { "" },
                g.statements.load(std::sync::atomic::Ordering::Relaxed),
                g.read_ms.load(std::sync::atomic::Ordering::Relaxed),
                g.write_ms.load(std::sync::atomic::Ordering::Relaxed),
                t0.elapsed().as_secs_f64()
            );
        }
        Err(e) => {
            eprintln!("kgsync: {e}");
            std::process::exit(1);
        }
    }
}
