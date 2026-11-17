//! Command-line parsing and dispatch (standard library only).
//!
//! The command line is `rucene-base <entity> <subcommand> <root> [flags]`:
//! one top-level command per index ([`crate::entities::ALL`]), each with the
//! same subcommands, always addressed by the root directory that holds every
//! index in its own subdirectory.

use std::io::Write;
use std::path::{Path, PathBuf};

use rucene::search::Occur;

use crate::entities;
use crate::error::{CliError, Result};
use crate::index_ops::{
    add_documents, create_index, delete_documents, get_documents, index_stats, update_documents,
};
use crate::input::{record_from_pairs, RecordReader};
use crate::schema::{markdown_table, EntitySchema, Record};
use crate::search::{
    count, search, write_tsv, Clause, Criterion, SearchRequest, SortKey, SortSpec,
};

/// The number of hits printed by `search` when `--limit` is not given.
pub const DEFAULT_LIMIT: usize = 10;

/// The usage text printed by `rucene-base help`, before the list of
/// indices.
pub const USAGE: &str = "\
rucene-base: one rucene index per Portal BASE entity, searchable insensitive
to case, accents and diacritics (pt-PT analysis)

USAGE:
  rucene-base <entity> create <root>
  rucene-base <entity> add    <root> (--file F.jsonl | --set field=value ...)
  rucene-base <entity> update <root> (--file F.jsonl | --set field=value ...)
  rucene-base <entity> delete <root> --key K [--key K ...]
  rucene-base <entity> get    <root> --key K [--key K ...]
  rucene-base <entity> search <root> [criteria ...] [--sort KEY[:asc|desc],...]
                                     [--limit N] [--fields f,...|all] [--exact-total]
  rucene-base <entity> count  <root> [criteria ...]
  rucene-base <entity> stats  <root>
  rucene-base <entity> help          the fields of the index
  rucene-base help

Each index lives in <root>/<entity>; create makes only that subdirectory.
add/update read JSON Lines (one flat object per line) or one record from
--set pairs (repeat a multi-valued field). update replaces by key; add does
not look at the key. get prints stored documents as JSON Lines.

CRITERIA (combined with AND; prefix one with --should or --not to make it
optional or prohibited):
  --text WORDS              every word in some default text field
  --text-in FIELD=WORDS     every word in the text or name field
  --phrase WORDS            the phrase in some default text field
  --phrase-in FIELD=WORDS   the phrase in the text or name field
  --term FIELD=VALUE        exact value (id, code, flag, name, number, date)
  --terms FIELD=V1,V2,..    any of the values
  --range FIELD=LO..HI      inclusive number or date range; LO or HI may be empty
  --key KEY                 document key

SORT KEYS: a keyword, flag, name, number or date field, _score, _doc.
Default: _score (relevance).

TOTAL HITS: above 1 000 hits the total may be a lower bound, printed as
\"N (lower bound)\"; --exact-total counts it exactly (IndexSearcher.count).

EXIT CODES: 0 success, 1 input or index error, 2 usage error.
";

/// The full text of `rucene-base help`: [`USAGE`] plus the indices.
#[must_use]
pub fn usage() -> String {
    let mut text = USAGE.to_owned();
    text.push_str("\nENTITIES:\n");
    for schema in entities::ALL {
        text.push_str(&format!("  {:<14}{}\n", schema.name, schema.summary));
    }
    text
}

/// The text of `rucene-base <entity> help`: key, defaults and the schema
/// table.
#[must_use]
pub fn entity_help(schema: &EntitySchema) -> String {
    format!(
        "rucene-base {name}: {summary}\n\n\
         index directory: <root>/{name}\n\
         key: {key}\n\
         default text fields (--text, --phrase): {text}\n\
         default output fields (search): {output}\n\n\
         {table}",
        name = schema.name,
        summary = schema.summary,
        key = schema.key_label(),
        text = schema.text_fields.join(", "),
        output = schema.output_fields.join(", "),
        table = markdown_table(schema),
    )
}

/// Runs the command line `args` (without the program name), writing normal
/// output to `out`.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for a malformed command line and the error of
/// the command otherwise.
pub fn run(args: &[String], out: &mut dyn Write) -> Result<()> {
    let Some((command, rest)) = args.split_first() else {
        return Err(CliError::Usage("no command given".to_owned()));
    };
    if matches!(command.as_str(), "help" | "--help" | "-h") {
        out.write_all(usage().as_bytes())?;
        return Ok(());
    }
    let Some(schema) = entities::by_name(command) else {
        let known: Vec<&str> = entities::ALL.iter().map(|schema| schema.name).collect();
        return Err(CliError::Usage(format!(
            "unknown command {command:?}; the commands are help and {}",
            known.join(", ")
        )));
    };
    let Some((subcommand, rest)) = rest.split_first() else {
        return Err(CliError::Usage(format!("{command}: no subcommand given")));
    };
    let name = schema.name;
    match subcommand.as_str() {
        "help" | "--help" | "-h" => {
            out.write_all(entity_help(schema).as_bytes())?;
            Ok(())
        }
        "create" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            if let Some(extra) = rest.first() {
                return Err(CliError::Usage(format!(
                    "{name} create: unexpected argument {extra:?}"
                )));
            }
            let dir = create_index(&root, schema)?;
            writeln!(out, "created empty {name} index at {}", dir.display())?;
            Ok(())
        }
        "add" | "update" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            let label = format!("{name} {subcommand}");
            let count = match parse_records(&label, rest)? {
                RecordSource::File(path) => {
                    let records = open_jsonl_file(schema, path)?;
                    write_records(&root, schema, subcommand, records)?
                }
                RecordSource::Pairs(pairs) => {
                    let record = record_from_pairs(schema, &pairs)?;
                    write_records(&root, schema, subcommand, std::iter::once(Ok(record)))?
                }
            };
            let verb = if subcommand == "add" {
                "added"
            } else {
                "updated"
            };
            writeln!(
                out,
                "{verb} {count} document(s) in {}",
                crate::index_ops::index_dir(&root, schema).display()
            )?;
            Ok(())
        }
        "delete" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            let keys = parse_keys(&format!("{name} delete"), rest)?;
            let count = delete_documents(&root, schema, &keys)?;
            writeln!(
                out,
                "deleted documents for {count} key(s) in {}",
                crate::index_ops::index_dir(&root, schema).display()
            )?;
            Ok(())
        }
        "get" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            let keys = parse_keys(&format!("{name} get"), rest)?;
            let result = get_documents(&root, schema, &keys)?;
            for document in &result.documents {
                writeln!(out, "{document}")?;
            }
            if result.missing.is_empty() {
                Ok(())
            } else {
                Err(CliError::Input(format!(
                    "no {name} document with key(s) {}",
                    result
                        .missing
                        .iter()
                        .map(|key| format!("{key:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
            }
        }
        "search" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            let request = parse_search(schema, rest, true)?;
            let result = search(&root, schema, &request)?;
            write_tsv(&result, &request.fields, out)?;
            Ok(())
        }
        "count" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            let request = parse_search(schema, rest, false)?;
            writeln!(out, "{}", count(&root, schema, &request.clauses)?)?;
            Ok(())
        }
        "stats" => {
            let (root, rest) = split_root(name, subcommand, rest)?;
            if let Some(extra) = rest.first() {
                return Err(CliError::Usage(format!(
                    "{name} stats: unexpected argument {extra:?}"
                )));
            }
            let stats = index_stats(&root, schema)?;
            writeln!(out, "index: {name}")?;
            writeln!(out, "directory: {}", stats.directory.display())?;
            writeln!(out, "documents: {}", stats.documents)?;
            writeln!(out, "deleted: {}", stats.deleted)?;
            writeln!(out, "max_doc: {}", stats.max_doc)?;
            writeln!(out, "segments: {}", stats.segments)?;
            Ok(())
        }
        other => Err(CliError::Usage(format!(
            "{name}: unknown subcommand {other:?}; the subcommands are create, add, update, \
             delete, get, search, count, stats and help"
        ))),
    }
}

fn write_records(
    root: &Path,
    schema: &EntitySchema,
    subcommand: &str,
    records: impl Iterator<Item = Result<Record>>,
) -> Result<usize> {
    if subcommand == "add" {
        add_documents(root, schema, records)
    } else {
        update_documents(root, schema, records)
    }
}

fn split_root<'a>(
    entity: &str,
    subcommand: &str,
    rest: &'a [String],
) -> Result<(PathBuf, &'a [String])> {
    match rest.split_first() {
        Some((root, rest)) if !root.starts_with("--") => Ok((PathBuf::from(root), rest)),
        _ => Err(CliError::Usage(format!(
            "{entity} {subcommand}: the root directory is required"
        ))),
    }
}

/// Takes the value of flag `flag` at `args[*index + 1]`, advancing `index`.
fn flag_value<'a>(args: &'a [String], index: &mut usize, flag: &str) -> Result<&'a str> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| CliError::Usage(format!("{flag} needs a value")))
}

fn split_pair<'a>(flag: &str, value: &'a str) -> Result<(&'a str, &'a str)> {
    value
        .split_once('=')
        .ok_or_else(|| CliError::Usage(format!("{flag} expects FIELD=VALUE, got {value:?}")))
}

/// Where the records of `add` / `update` come from.
enum RecordSource<'a> {
    /// The JSON Lines file given with `--file`.
    File(&'a Path),
    /// The `--set` pairs of one record.
    Pairs(Vec<(&'a str, &'a str)>),
}

fn parse_records<'a>(label: &str, args: &'a [String]) -> Result<RecordSource<'a>> {
    let mut file: Option<&str> = None;
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--file" => {
                if file.is_some() {
                    return Err(CliError::Usage(format!("{label}: --file given twice")));
                }
                file = Some(flag_value(args, &mut index, "--file")?);
            }
            "--set" => pairs.push(split_pair("--set", flag_value(args, &mut index, "--set")?)?),
            other => {
                return Err(CliError::Usage(format!(
                    "{label}: unexpected argument {other:?}"
                )))
            }
        }
        index += 1;
    }
    match (file, pairs.is_empty()) {
        (Some(_), false) => Err(CliError::Usage(format!(
            "{label}: give either --file or --set, not both"
        ))),
        (None, true) => Err(CliError::Usage(format!(
            "{label}: give --file F.jsonl or at least one --set field=value"
        ))),
        (Some(path), true) => Ok(RecordSource::File(Path::new(path))),
        (None, false) => Ok(RecordSource::Pairs(pairs)),
    }
}

/// Opens a JSON Lines file of `schema` records for streaming; errors carry
/// the file name.
///
/// # Errors
///
/// Returns [`CliError::Input`] naming the file when it cannot be opened; the
/// records yield the same, with the line number, for a later failure.
pub fn open_jsonl_file(
    schema: &'static EntitySchema,
    path: &Path,
) -> Result<impl Iterator<Item = Result<Record>>> {
    let name = path.display().to_string();
    let file = std::fs::File::open(path)
        .map_err(|error| CliError::Input(format!("cannot read {name}: {error}")))?;
    let reader = RecordReader::new(schema, std::io::BufReader::with_capacity(1 << 16, file));
    Ok(reader.map(move |record| record.map_err(|error| error.context(&name))))
}

fn parse_keys(label: &str, args: &[String]) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--key" => keys.push(flag_value(args, &mut index, "--key")?.to_owned()),
            other => {
                return Err(CliError::Usage(format!(
                    "{label}: unexpected argument {other:?}"
                )))
            }
        }
        index += 1;
    }
    if keys.is_empty() {
        return Err(CliError::Usage(format!("{label}: give at least one --key")));
    }
    Ok(keys)
}

/// Parses the arguments of `search` (or, with `full` false, of `count`,
/// which takes criteria only) after the root directory.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for an unknown flag, a missing value, or a
/// value of the wrong shape.
pub fn parse_search(schema: &EntitySchema, args: &[String], full: bool) -> Result<SearchRequest> {
    let mut request = SearchRequest {
        clauses: Vec::new(),
        sort: Vec::new(),
        limit: DEFAULT_LIMIT,
        fields: schema
            .output_fields
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        exact_total: false,
    };
    let mut pending: Option<Occur> = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let criterion = match flag {
            "--must" | "--should" | "--not" => {
                if pending.is_some() {
                    return Err(CliError::Usage(format!(
                        "{flag} must be followed by a criterion"
                    )));
                }
                pending = Some(match flag {
                    "--must" => Occur::MUST,
                    "--should" => Occur::SHOULD,
                    _ => Occur::MUST_NOT,
                });
                index += 1;
                continue;
            }
            "--text" | "--phrase" => {
                let words = flag_value(args, &mut index, flag)?.to_owned();
                if flag == "--text" {
                    Criterion::Text { field: None, words }
                } else {
                    Criterion::Phrase { field: None, words }
                }
            }
            "--text-in" | "--phrase-in" => {
                let (field, words) = split_pair(flag, flag_value(args, &mut index, flag)?)?;
                let (field, words) = (Some(field.to_owned()), words.to_owned());
                if flag == "--text-in" {
                    Criterion::Text { field, words }
                } else {
                    Criterion::Phrase { field, words }
                }
            }
            "--key" => Criterion::Key(flag_value(args, &mut index, flag)?.to_owned()),
            "--term" => {
                let (field, value) = split_pair(flag, flag_value(args, &mut index, flag)?)?;
                Criterion::Term {
                    field: field.to_owned(),
                    value: value.to_owned(),
                }
            }
            "--terms" => {
                let (field, values) = split_pair(flag, flag_value(args, &mut index, flag)?)?;
                Criterion::Terms {
                    field: field.to_owned(),
                    values: values.split(',').map(str::to_owned).collect(),
                }
            }
            "--range" => {
                let (field, range) = split_pair(flag, flag_value(args, &mut index, flag)?)?;
                let (lower, upper) = range.split_once("..").ok_or_else(|| {
                    CliError::Usage(format!("--range expects FIELD=LO..HI, got {range:?}"))
                })?;
                let bound = |text: &str| (!text.is_empty()).then(|| text.to_owned());
                Criterion::Range {
                    field: field.to_owned(),
                    lower: bound(lower),
                    upper: bound(upper),
                }
            }
            "--exact-total" | "--sort" | "--limit" | "--fields" if full => {
                if pending.is_some() {
                    return Err(CliError::Usage(format!(
                        "a criterion must follow --must/--should/--not, got {flag}"
                    )));
                }
                match flag {
                    "--exact-total" => request.exact_total = true,
                    "--sort" => request.sort = parse_sort(flag_value(args, &mut index, flag)?)?,
                    "--limit" => {
                        let value = flag_value(args, &mut index, flag)?;
                        request.limit = value.parse().map_err(|_| {
                            CliError::Usage(format!("--limit expects a number, got {value:?}"))
                        })?;
                    }
                    _ => {
                        let value = flag_value(args, &mut index, flag)?;
                        request.fields = if value == "all" {
                            schema
                                .stored_fields()
                                .map(|spec| spec.name.to_owned())
                                .collect()
                        } else {
                            value.split(',').map(str::to_owned).collect()
                        };
                    }
                }
                index += 1;
                continue;
            }
            other => {
                return Err(CliError::Usage(format!(
                    "{} {}: unexpected argument {other:?}",
                    schema.name,
                    if full { "search" } else { "count" }
                )))
            }
        };
        request.clauses.push(Clause {
            occur: pending.take().unwrap_or(Occur::MUST),
            criterion,
        });
        index += 1;
    }
    if let Some(occur) = pending {
        return Err(CliError::Usage(format!(
            "{} must be followed by a criterion",
            match occur {
                Occur::SHOULD => "--should",
                Occur::MUST_NOT => "--not",
                _ => "--must",
            }
        )));
    }
    Ok(request)
}

/// Parses `KEY[:asc|desc],...`. Fields and `_doc` default to ascending,
/// `_score` to descending (most relevant first).
///
/// # Errors
///
/// Returns [`CliError::Usage`] for an empty key or an unknown direction.
pub fn parse_sort(value: &str) -> Result<Vec<SortSpec>> {
    value
        .split(',')
        .map(|part| {
            let (name, direction) = match part.split_once(':') {
                Some((name, direction)) => (name, Some(direction)),
                None => (part, None),
            };
            let key = match name {
                "" => return Err(CliError::Usage(format!("--sort: empty key in {value:?}"))),
                "_score" => SortKey::Score,
                "_doc" => SortKey::Doc,
                field => SortKey::Field(field.to_owned()),
            };
            let descending = match direction {
                None => key == SortKey::Score,
                Some("asc") => false,
                Some("desc") => true,
                Some(other) => {
                    return Err(CliError::Usage(format!(
                        "--sort: direction must be asc or desc, got {other:?}"
                    )))
                }
            };
            Ok(SortSpec { key, descending })
        })
        .collect()
}
