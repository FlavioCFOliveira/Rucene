//! `create`, `add`, `update` and `delete`, each verified by reopening the
//! index with plain `rucene` calls.

mod common;

use rucene_cli_example::schema::{parse_csv, require_field, stored_value};
use rucene_cli_example::{index_ops, CliError};

use common::{
    fixture_index, fixture_path, fixture_rows, open_reader, run, run_err, stored_by_id, temp_dir,
};

fn path_str(dir: &std::path::Path) -> &str {
    dir.to_str().unwrap()
}

fn stored(dir: &std::path::Path, id: &str, column: &str) -> Option<String> {
    let documents = stored_by_id(dir, id);
    assert_eq!(documents.len(), 1, "exactly one document for {id}");
    stored_value(&documents[0], require_field(column).unwrap())
}

#[test]
fn create_makes_an_empty_index_and_recreates_an_existing_one() {
    let dir = temp_dir("create");
    let output = run(&["create", path_str(&dir)]);
    assert!(output.starts_with("created empty index at "));
    let reader = open_reader(&dir);
    assert_eq!(reader.num_docs(), 0);

    run(&["add", path_str(&dir), "--set", "id=x1", "--set", "mag=1.5"]);
    assert_eq!(open_reader(&dir).num_docs(), 1);
    // OpenMode.CREATE replaces the existing index.
    run(&["create", path_str(&dir)]);
    assert_eq!(open_reader(&dir).num_docs(), 0);
}

#[test]
fn add_one_document_with_set_pairs() {
    let dir = temp_dir("add-one");
    run(&["create", path_str(&dir)]);
    let output = run(&[
        "add",
        path_str(&dir),
        "--set",
        "id=test0001",
        "--set",
        "time=2024-02-03T04:05:06.789Z",
        "--set",
        "mag=4.25",
        "--set",
        "nst=17",
        "--set",
        "magType=mw",
        "--set",
        "place=10 km N of Testville, Nowhere",
        "--set",
        "latitude=10.5",
        "--set",
        "longitude=-20.25",
    ]);
    assert_eq!(
        output.trim_end(),
        format!("added 1 document(s) in {}", dir.display())
    );
    assert_eq!(open_reader(&dir).num_docs(), 1);
    assert_eq!(
        stored(&dir, "test0001", "time").as_deref(),
        Some("2024-02-03T04:05:06.789Z")
    );
    assert_eq!(stored(&dir, "test0001", "mag").as_deref(), Some("4.25"));
    assert_eq!(stored(&dir, "test0001", "nst").as_deref(), Some("17"));
    assert_eq!(stored(&dir, "test0001", "magType").as_deref(), Some("mw"));
    assert_eq!(
        stored(&dir, "test0001", "place").as_deref(),
        Some("10 km N of Testville, Nowhere")
    );
    assert_eq!(
        stored(&dir, "test0001", "latitude").as_deref(),
        Some("10.5")
    );
    assert_eq!(
        stored(&dir, "test0001", "depth"),
        None,
        "absent values store nothing"
    );
}

#[test]
fn add_many_documents_from_csv() {
    let dir = fixture_index("add-many");
    let rows = fixture_rows();
    assert_eq!(open_reader(&dir).num_docs(), rows.len() as i32);
    for row in rows.iter().step_by(17) {
        for column in [
            "time", "mag", "depth", "place", "net", "nst", "magError", "updated",
        ] {
            let raw = row.raw(column);
            let expected = (!raw.is_empty()).then(|| {
                // Numbers are stored as numbers: compare their values.
                match column {
                    "mag" | "depth" | "magError" => raw.parse::<f64>().unwrap().to_string(),
                    _ => raw.to_owned(),
                }
            });
            assert_eq!(
                stored(&dir, row.id(), column),
                expected,
                "{} {column}",
                row.id()
            );
        }
    }
}

#[test]
fn update_one_document_replaces_its_stored_values() {
    let dir = fixture_index("update-one");
    let rows = fixture_rows();
    let target = &rows[5];
    let before = open_reader(&dir).num_docs();
    let output = run(&[
        "update",
        path_str(&dir),
        "--set",
        &format!("id={}", target.id()),
        "--set",
        "mag=9.9",
        "--set",
        "place=Updated place",
    ]);
    assert!(output.starts_with("updated 1 document(s)"));
    let reader = open_reader(&dir);
    assert_eq!(reader.num_docs(), before, "update keeps the document count");
    assert_eq!(stored(&dir, target.id(), "mag").as_deref(), Some("9.9"));
    assert_eq!(
        stored(&dir, target.id(), "place").as_deref(),
        Some("Updated place")
    );
    // The whole document is replaced: columns not given are gone.
    assert_eq!(stored(&dir, target.id(), "time"), None);
    // Other documents are untouched.
    assert_eq!(
        stored(&dir, rows[6].id(), "mag"),
        Some(rows[6].raw("mag").parse::<f64>().unwrap().to_string())
    );
}

#[test]
fn update_many_documents_from_csv() {
    let dir = fixture_index("update-many");
    let rows = fixture_rows();
    let before = open_reader(&dir).num_docs();

    // Rewrite the fixture rows 10..40 with mag + 1 and a new status.
    let text = std::fs::read_to_string(fixture_path()).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap();
    let columns: Vec<&str> = header.split(',').collect();
    let mag_at = columns.iter().position(|c| *c == "mag").unwrap();
    let status_at = columns.iter().position(|c| *c == "status").unwrap();
    let mut csv = format!("{header}\n");
    let mut expected = Vec::new();
    for (index, line) in lines.enumerate().skip(10).take(30) {
        // Fields before `place` hold no comma, and place is the only quoted
        // field: rebuild the line around it.
        let (head, rest) = line.split_once('"').unwrap();
        let (place, tail) = rest.split_once('"').unwrap();
        let mut head_fields: Vec<String> = head.split(',').map(str::to_owned).collect();
        let mut tail_fields: Vec<String> = tail.split(',').map(str::to_owned).collect();
        let new_mag = rows[index].f64("mag").unwrap() + 1.0;
        head_fields[mag_at] = new_mag.to_string();
        let status_in_tail = status_at - (head_fields.len() - 1);
        tail_fields[status_in_tail] = "corrected".to_owned();
        csv.push_str(&format!(
            "{}\"{place}\"{}\n",
            head_fields.join(","),
            tail_fields.join(",")
        ));
        expected.push((rows[index].id().to_owned(), new_mag));
    }
    assert_eq!(parse_csv(&csv).unwrap().len(), 30);
    let file = temp_dir("update-many-csv").join("changes.csv");
    std::fs::write(&file, csv).unwrap();

    let output = run(&["update", path_str(&dir), "--file", file.to_str().unwrap()]);
    assert!(output.starts_with("updated 30 document(s)"));
    assert_eq!(
        open_reader(&dir).num_docs(),
        before,
        "update keeps the document count"
    );
    for (id, new_mag) in expected {
        assert_eq!(stored(&dir, &id, "mag"), Some(new_mag.to_string()), "{id}");
        assert_eq!(
            stored(&dir, &id, "status").as_deref(),
            Some("corrected"),
            "{id}"
        );
    }
    assert_eq!(
        stored(&dir, rows[9].id(), "status").as_deref(),
        Some("reviewed")
    );
}

#[test]
fn delete_one_and_many_documents() {
    let dir = fixture_index("delete");
    let rows = fixture_rows();
    let total = rows.len() as i32;

    let output = run(&["delete", path_str(&dir), "--id", rows[0].id()]);
    assert!(output.starts_with("deleted documents for 1 id(s)"));
    assert_eq!(open_reader(&dir).num_docs(), total - 1);
    assert!(stored_by_id(&dir, rows[0].id()).is_empty());

    let ids: Vec<&str> = rows[100..110].iter().map(|row| row.id()).collect();
    let mut args = vec!["delete", path_str(&dir)];
    for id in &ids {
        args.extend_from_slice(&["--id", id]);
    }
    // An id that matches nothing is not an error, as in Lucene.
    args.extend_from_slice(&["--id", "no-such-id"]);
    run(&args);
    assert_eq!(open_reader(&dir).num_docs(), total - 11);
    for id in ids {
        assert!(stored_by_id(&dir, id).is_empty(), "{id} still present");
    }
    assert_eq!(stored_by_id(&dir, rows[99].id()).len(), 1);
}

#[test]
fn write_commands_report_clear_errors() {
    let dir = temp_dir("errors");
    let missing = dir.join("missing");
    let error = run_err(&["add", missing.to_str().unwrap(), "--set", "id=a"]);
    assert!(matches!(error, CliError::Input(_)));
    assert!(
        error.to_string().starts_with("no index found at"),
        "{error}"
    );
    assert_eq!(error.exit_code(), 1);

    run(&["create", path_str(&dir)]);
    let error = run_err(&["add", path_str(&dir), "--set", "mag=1"]);
    assert_eq!(
        error.to_string(),
        "the \"id\" field is required and must not be empty"
    );
    let error = run_err(&[
        "add",
        path_str(&dir),
        "--set",
        "id=a",
        "--set",
        "time=yesterday",
    ]);
    assert!(
        error
            .to_string()
            .starts_with("field \"time\": invalid ISO 8601"),
        "{error}"
    );
    let error = run_err(&["add", path_str(&dir)]);
    assert_eq!(error.exit_code(), 2);
    let error = run_err(&["delete", path_str(&dir)]);
    assert_eq!(error.exit_code(), 2);

    let bad_csv = dir.join("bad.csv");
    std::fs::write(&bad_csv, "id,mag\na,1\nb,oops\n").unwrap();
    let error = run_err(&["add", path_str(&dir), "--file", bad_csv.to_str().unwrap()]);
    assert!(
        error.to_string().contains("CSV line 3: field \"mag\""),
        "{error}"
    );
    // Nothing of a failed command is committed.
    assert_eq!(open_reader(&dir).num_docs(), 0);

    // The library API reports the same.
    assert!(index_ops::delete_documents(&missing, &["a".to_owned()]).is_err());
}
