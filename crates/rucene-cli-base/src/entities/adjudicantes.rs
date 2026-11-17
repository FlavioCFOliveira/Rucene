//! `adjudicantes`: one document per contracting entity NIF.
//!
//! Built by the ingestion script from `contratos[].adjudicante[]` and the
//! `entidades` dataset (matched by NIF; it covers 100% of contracting-entity
//! NIFs, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `adjudicante[].nifEntContrato` | `nif`, exact (key) |
//! | `adjudicante[].descEntContrato` | `nome[]`, name (sub-units and renamings share a NIF, #531) |
//! | `adjudicante[].codPaisEntContrato` | `codPais[]`, keyword |
//! | `adjudicante[].NUTsCod` / `NUTsDesc` | `nutsCod[]` (keyword, derived levels 1-3), `nutsDesc[]` (text) |
//! | `adjudicante[].tipoEntidade` | `tipoEntidade[]`, keyword (20 values) |
//! | `adjudicante[].atividadeEntidade` | `atividadeEntidade[]`, keyword (11 values) |
//! | (aggregates over contracts) | `numContratos`, `valorTotal` (price split equally among co-entities), `primeiroAno`, `ultimoAno` |
//! | `entidades.numContratos`, `totAdjudicante`, `totAdjudicanteValorContratIni`, `descPais` | `entNumContratos`, `entTotAdjudicante`, `entTotAdjudicanteValorContratIni`, `descPais` |
//!
//! **Not indexed, on purpose:** `adjudicante[].adjudicatarioPME` (always null
//! for contracting entities, #531); the `entidades` supplier totals (in
//! [`fornecedores`](super::fornecedores)); `entidades.AliasPais` (duplicates
//! `codPais`).

use crate::entities::NUTS_LEVELS;
use crate::schema::FieldKind::{Double, Exact, Int, Keyword, Name, Text};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `adjudicantes` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "adjudicantes",
    summary: "contracting entities / adjudicantes (key nif)",
    fields: &[
        F::new("nif", Exact),
        F::new("nome", Name).multi(),
        F::new("codPais", Keyword).multi(),
        F::new("nutsCod", Keyword)
            .multi()
            .levels(Scheme::Nuts, NUTS_LEVELS),
        F::derived("nutsNivel1"),
        F::derived("nutsNivel2"),
        F::derived("nutsNivel3"),
        F::new("nutsDesc", Text).multi(),
        F::new("tipoEntidade", Keyword).multi(),
        F::new("atividadeEntidade", Keyword).multi(),
        F::new("numContratos", Int),
        F::new("valorTotal", Double),
        F::new("primeiroAno", Int),
        F::new("ultimoAno", Int),
        F::new("entNumContratos", Int),
        F::new("entTotAdjudicante", Int),
        F::new("entTotAdjudicanteValorContratIni", Double),
        F::new("descPais", Keyword),
    ],
    key: &["nif"],
    text_fields: &["nome"],
    output_fields: &["nif", "nome", "numContratos", "valorTotal"],
};
