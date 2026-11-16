# rucene-cli-base

A second command-line example of the [`rucene`](../../README.md) crate, a Rust
port of Apache Lucene Core 10.5.0, over a European Portuguese dataset: the
public procurement records of **Portal BASE** (IMPIC), published as open data
on dados.gov.pt. The `rucene-base` binary manages **one Lucene index per
entity** of the dataset, each with its own command, and searches them
**insensitive to case, accents and diacritics**: `acao`, `AÇÃO` and `ação`
match the same documents.

Like [`rucene-cli-example`](../rucene-cli-example/README.md), everything goes
through the public `rucene` API (`IndexWriter`, `DirectoryReader`,
`IndexSearcher`, the Lucene 10 field classes, points, doc values, `Sort`,
`QueryBuilder`), and the analysis chain is built from the ported Lucene
components of `rucene-analysis-common`.

## Commands

```text
rucene-base <entity> create <root>
rucene-base <entity> add    <root> (--file F.jsonl | --set field=value ...)
rucene-base <entity> update <root> (--file F.jsonl | --set field=value ...)
rucene-base <entity> delete <root> --key K [--key K ...]
rucene-base <entity> get    <root> --key K [--key K ...]
rucene-base <entity> search <root> [criteria ...] [--sort KEY[:asc|desc],...]
                                   [--limit N] [--fields f,...|all] [--exact-total]
rucene-base <entity> count  <root> [criteria ...]
rucene-base <entity> stats  <root>
rucene-base <entity> help
rucene-base help
```

| Entity | Documents | Key |
| --- | --- | --- |
| `contratos` | contracts | `idcontrato` |
| `fornecedores` | suppliers (adjudicatários) with a NIF | `nif` |
| `adjudicantes` | contracting entities | `nif` |
| `procedimentos` | procurement procedures | `idprocedimento` |
| `cpv` | CPV codes | `cpv` |
| `locais` | execution places | `local` |
| `nuts` | NUTS regions | `nuts` |
| `modificacoes` | contract modifications | `idcontrato/sequencia` |
| `anuncios` | Diário da República announcements | `IDAnuncio` |
| `convidadas` | entities invited to tender, with a NIF | `nif` |

* **Layout.** Every command takes the **root** directory; the index of an
  entity lives only in `<root>/<entity>`. `create` creates that subdirectory
  (and the root when missing) with `OpenMode.CREATE`, replacing an existing
  index of the same entity and never touching the others. No file is written
  in the root itself.
* **Writes.** Each write command is one `IndexWriter` session: commit on
  success, rollback on any error, so an index keeps its last commit. `add`
  uses `addDocument` and ignores the key; `update` uses `updateDocument` on
  the key, so a key repeated in the input (or already indexed) leaves one
  document, the last. `delete` removes every document of each key.
* **`get`** prints the stored fields of each key as one JSON object per line,
  in the input format, so its output can be fed back to `update`. A key with
  no document is reported after the documents found (exit code 1).
* **`search`** prints tab-separated values: a `# total hits` line, a header,
  one row per hit (the requested fields, the sort values, the score).
  Multi-valued fields are joined with ` | `. `count` prints the exact number
  of matches. `stats` prints the directory, live and deleted documents,
  `maxDoc` and segment count.
* **Exit codes:** 0 success, 1 input or index error, 2 usage error. Errors go
  to standard error as `rucene-base: error: <message>`.

### Search criteria

Criteria combine with AND; prefix one with `--should` or `--not` to make it
optional or prohibited.

| Flag | Matches |
| --- | --- |
| `--text WORDS` | every word in at least one default text field of the index (Lucene's `MultiFieldQueryParser` with `AND`) |
| `--text-in FIELD=WORDS` | every word in that text or name field (`QueryBuilder.createBooleanQuery(field, text, MUST)`) |
| `--phrase WORDS` | the phrase in any default text field |
| `--phrase-in FIELD=WORDS` | the phrase in that field (`QueryBuilder.createPhraseQuery`) |
| `--term FIELD=VALUE` | the exact value: `TermQuery` on ids, `KeywordField.newExactQuery` on codes, flags and names (on the `_exact` copy), `newExactQuery` of the point field on numbers and dates |
| `--terms FIELD=V1,V2` | any of the values (`TermInSetQuery`, `newSetQuery`) |
| `--range FIELD=LO..HI` | an inclusive number or date range; either bound may be empty |
| `--key KEY` | the document key |

Sort keys: any keyword, flag, name, number or date field (`KeywordField`,
`LongField`, `IntField`, `DoubleField` sort fields, smallest value of a
multi-valued field, missing values last), `_score`, `_doc`. Ids, analysed
text without a copy and stored-only fields have no doc values and cannot be
sorted. Dates are given as `YYYY-MM-DD` or `DD/MM/YYYY`.

Examples:

```text
rucene-base contratos search /data/base --text "AQUISIÇÃO formacao" --sort precoContratual:desc
rucene-base contratos search /data/base --term cpvDivisao=45 --range dataPublicacao=2025-01-01.. --exact-total
rucene-base contratos count  /data/base --term localNivel2="Portugal, Lisboa" --not --term CritMateriais=nao
rucene-base fornecedores search /data/base --text "CONSTRUCAO" --sort valorTotal:desc --limit 20
rucene-base modificacoes get /data/base --key 11103352/2
```

## Analysis: case, accent and diacritic insensitivity

Every text and name field is analysed, at index and at query time, by
`PortugueseFoldingAnalyzer` (`src/analysis.rs`):

```text
StandardTokenizer -> LowerCaseFilter -> ASCIIFoldingFilter -> StopFilter(folded Portuguese stop words)
  -> PatternReplaceFilter(oes$ -> ões) -> PatternReplaceFilter(aes$ -> ães) -> PatternReplaceFilter(ois$ -> óis)
  -> PortugueseLightStemFilter -> ASCIIFoldingFilter
```

In Java this is a `CustomAnalyzer` with tokenizer `standard` and filters
`lowercase`, `asciiFolding`, `stop` (the folded words of Lucene's
`portuguese_stop.txt`), three `patternReplace` (`replace=first`),
`portugueseLightStem` and `asciiFolding`; the module documentation of
`src/analysis.rs` gives the exact builder calls.

* **Folding comes first.** Spellings that differ only in diacritics become
  the same string before the stop filter and the stemmer: `preço`/`preco`,
  `música`/`musica`, `português`/`portugues`, `ação`/`acao`.
* **Three plural endings are restored before stemming.**
  `PortugueseLightStemmer` keys its `-ões`/`-ães` → `-ão` and `-óis` → `-ol`
  rules on accented letters, which folding removed (`acoes` would stem to
  `acoe`, `anzois` to `anzoil`). The `PatternReplaceFilter`s put back exactly
  those accents on the last three letters, so the stemmer applies the rule
  Lucene applies to the accented spelling. Its other rules (`-eis`/`-éis`,
  `-ais`, `-es`, `-ns`, `-mente`, feminine and vowel endings) behave the same
  on folded input.
* **The last `ASCIIFoldingFilter`** folds the `ã`/`ê` the stemmer writes on
  stems it leaves unfolded.
* **The stop words are folded** to match the folded tokens, so `não` and
  `nao` are both removed.

Result: `ação`, `acao`, `AÇÃO`, `ações`, `acoes`, `AÇÕES` → `acao`;
`acção`, `acções`, `accao`, `accoes` → `acca`; `pão`, `pães`, `paes` → `pao`;
`limão`, `limões`, `limoes` → `lima`; `papéis`, `papeis`, `papel` → `papel`;
`anzóis`, `anzois`, `anzol` → `anzol`. The cost: an unaccented word ending in
`-oes`, `-aes` or `-ois` that is not such a plural (a foreign word such as
`shoes`) gets the plural rule, exactly as its accented spelling would in
Lucene's `PortugueseAnalyzer`.

Ids and codes are never analysed; names also get a not-stored
`KeywordField` copy (`<field>_exact`) for exact match and sorting.

## Bulk input: JSON Lines

`add --file` and `update --file` read **JSON Lines**: one flat JSON object per
line, one document of the command's index; blank lines are skipped. The full
specification is the module documentation of `src/input.rs`; in short:

* member names are the field names of the index (tables below);
* multi-valued fields (`name[]`) take an array of scalars; nested objects
  are rejected, nested source lists are flattened into parallel fields;
* flags take `"Sim"`, `"Não"` (any case) or `true`/`false`; ints and doubles
  take a JSON number or a string of one; dates take `YYYY-MM-DD` or
  `DD/MM/YYYY`, optionally followed by ` HH:MM:SS`;
* `null`, `""`, `[]`, a missing member, or a placeholder listed in the
  *Absent when* column adds no field (placeholders mean "unknown" in the
  source, and indexing them would skew ranges and sorts);
* derived fields are computed by the CLI and must not be given;
* an unknown or repeated member, a bad value or a record without its key
  fails the command; nothing of the file is committed.

`--set field=value` builds one record; repeat a multi-valued field to give
several values.

## Schemas

Each entity module (`src/entities/<entity>.rs`) documents where every field
comes from in the Portal BASE files and which source fields are intentionally
not indexed, with the reason. The kinds map to Lucene field classes as
follows (`src/schema.rs`): **exact** ids and foreign keys are `StringField`
(no doc values: never sorted); **keyword** codes and closed value lists are
`KeywordField`; **flag** is a `KeywordField` holding `sim`/`nao`; **text** is
an analysed `TextField`; **name** adds a not-stored exact/sort copy;
**stored** is a `StoredField`; **date** is a `LongField` of epoch
milliseconds (UTC midnight) plus a `StoredField` with `YYYY-MM-DD`; **int**
and **double** are `IntField` and `DoubleField`. Derived fields hold the
ancestors of hierarchical codes: CPV division, group, class and category (2,
3, 4 and 5 leading digits), NUTS levels 1 to 3 (3, 4 and 5 leading
characters), and place prefixes (`Portugal`, `Portugal, Lisboa`, ...), so a
whole branch is one exact term. Every document also has the not-stored
`StringField` `_key`, the term `update`, `delete` and `get` act on.

These tables are produced by `schema::markdown_table`, which also backs
`rucene-base <entity> help`; `tests/schemas.rs` checks that this README holds
the current table of every index.

### `contratos`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `idcontrato` | exact (key) | `StringField` | yes | exact, set | no |  |
| `idprocedimento` | exact | `StringField` | yes | exact, set | no |  |
| `tipoprocedimento` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `tipoContrato[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `objectoContrato` | text | `TextField` | yes | words, phrase | no |  |
| `descContrato` | text | `TextField` | yes | words, phrase | no |  |
| `dataPublicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `dataCelebracaoContrato` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `dataDecisaoAdjudicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `dataFechoContrato` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `precoContratual` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `precoBaseProcedimento` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `PrecoTotalEfetivo` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `precoEstimadoProc` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `prazoExecucao` | int | `IntField` | yes | exact, set, range | yes | `99999999` |
| `Ano` | int | `IntField` | yes | exact, set, range | yes |  |
| `regime` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `fundamentacao` | name | `TextField` + `KeywordField` `fundamentacao_exact` | yes | words, phrase; exact, set | yes |  |
| `fundamentAjusteDireto` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `tipoFimContrato` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `TipoMEC` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `estadoContrato` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `ProcedimentoCentralizado` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `CritMateriais` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `ContratEcologico` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `compraConjunta` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `ocorrencias` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `numAcordoQuadro` | exact | `StringField` | yes | exact, set | no | `1` |
| `DescrAcordoQuadro` | text | `TextField` | yes | words, phrase | no |  |
| `nAnuncio` | exact | `StringField` | yes | exact, set | no |  |
| `IDAnuncio` | exact | `StringField` | yes | exact, set | no |  |
| `idINCM` | exact | `StringField` | yes | exact, set | no | `-1` |
| `TipoAnuncio` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `adjudicanteNif[]` | exact | `StringField` | yes | exact, set | no | `-` |
| `adjudicanteNome[]` | name | `TextField` + `KeywordField` `adjudicanteNome_exact` | yes | words, phrase; exact, set | yes |  |
| `adjudicatarioNif[]` | exact | `StringField` | yes | exact, set | no | `-` |
| `adjudicatarioNome[]` | name | `TextField` + `KeywordField` `adjudicatarioNome_exact` | yes | words, phrase; exact, set | yes |  |
| `cpv[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `cpvDivisao` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvGrupo` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvClasse` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvCategoria` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvDescricao[]` | text | `TextField` | yes | words, phrase | no |  |
| `cpvPrincipal` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `localExecucao[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `localNivel1` | keyword (derived) | `KeywordField`, from `localExecucao` | no | exact, set | yes |  |
| `localNivel2` | keyword (derived) | `KeywordField`, from `localExecucao` | no | exact, set | yes |  |
| `localNivel3` | keyword (derived) | `KeywordField`, from `localExecucao` | no | exact, set | yes |  |
| `nuts[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `nutsNivel1` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `nutsNivel2` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `nutsNivel3` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `justifNReducEscrContrato[]` | text | `TextField` | yes | words, phrase | no |  |
| `concorrenteNif[]` | exact | `StringField` | yes | exact, set | no | `-` |
| `concorrenteNome[]` | text | `TextField` | yes | words, phrase | no |  |
| `convidadaNif[]` | exact | `StringField` | yes | exact, set | no | `-` |
| `convidadaNome[]` | text | `TextField` | yes | words, phrase | no |  |
| `loteDesignacao[]` | text | `TextField` | yes | words, phrase | no |  |
| `criterioLote[]` | text | `TextField` | yes | words, phrase | no |  |
| `criterioAdjudicacaoTipo[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `criterioAdjudicacaoTipoFator[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `criterioAdjudicacaoFator[]` | text | `TextField` | yes | words, phrase | no |  |
| `numPropostas` | int | `IntField` | yes | exact, set, range | yes |  |
| `linkPecasProc` | stored | `StoredField` | yes | no | no |  |
| `Observacoes` | text | `TextField` | yes | words, phrase | no | `n/a`, `N/A`, `.`, `-` |
| `CausasAlteraPrazo` | text | `TextField` | yes | words, phrase | no | `n/a`, `N/A`, `.`, `-` |
| `CausasAlteraPreco` | text | `TextField` | yes | words, phrase | no | `n/a`, `N/A`, `.`, `-` |
| `numModificacoes` | int | `IntField` | yes | exact, set, range | yes |  |

### `fornecedores`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `nif` | exact (key) | `StringField` | yes | exact, set | no |  |
| `nome[]` | name | `TextField` + `KeywordField` `nome_exact` | yes | words, phrase; exact, set | yes |  |
| `codPais[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `nutsCod[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `nutsNivel1` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsNivel2` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsNivel3` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsDesc[]` | text | `TextField` | yes | words, phrase | no |  |
| `pme` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `primeiroAno` | int | `IntField` | yes | exact, set, range | yes |  |
| `ultimoAno` | int | `IntField` | yes | exact, set, range | yes |  |
| `entNumContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `entTotAdjudicatario` | int | `IntField` | yes | exact, set, range | yes |  |
| `entTotValorContratIni` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `descPais` | keyword | `KeywordField` | yes | exact, set | yes |  |

### `adjudicantes`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `nif` | exact (key) | `StringField` | yes | exact, set | no |  |
| `nome[]` | name | `TextField` + `KeywordField` `nome_exact` | yes | words, phrase; exact, set | yes |  |
| `codPais[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `nutsCod[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `nutsNivel1` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsNivel2` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsNivel3` | keyword (derived) | `KeywordField`, from `nutsCod` | no | exact, set | yes |  |
| `nutsDesc[]` | text | `TextField` | yes | words, phrase | no |  |
| `tipoEntidade[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `atividadeEntidade[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `primeiroAno` | int | `IntField` | yes | exact, set, range | yes |  |
| `ultimoAno` | int | `IntField` | yes | exact, set, range | yes |  |
| `entNumContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `entTotAdjudicante` | int | `IntField` | yes | exact, set, range | yes |  |
| `entTotAdjudicanteValorContratIni` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `descPais` | keyword | `KeywordField` | yes | exact, set | yes |  |

### `procedimentos`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `idprocedimento` | exact (key) | `StringField` | yes | exact, set | no |  |
| `tipoprocedimento` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `precoBaseProcedimento` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `precoEstimadoProc` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `idcontrato[]` | exact | `StringField` | yes | exact, set | no |  |
| `objectoContrato[]` | text | `TextField` | yes | words, phrase | no |  |
| `adjudicanteNif[]` | exact | `StringField` | yes | exact, set | no |  |
| `IDAnuncio[]` | exact | `StringField` | yes | exact, set | no |  |
| `nAnuncio[]` | exact | `StringField` | yes | exact, set | no |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `primeiraPublicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `ultimaPublicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |

### `cpv`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `cpv` | keyword (key) | `KeywordField` | yes | exact, set | yes |  |
| `cpvDivisao` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvGrupo` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvClasse` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvCategoria` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `descricao` | name | `TextField` + `KeywordField` `descricao_exact` | yes | words, phrase; exact, set | yes |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `numPrincipal` | int | `IntField` | yes | exact, set, range | yes |  |
| `numComplementar` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |

### `locais`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `local` | keyword (key) | `KeywordField` | yes | exact, set | yes |  |
| `localNivel1` | keyword (derived) | `KeywordField`, from `local` | no | exact, set | yes |  |
| `localNivel2` | keyword (derived) | `KeywordField`, from `local` | no | exact, set | yes |  |
| `localNivel3` | keyword (derived) | `KeywordField`, from `local` | no | exact, set | yes |  |
| `pais` | name | `TextField` + `KeywordField` `pais_exact` | yes | words, phrase; exact, set | yes |  |
| `distrito` | name | `TextField` + `KeywordField` `distrito_exact` | yes | words, phrase; exact, set | yes |  |
| `concelho` | name | `TextField` + `KeywordField` `concelho_exact` | yes | words, phrase; exact, set | yes |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |

### `nuts`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `nuts` | keyword (key) | `KeywordField` | yes | exact, set | yes |  |
| `nutsNivel1` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `nutsNivel2` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `nutsNivel3` | keyword (derived) | `KeywordField`, from `nuts` | no | exact, set | yes |  |
| `descricao` | name | `TextField` + `KeywordField` `descricao_exact` | yes | words, phrase; exact, set | yes |  |
| `numContratos` | int | `IntField` | yes | exact, set, range | yes |  |
| `numAdjudicantes` | int | `IntField` | yes | exact, set, range | yes |  |
| `numFornecedores` | int | `IntField` | yes | exact, set, range | yes |  |
| `valorTotal` | double | `DoubleField` | yes | exact, set, range | yes |  |

### `modificacoes`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `idcontrato` | exact (key) | `StringField` | yes | exact, set | no |  |
| `sequencia` | int (key) | `IntField` | yes | exact, set, range | yes |  |
| `modifContratoFundamento` | name | `TextField` + `KeywordField` `modifContratoFundamento_exact` | yes | words, phrase; exact, set | yes |  |
| `modifContratoTipoAto` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `modifContratoData` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `modifDataPublicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `modifContratoPrecoAlterado` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `modifPrazoExecucao` | int | `IntField` | yes | exact, set, range | yes | `99999999` |
| `Ano` | int | `IntField` | yes | exact, set, range | yes |  |
| `subcontratadaNif[]` | exact | `StringField` | yes | exact, set | no | `-` |
| `subcontratadaNome[]` | text | `TextField` | yes | words, phrase | no |  |
| `modifJustNContratEscrito` | name | `TextField` + `KeywordField` `modifJustNContratEscrito_exact` | yes | words, phrase; exact, set | yes |  |

### `anuncios`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `IDAnuncio` | exact (key) | `StringField` | yes | exact, set | no |  |
| `nAnuncio` | exact | `StringField` | yes | exact, set | no |  |
| `IdIncm` | exact | `StringField` | yes | exact, set | no | `-1` |
| `dataPublicacao` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `nifEntidade` | exact | `StringField` | yes | exact, set | no | `-` |
| `designacaoEntidade` | name | `TextField` + `KeywordField` `designacaoEntidade_exact` | yes | words, phrase; exact, set | yes |  |
| `descricaoAnuncio` | text | `TextField` | yes | words, phrase | no |  |
| `url` | stored | `StoredField` | yes | no | no |  |
| `numDR` | exact | `StringField` | yes | exact, set | no |  |
| `serie` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `tipoActo` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `tiposContrato[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `PrecoBase` | double | `DoubleField` | yes | exact, set, range | yes | `0` |
| `cpv[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `cpvDivisao` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvGrupo` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvClasse` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvCategoria` | keyword (derived) | `KeywordField`, from `cpv` | no | exact, set | yes |  |
| `cpvDescricao[]` | text | `TextField` | yes | words, phrase | no |  |
| `modeloAnuncio` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `Ano` | int | `IntField` | yes | exact, set, range | yes |  |
| `CriterAmbient` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `PrazoPropostas` | int | `IntField` | yes | exact, set, range | yes |  |
| `DataLimitePropostas` | date | `LongField` (epoch ms) + `StoredField` | yes | exact, set, range | yes |  |
| `PecasProcedimento` | stored | `StoredField` | yes | no | no |  |
| `AcordoQuadro` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `sistemaAquisicaoDinamico` | flag | `KeywordField` (`sim`/`nao`) | yes | exact, set | yes |  |
| `loteDesignacao[]` | text | `TextField` | yes | words, phrase | no |  |
| `loteValorBase[]` | double | `DoubleField` | yes | exact, set, range | yes |  |
| `criterioLote[]` | text | `TextField` | yes | words, phrase | no |  |
| `criterioAquisicaoTipo[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `criterioAquisicaoTipoFator[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `anteriorIDAnuncio[]` | exact | `StringField` | yes | exact, set | no |  |
| `anteriorNAnuncio[]` | exact | `StringField` | yes | exact, set | no |  |

### `convidadas`

| Field | Kind | Lucene fields | Stored | Searchable | Sortable | Absent when |
| --- | --- | --- | --- | --- | --- | --- |
| `nif` | exact (key) | `StringField` | yes | exact, set | no |  |
| `nome[]` | name | `TextField` + `KeywordField` `nome_exact` | yes | words, phrase; exact, set | yes |  |
| `aliasPais[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `pais[]` | keyword | `KeywordField` | yes | exact, set | yes |  |
| `numConvites` | int | `IntField` | yes | exact, set, range | yes |  |
| `primeiroAno` | int | `IntField` | yes | exact, set, range | yes |  |
| `ultimoAno` | int | `IntField` | yes | exact, set, range | yes |  |

## Test fixtures

`tests/fixtures/<entity>.jsonl` holds a small sample per index (11 to 45
records) built from the Portal BASE files of the 2025 snapshot downloaded on
2026-11-15: 37 contracts chosen to cover accented, upper-case and pre-1990
spellings of `ação`, invited entities, competitors, NUTS, several CPVs and
suppliers, lots with criteria, closing dates, announcements and framework
agreements, plus their modifications and announcements; the other indices are
aggregated over those contracts and the `entidades` dataset. Contracts with a
supplier, competitor or invitee that has no NIF or a personal NIF (first digit
1 to 3, or 45) were excluded, so the fixtures hold no names of private
individuals.

## Tests

```text
cargo test -p rucene-cli-base
```

Unit tests cover the JSON reader, dates, the analysis chain, value parsing,
placeholders and hierarchy levels. Integration tests drive every subcommand
of every index on the fixtures (`tests/contratos.rs`, `tests/entities.rs`),
the layout under one root (`tests/layout.rs`), the binary's exit codes
(`tests/binary.rs`) and the schema declarations (`tests/schemas.rs`).
Expected results are computed from the fixture records with plain string
operations, never from the crate's analyzer or index. Each test uses its own
directory, so the tests run in parallel.
