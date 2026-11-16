//! `nuts`: one document per NUTS region code (256 codes in `NUTs[]`, plus
//! party `NUTsCod` values such as `PT` and `PTZZZ`, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `NUTs[]` code (before `" - "`), `NUTsCod` | `nuts`, keyword (key), with derived `nutsNivel1-3` |
//! | `NUTs[]` description (after `" - "`), `NUTsDesc` | `descricao`, name |
//! | (aggregates) | `numContratos` (contracts listing the code), `numAdjudicantes`, `numFornecedores` (entities whose `NUTsCod` is the code), `valorTotal` |
//!
//! NUTS 2013 and 2024 codes are mixed in the source (#531); both are kept as
//! given, each its own document.

use crate::entities::NUTS_LEVELS;
use crate::schema::FieldKind::{Double, Int, Keyword, Name};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `nuts` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "nuts",
    summary: "NUTS regions (key nuts)",
    fields: &[
        F::new("nuts", Keyword).levels(Scheme::Nuts, NUTS_LEVELS),
        F::derived("nutsNivel1"),
        F::derived("nutsNivel2"),
        F::derived("nutsNivel3"),
        F::new("descricao", Name),
        F::new("numContratos", Int),
        F::new("numAdjudicantes", Int),
        F::new("numFornecedores", Int),
        F::new("valorTotal", Double),
    ],
    key: &["nuts"],
    text_fields: &["descricao"],
    output_fields: &["nuts", "descricao", "numContratos"],
};
