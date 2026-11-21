//! `convidadas`: one document per invited entity NIF
//! (`contratos[].entidadesConvidadas[]`, 78,446 keyed, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `entidadesConvidadas[].NIPCEntidade` | `nif`, exact (key) |
//! | `entidadesConvidadas[].Entidade` | `nome[]`, name |
//! | `entidadesConvidadas[].AliasPais` | `aliasPais[]`, keyword (country code) |
//! | `entidadesConvidadas[].Pais` | `pais[]`, keyword (country name, closed list) |
//! | (aggregates) | `numConvites` (contracts listing it), `primeiroAno`, `ultimoAno` |
//!
//! **Not indexed, on purpose:** invitees without a NIF (`-`, 169,125
//! references, #531) have no stable key; their names stay searchable in
//! [`contratos`](super::contratos) (`convidadaNome`).

use crate::schema::FieldKind::{Exact, Int, Keyword, Name};
use crate::schema::{EntitySchema, FieldSpec as F};

/// The `convidadas` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "convidadas",
    summary: "entities invited to tender (key nif)",
    fields: &[
        F::new("nif", Exact),
        F::new("nome", Name).multi(),
        F::new("aliasPais", Keyword).multi(),
        F::new("pais", Keyword).multi(),
        F::new("numConvites", Int),
        F::new("primeiroAno", Int),
        F::new("ultimoAno", Int),
    ],
    key: &["nif"],
    text_fields: &["nome"],
    output_fields: &["nif", "nome", "numConvites"],
};
