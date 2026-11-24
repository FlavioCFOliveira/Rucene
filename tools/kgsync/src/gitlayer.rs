//! History and planning: every commit reachable from `HEAD` (`Commit`), the
//! files each commit changed (`COMMITTED_IN`), the task a commit closes
//! (`CLOSES_TASK`: `rmp`'s `commit_close` first, else a `Closes`/`Fixes` footer,
//! a `Task:` trailer or a subject tag) and the tasks it refers to
//! (`REFERENCES_TASK`: `Refs #N`), and every `rmp` task (`Task`).
//!
//! `rmp task list` returns at most 100 tasks, so the tasks are read from the
//! roadmap's own store (`~/.roadmaps/<name>/project.db`), read-only.

use crate::graph::Graph;
use crate::layers::{edge, props, s, Desired};
use crate::reconcile::{EdgeSlice, NodeSlice, Props};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use serde_json::{json, Value};

use std::path::Path;
use std::process::Command;

struct GitCommit {
    hash: String,
    date: String,
    subject: String,
    body: String,
    files: Vec<String>,
}

fn commits(repo: &Path) -> Result<Vec<GitCommit>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--no-renames",
            "--name-only",
            "--format=%x1e%H%x1f%aI%x1f%s%x1f%b%x1f",
        ])
        .output()
        .map_err(|e| format!("git log: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for rec in text.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let parts: Vec<&str> = rec.splitn(5, '\u{1f}').collect();
        if parts.len() < 5 {
            continue;
        }
        v.push(GitCommit {
            hash: parts[0].trim().to_string(),
            date: parts[1].to_string(),
            subject: parts[2].to_string(),
            body: parts[3].to_string(),
            files: parts[4]
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
        });
    }
    Ok(v)
}

/// Tasks from the roadmap store: (id, title, status, priority, commit_close).
/// (id, title, status, priority, commit_close).
type TaskRow = (i64, String, String, String, Option<String>);

fn tasks(roadmap: &str) -> Result<Vec<TaskRow>, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let db = format!("{home}/.roadmaps/{roadmap}/project.db");
    let script = "import sqlite3,json,sys\n\
c=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True)\n\
print(json.dumps([list(r) for r in c.execute('select id,title,status,priority,commit_close from tasks order by id')]))";
    let out = Command::new("python3")
        .args(["-I", "-c", script, &db])
        .output()
        .map_err(|e| format!("python3: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "reading {db}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let rows: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    Ok(rows
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| {
            Some((
                r.get(0)?.as_i64()?,
                r.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                r.get(2).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                r.get(3).map(|v| match v {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                })?,
                r.get(4).and_then(|v| v.as_str()).map(|x| x.to_lowercase()),
            ))
        })
        .collect())
}

/// The roadmap store's size and modification time: any task change moves them.
pub fn tasks_fingerprint(roadmap: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::fs::metadata(format!("{home}/.roadmaps/{roadmap}/project.db")) {
        Ok(m) => format!(
            "{}:{}",
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ),
        Err(_) => String::new(),
    }
}

/// Task numbers after `#` in a text fragment.
fn task_refs(s: &str) -> Vec<i64> {
    let mut v = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 {
                if let Ok(n) = s[i + 1..j].parse() {
                    v.push(n);
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    v
}

/// The task a commit message says it closes, and the tasks it refers to.
fn message_tasks(c: &GitCommit) -> (Option<i64>, Vec<i64>) {
    let mut closes = None;
    let mut refs = Vec::new();
    for line in c.body.lines() {
        let t = line.trim();
        let lower = t.to_lowercase();
        if ["closes", "close", "fixes", "fix", "task:"]
            .iter()
            .any(|k| lower.starts_with(k))
        {
            if closes.is_none() {
                closes = task_refs(t).first().copied();
            }
        } else if lower.starts_with("refs") {
            refs.extend(task_refs(t));
        }
    }
    if closes.is_none() {
        if let Some(first) = c.body.lines().map(str::trim).find(|l| !l.is_empty()) {
            if first.starts_with('#') || first.starts_with("[#") {
                closes = task_refs(first).first().copied();
            }
        }
    }
    if closes.is_none() {
        closes = task_refs(&c.subject).first().copied();
    }
    refs.sort();
    refs.dedup();
    (closes, refs)
}

/// `spec_only`: declare the slices with no data (see `mapping::layer`).
pub fn layer(
    _g: &Graph,
    d: &mut Desired,
    repo: &Path,
    roadmap: &str,
    spec_only: bool,
) -> Result<(), String> {
    let cs = if spec_only {
        Vec::new()
    } else {
        commits(repo)?
    };
    let ts = if spec_only {
        Vec::new()
    } else {
        tasks(roadmap)?
    };
    let task_ids: HashSet<i64> = ts.iter().map(|t| t.0).collect();

    let task_i = d.node_slice(NodeSlice {
        label: "Task",
        key: "id",
        owned: vec!["name", "status", "priority"],
        filter: "true".into(),
    });
    for (id, title, status, prio, _) in &ts {
        d.nodes[task_i].1.insert(
            id.to_string(),
            (
                json!(id),
                props(&[
                    ("name", s(title)),
                    ("status", s(status)),
                    ("priority", s(prio)),
                ]),
            ),
        );
    }
    // rmp's closing commit (short hash) → full hash.
    let mut closed_by: HashMap<String, Vec<i64>> = HashMap::default();
    let mut unresolved_close = 0;
    for (id, _, _, _, cc) in &ts {
        if let Some(h) = cc {
            match cs.iter().find(|c| c.hash.starts_with(h.as_str())) {
                Some(c) => closed_by.entry(c.hash.clone()).or_default().push(*id),
                None => unresolved_close += 1,
            }
        }
    }
    if unresolved_close > 0 {
        eprintln!(
            "kgsync: {unresolved_close} rmp commit_close hashes are not in the local history"
        );
    }
    let rmp_closed: HashSet<i64> = closed_by.values().flatten().copied().collect();

    let commit_i = d.node_slice(NodeSlice {
        label: "Commit",
        key: "hash",
        owned: vec!["message", "date", "task_id"],
        filter: "true".into(),
    });
    let mut committed = Vec::new();
    let mut closes = Vec::new();
    let mut references = Vec::new();
    for c in &cs {
        let (msg_close, refs) = message_tasks(c);
        let mut close: Vec<i64> = closed_by.get(&c.hash).cloned().unwrap_or_default();
        if let Some(t) = msg_close.filter(|t| task_ids.contains(t) && !rmp_closed.contains(t)) {
            close.push(t);
        }
        close.sort();
        close.dedup();
        d.nodes[commit_i].1.insert(
            c.hash.clone(),
            (
                s(&c.hash),
                props(&[
                    ("message", s(&c.subject)),
                    ("date", s(&c.date)),
                    (
                        "task_id",
                        close.first().map(|t| json!(t)).unwrap_or(Value::Null),
                    ),
                ]),
            ),
        );
        for f in &c.files {
            committed.push(edge(s(f), s(&c.hash), Props::new()));
        }
        for t in &close {
            closes.push(edge(s(&c.hash), json!(t), Props::new()));
        }
        for t in refs {
            if task_ids.contains(&t) && !close.contains(&t) {
                references.push(edge(s(&c.hash), json!(t), Props::new()));
            }
        }
    }
    let i = d.edge_slice(EdgeSlice {
        rel: "COMMITTED_IN",
        src: ("File", "path"),
        dst: ("Commit", "hash"),
        owned: vec![],
        filter: "true".into(),
    });
    d.edges[i].1 = committed;
    let i = d.edge_slice(EdgeSlice {
        rel: "CLOSES_TASK",
        src: ("Commit", "hash"),
        dst: ("Task", "id"),
        owned: vec![],
        filter: "true".into(),
    });
    d.edges[i].1 = closes;
    let i = d.edge_slice(EdgeSlice {
        rel: "REFERENCES_TASK",
        src: ("Commit", "hash"),
        dst: ("Task", "id"),
        owned: vec![],
        filter: "true".into(),
    });
    d.edges[i].1 = references;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(subject: &str, body: &str) -> GitCommit {
        GitCommit {
            hash: "h".into(),
            date: String::new(),
            subject: subject.into(),
            body: body.into(),
            files: vec![],
        }
    }

    #[test]
    fn closing_and_referenced_tasks_come_from_footers_trailers_and_tags() {
        assert_eq!(
            message_tasks(&commit("feat: x", "text\n\nCloses #12\nRefs #3, #4")),
            (Some(12), vec![3, 4])
        );
        assert_eq!(message_tasks(&commit("fix: y (#7)", "")), (Some(7), vec![]));
        assert_eq!(
            message_tasks(&commit("chore", "Task: #9")),
            (Some(9), vec![])
        );
        assert_eq!(
            message_tasks(&commit("chore", "#5: first line tag")),
            (Some(5), vec![])
        );
        assert_eq!(task_refs("a #1 b #22 c # d"), vec![1, 22]);
    }
}
