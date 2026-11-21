//! `contratos`: one document per contract (`idcontrato`), from the yearly
//! `contratos<year>.json` files of the Portal BASE contracts dataset.
//!
//! The 46 top-level source fields map as follows (nested lists are flattened
//! by the ingestion script into parallel multi-valued fields; see the bulk
//! format in [`crate::input`]):
//!
//! | Source | Field(s) |
//! | --- | --- |
//! | `idcontrato`, `idprocedimento`, `nAnuncio`, `IDAnuncio`, `idINCM`, `numAcordoQuadro` | same names, exact (ids, foreign keys) |
//! | `tipoprocedimento`, `TipoAnuncio`, `regime`, `fundamentAjusteDireto`, `tipoFimContrato`, `TipoMEC`, `estadoContrato` | same names, keyword (closed value lists, 3 to 30 values) |
//! | `tipoContrato[]` | `tipoContrato[]`, keyword |
//! | `objectoContrato`, `descContrato`, `DescrAcordoQuadro`, `Observacoes`, `CausasAlteraPrazo`, `CausasAlteraPreco` | same names, text |
//! | `fundamentacao` | name (legal article text, 83-161 values per year: searchable and facetable) |
//! | `justifNReducEscrContrato[]` | text, multi-valued |
//! | four dates | date |
//! | `precoContratual`, `precoBaseProcedimento`, `PrecoTotalEfetivo`, `precoEstimadoProc` | double (0 = absent for the last three) |
//! | `prazoExecucao` | int days (99999999 = absent) |
//! | `Ano` | int |
//! | `ProcedimentoCentralizado`, `CritMateriais`, `ContratEcologico`, `compraConjunta`, `ocorrencias` | flag |
//! | `adjudicante[]` | `adjudicanteNif[]` (exact), `adjudicanteNome[]` (name) |
//! | `adjudicatarios[]` | `adjudicatarioNif[]` (exact), `adjudicatarioNome[]` (name) |
//! | `CPV[]` | `cpv[]` (keyword, with derived division/group/class/category), `cpvDescricao[]` (text), `cpvPrincipal` (keyword: the `Principal` code) |
//! | `localExecucao[]` | `localExecucao[]` (keyword, with derived country/district/municipality) |
//! | `NUTs[]` | `nuts[]` (keyword code, with derived levels 1-3) |
//! | `concorrentes[]` (`"NIF-Name"`) | `concorrenteNif[]` (exact), `concorrenteNome[]` (text) |
//! | `entidadesConvidadas[]` | `convidadaNif[]` (exact), `convidadaNome[]` (text) |
//! | `Lotes[]` | `loteDesignacao[]`, `criterioLote[]`, `criterioAdjudicacaoFator[]` (text), `criterioAdjudicacaoTipo[]`, `criterioAdjudicacaoTipoFator[]` (keyword), `numPropostas` (int, maximum over the lots) |
//! | `linkPecasProc` | stored only |
//! | (aggregate) | `numModificacoes`: modification rows of the contract |
//!
//! **Not indexed, on purpose:**
//!
//! * party attributes other than NIF and name (`codPaisEntContrato`,
//!   `NUTsCod`/`NUTsDesc`, `tipoEntidade`, `atividadeEntidade`,
//!   `adjudicatarioPME`): they describe the entity, not the contract (#531
//!   found them to be current entity attributes), and live in
//!   [`adjudicantes`](super::adjudicantes) and
//!   [`fornecedores`](super::fornecedores), reachable by NIF;
//! * `entidadesConvidadas[].AliasPais`/`Pais`: in [`convidadas`](super::convidadas);
//! * `CPV[].cpvValor`: a per-code split that equals `precoContratual` for the
//!   98.9% single-CPV contracts; as a multi-valued number it would lose its
//!   pairing with the code. Its sums are in [`cpv`](super::cpv);
//! * `CPV[].CPVObjeto`: carried by `cpvPrincipal`;
//! * `NUTs[]` descriptions: one per code, in [`nuts`](super::nuts);
//! * `Lotes[].numLot` (not unique, #531) and
//!   `Lotes[].criterioAdjudicacaoPonderacao` (a weight meaningless apart from
//!   its criterion, which flattening separates).

use crate::entities::{
    CPV_LEVELS, FREE_TEXT_ABSENT, INCM_ABSENT, NIF_ABSENT, NUTS_LEVELS, PLACE_LEVELS, TERM_ABSENT,
    ZERO_ABSENT,
};
use crate::schema::FieldKind::{Date, Double, Exact, Flag, Int, Keyword, Name, Stored, Text};
use crate::schema::{EntitySchema, FieldSpec as F, Scheme};

/// The `contratos` index.
pub const SCHEMA: EntitySchema = EntitySchema {
    name: "contratos",
    summary: "contracts (key idcontrato)",
    fields: &[
        F::new("idcontrato", Exact),
        F::new("idprocedimento", Exact),
        F::new("tipoprocedimento", Keyword),
        F::new("tipoContrato", Keyword).multi(),
        F::new("objectoContrato", Text),
        F::new("descContrato", Text),
        F::new("dataPublicacao", Date),
        F::new("dataCelebracaoContrato", Date),
        F::new("dataDecisaoAdjudicacao", Date),
        F::new("dataFechoContrato", Date),
        F::new("precoContratual", Double),
        F::new("precoBaseProcedimento", Double).absent(ZERO_ABSENT),
        F::new("PrecoTotalEfetivo", Double).absent(ZERO_ABSENT),
        F::new("precoEstimadoProc", Double).absent(ZERO_ABSENT),
        F::new("prazoExecucao", Int).absent(TERM_ABSENT),
        F::new("Ano", Int),
        F::new("regime", Keyword),
        F::new("fundamentacao", Name),
        F::new("fundamentAjusteDireto", Keyword),
        F::new("tipoFimContrato", Keyword),
        F::new("TipoMEC", Keyword),
        F::new("estadoContrato", Keyword),
        F::new("ProcedimentoCentralizado", Flag),
        F::new("CritMateriais", Flag),
        F::new("ContratEcologico", Flag),
        F::new("compraConjunta", Flag),
        F::new("ocorrencias", Flag),
        F::new("numAcordoQuadro", Exact).absent(&["1"]),
        F::new("DescrAcordoQuadro", Text),
        F::new("nAnuncio", Exact),
        F::new("IDAnuncio", Exact),
        F::new("idINCM", Exact).absent(INCM_ABSENT),
        F::new("TipoAnuncio", Keyword),
        F::new("adjudicanteNif", Exact).multi().absent(NIF_ABSENT),
        F::new("adjudicanteNome", Name).multi(),
        F::new("adjudicatarioNif", Exact).multi().absent(NIF_ABSENT),
        F::new("adjudicatarioNome", Name).multi(),
        F::new("cpv", Keyword)
            .multi()
            .levels(Scheme::Cpv, CPV_LEVELS),
        F::derived("cpvDivisao"),
        F::derived("cpvGrupo"),
        F::derived("cpvClasse"),
        F::derived("cpvCategoria"),
        F::new("cpvDescricao", Text).multi(),
        F::new("cpvPrincipal", Keyword),
        F::new("localExecucao", Keyword)
            .multi()
            .levels(Scheme::Place, PLACE_LEVELS),
        F::derived("localNivel1"),
        F::derived("localNivel2"),
        F::derived("localNivel3"),
        F::new("nuts", Keyword)
            .multi()
            .levels(Scheme::Nuts, NUTS_LEVELS),
        F::derived("nutsNivel1"),
        F::derived("nutsNivel2"),
        F::derived("nutsNivel3"),
        F::new("justifNReducEscrContrato", Text).multi(),
        F::new("concorrenteNif", Exact).multi().absent(NIF_ABSENT),
        F::new("concorrenteNome", Text).multi(),
        F::new("convidadaNif", Exact).multi().absent(NIF_ABSENT),
        F::new("convidadaNome", Text).multi(),
        F::new("loteDesignacao", Text).multi(),
        F::new("criterioLote", Text).multi(),
        F::new("criterioAdjudicacaoTipo", Keyword).multi(),
        F::new("criterioAdjudicacaoTipoFator", Keyword).multi(),
        F::new("criterioAdjudicacaoFator", Text).multi(),
        F::new("numPropostas", Int),
        F::new("linkPecasProc", Stored),
        F::new("Observacoes", Text).absent(FREE_TEXT_ABSENT),
        F::new("CausasAlteraPrazo", Text).absent(FREE_TEXT_ABSENT),
        F::new("CausasAlteraPreco", Text).absent(FREE_TEXT_ABSENT),
        F::new("numModificacoes", Int),
    ],
    key: &["idcontrato"],
    text_fields: &[
        "objectoContrato",
        "descContrato",
        "adjudicanteNome",
        "adjudicatarioNome",
        "cpvDescricao",
    ],
    output_fields: &[
        "idcontrato",
        "dataPublicacao",
        "precoContratual",
        "adjudicanteNome",
        "objectoContrato",
    ],
};
