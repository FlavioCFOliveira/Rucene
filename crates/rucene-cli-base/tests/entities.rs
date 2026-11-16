//! Every subcommand of the nine indices besides `contratos`, on their
//! real-data fixtures, plus the accent-insensitive search of each index's
//! names and texts and the entity-specific fields (composite key, hierarchy
//! levels, foreign keys, string-encoded numbers, `DD/MM/YYYY` dates).

mod common;

use std::collections::BTreeSet;

use rucene_cli_base::entities;
use rucene_cli_base::schema::EntitySchema;

use common::{
    fixture_path, fixture_rows, fold_pt, loaded_root, path_str, run, run_both, run_err, search,
    temp_dir, words, Row,
};

const ENTITIES: &[&str] = &[
    "fornecedores",
    "adjudicantes",
    "procedimentos",
    "cpv",
    "locais",
    "nuts",
    "modificacoes",
    "anuncios",
    "convidadas",
];

fn schema(entity: &str) -> &'static EntitySchema {
    entities::by_name(entity).unwrap()
}

/// The key of a fixture row: its key fields joined with `/`.
fn key_of(entity: &str, row: &Row) -> String {
    schema(entity)
        .key
        .iter()
        .map(|field| row.one(field))
        .collect::<Vec<_>>()
        .join("/")
}

fn count(entity: &str, root: &std::path::Path, args: &[&str]) -> i64 {
    let mut all = vec![entity, "count", path_str(root)];
    all.extend_from_slice(args);
    run(&all).trim().parse().unwrap()
}

/// The keys `search` returns for `args`, read from the key columns.
fn keys(entity: &str, root: &std::path::Path, args: &[&str]) -> BTreeSet<String> {
    let schema = schema(entity);
    let fields = schema.key.join(",");
    let mut all = args.to_vec();
    all.extend_from_slice(&["--fields", &fields]);
    let tsv = search(entity, root, &all);
    let columns: Vec<Vec<String>> = schema.key.iter().map(|field| tsv.column(field)).collect();
    (0..tsv.rows.len())
        .map(|row| {
            columns
                .iter()
                .map(|column| column[row].clone())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

#[test]
fn every_index_supports_every_subcommand() {
    for entity in ENTITIES {
        let rows = fixture_rows(entity);
        assert!(rows.len() >= 10, "{entity}: fixture too small");
        let root = temp_dir(&format!("e-{entity}"));
        let created = run(&[entity, "create", path_str(&root)]);
        assert!(created.starts_with(&format!("created empty {entity} index at ")));

        // add, then update the same file: add ignores the key, update dedupes.
        let file = fixture_path(entity);
        run(&[
            entity,
            "add",
            path_str(&root),
            "--file",
            file.to_str().unwrap(),
        ]);
        assert_eq!(count(entity, &root, &[]), rows.len() as i64, "{entity}");
        run(&[
            entity,
            "add",
            path_str(&root),
            "--file",
            file.to_str().unwrap(),
        ]);
        assert_eq!(count(entity, &root, &[]), 2 * rows.len() as i64, "{entity}");
        run(&[
            entity,
            "update",
            path_str(&root),
            "--file",
            file.to_str().unwrap(),
        ]);
        assert_eq!(count(entity, &root, &[]), rows.len() as i64, "{entity}");

        // get: one document per key, holding the key fields as given.
        let all_keys: BTreeSet<String> = rows.iter().map(|row| key_of(entity, row)).collect();
        assert_eq!(
            all_keys.len(),
            rows.len(),
            "{entity}: fixture keys are unique"
        );
        for row in &rows {
            let key = key_of(entity, row);
            let output = run(&[entity, "get", path_str(&root), "--key", &key]);
            assert_eq!(output.lines().count(), 1, "{entity} {key}");
            for field in schema(entity).key {
                let value = row.one(field);
                assert!(
                    output.contains(&format!("\"{field}\": \"{value}\""))
                        || output.contains(&format!("\"{field}\": {value}")),
                    "{entity} {key}: {output}"
                );
            }
        }
        let (output, error) = run_both(&[entity, "get", path_str(&root), "--key", "no-such-key"]);
        assert!(output.is_empty() && error.is_some(), "{entity}");

        // search with no criteria lists everything; keys come back intact.
        assert_eq!(keys(entity, &root, &[]), all_keys, "{entity}");

        // update one record with --set: the whole document is replaced.
        let target = &rows[1];
        let target_key = key_of(entity, target);
        let mut args = vec![entity.to_string(), "update".into(), path_str(&root).into()];
        for field in schema(entity).key {
            args.push("--set".into());
            args.push(format!("{field}={}", target.one(field)));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&args);
        assert_eq!(count(entity, &root, &[]), rows.len() as i64, "{entity}");
        let output = run(&[entity, "get", path_str(&root), "--key", &target_key]);
        let expected_members = schema(entity).key.len();
        assert_eq!(
            output.matches("\": ").count(),
            expected_members,
            "{entity}: only the key fields remain: {output}"
        );

        // delete one key.
        let victim = key_of(entity, &rows[0]);
        run(&[entity, "delete", path_str(&root), "--key", &victim]);
        assert_eq!(count(entity, &root, &[]), rows.len() as i64 - 1, "{entity}");
        assert_eq!(count(entity, &root, &["--key", &victim]), 0, "{entity}");

        // stats.
        let stats = run(&[entity, "stats", path_str(&root)]);
        assert!(stats.starts_with(&format!("index: {entity}\n")), "{stats}");
        assert!(
            stats.contains(&format!("documents: {}\n", rows.len() - 1)),
            "{stats}"
        );
        assert!(stats.contains(&format!("directory: {}\n", root.join(entity).display())));

        // help lists the schema.
        let help = run(&[entity, "help"]);
        assert!(help.contains(&format!("index directory: <root>/{entity}")));
        assert!(help.contains("| Field | Kind |"));
    }
}

/// Whether `word` is a useful accent probe: four or more letters, at least
/// one of them accented.
fn accent_probe(word: &str) -> bool {
    word.chars().count() >= 4 && !word.is_ascii() && word.chars().all(char::is_alphabetic)
}

#[test]
fn names_and_texts_of_every_index_ignore_case_and_accents() {
    for entity in ENTITIES {
        let rows = fixture_rows(entity);
        let root = loaded_root(&format!("e-accent-{entity}"), entity);
        let text_fields = schema(entity).text_fields;
        // The first accented word of a default text field in the fixture.
        let (row, word) = rows
            .iter()
            .find_map(|row| {
                text_fields.iter().find_map(|field| {
                    row.strings(field)
                        .iter()
                        .flat_map(|text| words(text))
                        .find(|word| accent_probe(word))
                        .map(|word| (row, word))
                })
            })
            .unwrap_or_else(|| panic!("{entity}: no accented word in the fixture"));
        let as_written = keys(entity, &root, &["--text", &word]);
        assert!(
            as_written.contains(&key_of(entity, row)),
            "{entity}: {word}"
        );
        for variant in [
            fold_pt(&word),
            fold_pt(&word).to_uppercase(),
            word.to_uppercase(),
        ] {
            assert_eq!(
                keys(entity, &root, &["--text", &variant]),
                as_written,
                "{entity}: {word:?} vs {variant:?}"
            );
        }
    }
}

#[test]
fn hierarchy_levels_are_exact_terms() {
    // cpv: division and group of the key.
    let rows = fixture_rows("cpv");
    let root = loaded_root("e-cpv-levels", "cpv");
    let code = rows[0].one("cpv");
    for (field, digits) in [
        ("cpvDivisao", 2),
        ("cpvGrupo", 3),
        ("cpvClasse", 4),
        ("cpvCategoria", 5),
    ] {
        let prefix = &code[..digits];
        let expected: BTreeSet<String> = rows
            .iter()
            .map(|row| row.one("cpv"))
            .filter(|cpv| cpv.starts_with(prefix))
            .collect();
        assert_eq!(
            keys("cpv", &root, &["--term", &format!("{field}={prefix}")]),
            expected,
            "{field}"
        );
    }
    // The key sorts as a keyword.
    let mut sorted: Vec<String> = rows.iter().map(|row| row.one("cpv")).collect();
    sorted.sort();
    let tsv = search("cpv", &root, &["--sort", "cpv", "--fields", "cpv"]);
    assert_eq!(tsv.column("cpv"), sorted);

    // nuts: level 1 (3 characters) and level 2 (4); "PT" alone has no level.
    let rows = fixture_rows("nuts");
    let root = loaded_root("e-nuts-levels", "nuts");
    let expected: BTreeSet<String> = rows
        .iter()
        .map(|row| row.one("nuts"))
        .filter(|code| code.len() >= 3 && code.starts_with("PT1"))
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(keys("nuts", &root, &["--term", "nutsNivel1=PT1"]), expected);
    let expected: BTreeSet<String> = rows
        .iter()
        .map(|row| row.one("nuts"))
        .filter(|code| code.starts_with("PT16"))
        .collect();
    assert_eq!(
        keys("nuts", &root, &["--term", "nutsNivel2=PT16"]),
        expected
    );

    // locais: district as a path prefix, and the district name as text.
    let rows = fixture_rows("locais");
    let root = loaded_root("e-locais-levels", "locais");
    let district = rows
        .iter()
        .map(|row| row.one("distrito"))
        .find(|district| !district.is_empty())
        .unwrap();
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("distrito") == district && row.one("pais") == "Portugal")
        .map(|row| row.one("local"))
        .collect();
    assert_eq!(
        keys(
            "locais",
            &root,
            &["--term", &format!("localNivel2=Portugal, {district}")]
        ),
        expected
    );
    assert_eq!(
        keys(
            "locais",
            &root,
            &["--term", &format!("distrito={district}")]
        ),
        rows.iter()
            .filter(|row| row.one("distrito") == district)
            .map(|row| row.one("local"))
            .collect::<BTreeSet<_>>()
    );
    let evora: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("concelho") == "Évora" || row.one("distrito") == "Évora")
        .map(|row| row.one("local"))
        .collect();
    if !evora.is_empty() {
        assert_eq!(keys("locais", &root, &["--text", "EVORA"]), evora);
    }
}

#[test]
fn modifications_have_a_composite_key_and_day_month_year_dates() {
    let rows = fixture_rows("modificacoes");
    let root = loaded_root("e-mods", "modificacoes");
    let contract = rows
        .iter()
        .map(|row| row.one("idcontrato"))
        .find(|id| {
            rows.iter()
                .filter(|row| row.one("idcontrato") == *id)
                .count()
                >= 3
        })
        .expect("a contract with three modifications");
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("idcontrato") == contract)
        .map(|row| key_of("modificacoes", row))
        .collect();
    assert_eq!(
        keys(
            "modificacoes",
            &root,
            &["--term", &format!("idcontrato={contract}")]
        ),
        expected
    );
    let second = format!("{contract}/2");
    let output = run(&["modificacoes", "get", path_str(&root), "--key", &second]);
    assert!(output.contains("\"sequencia\": 2"), "{output}");
    // DD/MM/YYYY 00:00:00 is stored as YYYY-MM-DD and ranges by day.
    let first = &rows[0];
    let raw = first.one("modifContratoData");
    let (day, rest) = raw.split_at(2);
    let (month, year) = (&rest[1..3], &rest[4..8]);
    let iso = format!("{year}-{month}-{day}");
    let output = run(&[
        "modificacoes",
        "get",
        path_str(&root),
        "--key",
        &key_of("modificacoes", first),
    ]);
    assert!(
        output.contains(&format!("\"modifContratoData\": \"{iso}\"")),
        "{output}"
    );
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("modifContratoData").get(..10) == Some(&raw[..10]))
        .map(|row| key_of("modificacoes", row))
        .collect();
    assert_eq!(
        keys(
            "modificacoes",
            &root,
            &["--range", &format!("modifContratoData={iso}..{iso}")]
        ),
        expected
    );
    // The term given as digits in a string is an int.
    let term: i32 = first.one("modifPrazoExecucao").parse().unwrap();
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("modifPrazoExecucao").parse::<i32>().ok() == Some(term))
        .map(|row| key_of("modificacoes", row))
        .collect();
    assert_eq!(
        keys(
            "modificacoes",
            &root,
            &["--term", &format!("modifPrazoExecucao={term}")]
        ),
        expected
    );
    let error = run_err(&[
        "modificacoes",
        "add",
        path_str(&root),
        "--set",
        "idcontrato=1",
    ]);
    assert!(error.to_string().contains("sequencia"), "{error}");
}

#[test]
fn announcements_parse_string_prices_and_sort_by_date() {
    let rows = fixture_rows("anuncios");
    let root = loaded_root("e-anuncios", "anuncios");
    let price = |row: &Row| row.one("PrecoBase").parse::<f64>().unwrap_or(0.0);
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| price(row) >= 100_000.0)
        .map(|row| row.one("IDAnuncio"))
        .collect();
    assert_eq!(
        keys("anuncios", &root, &["--range", "PrecoBase=100000.."]),
        expected
    );
    let mut by_date: Vec<&Row> = rows.iter().collect();
    by_date.sort_by_key(|row| std::cmp::Reverse(row.one("dataPublicacao")));
    let tsv = search(
        "anuncios",
        &root,
        &[
            "--sort",
            "dataPublicacao:desc",
            "--fields",
            "dataPublicacao",
        ],
    );
    assert_eq!(
        tsv.column("dataPublicacao"),
        by_date
            .iter()
            .map(|row| row.one("dataPublicacao"))
            .collect::<Vec<_>>()
    );
    // Lot CPV codes and announcement CPVs share one field with derived levels.
    let code = rows
        .iter()
        .find_map(|row| row.strings("cpv").into_iter().next())
        .unwrap();
    let division = &code[..2];
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| {
            row.strings("cpv")
                .iter()
                .any(|cpv| cpv.starts_with(division))
        })
        .map(|row| row.one("IDAnuncio"))
        .collect();
    assert_eq!(
        keys(
            "anuncios",
            &root,
            &["--term", &format!("cpvDivisao={division}")]
        ),
        expected
    );
    // Foreign key to the contracting entity.
    let nif = rows[0].one("nifEntidade");
    let expected: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.one("nifEntidade") == nif)
        .map(|row| row.one("IDAnuncio"))
        .collect();
    assert_eq!(
        keys(
            "anuncios",
            &root,
            &["--term", &format!("nifEntidade={nif}")]
        ),
        expected
    );
}

#[test]
fn party_indices_link_back_to_contracts() {
    // Every supplier NIF of the fixture is a key of fornecedores and an
    // adjudicatarioNif of some contract.
    let contracts = fixture_rows("contratos");
    let suppliers = fixture_rows("fornecedores");
    let root = loaded_root("e-links", "contratos");
    run(&["fornecedores", "create", path_str(&root)]);
    run(&[
        "fornecedores",
        "update",
        path_str(&root),
        "--file",
        fixture_path("fornecedores").to_str().unwrap(),
    ]);
    for supplier in suppliers.iter().take(5) {
        let nif = supplier.one("nif");
        let expected: BTreeSet<String> = contracts
            .iter()
            .filter(|row| row.strings("adjudicatarioNif").contains(&nif))
            .map(|row| row.one("idcontrato"))
            .collect();
        assert_eq!(
            expected.len() as i64,
            supplier.one("numContratos").parse::<i64>().unwrap()
        );
        assert_eq!(
            keys(
                "contratos",
                &root,
                &["--term", &format!("adjudicatarioNif={nif}")]
            ),
            expected
        );
    }
    // Aggregates sort numerically.
    let mut by_value: Vec<&Row> = suppliers.iter().collect();
    by_value.sort_by(|a, b| {
        let (a, b) = (
            a.one("valorTotal").parse::<f64>().unwrap(),
            b.one("valorTotal").parse::<f64>().unwrap(),
        );
        b.partial_cmp(&a).unwrap()
    });
    let tsv = search(
        "fornecedores",
        &root,
        &[
            "--sort",
            "valorTotal:desc",
            "--limit",
            "5",
            "--fields",
            "nif",
        ],
    );
    assert_eq!(
        tsv.column("nif"),
        by_value
            .iter()
            .take(5)
            .map(|row| row.one("nif"))
            .collect::<Vec<_>>()
    );
    // Flags and country keywords.
    let sme: BTreeSet<String> = suppliers
        .iter()
        .filter(|row| row.one("pme") == "Sim")
        .map(|row| row.one("nif"))
        .collect();
    assert_eq!(keys("fornecedores", &root, &["--term", "pme=sim"]), sme);
    let invited = fixture_rows("convidadas");
    run(&["convidadas", "create", path_str(&root)]);
    run(&[
        "convidadas",
        "update",
        path_str(&root),
        "--file",
        fixture_path("convidadas").to_str().unwrap(),
    ]);
    let portugal: BTreeSet<String> = invited
        .iter()
        .filter(|row| row.strings("pais").contains(&"Portugal".to_owned()))
        .map(|row| row.one("nif"))
        .collect();
    assert_eq!(
        keys("convidadas", &root, &["--term", "pais=Portugal"]),
        portugal
    );
    // Procedures list their contracts as foreign keys.
    let procedures = fixture_rows("procedimentos");
    run(&["procedimentos", "create", path_str(&root)]);
    run(&[
        "procedimentos",
        "update",
        path_str(&root),
        "--file",
        fixture_path("procedimentos").to_str().unwrap(),
    ]);
    let contract = contracts[0].one("idcontrato");
    let expected: BTreeSet<String> = procedures
        .iter()
        .filter(|row| row.strings("idcontrato").contains(&contract))
        .map(|row| row.one("idprocedimento"))
        .collect();
    assert_eq!(expected.len(), 1);
    assert_eq!(
        keys(
            "procedimentos",
            &root,
            &["--term", &format!("idcontrato={contract}")]
        ),
        expected
    );
    // Absent placeholders: a 0 base price is not indexed.
    let zero = procedures
        .iter()
        .filter(|row| row.one("precoBaseProcedimento").parse::<f64>().unwrap() == 0.0)
        .count();
    assert_eq!(
        count(
            "procedimentos",
            &root,
            &["--range", "precoBaseProcedimento=.."]
        ),
        (procedures.len() - zero) as i64
    );
}
