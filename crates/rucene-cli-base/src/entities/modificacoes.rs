//! `modificacoes`: one document per contract modification, from the yearly
//! `modcontrats<year>.json` files.
//!
//! The source has no modification id (#531): the key is `idcontrato` plus a
//! `sequencia` (1..n within the contract) that the ingestion script assigns
//! by `modifContratoData`, then `modifDataPublicacao`, then file order. The
//! key text is `idcontrato/sequencia`.
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `idcontrato` | exact (key part 1, foreign key to [`contratos`](super::contratos)) |
//! | (assigned) | `sequencia`, int (key part 2) |
//! | `modifContratoFundamento` | name (42 legal grounds) |
//! | `modifContratoTipoAto` | keyword (4 values) |
//! | `modifContratoData`, `modifDataPublicacao` (`DD/MM/YYYY 00:00:00`) | date |
//! | `modifContratoPrecoAlterado` | double |
//! | `modifPrazoExecucao` (digits in a string) | int days (99999999 = absent) |
//! | `Ano` (the contract's year) | int |
//! | `modifEntidadesSubcontrat[]` | `subcontratadaNif[]` (exact, `-` = absent), `subcontratadaNome[]` (text) |
//! | `modifJustNContratEscrito` | name (20 values) |
//!
//! All 10 source fields are mapped.

use crate::entities::{NIF_ABSENT, TERM_ABSENT};
use crate::schema::FieldKind::{Date, Double, Exact, Int, Keyword, Name, Text};
use crate::schema::{EntitySchema, FieldSpec as F};

/// The `modificacoes` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "modificacoes",
    summary: "contract modifications (key idcontrato/sequencia)",
    fields: &[
        F::new("idcontrato", Exact),
        F::new("sequencia", Int),
        F::new("modifContratoFundamento", Name),
        F::new("modifContratoTipoAto", Keyword),
        F::new("modifContratoData", Date),
        F::new("modifDataPublicacao", Date),
        F::new("modifContratoPrecoAlterado", Double),
        F::new("modifPrazoExecucao", Int).absent(TERM_ABSENT),
        F::new("Ano", Int),
        F::new("subcontratadaNif", Exact).multi().absent(NIF_ABSENT),
        F::new("subcontratadaNome", Text).multi(),
        F::new("modifJustNContratEscrito", Name),
    ],
    key: &["idcontrato", "sequencia"],
    text_fields: &[
        "modifContratoFundamento",
        "subcontratadaNome",
        "modifJustNContratEscrito",
    ],
    output_fields: &[
        "idcontrato",
        "sequencia",
        "modifContratoData",
        "modifContratoPrecoAlterado",
        "modifContratoFundamento",
    ],
};
