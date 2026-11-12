# rucene-cli-example

A command-line example of the [`rucene`](../../README.md) crate, a Rust port of
Apache Lucene Core 10.5.0. The `rucene-cli` binary indexes the USGS Earthquake
Catalog and searches it by full text, exact keyword, numeric and date range,
geographic box and distance, with multi-key sorting.

The example has two purposes:

1. **Demonstrate** the `rucene` API end to end: `IndexWriter` (create, add,
   update, delete, commit), `DirectoryReader`, `IndexSearcher`, the Lucene 10
   field classes (`KeywordField`, `TextField`, `LongField`, `DoubleField`,
   `IntField`, `LatLonPoint`, `LatLonDocValuesField`), points, doc values,
   `BooleanQuery`, `TermInSetQuery`, phrase queries and `Sort`.
2. **Exercise** the crate for profiling and correctness work. Every command goes
   through the public `rucene` API; nothing is shortcut.

There is no query parser: the Lucene query parser is not part of Lucene core.
Free text is turned into queries with `QueryBuilder`
(`org.apache.lucene.util.QueryBuilder`), which is part of core.

## Dataset

The fixture
[`data/usgs-2024-01-01-to-2024-01-04-m2.5.csv`](data/usgs-2024-01-01-to-2024-01-04-m2.5.csv)
holds the 257 events of magnitude 2.5 or more between 2024-01-01T00:00:00Z
and 2024-01-04T00:00:00Z. It is an unmodified download from the USGS
Earthquake Catalog API. The window includes the 2024 Noto Peninsula
earthquake and its aftershocks. The exact URL used:

```text
https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&starttime=2024-01-01T00:00:00&endtime=2024-01-04T00:00:00&minmagnitude=2.5&orderby=time-asc
```

USGS data is a work of the United States Government and is in the public
domain. The CSV is RFC 4180, with 22 columns: `time`, `latitude`,
`longitude`, `depth`, `mag`, `magType`, `nst`, `gap`, `dmin`, `rms`, `net`,
`id`, `updated`, `place`, `type`, `horizontalError`, `depthError`, `magError`,
`magNst`, `status`, `locationSource`, `magSource`. Some values are empty (for
example `nst`, `gap` and `magError`). An empty value adds no field to the
document.

To profile with a larger index, download a wider window. The one below holds
about 17 000 events; the API returns at most 20 000 events per request:

```sh
curl -sS -o quakes-2023.csv \
  'https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&starttime=2023-01-01T00:00:00&endtime=2023-03-01T00:00:00&minmagnitude=1&orderby=time-asc'
rucene-cli create big-index
rucene-cli add big-index --file quakes-2023.csv
```

## Schema

The schema is declared once, in [`src/schema.rs`](src/schema.rs). The writer
and the search side are both derived from it. Every column is indexed under its
CSV name. Two fields are derived:

- `location`, built from `latitude` and `longitude` when both are present;
- `place_exact`, an exact copy of `place`.

| Column | Lucene field type(s) | Stored | Searchable how | Sortable |
| --- | --- | --- | --- | --- |
| `time` | `LongField` (epoch milliseconds: point + sorted-numeric doc values) | yes (long) | `--term`/`--terms` (exact instant), `--range` (ISO 8601) | yes (`LONG`) |
| `latitude` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes (`DOUBLE`) |
| `longitude` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes (`DOUBLE`) |
| `location` (derived) | `LatLonPoint` + `LatLonDocValuesField` | no (see `latitude`/`longitude`) | `--box`, `--distance` | `_distance` (ascending) |
| `depth` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `mag` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `magType` | `KeywordField` (term + sorted-set doc values) | yes | `--term`, `--terms` (exact) | yes (`SortedSetSortField`) |
| `nst` | `IntField` | yes | `--term`, `--terms`, `--range` | yes (`INT`) |
| `gap` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `dmin` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `rms` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `net` | `KeywordField` | yes | `--term`, `--terms` | yes |
| `id` | `KeywordField` | yes | `--id`, `--term`, `--terms` | yes |
| `updated` | `LongField` (epoch milliseconds) | yes (long) | `--term`, `--terms`, `--range` (ISO 8601) | yes |
| `place` | `TextField` (`StandardAnalyzer`) | yes | `--text` (all words), `--phrase` | via `place_exact` |
| `place_exact` (derived) | `KeywordField` | no | `--term place=...` (whole value, exact) | `--sort place` |
| `type` | `KeywordField` | yes | `--term`, `--terms` | yes |
| `horizontalError` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `depthError` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `magError` | `DoubleField` | yes | `--term`, `--terms`, `--range` | yes |
| `magNst` | `IntField` | yes | `--term`, `--terms`, `--range` | yes |
| `status` | `KeywordField` | yes | `--term`, `--terms` | yes |
| `locationSource` | `KeywordField` | yes | `--term`, `--terms` | yes |
| `magSource` | `KeywordField` | yes | `--term`, `--terms` | yes |

`id` is the key that `update` and `delete` act on (`Term("id", value)`).

## Build

From the repository root:

```sh
cargo build --release -p rucene-cli-example
# the binary is target/release/rucene-cli
cargo test -p rucene-cli-example
```

## Commands

`rucene-cli help` prints a usage summary. Every command commits and closes its
`IndexWriter` before it exits. If a command fails, the writer is rolled back, so
the index keeps its last commit.

Exit codes:

- `0`: success.
- `1`: invalid input, a missing index, or a library or I/O error.
- `2`: a malformed command line.

Errors are printed to standard error as `rucene-cli: error: ...`.

### `create <dir>`

Creates an empty index at `<dir>` (`OpenMode.CREATE`, then a commit). The
directory is created if it is missing. An index already at `<dir>` is replaced.

```console
$ rucene-cli create quakes
created empty index at quakes
```

### `add <dir> (--file F.csv | --set field=value ...)`

Adds documents with `IndexWriter.addDocument`.

- `--file F.csv`: one document per CSV data line. The header uses USGS column
  names. It may contain any subset of the columns, in any order, but must
  contain `id`.
- `--set field=value`: builds one document. Repeat the flag for each column.
  `id` is required. Dates are ISO 8601.

`add` does not check `id`. Adding an `id` that is already indexed creates a
second document with that key. Use `update` to replace a document.

```console
$ rucene-cli add quakes --file data/usgs-2024-01-01-to-2024-01-04-m2.5.csv
added 257 document(s) in quakes
$ rucene-cli add quakes --set id=test1 --set time=2024-02-01T10:00:00Z --set mag=3.1 \
    --set place="5 km N of Example, Nowhere" --set latitude=10 --set longitude=20
added 1 document(s) in quakes
```

### `update <dir> (--file F.csv | --set field=value ...)`

Replaces documents by `id`. For each record, the command calls
`IndexWriter.updateDocument(Term("id", id), doc)`, which atomically deletes
every document that holds the term and adds the new one. The new document
replaces the old one completely: a column that is not given is no longer
present. As in Lucene, a record whose `id` is not indexed is added.

```console
$ rucene-cli update quakes --set id=us6000m0xl --set time=2024-01-01T07:10:09.476Z \
    --set mag=7.6 --set magType=mww --set place="2024 Noto Peninsula, Japan Earthquake" \
    --set latitude=37.4874 --set longitude=137.2710
updated 1 document(s) in quakes
$ rucene-cli update quakes --file corrections.csv
```

### `delete <dir> --id ID [--id ID ...]`

Deletes every document whose `id` is one of the given values, with a single
`IndexWriter.deleteDocuments(Term...)` call. As in Lucene, an id that matches
no document is not an error.

```console
$ rucene-cli delete quakes --id us6000m0xl --id pr71435948
deleted documents for 2 id(s) in quakes
```

### `search <dir> [criteria ...] [--sort ...] [--limit N] [--fields ...] [--origin LAT,LON]`

Builds a `BooleanQuery` from the criteria and a `Sort` from the sort keys. It
then runs `IndexSearcher.search(query, limit, sort, doDocScores=true)`, which
takes the top-field-collector path, and prints the hits as TSV.

**Criteria.** Each criterion is a `MUST` clause unless one of these flags
precedes it:

- `--should`: the clause is optional (`SHOULD`);
- `--not`: the clause is prohibited (`MUST_NOT`);
- `--must`: the default, stated explicitly.

The clauses follow Lucene's `BooleanQuery` rules:

- if there is at least one `MUST` clause, `SHOULD` clauses only add to the
  score;
- if there are only `SHOULD` clauses, a document must match at least one of
  them;
- if every clause is `--not`, a `MatchAllDocsQuery` is added as `MUST`,
  because a query with only prohibited clauses matches nothing in Lucene;
- if there are no criteria, every document matches.

| Flag | Query | Notes |
| --- | --- | --- |
| `--text WORDS` | `QueryBuilder.createBooleanQuery("place", WORDS, MUST)` | every analysed word must occur |
| `--phrase WORDS` | `QueryBuilder.createPhraseQuery("place", WORDS)` | the words, adjacent and in order |
| `--term FIELD=VALUE` | `KeywordField.newExactQuery`; `LongField`/`DoubleField`/`IntField.newExactQuery` | `place` matches the whole value via `place_exact`; dates are ISO 8601 |
| `--terms FIELD=V1,V2,...` | `KeywordField.newSetQuery` (`TermInSetQuery`); `*Field.newSetQuery` | any of the values |
| `--range FIELD=LO..HI` | `LongField`/`DoubleField`/`IntField.newRangeQuery` | inclusive; an empty bound is open (`5..`, `..10`); numeric and date fields only |
| `--box MINLAT,MAXLAT,MINLON,MAXLON` | `LatLonPoint.newBoxQuery("location", ...)` | `MINLON > MAXLON` crosses the date line |
| `--distance LAT,LON,KM` | `LatLonPoint.newDistanceQuery("location", ...)` | radius in kilometres |
| `--id ID` | `KeywordField.newExactQuery("id", ID)` | |

ISO 8601 values have one of these forms:

- `YYYY-MM-DD`
- `YYYY-MM-DDTHH:MM`
- `YYYY-MM-DDTHH:MM:SS[.fff]`

The time forms take an optional zone, `Z` or `±HH:MM`. A value with no zone is
UTC. A date alone means midnight UTC of that day, so the range
`time=2024-01-02..2024-01-02T12:00` covers the first half of 2 January.

**Sort.** `--sort KEY[:asc|desc],...` takes one or more keys:

- a field name. `KeywordField.newSortField` is used for keywords and for
  `place`, which sorts on `place_exact`. `LongField`, `DoubleField` and
  `IntField.newSortField` are used for numbers and dates.
- `_score`: relevance, descending by default.
- `_doc`: index order.
- `_distance`: `LatLonDocValuesField.newDistanceSort`, ascending only. Its
  origin is `--origin LAT,LON`, or the centre of the first `--distance`
  criterion.

Fields and `_doc` sort ascending by default. A document that lacks the field
sorts after the documents that have it, in both directions. Remaining ties are
broken by document number, as Lucene does. Without `--sort`, hits are sorted by
relevance.

**Output.** `--fields f1,f2,...` (or `all`) selects the stored columns to
print. The default is `id,time,mag,magType,place`. `--limit N` caps the number
of hits; the default is 10.

The output has three parts:

1. a `# total hits: N` line;
2. a header row;
3. one TSV row per hit: the selected columns, then one `sort:KEY:DIR` column
   for each sort key (dates as ISO 8601, distances in metres), then the score.

### Worked examples

These are real outputs from the fixture index.

Full text combined with a numeric range, sorted by two keys:

```console
$ rucene-cli search quakes --text japan --range mag=5.5.. --sort mag:desc,time \
    --fields id,time,mag,place --limit 5
# total hits: 6
id	time	mag	place	sort:mag:desc	sort:time:asc	score
us6000m0xl	2024-01-01T07:10:09.476Z	7.5	2024 Noto Peninsula, Japan Earthquake	7.5	2024-01-01T07:10:09.476Z	1.4783533
us6000m0xm	2024-01-01T07:18:41.584Z	6.2	8 km SW of Anamizu, Japan	6.2	2024-01-01T07:18:41.584Z	1.4461455
us6000m0xj	2024-01-01T07:06:05.477Z	5.8	42 km NE of Anamizu, Japan	5.8	2024-01-01T07:06:05.477Z	1.4461455
us6000m0xt	2024-01-01T07:56:47.661Z	5.6	7 km N of Anamizu, Japan	5.6	2024-01-01T07:56:47.661Z	1.4461455
us6000m0ya	2024-01-01T09:08:17.603Z	5.6	54 km NE of Anamizu, Japan	5.6	2024-01-01T09:08:17.603Z	1.4461455
```

Geographic distance, nearest first:

```console
$ rucene-cli search quakes --distance 37.5,137.2,25 --sort _distance --fields id,mag,place --limit 3
# total hits: 37
id	mag	place	sort:_distance(m):asc	score
us6000m0xx	4.8	39 km NE of Anamizu, Japan	155.26330531839088	1
us6000m0xj	5.8	42 km NE of Anamizu, Japan	4395.247683778288	1
us6000m0xy	4.9	38 km NE of Anamizu, Japan	4613.338408576323	1
```

A keyword set with a prohibited range, newest first:

```console
$ rucene-cli search quakes --terms net=pr,tx --not --range mag=..2.6 --sort time:desc \
    --fields id,net,mag,place --limit 4
# total hits: 14
id	net	mag	place	sort:time:desc	score
tx2024aerk	tx	3.1	43 km NW of Toyah, Texas	2024-01-03T13:19:58.307Z	1
pr2024003000	pr	3.72	88 km NNE of Cruz Bay, U.S. Virgin Islands	2024-01-03T09:36:16.980Z	1
pr71436073	pr	2.77	11 km SSE of Guánica, Puerto Rico	2024-01-03T04:51:17.090Z	1
tx2024adxi	tx	3.5	34 km NNW of Toyah, Texas	2024-01-03T03:11:31.513Z	1
```

A date range:

```console
$ rucene-cli search quakes --range time=2024-01-03T12:00..2024-01-03T13:00 --sort time \
    --fields id,time,mag,place
# total hits: 3
id	time	mag	place	sort:time:asc	score
us6000m1uk	2024-01-03T12:26:49.558Z	3	44 km SSE of Golfo de Santa Clara, Mexico	2024-01-03T12:26:49.558Z	1
ak02455xzqo	2024-01-03T12:31:48.780Z	2.9	72 km ENE of Wiseman, Alaska	2024-01-03T12:31:48.780Z	1
us6000m3bn	2024-01-03T12:59:19.567Z	4.4	26 km SW of Burias, Philippines	2024-01-03T12:59:19.567Z	1
```

A phrase, sorted by relevance:

```console
$ rucene-cli search quakes --phrase "puerto rico" --limit 3 --fields id,place
# total hits: 6
id	place	score
pr71436063	6 km SSW of Ponce, Puerto Rico	3.0472727
pr71436073	11 km SSE of Guánica, Puerto Rico	3.0472727
pr71435948	2 km SSE of Coto Norte, Puerto Rico	2.8664484
```

A geographic box, deepest first:

```console
$ rucene-cli search quakes --box 50,72,-180,-130 --sort depth:desc --fields id,depth,place --limit 3
# total hits: 33
id	depth	place	sort:depth:desc	score
ak0243oi2c0	104.3	39 km WNW of Skwentna, Alaska	104.3	1
us7000lt9l	94.571	16 km NE of Adak, Alaska	94.571	1
ak0243cd25b	92.5	46 km WNW of Ninilchik, Alaska	92.5	1
```

Optional clauses (Alaska network or Vanuatu), and an id lookup:

```console
$ rucene-cli search quakes --should --term net=ak --should --text vanuatu --limit 50
$ rucene-cli search quakes --id us6000m0xl --fields all
```

## Layout

| Path | Content |
| --- | --- |
| `src/schema.rs` | The schema: column declarations, record parsing, document building |
| `src/csv.rs` | The RFC 4180 reader |
| `src/datetime.rs` | ISO 8601 to and from epoch milliseconds |
| `src/index_ops.rs` | `create`, `add`, `update`, `delete` |
| `src/search.rs` | Query and sort construction, sorted search, TSV output |
| `src/cli.rs` | Argument parsing and dispatch |
| `src/main.rs` | The `rucene-cli` entry point |
| `tests/` | Integration tests. Their expected results are computed from the fixture rows in plain Rust. |

The crate uses only the standard library and `rucene`.
