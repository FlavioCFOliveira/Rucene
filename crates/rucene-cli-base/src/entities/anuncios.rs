//! `anuncios`: one document per Diário da República announcement
//! (`IDAnuncio`), from the yearly `anuncios<year>.json` files (272,203 rows,
//! 268,733 distinct ids: `update` keeps the last row of each, #531).
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `IDAnuncio`, `nAnuncio`, `IdIncm`, `numDR` | same names, exact (`IdIncm` `-1` = absent) |
//! | `dataPublicacao`, `DataLimitePropostas` | date |
//! | `nifEntidade` | exact (foreign key to [`adjudicantes`](super::adjudicantes)) |
//! | `designacaoEntidade` | name |
//! | `descricaoAnuncio` | text |
//! | `url`, `PecasProcedimento` | stored only |
//! | `serie`, `tipoActo`, `modeloAnuncio` | keyword |
//! | `tiposContrato[]` | keyword, multi-valued |
//! | `PrecoBase` (a decimal string) | double (0 = absent) |
//! | `CPVs[]` (`"code - description"`) + `Lotes[].cpvAliasId`/`cpvDescription` | `cpv[]` (keyword, derived levels), `cpvDescricao[]` (text) |
//! | `Ano` | int |
//! | `PrazoPropostas` | int days |
//! | `CriterAmbient`, `AcordoQuadro`, `sistemaAquisicaoDinamico` | flag |
//! | `Lotes[]` | `loteDesignacao[]`, `criterioLote[]` (text), `loteValorBase[]` (double), `criterioAquisicaoTipo[]`, `criterioAquisicaoTipoFator[]` (keyword) |
//! | `anuncioAnterior[]` | `anteriorIDAnuncio[]`, `anteriorNAnuncio[]` (exact) |
//!
//! **Not indexed, on purpose:** `Lotes[].numLot` (not unique),
//! `Lotes[].criterioLotePercentagem` and `criterioAquisicaoPonderacao`
//! (weights meaningless apart from their criterion, which flattening
//! separates); `anuncioAnterior[].antIdIncm`, `antDataPublicacao` and
//! `antTipoActo`, which are fields of the earlier announcement, reachable by
//! its id.

use crate::entities::{CPV_LEVELS, INCM_ABSENT, NIF_ABSENT, ZERO_ABSENT};
use crate::schema::FieldKind::{Date, Double, Exact, Flag, Int, Keyword, Name, Stored, Text};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `anuncios` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "anuncios",
    summary: "procurement announcements (key IDAnuncio)",
    fields: &[
        F::new("IDAnuncio", Exact),
        F::new("nAnuncio", Exact),
        F::new("IdIncm", Exact).absent(INCM_ABSENT),
        F::new("dataPublicacao", Date),
        F::new("nifEntidade", Exact).absent(NIF_ABSENT),
        F::new("designacaoEntidade", Name),
        F::new("descricaoAnuncio", Text),
        F::new("url", Stored),
        F::new("numDR", Exact),
        F::new("serie", Keyword),
        F::new("tipoActo", Keyword),
        F::new("tiposContrato", Keyword).multi(),
        F::new("PrecoBase", Double).absent(ZERO_ABSENT),
        F::new("cpv", Keyword)
            .multi()
            .levels(Scheme::Cpv, CPV_LEVELS),
        F::derived("cpvDivisao"),
        F::derived("cpvGrupo"),
        F::derived("cpvClasse"),
        F::derived("cpvCategoria"),
        F::new("cpvDescricao", Text).multi(),
        F::new("modeloAnuncio", Keyword),
        F::new("Ano", Int),
        F::new("CriterAmbient", Flag),
        F::new("PrazoPropostas", Int),
        F::new("DataLimitePropostas", Date),
        F::new("PecasProcedimento", Stored),
        F::new("AcordoQuadro", Flag),
        F::new("sistemaAquisicaoDinamico", Flag),
        F::new("loteDesignacao", Text).multi(),
        F::new("loteValorBase", Double).multi(),
        F::new("criterioLote", Text).multi(),
        F::new("criterioAquisicaoTipo", Keyword).multi(),
        F::new("criterioAquisicaoTipoFator", Keyword).multi(),
        F::new("anteriorIDAnuncio", Exact).multi(),
        F::new("anteriorNAnuncio", Exact).multi(),
    ],
    key: &["IDAnuncio"],
    text_fields: &["descricaoAnuncio", "designacaoEntidade", "cpvDescricao"],
    output_fields: &[
        "IDAnuncio",
        "dataPublicacao",
        "designacaoEntidade",
        "descricaoAnuncio",
    ],
};
