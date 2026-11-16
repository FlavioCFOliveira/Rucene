//! `fornecedores`: one document per supplier (adjudicatário) NIF.
//!
//! Built by the ingestion script from `contratos[].adjudicatarios[]` (every
//! distinct valid NIF) and the `entidades` dataset (its totals, matched by
//! NIF).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `adjudicatarios[].nifEntContrato` | `nif`, exact (key) |
//! | `adjudicatarios[].descEntContrato` | `nome[]`, name: every spelling seen (47.4% of suppliers have several, #531) |
//! | `adjudicatarios[].codPaisEntContrato` | `codPais[]`, keyword |
//! | `adjudicatarios[].NUTsCod` / `NUTsDesc` | `nutsCod[]` (keyword, derived levels 1-3), `nutsDesc[]` (text) |
//! | `adjudicatarios[].adjudicatarioPME` | `pme`, flag (`Sim` when any contract says so) |
//! | (aggregates over contracts) | `numContratos`, `valorTotal` (price split equally among co-suppliers), `primeiroAno`, `ultimoAno` |
//! | `entidades.numContratos`, `totAdjudicatario`, `totValorContratIni`, `descPais` | `entNumContratos`, `entTotAdjudicatario`, `entTotValorContratIni`, `descPais` |
//!
//! **Not indexed, on purpose:** suppliers without a NIF (`-`, 10.2% of
//! references, mostly individuals named personally, #531) have no stable key
//! and are not documents here; their names stay searchable in
//! [`contratos`](super::contratos) (`adjudicatarioNome`).
//! `adjudicatarios[].tipoEntidade`/`atividadeEntidade` are null for suppliers
//! in 85-90% of references and are indexed for contracting entities only.
//! The `entidades` contracting-entity totals belong to
//! [`adjudicantes`](super::adjudicantes).

use crate::entities::NUTS_LEVELS;
use crate::schema::FieldKind::{Double, Exact, Flag, Int, Keyword, Name, Text};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `fornecedores` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "fornecedores",
    summary: "suppliers / adjudicatários (key nif)",
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
        F::new("pme", Flag),
        F::new("numContratos", Int),
        F::new("valorTotal", Double),
        F::new("primeiroAno", Int),
        F::new("ultimoAno", Int),
        F::new("entNumContratos", Int),
        F::new("entTotAdjudicatario", Int),
        F::new("entTotValorContratIni", Double),
        F::new("descPais", Keyword),
    ],
    key: &["nif"],
    text_fields: &["nome"],
    output_fields: &["nif", "nome", "numContratos", "valorTotal"],
};
