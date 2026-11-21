//! `procedimentos`: one document per procurement procedure
//! (`idprocedimento`), aggregated by the ingestion script over the contracts
//! that share it (1 contract in 95.24% of procedures, at most 541, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `idprocedimento` | exact (key) |
//! | `tipoprocedimento` | keyword (constant within a procedure, #531) |
//! | `precoBaseProcedimento`, `precoEstimadoProc` | double, maximum over the contracts (0 = absent) |
//! | `idcontrato` | `idcontrato[]`, exact (foreign keys to [`contratos`](super::contratos)) |
//! | `objectoContrato` | `objectoContrato[]`, text (distinct objects) |
//! | `adjudicante[].nifEntContrato` | `adjudicanteNif[]`, exact (4,457 procedures have several entity sets) |
//! | `IDAnuncio`, `nAnuncio` | `IDAnuncio[]`, `nAnuncio[]`, exact |
//! | (aggregates) | `numContratos`, `valorTotal` (sum of `precoContratual`), `primeiraPublicacao`, `ultimaPublicacao` (dates) |
//!
//! **Not indexed, on purpose:** every other contract field: it describes a
//! contract and is searched in [`contratos`](super::contratos) by
//! `idprocedimento`.

use crate::entities::ZERO_ABSENT;
use crate::schema::FieldKind::{Date, Double, Exact, Int, Keyword, Text};
use crate::schema::{EntitySchema, FieldSpec as F};

/// The `procedimentos` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "procedimentos",
    summary: "procurement procedures (key idprocedimento)",
    fields: &[
        F::new("idprocedimento", Exact),
        F::new("tipoprocedimento", Keyword),
        F::new("precoBaseProcedimento", Double).absent(ZERO_ABSENT),
        F::new("precoEstimadoProc", Double).absent(ZERO_ABSENT),
        F::new("idcontrato", Exact).multi(),
        F::new("objectoContrato", Text).multi(),
        F::new("adjudicanteNif", Exact).multi(),
        F::new("IDAnuncio", Exact).multi(),
        F::new("nAnuncio", Exact).multi(),
        F::new("numContratos", Int),
        F::new("valorTotal", Double),
        F::new("primeiraPublicacao", Date),
        F::new("ultimaPublicacao", Date),
    ],
    key: &["idprocedimento"],
    text_fields: &["objectoContrato"],
    output_fields: &[
        "idprocedimento",
        "tipoprocedimento",
        "numContratos",
        "valorTotal",
    ],
};
