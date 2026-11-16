//! The ten Portal BASE indices, one [`EntitySchema`] each.
//!
//! The entities, keys and placeholders come from the data profile of rmp
//! task #531 (Portal BASE / IMPIC datasets on dados.gov.pt, snapshot
//! 2026-11-15). Each module documents where every field comes from, and lists
//! the source fields it intentionally does not index, with the reason; the
//! crate README holds the per-index schema tables (Lucene field classes,
//! stored, searchable, sortable), which a test checks against these
//! declarations.
//!
//! | Command / directory | Key | Documents (full data, #531) |
//! | --- | --- | --- |
//! | [`contratos`] | `idcontrato` | 2,295,074 |
//! | [`fornecedores`] | `nif` | 95,321 keyed suppliers |
//! | [`adjudicantes`] | `nif` | 11,209 |
//! | [`procedimentos`] | `idprocedimento` | 1,938,286 |
//! | [`cpv`] | `cpv` | 7,669 |
//! | [`locais`] | `local` | 477 |
//! | [`nuts`] | `nuts` | 256 |
//! | [`modificacoes`] | `idcontrato/sequencia` | 90,135 |
//! | [`anuncios`] | `IDAnuncio` | 268,733 |
//! | [`convidadas`] | `nif` | 78,446 keyed |
//!
//! # Type choices shared by every index
//!
//! * **Ids and foreign keys** (`idcontrato`, `idprocedimento`, NIFs,
//!   `IDAnuncio`): [`Exact`](crate::schema::FieldKind::Exact) (`StringField`):
//!   they are matched, never sorted, so no doc values.
//! * **Codes and closed value lists** (procedure and contract types, regime,
//!   CPV, NUTS, places, countries): [`Keyword`](crate::schema::FieldKind::Keyword)
//!   (`KeywordField`): exact match plus sorted-set doc values for sorting.
//! * **`Sim`/`Não` and JSON booleans**: [`Flag`](crate::schema::FieldKind::Flag),
//!   indexed as `sim`/`nao` so the query value is spelling-independent.
//! * **Free pt-PT text** (objects, descriptions, causes, observations):
//!   [`Text`](crate::schema::FieldKind::Text); **names and short labels**
//!   (entity names, CPV/NUTS descriptions, places, legal grounds):
//!   [`Name`](crate::schema::FieldKind::Name), text plus an exact/sort copy.
//!   Long free text gets no copy: `objectoContrato` reaches 49,311 characters,
//!   and Lucene rejects a term over 32,766 bytes.
//! * **Money**: [`Double`](crate::schema::FieldKind::Double). Values have at
//!   most 2 decimals and stay below 1e11, i.e. at most 13 significant digits,
//!   which `f64` (15 to 17) round-trips to the same decimal text. Only the
//!   rare `precoEstimadoProc` outliers written with 6 decimals can exceed
//!   that and lose their last digit; range and sort need no exact decimal
//!   arithmetic, and no sum is computed in the index.
//! * **Counts, years, days** (largest measured: 99,999,999 days, 197,218
//!   contracts per code): [`Int`](crate::schema::FieldKind::Int).
//! * **Dates**: [`Date`](crate::schema::FieldKind::Date), epoch milliseconds of
//!   UTC midnight.
//! * **Hierarchical codes** (CPV, NUTS, places) feed derived keyword fields
//!   holding their ancestors ([`Scheme`](crate::schema::Scheme)), so a whole
//!   CPV division, NUTS region or district is one exact term.
//! * **Links**: [`Stored`](crate::schema::FieldKind::Stored) only.

use crate::schema::EntitySchema;

pub mod adjudicantes;
pub mod anuncios;
pub mod contratos;
pub mod convidadas;
pub mod cpv;
pub mod fornecedores;
pub mod locais;
pub mod modificacoes;
pub mod nuts;
pub mod procedimentos;

/// NIF placeholder: `-` marks a party without a tax number (#531).
pub const NIF_ABSENT: &[&str] = &["-"];

/// Price placeholder: `0` means "not filled" in the base, effective and
/// estimated prices (#531).
pub const ZERO_ABSENT: &[&str] = &["0"];

/// Term placeholder: `99999999` days is a sentinel (#531).
pub const TERM_ABSENT: &[&str] = &["99999999"];

/// Free-text placeholders written for "nothing to say" (#531).
pub const FREE_TEXT_ABSENT: &[&str] = &["n/a", "N/A", ".", "-"];

/// `idINCM` placeholder (#531).
pub const INCM_ABSENT: &[&str] = &["-1"];

/// The derived CPV levels: division, group, class, category.
pub const CPV_LEVELS: &[&str] = &["cpvDivisao", "cpvGrupo", "cpvClasse", "cpvCategoria"];

/// The derived NUTS levels 1, 2 and 3.
pub const NUTS_LEVELS: &[&str] = &["nutsNivel1", "nutsNivel2", "nutsNivel3"];

/// The derived place levels: country, district, municipality.
pub const PLACE_LEVELS: &[&str] = &["localNivel1", "localNivel2", "localNivel3"];

/// Every index, in usage order.
pub const ALL: &[&EntitySchema] = &[
    &contratos::SCHEMA,
    &fornecedores::SCHEMA,
    &adjudicantes::SCHEMA,
    &procedimentos::SCHEMA,
    &cpv::SCHEMA,
    &locais::SCHEMA,
    &nuts::SCHEMA,
    &modificacoes::SCHEMA,
    &anuncios::SCHEMA,
    &convidadas::SCHEMA,
];

/// The index whose command is `name`, if any.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static EntitySchema> {
    ALL.iter().copied().find(|schema| schema.name == name)
}
