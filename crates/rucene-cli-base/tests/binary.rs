//! The built `rucene-base` binary end to end: exit codes, standard output
//! and standard error.

mod common;

use std::process::{Command, Output};

use common::{fixture_path, fixture_rows, temp_dir, Tsv};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rucene-base"))
        .args(args)
        .output()
        .expect("spawn rucene-base")
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn create_update_search_get_delete_through_the_binary() {
    let root = temp_dir("binary");
    let root = root.to_str().unwrap();
    let rows = fixture_rows("contratos");
    stdout(&cli(&["contratos", "create", root]));
    let updated = stdout(&cli(&[
        "contratos",
        "update",
        root,
        "--file",
        fixture_path("contratos").to_str().unwrap(),
    ]));
    assert!(updated.starts_with(&format!("updated {} document(s)", rows.len())));
    let a = Tsv::parse(&stdout(&cli(&[
        "contratos",
        "search",
        root,
        "--text",
        "AÇÃO",
        "--limit",
        "100",
    ])));
    let b = Tsv::parse(&stdout(&cli(&[
        "contratos",
        "search",
        root,
        "--text",
        "acao",
        "--limit",
        "100",
    ])));
    assert!(a.total_hits > 0);
    assert_eq!(a.column_set("idcontrato"), b.column_set("idcontrato"));
    let key = rows[0].one("idcontrato");
    let got = stdout(&cli(&["contratos", "get", root, "--key", &key]));
    assert!(
        got.starts_with(&format!("{{\"idcontrato\": \"{key}\"")),
        "{got}"
    );
    stdout(&cli(&["contratos", "delete", root, "--key", &key]));
    let missing = cli(&["contratos", "get", root, "--key", &key]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr).trim_end(),
        format!("rucene-base: error: no contratos document with key(s) \"{key}\"")
    );
}

#[test]
fn usage_errors_exit_2_and_input_errors_exit_1() {
    for args in [
        &[][..],
        &["bogus"][..],
        &["contratos"][..],
        &["contratos", "frobnicate", "x"][..],
        &["contratos", "search"][..],
        &["contratos", "delete", "x"][..],
        &["cpv", "add", "x", "--file", "a", "--set", "cpv=1"][..],
        &["cpv", "search", "x", "--sort", "cpv:sideways"][..],
    ] {
        let output = cli(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("rucene-base: error: "), "{stderr}");
        assert!(stderr.contains("run `rucene-base help`"), "{stderr}");
        assert!(output.stdout.is_empty());
    }
    let root = temp_dir("binary-errors");
    let root = root.to_str().unwrap();
    let output = cli(&["nuts", "count", root]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("rucene-base nuts create"));
    let help = stdout(&cli(&["help"]));
    for schema in rucene_cli_base::entities::ALL {
        assert!(
            help.contains(&format!("  {}", schema.name)),
            "{}",
            schema.name
        );
    }
    assert!(stdout(&cli(&["anuncios", "help"])).contains("key: IDAnuncio"));
}
