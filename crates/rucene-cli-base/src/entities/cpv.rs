//! `cpv`: one document per Common Procurement Vocabulary code seen in the
//! contracts (`CPV[].cpvID`, 7,669 codes, exactly one description each,
//! #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `CPV[].cpvID` (`NNNNNNNN-N`) | `cpv`, keyword (key), with derived `cpvDivisao`, `cpvGrupo`, `cpvClasse`, `cpvCategoria` |
//! | `CPV[].cpvdescricao` | `descricao`, name |
//! | (aggregates) | `numContratos`, `numPrincipal`, `numComplementar` (`CPV[].CPVObjeto`), `valorTotal` (sum of `CPV[].cpvValor`) |
//!
//! The key is a [`Keyword`] rather than an
//! exact id because codes sort meaningfully (by hierarchy).

use crate::entities::CPV_LEVELS;
use crate::schema::FieldKind::{Double, Int, Keyword, Name};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `cpv` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "cpv",
    summary: "CPV codes (key cpv)",
    fields: &[
        F::new("cpv", Keyword).levels(Scheme::Cpv, CPV_LEVELS),
        F::derived("cpvDivisao"),
        F::derived("cpvGrupo"),
        F::derived("cpvClasse"),
        F::derived("cpvCategoria"),
        F::new("descricao", Name),
        F::new("numContratos", Int),
        F::new("numPrincipal", Int),
        F::new("numComplementar", Int),
        F::new("valorTotal", Double),
    ],
    key: &["cpv"],
    text_fields: &["descricao"],
    output_fields: &["cpv", "descricao", "numContratos", "valorTotal"],
};
