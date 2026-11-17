//! Every subcommand of `contratos` on the real-data fixture, and the case,
//! accent and diacritic insensitivity of its text search.

mod common;

use std::collections::BTreeSet;

use rucene_cli_base::json::{self, JsonValue};
use rucene_cli_base::CliError;

use common::{
    fixture_path, fixture_rows, keys_with_word, loaded_root, path_str, run, run_both, run_err,
    search, set, temp_dir, words, Row,
};

const E: &str = "contratos";

/// The default text fields of `contratos`, as documented.
const TEXT_FIELDS: &[&str] = &[
    "objectoContrato",
    "descContrato",
    "adjudicanteNome",
    "adjudicatarioNome",
    "cpvDescricao",
];

fn ids(rows: &[&Row]) -> Vec<String> {
    rows.iter().map(|row| row.one("idcontrato")).collect()
}

fn stat(root: &std::path::Path, line: &str) -> i64 {
    run(&[E, "stats", path_str(root)])
        .lines()
        .find_map(|text| text.strip_prefix(&format!("{line}: ")))
        .unwrap_or_else(|| panic!("no {line} line"))
        .parse()
        .unwrap()
}

fn count(root: &std::path::Path, args: &[&str]) -> i64 {
    let mut all = vec![E, "count", path_str(root)];
    all.extend_from_slice(args);
    run(&all).trim().parse().unwrap()
}

fn get_json(root: &std::path::Path, key: &str) -> Vec<(String, JsonValue)> {
    let output = run(&[E, "get", path_str(root), "--key", key]);
    let mut lines = output.lines();
    let line = lines.next().expect("one document");
    assert!(lines.next().is_none(), "exactly one document for {key}");
    match json::parse(line).unwrap() {
        JsonValue::Object(members) => members,
        other => panic!("not an object: {other:?}"),
    }
}

fn member<'a>(members: &'a [(String, JsonValue)], name: &str) -> Option<&'a JsonValue> {
    members
        .iter()
        .find(|(member, _)| member == name)
        .map(|(_, value)| value)
}

#[test]
fn create_makes_only_the_entity_directory_and_recreates() {
    let root = temp_dir("c-create");
    let output = run(&[E, "create", path_str(&root)]);
    assert_eq!(
        output.trim_end(),
        format!(
            "created empty contratos index at {}",
            root.join(E).display()
        )
    );
    assert_eq!(stat(&root, "documents"), 0);
    run(&[E, "add", path_str(&root), "--set", "idcontrato=1"]);
    assert_eq!(stat(&root, "documents"), 1);
    run(&[E, "create", path_str(&root)]);
    assert_eq!(stat(&root, "documents"), 0, "OpenMode.CREATE replaces");
    let entries: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(entries, vec![E.to_owned()]);
}

#[test]
fn commands_on_a_missing_index_fail_with_a_hint() {
    let root = temp_dir("c-missing");
    let error = run_err(&[E, "add", path_str(&root), "--set", "idcontrato=1"]);
    assert!(matches!(error, CliError::Input(_)));
    assert!(
        error.to_string().contains("rucene-base contratos create"),
        "{error}"
    );
    assert!(
        !root.join(E).exists(),
        "a failed command must not create the index"
    );
}

#[test]
fn add_appends_and_update_replaces_by_key() {
    let rows = fixture_rows(E);
    let root = temp_dir("c-add");
    let file = fixture_path(E);
    run(&[E, "create", path_str(&root)]);
    let output = run(&[E, "add", path_str(&root), "--file", file.to_str().unwrap()]);
    assert_eq!(
        output.trim_end(),
        format!(
            "added {} document(s) in {}",
            rows.len(),
            root.join(E).display()
        )
    );
    run(&[E, "add", path_str(&root), "--file", file.to_str().unwrap()]);
    assert_eq!(
        stat(&root, "documents"),
        2 * rows.len() as i64,
        "add ignores the key"
    );
    let output = run(&[
        E,
        "update",
        path_str(&root),
        "--file",
        file.to_str().unwrap(),
    ]);
    assert!(output.starts_with(&format!("updated {} document(s)", rows.len())));
    assert_eq!(
        stat(&root, "documents"),
        rows.len() as i64,
        "update dedupes"
    );
}

#[test]
fn update_keeps_the_last_record_of_a_repeated_key() {
    let root = temp_dir("c-dedupe");
    run(&[E, "create", path_str(&root)]);
    let file = root.join("dup.jsonl");
    std::fs::write(
        &file,
        "{\"idcontrato\": \"77\", \"precoContratual\": 1.5}\n\
         {\"idcontrato\": \"78\", \"precoContratual\": 2}\n\
         {\"idcontrato\": \"77\", \"precoContratual\": 3.25, \"objectoContrato\": \"Último\"}\n",
    )
    .unwrap();
    run(&[
        E,
        "update",
        path_str(&root),
        "--file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(stat(&root, "documents"), 2);
    let doc = get_json(&root, "77");
    assert_eq!(
        member(&doc, "precoContratual"),
        Some(&JsonValue::Number("3.25".into()))
    );
    assert_eq!(
        member(&doc, "objectoContrato"),
        Some(&JsonValue::String("Último".into()))
    );
}

#[test]
fn a_bad_line_rolls_back_the_whole_file() {
    let root = temp_dir("c-rollback");
    run(&[E, "create", path_str(&root)]);
    let file = root.join("bad.jsonl");
    std::fs::write(
        &file,
        "{\"idcontrato\": \"1\"}\n{\"idcontrato\": \"2\", \"dataPublicacao\": \"2025-02-30\"}\n",
    )
    .unwrap();
    let error = run_err(&[E, "add", path_str(&root), "--file", file.to_str().unwrap()]);
    let message = error.to_string();
    assert!(
        message.contains("bad.jsonl") && message.contains("line 2"),
        "{message}"
    );
    assert!(message.contains("dataPublicacao"), "{message}");
    assert_eq!(stat(&root, "documents"), 0, "nothing committed");
}

#[test]
fn set_pairs_build_one_record_with_lists_and_placeholders() {
    let root = temp_dir("c-set");
    run(&[E, "create", path_str(&root)]);
    run(&[
        E,
        "add",
        path_str(&root),
        "--set",
        "idcontrato=500",
        "--set",
        "cpv=45233140-2",
        "--set",
        "cpv=71311000-1",
        "--set",
        "adjudicatarioNif=-",
        "--set",
        "adjudicatarioNif=500000000",
        "--set",
        "prazoExecucao=99999999",
        "--set",
        "precoBaseProcedimento=0",
        "--set",
        "dataPublicacao=2025-03-04",
        "--set",
        "CritMateriais=Sim",
    ]);
    let doc = get_json(&root, "500");
    assert_eq!(
        member(&doc, "cpv"),
        Some(&JsonValue::Array(vec![
            JsonValue::String("45233140-2".into()),
            JsonValue::String("71311000-1".into())
        ]))
    );
    assert_eq!(
        member(&doc, "adjudicatarioNif"),
        Some(&JsonValue::Array(vec![JsonValue::String(
            "500000000".into()
        )]))
    );
    assert_eq!(member(&doc, "prazoExecucao"), None, "sentinel not indexed");
    assert_eq!(member(&doc, "precoBaseProcedimento"), None, "0 = unknown");
    assert_eq!(
        member(&doc, "dataPublicacao"),
        Some(&JsonValue::String("2025-03-04".into()))
    );
    assert_eq!(
        member(&doc, "CritMateriais"),
        Some(&JsonValue::String("sim".into()))
    );
    // Placeholders are never indexed, so they cannot be queried either.
    let error = run_err(&[
        E,
        "count",
        path_str(&root),
        "--term",
        "prazoExecucao=99999999",
    ]);
    assert!(error.to_string().contains("placeholder"), "{error}");
    assert_eq!(count(&root, &["--term", "cpvDivisao=45"]), 1);
    assert_eq!(count(&root, &["--term", "cpvGrupo=713"]), 1);
    assert_eq!(count(&root, &["--range", "prazoExecucao=0.."]), 0);
}

#[test]
fn delete_removes_by_key_and_ignores_unknown_keys() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-delete", E);
    let (a, b) = (rows[0].one("idcontrato"), rows[1].one("idcontrato"));
    let output = run(&[
        E,
        "delete",
        path_str(&root),
        "--key",
        &a,
        "--key",
        &b,
        "--key",
        "nope",
    ]);
    assert_eq!(
        output.trim_end(),
        format!(
            "deleted documents for 3 key(s) in {}",
            root.join(E).display()
        )
    );
    assert_eq!(count(&root, &[]), rows.len() as i64 - 2);
    assert_eq!(count(&root, &["--key", &a]), 0);
    let error = run_err(&[E, "delete", path_str(&root)]);
    assert!(matches!(error, CliError::Usage(_)));
}

#[test]
fn get_round_trips_the_fixture_values() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-get", E);
    for row in &rows {
        let key = row.one("idcontrato");
        let doc = get_json(&root, &key);
        assert_eq!(
            member(&doc, "idcontrato"),
            Some(&JsonValue::String(key.clone()))
        );
        assert_eq!(
            member(&doc, "objectoContrato"),
            Some(&JsonValue::String(row.one("objectoContrato")))
        );
        let price = match member(&doc, "precoContratual") {
            Some(JsonValue::Number(text)) => text.parse::<f64>().unwrap(),
            other => panic!("precoContratual: {other:?}"),
        };
        assert_eq!(price, row.one("precoContratual").parse::<f64>().unwrap());
        assert_eq!(
            member(&doc, "dataPublicacao"),
            Some(&JsonValue::String(row.one("dataPublicacao")))
        );
        let base: f64 = row.one("precoBaseProcedimento").parse().unwrap();
        assert_eq!(member(&doc, "precoBaseProcedimento").is_none(), base == 0.0);
        assert_eq!(
            member(&doc, "adjudicanteNif").is_some(),
            !row.strings("adjudicanteNif").is_empty()
        );
    }
    // The output is valid input: reloading it reproduces the same documents.
    let keys: Vec<String> = rows.iter().map(|row| row.one("idcontrato")).collect();
    let mut args = vec![E, "get", path_str(&root)];
    for key in &keys {
        args.push("--key");
        args.push(key);
    }
    let dump = run(&args);
    assert_eq!(dump.lines().count(), rows.len());
    let copy = temp_dir("c-get-copy");
    run(&[E, "create", path_str(&copy)]);
    let file = copy.join("dump.jsonl");
    std::fs::write(&file, &dump).unwrap();
    run(&[
        E,
        "update",
        path_str(&copy),
        "--file",
        file.to_str().unwrap(),
    ]);
    let mut args = vec![E, "get", path_str(&copy)];
    for key in &keys {
        args.push("--key");
        args.push(key);
    }
    assert_eq!(run(&args), dump);
}

#[test]
fn get_prints_what_it_finds_and_reports_missing_keys() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-get-missing", E);
    let key = rows[0].one("idcontrato");
    let (output, error) = run_both(&[E, "get", path_str(&root), "--key", &key, "--key", "0"]);
    assert_eq!(output.lines().count(), 1);
    let error = error.expect("a missing key is an error");
    assert!(matches!(error, CliError::Input(_)));
    assert_eq!(error.to_string(), "no contratos document with key(s) \"0\"");
}

#[test]
fn text_search_ignores_case_accents_and_cedilla() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-accents", E);
    // Words the chain reduces to `acao`, as written in the source.
    let expected = keys_with_word(
        &rows,
        "idcontrato",
        TEXT_FIELDS,
        &[
            "ação", "acao", "ações", "acoes", "açao", "acão", "acões", "açoes",
        ],
    );
    assert!(
        expected.len() >= 3,
        "the fixture holds ação/AÇÃO/ações: {expected:?}"
    );
    for query in [
        "acao", "ação", "AÇÃO", "Ação", "ACAO", "ações", "AÇÕES", "acoes", "ACOES",
    ] {
        let tsv = search(E, &root, &["--text", query, "--fields", "idcontrato"]);
        assert_eq!(tsv.column_set("idcontrato"), expected, "query {query:?}");
        assert_eq!(tsv.total_hits, expected.len() as i64);
    }
    // The pre-1990 spelling reduces to its own stem, `acca`.
    let old = keys_with_word(
        &rows,
        "idcontrato",
        TEXT_FIELDS,
        &[
            "acção", "acções", "accao", "accoes", "acçao", "accão", "acçoes", "accões",
        ],
    );
    assert!(!old.is_empty(), "the fixture holds ACÇÃO");
    for query in ["acção", "ACÇÃO", "accao", "acções", "accoes", "ACCOES"] {
        let tsv = search(E, &root, &["--text", query, "--fields", "idcontrato"]);
        assert_eq!(tsv.column_set("idcontrato"), old, "query {query:?}");
    }
}

#[test]
fn every_query_word_must_match_in_some_default_field() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-multi", E);
    // Words the chain reduces to `manutenca` and to `elevador`.
    let maint = keys_with_word(
        &rows,
        "idcontrato",
        TEXT_FIELDS,
        &["manutenção", "manutencao", "manutenções"],
    );
    let lifts = keys_with_word(
        &rows,
        "idcontrato",
        TEXT_FIELDS,
        &["elevadores", "elevador"],
    );
    let expected: BTreeSet<String> = maint.intersection(&lifts).cloned().collect();
    assert!(!expected.is_empty());
    let tsv = search(
        E,
        &root,
        &["--text", "MANUTENCAO elevadores", "--fields", "idcontrato"],
    );
    assert_eq!(tsv.column_set("idcontrato"), expected);
    // A stop word, accented or not, is dropped from the query.
    let plain = search(
        E,
        &root,
        &["--text", "elevadores", "--fields", "idcontrato"],
    );
    let stop = search(
        E,
        &root,
        &["--text", "nao elevadores não", "--fields", "idcontrato"],
    );
    assert_eq!(
        plain.column_set("idcontrato"),
        stop.column_set("idcontrato")
    );
    let error = run_err(&[E, "search", path_str(&root), "--text", "não de a"]);
    assert!(error.to_string().contains("no searchable terms"), "{error}");
}

#[test]
fn phrase_and_field_text_queries() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-phrase", E);
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| {
            TEXT_FIELDS.iter().any(|field| {
                row.strings(field).iter().any(|text| {
                    words(text)
                        .windows(2)
                        .any(|pair| pair[0] == "bens" && pair[1] == "alimentares")
                })
            })
        })
        .map(|row| row.one("idcontrato"))
        .collect();
    assert!(!expected.is_empty());
    let tsv = search(
        E,
        &root,
        &["--phrase", "BENS ALIMENTARES", "--fields", "idcontrato"],
    );
    assert_eq!(tsv.column_set("idcontrato"), expected);
    let reversed = search(
        E,
        &root,
        &["--phrase", "alimentares bens", "--fields", "idcontrato"],
    );
    assert_eq!(reversed.total_hits, 0, "order matters in a phrase");

    // --text-in restricts to one field.
    let entity = rows
        .iter()
        .find(|row| row.one("adjudicanteNome").contains("Município"))
        .or_else(|| {
            rows.iter()
                .find(|row| !row.one("adjudicanteNome").is_empty())
        })
        .unwrap();
    let name = entity.one("adjudicanteNome");
    let in_field = search(
        E,
        &root,
        &[
            "--phrase-in",
            &format!("adjudicanteNome={}", name.to_uppercase()),
            "--fields",
            "idcontrato",
        ],
    );
    let exact_name: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.strings("adjudicanteNome").contains(&name))
        .map(|row| row.one("idcontrato"))
        .collect();
    assert!(exact_name.is_subset(&in_field.column_set("idcontrato")));
    // The exact copy matches the name as written only.
    let exact = search(
        E,
        &root,
        &[
            "--term",
            &format!("adjudicanteNome={name}"),
            "--fields",
            "idcontrato",
        ],
    );
    assert_eq!(exact.column_set("idcontrato"), exact_name);
    let error = run_err(&[E, "search", path_str(&root), "--text-in", "cpv=45"]);
    assert!(
        error.to_string().contains("not a text or name field"),
        "{error}"
    );
}

#[test]
fn exact_keyword_flag_and_hierarchy_terms() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-terms", E);
    let keys = |predicate: &dyn Fn(&Row) -> bool| -> BTreeSet<String> {
        rows.iter()
            .filter(|row| predicate(row))
            .map(|row| row.one("idcontrato"))
            .collect()
    };
    // Exact id set.
    let (a, b) = (rows[2].one("idcontrato"), rows[5].one("idcontrato"));
    let tsv = search(
        E,
        &root,
        &[
            "--terms",
            &format!("idcontrato={a},{b}"),
            "--fields",
            "idcontrato",
        ],
    );
    assert_eq!(tsv.column_set("idcontrato"), set(&[&a, &b]));
    // Keyword code.
    let code = rows[0].one("cpv");
    let expected = keys(&|row| row.strings("cpv").contains(&code));
    assert_eq!(
        search(
            E,
            &root,
            &["--term", &format!("cpv={code}"), "--fields", "idcontrato"]
        )
        .column_set("idcontrato"),
        expected
    );
    // Derived CPV division, NUTS level 2 and district.
    let expected = keys(&|row| row.strings("cpv").iter().any(|cpv| cpv.starts_with("50")));
    assert!(!expected.is_empty());
    assert_eq!(
        search(
            E,
            &root,
            &["--term", "cpvDivisao=50", "--fields", "idcontrato"]
        )
        .column_set("idcontrato"),
        expected
    );
    let expected = keys(&|row| {
        row.strings("nuts")
            .iter()
            .any(|nuts| nuts.starts_with("PT16"))
    });
    assert!(!expected.is_empty());
    assert_eq!(
        search(
            E,
            &root,
            &["--term", "nutsNivel2=PT16", "--fields", "idcontrato"]
        )
        .column_set("idcontrato"),
        expected
    );
    let parts = |place: &str| {
        place
            .split(',')
            .map(|part| part.trim().to_owned())
            .collect::<Vec<_>>()
    };
    let expected = keys(&|row| {
        row.strings("localExecucao")
            .iter()
            .any(|place| parts(place).starts_with(&["Portugal".to_owned(), "Leiria".to_owned()]))
    });
    assert!(!expected.is_empty());
    assert_eq!(
        search(
            E,
            &root,
            &[
                "--term",
                "localNivel2=Portugal, Leiria",
                "--fields",
                "idcontrato"
            ]
        )
        .column_set("idcontrato"),
        expected
    );
    let expected = keys(&|row| {
        row.strings("localExecucao")
            .iter()
            .any(|place| parts(place)[0] == "Portugal")
    });
    assert_eq!(
        search(
            E,
            &root,
            &["--term", "localNivel1=Portugal", "--fields", "idcontrato"]
        )
        .column_set("idcontrato"),
        expected
    );
    // Flags: any spelling of the value.
    let expected = keys(&|row| row.one("CritMateriais") == "Não");
    for value in ["Não", "nao", "NAO", "false"] {
        assert_eq!(
            search(
                E,
                &root,
                &[
                    "--term",
                    &format!("CritMateriais={value}"),
                    "--fields",
                    "idcontrato"
                ]
            )
            .column_set("idcontrato"),
            expected,
            "{value}"
        );
    }
}

#[test]
fn numeric_and_date_ranges() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-ranges", E);
    let price = |row: &Row| row.one("precoContratual").parse::<f64>().unwrap();
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| (10_000.0..=50_000.0).contains(&price(row)))
        .map(|row| row.one("idcontrato"))
        .collect();
    assert!(!expected.is_empty());
    let tsv = search(
        E,
        &root,
        &[
            "--range",
            "precoContratual=10000..50000",
            "--fields",
            "idcontrato",
        ],
    );
    assert_eq!(tsv.column_set("idcontrato"), expected);
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| price(row) >= 100_000.0)
        .map(|row| row.one("idcontrato"))
        .collect();
    assert_eq!(
        search(
            E,
            &root,
            &[
                "--range",
                "precoContratual=100000..",
                "--fields",
                "idcontrato"
            ]
        )
        .column_set("idcontrato"),
        expected
    );
    // ISO dates compare as text in the same order as days.
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| {
            let date = row.one("dataPublicacao");
            date.as_str() >= "2025-01-01" && date.as_str() <= "2025-01-31"
        })
        .map(|row| row.one("idcontrato"))
        .collect();
    assert_eq!(
        search(
            E,
            &root,
            &[
                "--range",
                "dataPublicacao=2025-01-01..31/01/2025",
                "--fields",
                "idcontrato"
            ]
        )
        .column_set("idcontrato"),
        expected
    );
    let error = run_err(&[E, "search", path_str(&root), "--range", "cpv=1..2"]);
    assert!(
        error.to_string().contains("not numeric or a date"),
        "{error}"
    );
}

#[test]
fn sorts_by_number_date_and_keyword() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-sort", E);
    let mut by_price: Vec<&Row> = rows.iter().collect();
    // Stable: ties keep file order, which is index order (Lucene's tie-break).
    by_price.sort_by(|a, b| {
        let (a, b) = (
            a.one("precoContratual").parse::<f64>().unwrap(),
            b.one("precoContratual").parse::<f64>().unwrap(),
        );
        b.partial_cmp(&a).unwrap()
    });
    let tsv = search(
        E,
        &root,
        &[
            "--sort",
            "precoContratual:desc",
            "--fields",
            "idcontrato,precoContratual",
        ],
    );
    assert_eq!(tsv.column("idcontrato"), ids(&by_price));
    assert!(tsv.header.contains(&"sort:precoContratual:desc".to_owned()));

    // Dates ascending; contracts without the date sort last.
    let mut by_date: Vec<&Row> = rows.iter().collect();
    by_date.sort_by(|a, b| {
        let (a, b) = (a.one("dataFechoContrato"), b.one("dataFechoContrato"));
        match (a.is_empty(), b.is_empty()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => a.cmp(&b),
        }
    });
    let tsv = search(
        E,
        &root,
        &["--sort", "dataFechoContrato", "--fields", "idcontrato"],
    );
    assert_eq!(tsv.column("idcontrato"), ids(&by_date));
    let sort_values = tsv.column("sort:dataFechoContrato:asc");
    assert_eq!(sort_values[0], by_date[0].one("dataFechoContrato"));

    // A limit keeps the top of the order; exact ids cannot be sorted.
    let top = search(
        E,
        &root,
        &[
            "--sort",
            "precoContratual:desc",
            "--limit",
            "3",
            "--fields",
            "idcontrato",
        ],
    );
    assert_eq!(top.column("idcontrato"), ids(&by_price[..3]));
    let error = run_err(&[E, "search", path_str(&root), "--sort", "idcontrato"]);
    assert!(error.to_string().contains("no doc values"), "{error}");
    let error = run_err(&[E, "search", path_str(&root), "--sort", "objectoContrato"]);
    assert!(error.to_string().contains("no doc values"), "{error}");
}

#[test]
fn boolean_occurs_and_output_columns() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-bool", E);
    let all: BTreeSet<String> = rows.iter().map(|row| row.one("idcontrato")).collect();
    let division_50: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.strings("cpv").iter().any(|cpv| cpv.starts_with("50")))
        .map(|row| row.one("idcontrato"))
        .collect();
    let tsv = search(
        E,
        &root,
        &["--not", "--term", "cpvDivisao=50", "--fields", "idcontrato"],
    );
    assert_eq!(
        tsv.column_set("idcontrato"),
        all.difference(&division_50).cloned().collect()
    );
    let tsv = search(
        E,
        &root,
        &[
            "--should",
            "--term",
            "cpvDivisao=50",
            "--should",
            "--term",
            "cpvDivisao=45",
            "--fields",
            "idcontrato",
        ],
    );
    let either: BTreeSet<String> = rows
        .iter()
        .filter(|row| {
            row.strings("cpv")
                .iter()
                .any(|cpv| cpv.starts_with("50") || cpv.starts_with("45"))
        })
        .map(|row| row.one("idcontrato"))
        .collect();
    assert_eq!(tsv.column_set("idcontrato"), either);

    // Default columns, --fields all, multi-valued cells, derived fields.
    let tsv = search(E, &root, &[]);
    assert_eq!(tsv.total_hits, rows.len() as i64);
    assert_eq!(
        &tsv.header[..5],
        &[
            "idcontrato",
            "dataPublicacao",
            "precoContratual",
            "adjudicanteNome",
            "objectoContrato"
        ]
    );
    let multi = rows
        .iter()
        .find(|row| row.strings("cpv").len() > 1)
        .unwrap();
    let tsv = search(
        E,
        &root,
        &["--key", &multi.one("idcontrato"), "--fields", "all"],
    );
    assert_eq!(tsv.rows.len(), 1);
    assert_eq!(tsv.column("cpv")[0], multi.strings("cpv").join(" | "));
    assert!(tsv.header.contains(&"linkPecasProc".to_owned()));
    assert!(!tsv.header.contains(&"cpvDivisao".to_owned()));
    let error = run_err(&[E, "search", path_str(&root), "--fields", "cpvDivisao"]);
    assert!(error.to_string().contains("derived"), "{error}");
    let error = run_err(&[E, "search", path_str(&root), "--term", "objectoContrato=x"]);
    assert!(error.to_string().contains("--text-in"), "{error}");
    let error = run_err(&[E, "search", path_str(&root), "--term", "linkPecasProc=x"]);
    assert!(error.to_string().contains("stored only"), "{error}");
    let error = run_err(&[E, "search", path_str(&root), "--term", "bogus=x"]);
    assert!(error.to_string().contains("unknown field"), "{error}");
}

#[test]
fn count_and_stats_agree_with_search() {
    let rows = fixture_rows(E);
    let root = loaded_root("c-count", E);
    assert_eq!(count(&root, &[]), rows.len() as i64);
    let tsv = search(E, &root, &["--text", "aquisicao", "--exact-total"]);
    assert!(tsv.exact);
    assert_eq!(count(&root, &["--text", "AQUISIÇÃO"]), tsv.total_hits);
    let error = run_err(&[E, "count", path_str(&root), "--limit", "3"]);
    assert!(matches!(error, CliError::Usage(_)));
    let stats = run(&[E, "stats", path_str(&root)]);
    assert!(stats.starts_with("index: contratos\n"), "{stats}");
    assert!(stats.contains(&format!("documents: {}\n", rows.len())));
    assert!(stats.contains("segments: "));
}

#[test]
fn synthetic_records_pin_the_analysis_contract() {
    let root = temp_dir("c-synthetic");
    run(&[E, "create", path_str(&root)]);
    for (id, text) in [
        ("s1", "Aquisição de AÇÕES de formação"),
        ("s2", "Acao social escolar"),
        ("s3", "Ação"),
        ("s4", "Acção de sensibilização"),
        ("s5", "Manutenção de elevadores"),
        ("s6", "Café e pão"),
        ("s7", "Fornecimento de limões e pães"),
        ("s8", "Anzóis e papéis de música portuguesa"),
    ] {
        run(&[
            E,
            "add",
            path_str(&root),
            "--set",
            &format!("idcontrato={id}"),
            "--set",
            &format!("objectoContrato={text}"),
        ]);
    }
    let hits = |query: &str| {
        search(E, &root, &["--text", query, "--fields", "idcontrato"]).column_set("idcontrato")
    };
    // Singular and plural, accented or not: one stem.
    for query in ["ação", "acao", "AÇÃO", "ações", "AÇÕES", "acoes", "ACOES"] {
        assert_eq!(hits(query), set(&["s1", "s2", "s3"]), "{query}");
    }
    for query in ["acção", "accao", "accoes", "ACÇÕES"] {
        assert_eq!(hits(query), set(&["s4"]), "{query}");
    }
    // Unaccented plurals of -ão, -el and -ol words.
    assert_eq!(hits("limoes paes"), set(&["s7"]));
    assert_eq!(hits("LIMAO PAO"), set(&["s7"]));
    assert_eq!(hits("pao"), set(&["s6", "s7"]));
    assert_eq!(hits("anzois papeis musica"), set(&["s8"]));
    assert_eq!(hits("anzol papel"), set(&["s8"]));
    assert_eq!(hits("MANUTENCAO"), set(&["s5"]));
    assert_eq!(hits("formacao"), set(&["s1"]));
    assert_eq!(hits("cafe pao"), set(&["s6"]));
    assert_eq!(hits("CAFÉ PÃO"), set(&["s6"]));
}
