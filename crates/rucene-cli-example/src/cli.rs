//! Command-line parsing and dispatch (standard library only).

use std::io::Write;
use std::path::{Path, PathBuf};

use rucene::search::Occur;

use crate::error::{CliError, Result};
use crate::index_ops::{add_documents, create_index, delete_documents, update_documents};
use crate::schema::{parse_csv, Record, FIELDS};
use crate::search::{search, write_tsv, Clause, Criterion, SearchRequest, SortKey, SortSpec};

/// The columns printed by `search` when `--fields` is not given.
pub const DEFAULT_FIELDS: &[&str] = &["id", "time", "mag", "magType", "place"];

/// The number of hits printed by `search` when `--limit` is not given.
pub const DEFAULT_LIMIT: usize = 10;

/// The usage text printed by `rucene-cli help`.
pub const USAGE: &str = "\
rucene-cli: index and search the USGS earthquake catalog with rucene

USAGE:
  rucene-cli create <dir>
  rucene-cli add    <dir> (--file F.csv | --set field=value ...)
  rucene-cli update <dir> (--file F.csv | --set field=value ...)
  rucene-cli delete <dir> --id ID [--id ID ...]
  rucene-cli search <dir> [criteria ...] [--sort KEY[:asc|desc],...]
                          [--limit N] [--fields f,...|all] [--origin LAT,LON]
                          [--exact-total]
  rucene-cli help

CRITERIA (combined with AND; prefix one with --should or --not to make it
optional or prohibited):
  --text WORDS            every analysed word must occur in place
  --phrase WORDS          the analysed words must occur in place as a phrase
  --term FIELD=VALUE      exact value (keyword, number or ISO date)
  --terms FIELD=V1,V2,..  any of the values
  --range FIELD=LO..HI    inclusive numeric or date range; LO or HI may be empty
  --box MINLAT,MAXLAT,MINLON,MAXLON
                          location inside the box
  --distance LAT,LON,KM   location within KM kilometres of the point
  --id ID                 document key

SORT KEYS: any field, _score, _doc, _distance (from --origin or the first
--distance). Default: _score (relevance).

TOTAL HITS: above 1 000 hits the total may be a lower bound, printed as
\"N (lower bound)\"; --exact-total counts it exactly (IndexSearcher.count).
";

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
    match command.as_str() {
        "help" | "--help" | "-h" => {
            out.write_all(USAGE.as_bytes())?;
            Ok(())
        }
        "create" => {
            let (dir, rest) = split_dir(command, rest)?;
            if let Some(extra) = rest.first() {
                return Err(CliError::Usage(format!(
                    "create: unexpected argument {extra:?}"
                )));
            }
            create_index(&dir)?;
            writeln!(out, "created empty index at {}", dir.display())?;
            Ok(())
        }
        "add" | "update" => {
            let (dir, rest) = split_dir(command, rest)?;
            let records = parse_records(command, rest)?;
            let count = if command == "add" {
                add_documents(&dir, &records)?
            } else {
                update_documents(&dir, &records)?
            };
            let verb = if command == "add" { "added" } else { "updated" };
            writeln!(out, "{verb} {count} document(s) in {}", dir.display())?;
            Ok(())
        }
        "delete" => {
            let (dir, rest) = split_dir(command, rest)?;
            let ids = parse_delete(rest)?;
            let count = delete_documents(&dir, &ids)?;
            writeln!(
                out,
                "deleted documents for {count} id(s) in {}",
                dir.display()
            )?;
            Ok(())
        }
        "search" => {
            let (dir, rest) = split_dir(command, rest)?;
            let request = parse_search(rest)?;
            let result = search(&dir, &request)?;
            write_tsv(&result, &request.fields, out)?;
            Ok(())
        }
        other => Err(CliError::Usage(format!("unknown command {other:?}"))),
    }
}

fn split_dir<'a>(command: &str, rest: &'a [String]) -> Result<(PathBuf, &'a [String])> {
    match rest.split_first() {
        Some((dir, rest)) if !dir.starts_with("--") => Ok((PathBuf::from(dir), rest)),
        _ => Err(CliError::Usage(format!(
            "{command}: the index directory is required"
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

/// Parses the record source of `add` / `update`: one `--file`, or `--set`
/// pairs forming one record.
fn parse_records(command: &str, args: &[String]) -> Result<Vec<Record>> {
    let mut file: Option<&str> = None;
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--file" => {
                if file.is_some() {
                    return Err(CliError::Usage(format!("{command}: --file given twice")));
                }
                file = Some(flag_value(args, &mut index, "--file")?);
            }
            "--set" => pairs.push(split_pair("--set", flag_value(args, &mut index, "--set")?)?),
            other => {
                return Err(CliError::Usage(format!(
                    "{command}: unexpected argument {other:?}"
                )))
            }
        }
        index += 1;
    }
    match (file, pairs.is_empty()) {
        (Some(_), false) => Err(CliError::Usage(format!(
            "{command}: give either --file or --set, not both"
        ))),
        (None, true) => Err(CliError::Usage(format!(
            "{command}: give --file F.csv or at least one --set field=value"
        ))),
        (Some(path), true) => read_csv_file(Path::new(path)),
        (None, false) => Ok(vec![Record::from_pairs(pairs)?]),
    }
}

/// Reads and parses a USGS CSV file.
///
/// # Errors
///
/// Returns [`CliError::Input`] naming the file when it cannot be read or does
/// not parse.
pub fn read_csv_file(path: &Path) -> Result<Vec<Record>> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| CliError::Input(format!("cannot read {}: {error}", path.display())))?;
    parse_csv(&text).map_err(|error| error.context(&path.display().to_string()))
}

fn parse_delete(args: &[String]) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--id" => ids.push(flag_value(args, &mut index, "--id")?.to_owned()),
            other => {
                return Err(CliError::Usage(format!(
                    "delete: unexpected argument {other:?}"
                )))
            }
        }
        index += 1;
    }
    if ids.is_empty() {
        return Err(CliError::Usage("delete: give at least one --id".to_owned()));
    }
    Ok(ids)
}

/// Parses the arguments of `search` (after the directory) into a request.
///
/// # Errors
///
/// Returns [`CliError::Usage`] for an unknown flag, a missing value, or a
/// value of the wrong shape.
pub fn parse_search(args: &[String]) -> Result<SearchRequest> {
    let mut request = SearchRequest {
        clauses: Vec::new(),
        sort: Vec::new(),
        limit: DEFAULT_LIMIT,
        fields: DEFAULT_FIELDS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        origin: None,
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
            "--text" => Criterion::Text(flag_value(args, &mut index, flag)?.to_owned()),
            "--phrase" => Criterion::Phrase(flag_value(args, &mut index, flag)?.to_owned()),
            "--id" => Criterion::Id(flag_value(args, &mut index, flag)?.to_owned()),
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
            "--box" => {
                let numbers = numbers(flag, flag_value(args, &mut index, flag)?, 4)?;
                Criterion::Box {
                    min_latitude: numbers[0],
                    max_latitude: numbers[1],
                    min_longitude: numbers[2],
                    max_longitude: numbers[3],
                }
            }
            "--distance" => {
                let numbers = numbers(flag, flag_value(args, &mut index, flag)?, 3)?;
                Criterion::Distance {
                    latitude: numbers[0],
                    longitude: numbers[1],
                    kilometers: numbers[2],
                }
            }
            "--exact-total" => {
                if pending.is_some() {
                    return Err(CliError::Usage(format!(
                        "a criterion must follow --must/--should/--not, got {flag}"
                    )));
                }
                request.exact_total = true;
                index += 1;
                continue;
            }
            "--origin" | "--sort" | "--limit" | "--fields" => {
                if pending.is_some() {
                    return Err(CliError::Usage(format!(
                        "a criterion must follow --must/--should/--not, got {flag}"
                    )));
                }
                let value = flag_value(args, &mut index, flag)?;
                match flag {
                    "--origin" => {
                        let numbers = numbers(flag, value, 2)?;
                        request.origin = Some((numbers[0], numbers[1]));
                    }
                    "--sort" => request.sort = parse_sort(value)?,
                    "--limit" => {
                        request.limit = value.parse().map_err(|_| {
                            CliError::Usage(format!("--limit expects a number, got {value:?}"))
                        })?;
                    }
                    _ => {
                        request.fields = if value == "all" {
                            FIELDS.iter().map(|spec| spec.name.to_owned()).collect()
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
                    "search: unexpected argument {other:?}"
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

fn numbers(flag: &str, value: &str, count: usize) -> Result<Vec<f64>> {
    let parsed: Vec<f64> = value
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| {
            CliError::Usage(format!(
                "{flag} expects {count} comma-separated numbers, got {value:?}"
            ))
        })?;
    if parsed.len() != count {
        return Err(CliError::Usage(format!(
            "{flag} expects {count} comma-separated numbers, got {value:?}"
        )));
    }
    Ok(parsed)
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
                "_distance" => SortKey::Distance,
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
