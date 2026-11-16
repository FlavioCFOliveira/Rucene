//! `locais`: one document per execution place string
//! (`contratos[].localExecucao[]`, 477 distinct, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `localExecucao[]` (`Country, District, Municipality`) | `local`, keyword (key, the string as is), with derived `localNivel1-3` |
//! | its parts 1, 2, 3 | `pais`, `distrito`, `concelho`, name (accent-insensitive search and exact sort) |
//! | (aggregates) | `numContratos`, `valorTotal` (sum of `precoContratual` of the contracts listing it) |
//!
//! The derived levels hold path prefixes (`Portugal, Lisboa`), so a district
//! is one exact term that cannot collide with a same-named district of
//! another country; `distrito` is the plain name for text search.

use crate::entities::PLACE_LEVELS;
use crate::schema::FieldKind::{Double, Int, Keyword, Name};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `locais` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "locais",
    summary: "execution places (key local)",
    fields: &[
        F::new("local", Keyword).levels(Scheme::Place, PLACE_LEVELS),
        F::derived("localNivel1"),
        F::derived("localNivel2"),
        F::derived("localNivel3"),
        F::new("pais", Name),
        F::new("distrito", Name),
        F::new("concelho", Name),
        F::new("numContratos", Int),
        F::new("valorTotal", Double),
    ],
    key: &["local"],
    text_fields: &["pais", "distrito", "concelho"],
    output_fields: &["local", "numContratos", "valorTotal"],
};
