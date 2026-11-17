//! The index layout under the root (user requirement for #532/#533): each
//! index lives only in `<root>/<entity>`, `create` never touches siblings,
//! and no file is ever written in the root itself.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use rucene_cli_base::entities;

use common::{fixture_path, path_str, run, temp_dir};

fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect()
}

#[test]
fn two_indices_under_one_root_keep_their_files_apart() {
    let root = temp_dir("layout-two");
    for entity in ["contratos", "cpv"] {
        run(&[entity, "create", path_str(&root)]);
        run(&[
            entity,
            "update",
            path_str(&root),
            "--file",
            fixture_path(entity).to_str().unwrap(),
        ]);
    }
    // The root holds the two entity directories and nothing else.
    assert_eq!(
        entries(&root),
        ["contratos", "cpv"].iter().map(|s| s.to_string()).collect()
    );
    for entity in ["contratos", "cpv"] {
        let dir = root.join(entity);
        assert!(dir.is_dir());
        let files = entries(&dir);
        assert!(
            files.iter().any(|file| file.starts_with("segments_")),
            "{entity}: {files:?}"
        );
        assert!(
            std::fs::read_dir(&dir).unwrap().all(|entry| entry
                .unwrap()
                .file_type()
                .unwrap()
                .is_file()),
            "{entity}: an index directory holds files only"
        );
    }
    // Both stay readable, each with its own documents.
    for entity in ["contratos", "cpv"] {
        let lines = std::fs::read_to_string(fixture_path(entity))
            .unwrap()
            .lines()
            .count();
        let documents = run(&[entity, "count", path_str(&root)]);
        assert_eq!(documents.trim(), lines.to_string(), "{entity}");
    }
    // Recreating one leaves the other untouched.
    let before = entries(&root.join("cpv"));
    run(&["contratos", "create", path_str(&root)]);
    assert_eq!(run(&["contratos", "count", path_str(&root)]).trim(), "0");
    assert_eq!(entries(&root.join("cpv")), before);
    let cpv_lines = std::fs::read_to_string(fixture_path("cpv"))
        .unwrap()
        .lines()
        .count();
    assert_eq!(
        run(&["cpv", "count", path_str(&root)]).trim(),
        cpv_lines.to_string()
    );
}

#[test]
fn every_entity_creates_its_own_fixed_subdirectory() {
    let root = temp_dir("layout-all").join("nested").join("root");
    for schema in entities::ALL {
        run(&[schema.name, "create", path_str(&root)]);
    }
    let expected: BTreeSet<String> = entities::ALL
        .iter()
        .map(|schema| schema.name.to_owned())
        .collect();
    assert_eq!(entries(&root), expected);
    assert_eq!(expected.len(), 10);
    for schema in entities::ALL {
        let stats = run(&[schema.name, "stats", path_str(&root)]);
        assert!(stats.contains(&format!(
            "directory: {}\n",
            root.join(schema.name).display()
        )));
        assert!(stats.contains("documents: 0\n"));
    }
}

#[test]
fn an_index_of_one_entity_is_not_visible_to_another() {
    let root = temp_dir("layout-isolated");
    run(&["nuts", "create", path_str(&root)]);
    let args: Vec<String> = ["locais", "count", path_str(&root)]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = Vec::new();
    let error = rucene_cli_base::cli::run(&args, &mut out).unwrap_err();
    assert!(
        error.to_string().contains("no locais index found"),
        "{error}"
    );
    assert_eq!(entries(&root), ["nuts".to_owned()].into_iter().collect());
}
