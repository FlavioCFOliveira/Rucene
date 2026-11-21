//! Consistency of the ten schema declarations, and of the README tables
//! with them.

use std::collections::BTreeSet;

use rucene_cli_base::entities;
use rucene_cli_base::schema::{markdown_table, FieldKind, EXACT_SUFFIX, KEY_FIELD};

#[test]
fn every_schema_is_consistent() {
    let mut names = BTreeSet::new();
    for schema in entities::ALL {
        assert!(
            names.insert(schema.name),
            "duplicate entity {}",
            schema.name
        );
        assert_eq!(
            entities::by_name(schema.name).map(|s| s.name),
            Some(schema.name)
        );
        let mut fields = BTreeSet::new();
        for spec in schema.fields {
            assert!(
                fields.insert(spec.name),
                "{}: field {} twice",
                schema.name,
                spec.name
            );
            assert_ne!(spec.name, KEY_FIELD);
            assert!(
                !spec.name.ends_with(EXACT_SUFFIX),
                "{}: {}",
                schema.name,
                spec.name
            );
            if spec.derived {
                assert_eq!(spec.kind, FieldKind::Keyword);
                let feeders = schema
                    .fields
                    .iter()
                    .filter(|other| {
                        other
                            .levels
                            .is_some_and(|levels| levels.fields.contains(&spec.name))
                    })
                    .count();
                assert_eq!(
                    feeders, 1,
                    "{}: derived {} needs one source",
                    schema.name, spec.name
                );
            }
            if let Some(levels) = spec.levels {
                for target in levels.fields {
                    assert!(
                        schema.field(target).is_some_and(|field| field.derived),
                        "{}: level {target} not declared as derived",
                        schema.name
                    );
                }
            }
        }
        assert!(!schema.key.is_empty());
        for key in schema.key {
            let spec = schema
                .field(key)
                .unwrap_or_else(|| panic!("{}: key {key}", schema.name));
            assert!(!spec.multi && !spec.derived, "{}: key {key}", schema.name);
            assert!(
                matches!(
                    spec.kind,
                    FieldKind::Exact | FieldKind::Keyword | FieldKind::Int
                ),
                "{}: key {key}",
                schema.name
            );
        }
        assert!(!schema.text_fields.is_empty());
        for name in schema.text_fields {
            let spec = schema.field(name).unwrap();
            assert!(
                matches!(spec.kind, FieldKind::Text | FieldKind::Name),
                "{}: {name}",
                schema.name
            );
        }
        for name in schema.output_fields {
            assert!(
                schema.field(name).is_some_and(|spec| !spec.derived),
                "{}: {name}",
                schema.name
            );
        }
    }
    assert_eq!(names.len(), 10);
}

#[test]
fn the_readme_holds_the_schema_table_of_every_index() {
    let readme =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    for schema in entities::ALL {
        let table = markdown_table(schema);
        assert!(
            readme.contains(&table),
            "README.md lacks the current table of {}:\n{table}",
            schema.name
        );
    }
}
