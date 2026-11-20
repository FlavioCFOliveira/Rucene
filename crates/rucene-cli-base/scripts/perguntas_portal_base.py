#!/usr/bin/env python3
"""Plans of 100 multi-index questions over the Portal BASE indices of the
``rucene-base`` CLI, and their end-to-end validation.

Usage (from the repository root)::

    python3 -I crates/rucene-cli-base/scripts/perguntas_portal_base.py \\
        [--binary target/release/rucene-base] \\
        [--root crates/rucene-cli-base/data/base/index] \\
        [--only h01,b03,...] [--workers N] [--write-md]

Each question needs data from at least two of the ten indices. Its plan is a
list of CLI requests (``search``, ``count`` or ``get``), later requests filled
from the results of earlier ones, plus a small client-side combination
(difference, ratio, comparison, intersection of short lists). Complexity is
**R**, the number of requests, and **I**, the number of distinct indices; the
question set is published ordered by R, then I, descending. The questions
document is ``crates/rucene-cli-base/perguntas-portal-base.md``.

The script runs every plan (or the ``--only`` ones) against the full indices,
at most ``--workers`` (1 to 4, default 4) processes at once, and prints one
line per question with its answer. Only the read-only commands ``search``,
``count``, ``get`` and ``stats`` can be issued: any other command is refused
before it runs. A question passes when every request exits 0 and the
combination gives a non-empty answer. With ``--write-md`` (full runs only) the
questions document is regenerated from the run: plans, per-step exit codes
and hit counts, answers and the coverage matrix.

Relative ``--binary`` and ``--root`` paths are resolved against the repository
root. The field kinds used by the document come from the entity schemas in
``crates/rucene-cli-base/src/entities/``, so no binary is needed to inspect
the plans (``scripts/tests/test_perguntas_portal_base.py``).

Exit status: 0 when every selected question passes, 1 when any fails, 2 on a
usage error. Only the Python standard library is used. Answers never print
the names of suppliers or invited entities, and NIFs of natural persons
(first digit 1-4, 7 or 8) are masked.
"""

import argparse
import concurrent.futures
import datetime
import json
import re
import shlex
import statistics
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
CRATE = REPO / 'crates' / 'rucene-cli-base'
DEFAULT_BINARY = 'target/release/rucene-base'
DEFAULT_ROOT = 'crates/rucene-cli-base/data/base/index'
DOCUMENT = CRATE / 'perguntas-portal-base.md'
SCRIPT = 'crates/rucene-cli-base/scripts/perguntas_portal_base.py'
ENTITIES = ['contratos', 'procedimentos', 'anuncios', 'fornecedores', 'modificacoes',
            'convidadas', 'adjudicantes', 'cpv', 'locais', 'nuts']
READ_ONLY = frozenset({'search', 'count', 'get', 'stats'})
WRITE_FLAGS = frozenset({'--set', '--file'})
SEP = ' | '

# --------------------------------------------------------------------------
# Privacy helpers: natural-person NIFs (first digit 1-4, 7, 8) are masked in
# every recorded string; supplier/invitee names are never requested.
# --------------------------------------------------------------------------

PERSONAL_NIF = re.compile(r'(?<!\d)(?<!Incm=)(?<!INCM=)[123478]\d{8}(?!\d)')
URL = re.compile(r'https?://\S+')


def mask(text):
    """Masks natural-person NIFs; URLs and INCM codes (also 9 digits) are kept."""
    out, last = [], 0
    for m in URL.finditer(text):
        out.append(PERSONAL_NIF.sub('<NIF singular omitido>', text[last:m.start()]))
        out.append(m.group(0))
        last = m.end()
    out.append(PERSONAL_NIF.sub('<NIF singular omitido>', text[last:]))
    return ''.join(out)


def nif(value):
    value = str(value)
    return value if value[:1] in '569' else '<NIF singular omitido>'


def collective(value):
    return str(value)[:1] in '569'


def eur(value):
    value = float(value)
    whole, frac = f'{value:,.2f}'.split('.')
    return whole.replace(',', ' ') + ',' + frac + ' €'


def pct(part, total):
    return '—' if not total else f'{100.0 * part / total:.1f} %'.replace('.', ',')


def days(d1, d2):
    a = datetime.date.fromisoformat(d1)
    b = datetime.date.fromisoformat(d2)
    return (b - a).days


# --------------------------------------------------------------------------
# Steps, fills and results
# --------------------------------------------------------------------------


class Fill:
    """A value computed from earlier steps: `label` is what the deliverable
    shows, `func(ctx)` the value (a string, or a list of argv tokens for a
    `{*name}` argument)."""

    def __init__(self, label, func):
        self.label = label
        self.func = func


class S:
    """One CLI request: `rucene-base <entity> <sub> <root> <args...>`."""

    def __init__(self, entity, sub, *args, **fills):
        if entity not in ENTITIES:
            raise ValueError(f'unknown index {entity!r}')
        if sub not in READ_ONLY:
            raise ValueError(f'{sub!r} is not a read-only command')
        self.entity, self.sub, self.args, self.fills = entity, sub, list(args), fills

    def display(self):
        out = ['rucene-base', self.entity, self.sub, '$R']
        for arg in self.args:
            m = re.fullmatch(r'\{\*(\w+)\}', arg)
            if m:
                out.append(self.fills[m.group(1)].label)
                continue
            text = arg
            for name, fill in self.fills.items():
                text = text.replace('{' + name + '}', fill.label)
            out.append(text if re.fullmatch(r'[\w.,:=/@%+-]+', text) else shlex.quote(text))
        return ' '.join(out)

    def argv(self, ctx, binary='rucene-base', root='$R'):
        """The full argv: `binary entity sub root args...`, with fills from `ctx`."""
        out = [str(binary), self.entity, self.sub, str(root)]
        for arg in self.args:
            m = re.fullmatch(r'\{\*(\w+)\}', arg)
            if m:
                tokens = self.fills[m.group(1)].func(ctx)
                if not tokens:
                    raise PlanError(f'sem valores para {self.fills[m.group(1)].label}')
                out.extend(tokens)
                continue
            text = arg
            for name, fill in self.fills.items():
                if '{' + name + '}' in text:
                    value = fill.func(ctx)
                    if value in (None, ''):
                        raise PlanError(f'sem valor para {fill.label}')
                    text = text.replace('{' + name + '}', str(value))
            out.append(text)
        return out


class PlanError(Exception):
    pass


class Res:
    def __init__(self, step, argv, proc):
        self.entity, self.sub = step.entity, step.sub
        self.argv = argv
        self.exit = proc.returncode
        self.stderr = proc.stderr.strip()
        self.rows, self.docs, self.count, self.total, self.exact = [], [], None, None, None
        out = proc.stdout
        if self.exit != 0:
            return
        if self.sub == 'search':
            lines = out.rstrip('\n').split('\n')
            m = re.match(r'# total hits: (\d+)( \(lower bound\))?', lines[0])
            self.total, self.exact = int(m.group(1)), m.group(2) is None
            header = lines[1].split('\t')
            self.rows = [dict(zip(header, line.split('\t'))) for line in lines[2:] if line]
        elif self.sub == 'count':
            self.count = int(out.strip())
            self.total = self.count
        elif self.sub == 'get':
            self.docs = [json.loads(line) for line in out.splitlines() if line.strip()]
            self.total = len(self.docs)

    def hits(self):
        return self.total


def _split(value):
    if isinstance(value, list):
        return [str(v) for v in value]
    if value is None or value == '':
        return []
    return [v for v in str(value).split(SEP) if v != '']


def vals(ctx, i, field, rows=None):
    """Unique values (order kept) of `field` in the rows or documents of
    step `i` (1-based)."""
    res = ctx[i - 1]
    seen, out = set(), []
    for rec in (rows if rows is not None else (res.rows or res.docs)):
        for v in _split(rec.get(field)):
            if v not in seen:
                seen.add(v)
                out.append(v)
    return out


def rows(ctx, i):
    r = ctx[i - 1]
    return r.rows or r.docs


def cnt(ctx, i):
    return ctx[i - 1].count


def tot(ctx, i):
    return ctx[i - 1].total


def first(ctx, i, field, k=0):
    rs = rows(ctx, i)
    if len(rs) <= k:
        return None
    v = _split(rs[k].get(field))
    return v[0] if v else None


def byfield(ctx, i, field):
    return {str(r.get(field)): r for r in rows(ctx, i)}


# Fill constructors -------------------------------------------------------


def J(i, field, n=None, label=None, flt=None):
    """Comma-joined unique values of `field` from step i (for --terms)."""
    def f(ctx):
        v = vals(ctx, i, field)
        if flt:
            v = [x for x in v if flt(x)]
        return ','.join(v[:n] if n else v)
    return Fill(label or f'<{field} do passo {i}>', f)


def ONE(i, field, k=0, label=None):
    """The value of `field` in the k-th row of step i."""
    return Fill(label or f'<{field} do passo {i}>', lambda ctx: first(ctx, i, field, k))


def KEYS(i, field, n=None, label=None, flt=None):
    """`--key V` for each unique value of `field` from step i (for get)."""
    def f(ctx):
        v = vals(ctx, i, field)
        if flt:
            v = [x for x in v if flt(x)]
        out = []
        for x in (v[:n] if n else v):
            out += ['--key', x]
        return out
    return Fill(label or f'--key <{field} do passo {i}> …', f)


def FN(label, func):
    return Fill(label, func)


def plus(f, g):
    """Answer `f` followed by the extra part `g` (both functions of ctx)."""
    return lambda c: f(c) + g(c)


# --------------------------------------------------------------------------
# Questions
# --------------------------------------------------------------------------

QUESTIONS = []


def Q(qid, r, i, cat, pergunta, inst, steps, why, comb, ans, zero_ok=None, notes=None):
    """Declares one question: id, declared R and I, category, text, instantiation,
    plan, why R is minimal, client-side combination and answer function."""
    QUESTIONS.append(dict(id=qid, r=r, i=i, cat=cat, q=pergunta, inst=inst, steps=steps,
                          why=why, comb=comb, ans=ans, zero_ok=zero_ok, notes=notes or []))


def plan_r(q):
    """R computed from the plan: its number of requests."""
    return len(q['steps'])


def plan_i(q):
    """I computed from the plan: its number of distinct indices."""
    return len({s.entity for s in q['steps']})


C, P, A, F, M, V, AD, K, L, N = ('contratos', 'procedimentos', 'anuncios', 'fornecedores',
                                 'modificacoes', 'convidadas', 'adjudicantes', 'cpv',
                                 'locais', 'nuts')
EMP = 'tipoContrato=Empreitadas de obras públicas'
ADRG = 'tipoprocedimento=Ajuste Direto Regime Geral'
CPUB = 'tipoprocedimento=Concurso público'

# ---- Batch 1: chains through many indices ----------------------------------


def a_h01(c):
    k = rows(c, 2)[0]
    an = rows(c, 3)
    p = rows(c, 4)[0]
    f = rows(c, 5)
    m = rows(c, 6)
    lo = rows(c, 7)[0]
    cp = rows(c, 8)[0]
    ent = byfield(c, 1, 'nif').get(k['adjudicanteNif'], {}).get('nome', k['adjudicanteNif'])
    base = float(k['precoBaseProcedimento']) if k.get('precoBaseProcedimento') else None
    sav = f', {pct(base - float(k["precoContratual"]), base)} abaixo do preço base' if base else ''
    a = an[0] if an else None
    atxt = (f'anúncio {a["IDAnuncio"]} ({a["modeloAnuncio"]}, {a["dataPublicacao"]}, prazo de propostas '
            f'{a["PrazoPropostas"]} dias, preço base anunciado {eur(a["PrecoBase"]) if a["PrecoBase"] else "não indicado"}, '
            f'critério ambiental {a["CriterAmbient"]})') if a else 'anúncio não indexado'
    sup = '; '.join(f'{nif(d["nif"])} (PME: {d.get("pme", "sem indicação")}, {d["numContratos"]} contratos)' for d in f)
    return (f'contrato {k["idcontrato"]} de {ent}: {eur(k["precoContratual"])}{sav}; {atxt}; procedimento '
            f'{p["idprocedimento"]} ({p["tipoprocedimento"]}, {p["numContratos"]} contrato(s), {eur(p["valorTotal"])}); '
            f'adjudicatário {sup}; {len(m)} modificação(ões); local {lo["local"]} ({lo["numContratos"]} contratos no total); '
            f'CPV {cp["cpv"]} «{cp["descricao"]}»')


Q('h01', 8, 8, 'Quem compra',
  'Qual a empreitada de maior valor celebrada num ano por uma entidade de âmbito municipal sediada numa NUTS III, '
  'e qual o seu percurso completo: anúncio, procedimento, adjudicatário, modificações, local de execução e CPV?',
  '[NUTS III PT11A – Área Metropolitana do Porto; nome com «município»; empreitadas de obras públicas; ano 2024]',
  [S(AD, 'search', '--term', 'nutsCod=PT11A', '--text-in', 'nome=município', '--limit', '100', '--fields', 'nif,nome'),
   S(C, 'search', '--terms', 'adjudicanteNif={n}', '--term', EMP, '--term', 'Ano=2024',
     '--sort', 'precoContratual:desc', '--limit', '1', '--fields',
     'idcontrato,idprocedimento,IDAnuncio,adjudicanteNif,adjudicatarioNif,precoContratual,precoBaseProcedimento,cpvPrincipal,localExecucao',
     n=J(1, 'nif')),
   S(A, 'search', '--term', 'IDAnuncio={a}', '--fields', 'IDAnuncio,dataPublicacao,modeloAnuncio,PrazoPropostas,PrecoBase,CriterAmbient', a=ONE(2, 'IDAnuncio')),
   S(P, 'get', '{*p}', p=KEYS(2, 'idprocedimento')),
   S(F, 'get', '{*f}', f=KEYS(2, 'adjudicatarioNif')),
   S(M, 'search', '--term', 'idcontrato={i}', '--fields', 'idcontrato,sequencia,modifContratoFundamento,modifContratoPrecoAlterado', i=ONE(2, 'idcontrato')),
   S(L, 'get', '{*l}', l=KEYS(2, 'localExecucao')),
   S(K, 'get', '{*k}', k=KEYS(2, 'cpvPrincipal'))],
  'O passo 2 depende dos NIF do passo 1 (a sede da entidade só existe em `adjudicantes`); os passos 3 a 8 dependem '
  'do contrato do passo 2 e cada um consulta um índice diferente, pelo que não se fundem.',
  'Escolhe-se a linha única do passo 2; poupança face ao preço base = (base − contratual)/base; '
  'o nome da entidade vem do passo 1.',
  a_h01)


def a_h02(c):
    out = []
    for k, r in enumerate(rows(c, 1)):
        eco, total = cnt(c, 2 + 2 * k), cnt(c, 3 + 2 * k)
        out.append(f'{r["nuts"]} {r["descricao"]}: {eco}/{total} = {pct(eco, total)}')
    return 'contratos ecológicos em 2025 — ' + '; '.join(out)


def _nuts_k(k):
    return ONE(1, 'nuts', k, label=f'<{k + 1}.ª NUTS do passo 1>')


Q('h02', 7, 2, 'Sustentabilidade',
  'Que percentagem dos contratos de um ano foi marcada como contrato ecológico em cada uma das três NUTS III '
  'com mais contratos registados?',
  '[ano 2025; as três NUTS com maior `nuts.numContratos`]',
  [S(N, 'search', '--sort', 'numContratos:desc', '--limit', '3', '--fields', 'nuts,descricao,numContratos')] +
  [s for k in range(3) for s in (
      S(C, 'count', '--term', 'nuts={x}', '--term', 'Ano=2025', '--term', 'ContratEcologico=sim', x=_nuts_k(k)),
      S(C, 'count', '--term', 'nuts={x}', '--term', 'Ano=2025', x=_nuts_k(k)))],
  'Sem agregação no índice, cada contagem (ecológicos e total por região) é um `count` próprio: 3 regiões × 2 '
  'contagens + 1 pedido para escolher as regiões. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Rácio ecológicos/total por região.',
  a_h02)


def a_h03(c):
    out = []
    for k, r in enumerate(rows(c, 1)):
        ad, cp = cnt(c, 2 + 2 * k), cnt(c, 3 + 2 * k)
        out.append(f'{r["concelho"]}: ajuste direto {ad}, concurso público {cp} (ajuste direto = {pct(ad, ad + cp)})')
    return 'contratos executados em 2025 — ' + '; '.join(out)


def _loc_k(k):
    return ONE(1, 'local', k, label=f'<{k + 1}.º local do passo 1>')


Q('h03', 7, 2, 'Procedimento e fundamento',
  'Nos três concelhos de um distrito com mais contratos executados, quantos contratos de um ano foram feitos por '
  'ajuste direto e quantos por concurso público?',
  '[distrito «Portugal, Évora» (excluindo o local só de distrito); ano 2025]',
  [S(L, 'search', '--term', 'localNivel2=Portugal, Évora', '--not', '--key', 'Portugal, Évora',
     '--sort', 'numContratos:desc', '--limit', '3', '--fields', 'local,concelho,numContratos')] +
  [s for k in range(3) for s in (
      S(C, 'count', '--term', 'localNivel3={x}', '--term', 'Ano=2025', '--term', ADRG, x=_loc_k(k)),
      S(C, 'count', '--term', 'localNivel3={x}', '--term', 'Ano=2025', '--term', CPUB, x=_loc_k(k)))],
  'Uma contagem por concelho e por tipo de procedimento (o CLI não agrega por valor) e um pedido para escolher os '
  'concelhos; os locais têm vírgulas, por isso nem `--terms` os aceitaria. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Percentagem de ajuste direto no total dos dois tipos.',
  a_h03)


def a_h04(c):
    k = rows(c, 1)[0]
    conc = vals(c, 1, 'concorrenteNif')
    fz = byfield(c, 2, 'nif')
    vz = byfield(c, 3, 'nif')
    winner = k['adjudicatarioNif']
    won = sum(1 for x in conc if x in fz)
    pme = sum(1 for x in conc if fz.get(x, {}).get('pme') == 'sim')
    inv = sum(1 for x in conc if x in vz)
    return (f'contrato {k["idcontrato"]} ({k["numPropostas"]} propostas, {eur(k["precoContratual"])}); '
            f'{len(conc)} concorrentes: {won} já foram adjudicatários (PME: {pme}), {inv} constam de `convidadas`; '
            f'vencedor {nif(winner)}: concorreu em {cnt(c, 4)} contratos no total e ganhou {cnt(c, 5)} '
            f'contratos ao Município de Lisboa')


Q('h04', 5, 3, 'Concorrência',
  'No concurso público com mais propostas de uma entidade num ano, quem foram os concorrentes, quantos já '
  'ganharam contratos ou foram convidados, e qual o historial do vencedor junto dessa entidade?',
  '[Município de Lisboa, NIF 500051070; concurso público; ano 2024]',
  [S(C, 'search', '--term', 'adjudicanteNif=500051070', '--term', CPUB, '--term', 'Ano=2024',
     '--sort', 'numPropostas:desc', '--limit', '1', '--fields', 'idcontrato,numPropostas,precoContratual,concorrenteNif,adjudicatarioNif'),
   S(F, 'search', '--terms', 'nif={x}', '--limit', '100', '--fields', 'nif,pme,numContratos', x=J(1, 'concorrenteNif')),
   S(V, 'search', '--terms', 'nif={x}', '--limit', '100', '--fields', 'nif,numConvites', x=J(1, 'concorrenteNif')),
   S(C, 'count', '--term', 'concorrenteNif={w}', w=ONE(1, 'adjudicatarioNif')),
   S(C, 'count', '--term', 'adjudicatarioNif={w}', '--term', 'adjudicanteNif=500051070', w=ONE(1, 'adjudicatarioNif'))],
  'Os passos 2 e 3 são índices diferentes; os passos 4 e 5 são contagens com critérios diferentes sobre o NIF '
  'vencedor do passo 1 (um `count` devolve um só número).',
  'Interseção da lista de concorrentes com as linhas dos passos 2 e 3; contagem de PME.',
  a_h04,
  notes=['NIF de concorrentes e vencedor só são mostrados quando são de pessoa coletiva.'])


def a_h05(c):
    alt = rows(c, 1)
    orig = byfield(c, 2, 'IDAnuncio')
    changes = []
    for a in alt:
        for prev in _split(a.get('anteriorIDAnuncio')):
            o = orig.get(prev)
            if o:
                changes.append(f'{a["IDAnuncio"]}←{prev} (prazo-limite {o["DataLimitePropostas"]}→{a["DataLimitePropostas"]})')
    ks = rows(c, 3)
    procs = rows(c, 4)
    mods = rows(c, 5)
    sup = rows(c, 6)
    cpvs = rows(c, 7)
    return (f'{len(alt)} anúncios de alteração; {len(changes)} ligados ao anterior, p.ex. {"; ".join(changes[:3])}; '
            f'{len(ks)} contrato(s) resultante(s) somando {eur(sum(float(k["precoContratual"]) for k in ks))}; '
            f'procedimentos: {", ".join(p["idprocedimento"] + " (" + p["tipoprocedimento"] + ")" for p in procs)}; '
            f'{len(mods)} modificação(ões); adjudicatários {", ".join(nif(d["nif"]) + " PME " + d.get("pme", "s/i") for d in sup)}; '
            f'CPV {", ".join(d["cpv"] + " «" + d["descricao"] + "»" for d in cpvs)}')


Q('h05', 7, 6, 'Anúncios',
  'Que anúncios de alteração publicou uma entidade num ano, que anúncio anterior alteraram e com que prazo-limite, '
  'que contratos resultaram, por que procedimentos, com que modificações, adjudicatários e CPV?',
  '[Infraestruturas de Portugal, NIF 503933813; tipo de ato «Anúncio de Alteração»; ano 2025; os 15 primeiros por data]',
  [S(A, 'search', '--term', 'tipoActo=Anúncio de Alteração', '--term', 'nifEntidade=503933813', '--term', 'Ano=2025',
     '--sort', 'dataPublicacao:asc', '--limit', '15', '--fields', 'IDAnuncio,anteriorIDAnuncio,dataPublicacao,DataLimitePropostas,PrecoBase'),
   S(A, 'search', '--terms', 'IDAnuncio={x}', '--limit', '50', '--fields', 'IDAnuncio,DataLimitePropostas,PrazoPropostas,PrecoBase', x=J(1, 'anteriorIDAnuncio')),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '50', '--fields', 'idcontrato,IDAnuncio,idprocedimento,adjudicatarioNif,precoContratual,cpvPrincipal',
     x=FN('<IDAnuncio e anteriorIDAnuncio do passo 1>', lambda c: ','.join(vals(c, 1, 'IDAnuncio') + vals(c, 1, 'anteriorIDAnuncio')))),
   S(P, 'get', '{*p}', p=KEYS(3, 'idprocedimento')),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '100', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(3, 'idcontrato')),
   S(F, 'get', '{*f}', f=KEYS(3, 'adjudicatarioNif')),
   S(K, 'get', '{*k}', k=KEYS(3, 'cpvPrincipal'))],
  'O passo 2 lê os anúncios anteriores cujo id só se conhece no passo 1; o passo 3 precisa dos ids de ambos; os '
  'passos 4 a 7 dependem dos contratos do passo 3 e são índices diferentes.',
  'Emparelha cada alteração com o anúncio anterior (prazo-limite antes→depois); soma do preço contratual dos contratos.',
  a_h05)


def a_h06(c):
    an = rows(c, 1)
    ks = rows(c, 2)
    procs = byfield(c, 3, 'idprocedimento')
    sup = rows(c, 4)
    mods = rows(c, 5)
    cpvs = rows(c, 6)
    pme = sum(1 for d in sup if d.get('pme') == 'sim')
    eco = sum(1 for k in ks if k['ContratEcologico'] == 'sim')
    return (f'{len(an)} anúncios; {len(ks)} contratos resultantes ({eco} marcados ecológicos), total '
            f'{eur(sum(float(k["precoContratual"]) for k in ks))}; {len(procs)} procedimentos; adjudicatários PME {pme}/{len(sup)}; '
            f'{len(mods)} modificações; CPV: {"; ".join(d["cpv"] + " «" + d["descricao"] + "»" for d in cpvs[:4])}')


Q('h06', 6, 6, 'Sustentabilidade',
  'Dos anúncios com critério ambiental de um ano para obras, de maior preço base, que contratos resultaram, quantos '
  'ficaram marcados como ecológicos, por que procedimentos, com que adjudicatários (PME?), modificações e CPV?',
  '[critério ambiental = sim; divisão CPV 45; ano 2023; os 20 anúncios de maior preço base]',
  [S(A, 'search', '--term', 'CriterAmbient=sim', '--term', 'cpvDivisao=45', '--term', 'Ano=2023',
     '--sort', 'PrecoBase:desc', '--limit', '20', '--fields', 'IDAnuncio,PrecoBase,designacaoEntidade'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '100', '--fields',
     'idcontrato,IDAnuncio,idprocedimento,adjudicatarioNif,precoContratual,ContratEcologico,cpv', x=J(1, 'IDAnuncio')),
   S(P, 'get', '{*p}', p=KEYS(2, 'idprocedimento')),
   S(F, 'get', '{*f}', f=KEYS(2, 'adjudicatarioNif')),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '200', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(2, 'idcontrato')),
   S(K, 'get', '{*k}', k=KEYS(2, 'cpv'))],
  'Cada passo a partir do 2 depende de ids do passo anterior ou do passo 2, e cada um consulta um índice diferente.',
  'Contagem de contratos com `ContratEcologico=sim` e de adjudicatários PME nas linhas devolvidas; soma dos preços.',
  a_h06)


def a_h07(c):
    reg = rows(c, 1)
    top = reg[0]
    ents = rows(c, 2)
    ks = rows(c, 3)
    sup = rows(c, 4)
    locs = rows(c, 5)
    mods = rows(c, 6)
    return (f'NUTS III de maior valor no Centro: {top["nuts"]} {top["descricao"]} ({eur(top["valorTotal"])}); '
            f'entidades de maior valor aí: {", ".join(e["nome"].split(SEP)[0] for e in ents)}; '
            f'5 maiores contratos: {", ".join(k["idcontrato"] + " " + eur(k["precoContratual"]) for k in ks)}; '
            f'adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}; '
            f'locais: {", ".join(d["local"] for d in locs)}; {len(mods)} modificações')


Q('h07', 6, 6, 'Localização',
  'Numa NUTS II, qual a NUTS III com maior valor contratado, que entidades aí sediadas mais contrataram, quais os '
  'seus maiores contratos, com que adjudicatários (PME?), onde foram executados e com quantas modificações?',
  '[NUTS II PT16 – Centro; 3 entidades; 5 contratos]',
  [S(N, 'search', '--term', 'nutsNivel2=PT16', '--sort', 'valorTotal:desc', '--limit', '5', '--fields', 'nuts,descricao,numContratos,valorTotal'),
   S(AD, 'search', '--term', 'nutsCod={x}', '--sort', 'valorTotal:desc', '--limit', '3', '--fields', 'nif,nome,valorTotal', x=ONE(1, 'nuts', label='<NUTS III de maior valor do passo 1>')),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--sort', 'precoContratual:desc', '--limit', '5', '--fields',
     'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual,localExecucao', x=J(2, 'nif')),
   S(F, 'get', '{*f}', f=KEYS(3, 'adjudicatarioNif')),
   S(L, 'get', '{*l}', l=KEYS(3, 'localExecucao')),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '200', '--fields', 'idcontrato,sequencia', x=J(3, 'idcontrato'))],
  'Cadeia estrita: região → entidades (sede só em `adjudicantes`) → contratos → três enriquecimentos em índices '
  'diferentes.',
  'Escolhe a primeira NUTS do passo 1; conta PME nas linhas do passo 4.',
  a_h07)


def a_h08(c):
    ks = rows(c, 1)
    mods = rows(c, 2)
    fund = {}
    for m in mods:
        fund[m['modifContratoFundamento']] = fund.get(m['modifContratoFundamento'], 0) + 1
    topf = sorted(fund.items(), key=lambda x: -x[1])[:2]
    procs = rows(c, 3)
    ans_ = rows(c, 4)
    sup = rows(c, 5)
    cpvs = rows(c, 6)
    return (f'contratos {", ".join(k["idcontrato"] + " (" + k["numModificacoes"] + " modif.)" for k in ks)}; '
            f'fundamentos mais frequentes: {"; ".join(f"{a} ({b})" for a, b in topf)}; '
            f'procedimentos: {", ".join(p["tipoprocedimento"] for p in procs)}; '
            f'{len(ans_)} anúncio(s) de origem (prazos: {", ".join(a["PrazoPropostas"] + " dias" for a in ans_)}); '
            f'adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}; '
            f'CPV: {"; ".join(d["descricao"] for d in cpvs)}')


Q('h08', 6, 6, 'Modificações',
  'Quais os contratos de uma entidade com mais modificações, com que fundamentos foram modificados, de que '
  'procedimento e anúncio vieram, quem os executa (PME?) e qual o seu CPV?',
  '[Infraestruturas de Portugal, NIF 503933813; os 5 contratos com mais modificações]',
  [S(C, 'search', '--term', 'adjudicanteNif=503933813', '--range', 'numModificacoes=1..', '--sort', 'numModificacoes:desc',
     '--limit', '5', '--fields', 'idcontrato,idprocedimento,IDAnuncio,adjudicatarioNif,precoContratual,cpvPrincipal,numModificacoes'),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '500', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(1, 'idcontrato')),
   S(P, 'get', '{*p}', p=KEYS(1, 'idprocedimento')),
   S(A, 'search', '--terms', 'IDAnuncio={x}', '--fields', 'IDAnuncio,PrazoPropostas,PrecoBase', x=J(1, 'IDAnuncio')),
   S(F, 'get', '{*f}', f=KEYS(1, 'adjudicatarioNif')),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpvPrincipal'))],
  'Os passos 2 a 6 dependem dos ids do passo 1 e cada um consulta um índice diferente.',
  'Contagem dos fundamentos nas linhas de modificação devolvidas (lista limitada aos 5 contratos).',
  a_h08)

# ---- Batch 2 -----------------------------------------------------------------


def pme_of(c, i):
    sup = rows(c, i)
    return f'{sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}'


def fmt_k(k, *fields):
    return ' '.join(str(k.get(f, '')) for f in fields)


def a_b01(c):
    ks = rows(c, 2)
    return (f'{tot(c, 2)} contratos de autoridades locais na divisão 90 em 2025; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks) +
            f'; adjudicatários PME {pme_of(c, 3)}')


Q('b01', 3, 3, 'Quem compra',
  'Que entidades classificadas com um dado tipo de entidade mais gastaram num ano em contratos de uma divisão CPV, '
  'quantos contratos foram ao todo, e os adjudicatários dos maiores são PME?',
  '[tipo de entidade «Autoridade local»; divisão CPV 90 (esgotos, resíduos, limpeza e ambiente); ano 2025; 5 maiores]',
  [S(AD, 'search', '--term', 'tipoEntidade=Autoridade local', '--limit', '400', '--fields', 'nif'),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--term', 'cpvDivisao=90', '--term', 'Ano=2025', '--sort', 'precoContratual:desc',
     '--limit', '5', '--exact-total', '--fields', 'idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual', x=J(1, 'nif')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'O tipo de entidade só existe em `adjudicantes` e o atributo PME em `fornecedores`; o `search --exact-total` dá total e '
  'maiores contratos num só pedido.',
  'Contagem de PME nos documentos do passo 3.',
  a_b01)


def a_b02(c):
    ks = rows(c, 2)
    return (f'{len(rows(c, 1))} entidades de saúde; {tot(c, 2)} contratos de produtos farmacêuticos por ajuste direto; '
            'mais recentes: ' + '; '.join(f'{k["adjudicanteNome"]} {k["dataPublicacao"]} {eur(k["precoContratual"])}' for k in ks))


Q('b02', 2, 2, 'Procedimento e fundamento',
  'Que entidades com uma dada área de atividade compraram por ajuste direto produtos de uma classe CPV, e quais os '
  'contratos mais recentes?',
  '[atividade «Saúde»; classe CPV 3360 (produtos farmacêuticos); Ajuste Direto Regime Geral]',
  [S(AD, 'search', '--term', 'atividadeEntidade=Saúde', '--limit', '100', '--fields', 'nif,nome'),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--term', 'cpvClasse=3360', '--term', ADRG, '--sort', 'dataPublicacao:desc',
     '--limit', '5', '--exact-total', '--fields', 'idcontrato,adjudicanteNome,dataPublicacao,precoContratual', x=J(1, 'nif'))],
  'A atividade da entidade só existe em `adjudicantes`; total e lista saem do mesmo `search --exact-total`.',
  'Nenhuma além da leitura das linhas.',
  a_b02)


def a_b03(c):
    ents = byfield(c, 1, 'nif')
    ks = rows(c, 2)
    nuts = {d['nuts']: d['descricao'] for d in rows(c, 3)}
    out = []
    for k in ks[:5]:
        e = ents[k['adjudicanteNif']]
        out.append(f'{e["nome"].split(SEP)[0]} ({nuts.get(e["nutsCod"].split(SEP)[0], "?")}) {eur(k["precoContratual"])}')
    return f'{tot(c, 2)} contratos ≥ 20 000 € por ajuste direto em 2025; maiores: ' + '; '.join(out)


Q('b03', 4, 4, 'Procedimento e fundamento',
  'Quais os maiores contratos por ajuste direto acima de um valor, celebrados num ano pelos municípios de uma NUTS II, '
  'a que NUTS III pertence cada município, e os adjudicatários são PME?',
  '[NUTS II PT18 – Alentejo; nome com «município»; Ajuste Direto Regime Geral; ≥ 20 000 €; ano 2025]',
  [S(AD, 'search', '--term', 'nutsNivel2=PT18', '--text-in', 'nome=município', '--limit', '100', '--fields', 'nif,nome,nutsCod'),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--term', ADRG, '--range', 'precoContratual=20000..', '--term', 'Ano=2025',
     '--sort', 'precoContratual:desc', '--limit', '10', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual', x=J(1, 'nif')),
   S(N, 'get', '{*n}', n=FN('--key <nutsCod das entidades do passo 1 presentes no passo 2> …',
                            lambda c: [t for x in sorted({byfield(c, 1, 'nif')[k['adjudicanteNif']]['nutsCod'].split(SEP)[0] for k in rows(c, 2)}) for t in ('--key', x)])),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Sede (`adjudicantes`), contratos, descrição NUTS e atributo PME estão em quatro índices; os passos 3 e 4 só conhecem '
  'as chaves depois do passo 2.',
  'Emparelha cada contrato com o `nutsCod` da sua entidade (passo 1) e com a descrição (passo 3); contagem de PME.',
  plus(a_b03, lambda c: f'; adjudicatários PME {pme_of(c, 4)}'))


def a_b04(c):
    ents = byfield(c, 2, 'nif')
    out = []
    for k in rows(c, 1)[:5]:
        e = ents.get(k['adjudicanteNif'], {})
        out.append(f'{e.get("nome", ["?"])[0]} [{(e.get("nutsDesc") or ["sem NUTS"])[0]}] {eur(k["precoContratual"])}')
    return (f'{tot(c, 1)} contratos ≥ 1 M€ por urgência imperiosa em 2020; maiores: ' + '; '.join(out) +
            f'; adjudicatários dos 10 maiores PME {pme_of(c, 3)}')


Q('b04', 3, 3, 'Procedimento e fundamento',
  'Que entidades celebraram, num ano, contratos acima de um valor com um dado fundamento legal, onde estão sediadas e '
  'os adjudicatários são PME?',
  '[fundamento «Artigo 24.º, n.º 1, alínea c) do Código dos Contratos Públicos» (urgência imperiosa); ≥ 1 000 000 €; ano 2020; 10 maiores]',
  [S(C, 'search', '--term', 'fundamentacao=Artigo 24.º, n.º 1, alínea c) do Código dos Contratos Públicos', '--term', 'Ano=2020',
     '--range', 'precoContratual=1000000..', '--sort', 'precoContratual:desc', '--limit', '10', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'A sede da entidade só está em `adjudicantes` e o atributo PME em `fornecedores`; ambos dependem do passo 1.',
  'Emparelha contrato e entidade por NIF; contagem de PME.',
  a_b04)


def a_b05(c):
    ks = rows(c, 2)
    return (f'{tot(c, 2)} contratos ao abrigo do DLR 27/2015/A em 2024 por entidades sediadas nos Açores; primeiros por nome: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks))


Q('b05', 2, 2, 'Procedimento e fundamento',
  'Que entidades sediadas numa região celebraram contratos ao abrigo de um regime jurídico regional num ano '
  '(listadas por ordem alfabética)?',
  '[NUTS PT200 – Região Autónoma dos Açores; regime «DLR n.º 27/2015/A, de 29.12»; ano 2024]',
  [S(AD, 'search', '--term', 'nutsCod=PT200', '--limit', '500', '--fields', 'nif'),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--term', 'regime=DLR n.º 27/2015/A, de 29.12', '--term', 'Ano=2024',
     '--sort', 'adjudicanteNome:asc,precoContratual:desc', '--limit', '5', '--exact-total', '--fields', 'adjudicanteNome,idcontrato,precoContratual', x=J(1, 'nif'))],
  'A sede só existe em `adjudicantes`; total e lista ordenada num só `search --exact-total`.',
  'Nenhuma além da leitura das linhas.',
  a_b05)


def a_b06(c):
    buyers = byfield(c, 1, 'nif')
    sup = rows(c, 2)
    return (f'{tot(c, 2)} dos 100 maiores compradores também são adjudicatários; maiores como fornecedores: ' +
            '; '.join(f'{buyers[s["nif"]]["nome"].split(SEP)[0]} recebeu {eur(s["valorTotal"])} em {s["numContratos"]} contratos '
                      f'(comprou {eur(buyers[s["nif"]]["valorTotal"])})' for s in sup[:4]))


Q('b06', 2, 2, 'Quem compra',
  'Quais dos maiores compradores públicos também aparecem como adjudicatários, e quanto receberam como fornecedores '
  'face ao que compraram?',
  '[os 100 adjudicantes de maior `valorTotal`]',
  [S(AD, 'search', '--sort', 'valorTotal:desc', '--limit', '100', '--fields', 'nif,nome,valorTotal'),
   S(F, 'search', '--terms', 'nif={x}', '--sort', 'valorTotal:desc', '--limit', '10', '--exact-total', '--fields', 'nif,numContratos,valorTotal', x=J(1, 'nif'))],
  'Comprador e fornecedor são índices distintos com a mesma chave NIF.',
  'Interseção por NIF; comparação do valor recebido com o valor comprado.',
  a_b06)


def a_b07(c):
    an = rows(c, 1)
    ents = byfield(c, 2, 'nif')
    regions = {}
    for e in ents.values():
        for d in _split(e.get('nutsDesc')) or ['sem NUTS']:
            regions[d] = regions.get(d, 0) + 1
    return (f'{tot(c, 1)} concursos de conceção desde 2020; {len(ents)} entidades distintas nos 30 mais recentes; regiões: ' +
            ', '.join(f'{a} ({b})' for a, b in sorted(regions.items(), key=lambda x: -x[1])[:5]))


Q('b07', 2, 2, 'Anúncios',
  'Que entidades lançaram anúncios de um modelo de procedimento pouco frequente desde uma data, e em que regiões '
  'estão sediadas?',
  '[modelo de anúncio «Concurso de concepção»; desde 2020-01-01; os 30 mais recentes]',
  [S(A, 'search', '--term', 'modeloAnuncio=Concurso de concepção', '--range', 'dataPublicacao=2020-01-01..', '--sort', 'dataPublicacao:desc',
     '--limit', '30', '--exact-total', '--fields', 'IDAnuncio,nifEntidade,dataPublicacao'),
   S(AD, 'search', '--terms', 'nif={x}', '--limit', '30', '--fields', 'nif,nome,nutsCod,nutsDesc', x=J(1, 'nifEntidade'))],
  'A sede das entidades só está em `adjudicantes`, a partir dos NIF do passo 1.',
  'Contagem de entidades por descrição NUTS nas linhas do passo 2.',
  a_b07)


def a_b08(c):
    sup = set(vals(c, 1, 'nif'))
    ents = byfield(c, 3, 'nif')
    out, outside = [], 0
    for k in rows(c, 2):
        e = ents.get(k['adjudicanteNif'], {})
        reg = (e.get('nutsCod') or ['—'])[0]
        if reg != 'PT16E':
            outside += 1
        out.append(f'{nif(k["adjudicatarioNif"].split(SEP)[0])}→{(e.get("nome") or ["?"])[0]} [{reg}] {eur(k["precoContratual"])}')
    return f'{len(sup)} PME sediadas na Região de Coimbra; dos 10 maiores contratos, {outside} foram com entidades sediadas fora da região: ' + '; '.join(out[:4])


Q('b08', 4, 4, 'PME',
  'Quais os maiores contratos ganhos por PME sediadas numa NUTS III, quantos foram celebrados com entidades sediadas '
  'fora dessa região, e o que foi contratado (descrição CPV)?',
  '[NUTS III PT16E – Região de Coimbra; PME = sim; 10 maiores contratos]',
  [S(F, 'search', '--term', 'pme=sim', '--term', 'nutsCod=PT16E', '--limit', '500', '--fields', 'nif'),
   S(C, 'search', '--terms', 'adjudicatarioNif={x}', '--sort', 'precoContratual:desc', '--limit', '10',
     '--fields', 'idcontrato,adjudicatarioNif,adjudicanteNif,cpvPrincipal,precoContratual', x=J(1, 'nif')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif')),
   S(K, 'get', '{*k}', k=KEYS(2, 'cpvPrincipal'))],
  'PME e sede do fornecedor estão em `fornecedores`, a sede da entidade em `adjudicantes`, a descrição CPV em `cpv`; os '
  'passos 3 e 4 dependem do passo 2.',
  'Compara o `nutsCod` de cada entidade com PT16E.',
  plus(a_b08, lambda c: '; CPV: ' + '; '.join(d['descricao'] for d in rows(c, 4)[:3])))


def a_b09(c):
    ks = rows(c, 2)
    return (f'{tot(c, 2)} contratos dos 20 fornecedores com atividade contínua 2012–2026 com o Município do Porto; maiores: ' +
            '; '.join(f'{"+".join(nif(x) for x in _split(k["adjudicatarioNif"]))} {eur(k["precoContratual"])} ({k["dataPublicacao"]})' for k in ks[:3]))


Q('b09', 2, 2, 'Quem fornece',
  'Dos fornecedores com atividade contínua entre dois anos e mais contratos, quantos contratos celebraram com uma '
  'dada entidade e quais os maiores?',
  '[primeiro ano 2012 e último ano 2026; os 20 primeiros por `numContratos`; Município do Porto, NIF 501306099]',
  [S(F, 'search', '--term', 'primeiroAno=2012', '--term', 'ultimoAno=2026', '--sort', 'numContratos:desc', '--limit', '20', '--fields', 'nif,numContratos,valorTotal'),
   S(C, 'search', '--terms', 'adjudicatarioNif={x}', '--term', 'adjudicanteNif=501306099', '--sort', 'precoContratual:desc', '--limit', '5',
     '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,precoContratual,dataPublicacao', x=J(1, 'nif'))],
  'Os anos de atividade são agregados de `fornecedores`; os contratos com a entidade estão em `contratos`.',
  'Nenhuma além da leitura das linhas.',
  a_b09)


def a_b10(c):
    sup = rows(c, 2)
    pme = sum(1 for d in sup if d.get('pme') == 'sim')
    return (f'{tot(c, 1)} contratos de refeições escolares do Município de Lisboa; {len(sup)} adjudicatários distintos, '
            f'{pme} PME: ' + ', '.join(f'{nif(d["nif"])} (PME {d.get("pme", "s/i")})' for d in sup))


Q('b10', 2, 2, 'PME',
  'Quem ganhou os contratos de uma entidade cujo objeto contém uma dada expressão, e esses adjudicatários são PME?',
  '[Município de Lisboa, NIF 500051070; expressão «refeições escolares» no objeto do contrato]',
  [S(C, 'search', '--term', 'adjudicanteNif=500051070', '--phrase-in', 'objectoContrato=refeições escolares', '--sort', 'dataPublicacao:desc',
     '--limit', '20', '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,precoContratual,dataPublicacao'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'O atributo PME só está em `fornecedores`, a partir dos NIF do passo 1.',
  'Contagem de PME nos documentos do passo 2.',
  a_b10)


def a_b11(c):
    inv = rows(c, 1)
    won = byfield(c, 2, 'nif')
    ratio = sorted(((int(won[r['nif']]['numContratos']) / int(r['numConvites']) if r['nif'] in won else 0.0, r) for r in inv),
                   key=lambda x: x[0])
    never = sum(1 for r in inv if r['nif'] not in won)
    return (f'{never} das 50 entidades mais convidadas nunca foram adjudicatárias; menores rácios contratos ganhos/convites: ' +
            ', '.join(f'{nif(r["nif"])} {won.get(r["nif"], {}).get("numContratos", 0)}/{r["numConvites"]}' for x, r in ratio[:4]))


Q('b11', 2, 2, 'Concorrência',
  'Das entidades mais convidadas a apresentar proposta, quantas nunca foram adjudicatárias e quais ganham menos contratos por convite?',
  '[as 50 entidades com maior `numConvites`]',
  [S(V, 'search', '--sort', 'numConvites:desc', '--limit', '50', '--fields', 'nif,numConvites,primeiroAno,ultimoAno'),
   S(F, 'search', '--terms', 'nif={x}', '--limit', '50', '--fields', 'nif,numContratos', x=J(1, 'nif'))],
  'Convidadas e adjudicatárias são índices distintos com chave NIF.',
  'Diferença de conjuntos (convidadas ausentes do passo 2) e rácio `numContratos`/`numConvites` por NIF.',
  a_b11, zero_ok=None)


def a_b12(c):
    sup = rows(c, 1)
    desc = {d['nuts']: d['descricao'] for d in rows(c, 2)}
    reg = {}
    for s in sup:
        code = s['nutsCod'].split(SEP)[0]
        reg[code] = reg.get(code, 0) + 1
    return ('sede dos 20 fornecedores de maior valor com NUTS conhecida: ' +
            ', '.join(f'{k} {desc.get(k, "?")} ({v})' for k, v in sorted(reg.items(), key=lambda x: -x[1])))


Q('b12', 2, 2, 'Quem fornece',
  'Em que regiões NUTS estão sediados os fornecedores de maior valor total adjudicado (entre os que têm sede conhecida)?',
  '[os 20 fornecedores de maior `valorTotal` com `nutsCod` preenchido (nível 1 em PT1, PT2 ou PT3)]',
  [S(F, 'search', '--terms', 'nutsNivel1=PT1,PT2,PT3', '--sort', 'valorTotal:desc', '--limit', '20', '--fields', 'nif,nutsCod,valorTotal,pme'),
   S(N, 'get', '{*k}', k=KEYS(1, 'nutsCod'))],
  'A descrição das regiões está em `nuts`, para os códigos do passo 1.',
  'Contagem de fornecedores por código NUTS.',
  a_b12,
  notes=['`--terms nutsNivel1=PT1,PT2,PT3` serve de filtro de presença: um `nutsCod` ausente não tem nível derivado.'])


def a_b13(c):
    subs = vals(c, 1, 'subcontratadaNif')
    found = byfield(c, 2, 'nif')
    pme = sum(1 for x in subs if found.get(x, {}).get('pme') == 'sim')
    return (f'{tot(c, 1)} modificações por cessão/subcontratação com subcontratadas «Lda»; '
            f'{len(subs)} subcontratadas distintas, {len([x for x in subs if x in found])} já foram adjudicatárias, {pme} PME')


Q('b13', 2, 2, 'Modificações',
  'Que empresas foram indicadas como subcontratadas nas modificações por cessão da posição contratual e subcontratação, '
  'e quantas já foram adjudicatárias e são PME?',
  '[fundamento «Cessão da posição contratual e subcontratação (art. 316.º e seguintes do CCP)»; subcontratada com «Lda» no nome; 20 mais recentes]',
  [S(M, 'search', '--term', 'modifContratoFundamento=Cessão da posição contratual e subcontratação (art. 316.º e seguintes do CCP)',
     '--text-in', 'subcontratadaNome=lda', '--sort', 'modifContratoData:desc', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,sequencia,subcontratadaNif,modifContratoData'),
   S(F, 'search', '--terms', 'nif={x}', '--limit', '500', '--fields', 'nif,pme,numContratos', x=J(1, 'subcontratadaNif'))],
  'O atributo PME e o historial de adjudicação estão em `fornecedores`, a partir dos NIF do passo 1.',
  'Interseção da lista de subcontratadas com o passo 2; contagem de PME.',
  a_b13)


def a_b14(c):
    p = rows(c, 1)[0]
    ks = rows(c, 2)
    top = ks[0]
    f = rows(c, 3)[0]
    return (f'procedimento {p["idprocedimento"]} ({p["numContratos"]} contratos, {eur(p["valorTotal"])}, preço base '
            f'{eur(p["precoBaseProcedimento"]) if p["precoBaseProcedimento"] else "não indicado"}); maior fatia: '
            f'{nif(top["adjudicatarioNif"])} com {eur(top["precoContratual"])} = {pct(float(top["precoContratual"]), float(p["valorTotal"]))}; '
            f'esse adjudicatário: PME {f.get("pme", "s/i")}, {f["numContratos"]} contratos e {eur(f["valorTotal"])} no total')


Q('b14', 3, 3, 'Valores',
  'No procedimento com mais contratos lançado por uma entidade, que adjudicatário recebeu a maior fatia do valor total, '
  'é PME e quanto ganhou no total?',
  '[Serviços Partilhados do Ministério da Saúde (SPMS), NIF 509540716]',
  [S(P, 'search', '--term', 'adjudicanteNif=509540716', '--sort', 'numContratos:desc', '--limit', '1',
     '--fields', 'idprocedimento,numContratos,valorTotal,precoBaseProcedimento'),
   S(C, 'search', '--term', 'idprocedimento={x}', '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total',
     '--fields', 'idcontrato,adjudicatarioNif,precoContratual', x=ONE(1, 'idprocedimento')),
   S(F, 'get', '{*k}', k=FN('--key <adjudicatarioNif da 1.ª linha do passo 2>', lambda c: ['--key', first(c, 2, 'adjudicatarioNif')]))],
  'O agregado por procedimento está em `procedimentos`, a repartição por adjudicatário em `contratos` e o historial em '
  '`fornecedores`; cada passo depende do anterior.',
  'Rácio entre o maior preço contratual e o `valorTotal` do procedimento.',
  a_b14)


def a_b15(c):
    ks = [k for k in rows(c, 1) if float(k['PrecoTotalEfetivo']) > float(k['precoContratual'])]
    sup = byfield(c, 2, 'nif')
    out = []
    for k in ks[:4]:
        s = sup.get(k['adjudicatarioNif'].split(SEP)[0], {})
        dev = float(k['PrecoTotalEfetivo']) - float(k['precoContratual'])
        out.append(f'{k["idcontrato"]}: +{eur(dev)} ({pct(dev, float(k["precoContratual"]))}), adjudicatário {nif(s.get("nif", "?"))} PME {s.get("pme", "s/i")}')
    return f'{tot(c, 1)} contratos com preço efetivo; {len(ks)} dos 20 maiores excederam o contratual: ' + '; '.join(out)


Q('b15', 2, 2, 'Valores',
  'Nos contratos concluídos de uma entidade com preço total efetivo registado, quais excederam o preço contratual, '
  'em quanto, e quem foram os adjudicatários?',
  '[Município de Braga, NIF 506901173; preço total efetivo preenchido (o marcador 0 não é indexado); 20 maiores]',
  [S(C, 'search', '--term', 'adjudicanteNif=506901173', '--range', 'PrecoTotalEfetivo=..', '--sort', 'PrecoTotalEfetivo:desc', '--limit', '20',
     '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,precoContratual,PrecoTotalEfetivo'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'O atributo PME só está em `fornecedores`; os NIF vêm do passo 1.',
  'Desvio = efetivo − contratual, filtrado aos positivos nas 20 linhas.',
  a_b15,
  notes=['`--range PrecoTotalEfetivo=..` sem limites seleciona apenas documentos com valor: o marcador `0` (ausente) não é indexado.'])


def a_b16(c):
    out = []
    for k, e in enumerate(rows(c, 1)):
        miss = cnt(c, 2 + k)
        out.append(f'{e["nome"].split(SEP)[0]}: {miss}/{e["numContratos"]} = {pct(miss, int(e["numContratos"]))}')
    return 'contratos sem preço base preenchido — ' + '; '.join(out)


Q('b16', 6, 2, 'Valores',
  'Nos cinco municípios com mais contratos, que proporção dos contratos não tem o preço base do procedimento preenchido?',
  '[nome com «município»; os 5 primeiros por `adjudicantes.numContratos`]',
  [S(AD, 'search', '--text-in', 'nome=município', '--sort', 'numContratos:desc', '--limit', '5', '--fields', 'nif,nome,numContratos')] +
  [S(C, 'count', '--term', 'adjudicanteNif={x}', '--not', '--range', 'precoBaseProcedimento=..', x=ONE(1, 'nif', k, label=f'<{k + 1}.º NIF do passo 1>')) for k in range(5)],
  'Uma contagem por município (o CLI não agrega por valor); o denominador é o agregado `numContratos` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Rácio contagem/`numContratos` por município.',
  a_b16,
  notes=['`--not --range precoBaseProcedimento=..` conta os documentos sem valor: o marcador `0` não é indexado.'])


def a_b17(c):
    r = rows(c, 1)[0]
    ks = rows(c, 2)
    return (f'{r["nuts"]} {r["descricao"]} ({eur(r["valorTotal"])}); {tot(c, 2)} contratos executados aí em 2025; maiores: ' +
            '; '.join(f'{k["idcontrato"]} {k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks) +
            f'; adjudicatários PME {pme_of(c, 3)}')


Q('b17', 3, 3, 'Localização',
  'Qual a região NUTS III com maior valor contratado, quais os maiores contratos aí executados num ano e os seus '
  'adjudicatários são PME?',
  '[exclui PTZZZ (Extra-Regio); ano 2025; 5 maiores]',
  [S(N, 'search', '--not', '--term', 'nuts=PTZZZ', '--sort', 'valorTotal:desc', '--limit', '1', '--fields', 'nuts,descricao,valorTotal'),
   S(C, 'search', '--term', 'nuts={x}', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual', x=ONE(1, 'nuts')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'O valor agregado por região está em `nuts`, os contratos em `contratos`, o atributo PME em `fornecedores`.',
  'Contagem de PME.',
  a_b17)


def a_b18(c):
    locs = rows(c, 1)
    k = rows(c, 2)[0]
    ms = rows(c, 3)
    return ('concelhos de maior valor: ' + ', '.join(f'{d["concelho"]} {eur(d["valorTotal"])}' for d in locs[:3]) +
            f'; maior contrato em {locs[0]["concelho"]}: {k["idcontrato"]} {k["adjudicanteNome"]} {eur(k["precoContratual"])}, '
            f'{len(ms)} modificação(ões)' + (f' (última: «{ms[-1]["modifContratoFundamento"]}»)' if ms else ''))


Q('b18', 3, 3, 'Localização',
  'Quais os concelhos de um distrito com maior valor contratado, qual o maior contrato executado no primeiro e quantas '
  'modificações teve?',
  '[distrito «Portugal, Faro»; 5 concelhos]',
  [S(L, 'search', '--term', 'localNivel2=Portugal, Faro', '--sort', 'valorTotal:desc', '--limit', '5', '--fields', 'local,concelho,numContratos,valorTotal'),
   S(C, 'search', '--term', 'localExecucao={x}', '--sort', 'precoContratual:desc', '--limit', '1',
     '--fields', 'idcontrato,adjudicanteNome,precoContratual', x=ONE(1, 'local')),
   S(M, 'search', '--term', 'idcontrato={x}', '--sort', 'sequencia:asc', '--limit', '100', '--fields', 'idcontrato,sequencia,modifContratoFundamento',
     x=ONE(2, 'idcontrato'))],
  'Agregado por local (`locais`), contrato (`contratos`) e modificações (`modificacoes`) em cadeia.',
  'Nenhuma além da leitura das linhas.',
  a_b18, zero_ok='zero modificações seria uma resposta válida («nenhuma»).')


def a_b19(c):
    cp = rows(c, 1)[0]
    ks = rows(c, 2)
    return (f'{cp["cpv"]} «{cp["descricao"]}» ({eur(cp["valorTotal"])}); {tot(c, 2)} contratos em 2025 com esse CPV principal; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks) + f'; adjudicatários PME {pme_of(c, 3)}')


Q('b19', 3, 3, 'Objeto (CPV)',
  'Qual o código CPV com maior valor dentro de uma classe, quais os maiores contratos de um ano com esse CPV principal e '
  'os adjudicatários são PME?',
  '[classe CPV 3314 (material médico de consumo); ano 2025; 3 maiores]',
  [S(K, 'search', '--term', 'cpvClasse=3314', '--sort', 'valorTotal:desc', '--limit', '1', '--fields', 'cpv,descricao,valorTotal'),
   S(C, 'search', '--term', 'cpvPrincipal={x}', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '3', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual', x=ONE(1, 'cpv')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'O valor por CPV é agregado de `cpv`, os contratos estão em `contratos`, o atributo PME em `fornecedores`.',
  'Contagem de PME.',
  a_b19)


def a_b20(c):
    procs = rows(c, 2)
    return (f'{tot(c, 1)} contratos entre 5 e 10 M€ em 2025; {len(procs)} pertencem a procedimentos com 2+ contratos, p.ex. ' +
            '; '.join(f'{p["idprocedimento"]} ({p["tipoprocedimento"]}, {p["numContratos"]} contratos, {eur(p["valorTotal"])})' for p in procs[:3]))


Q('b20', 2, 2, 'Valores',
  'Entre os contratos de um intervalo de valor celebrados num ano, quais pertencem a procedimentos com vários contratos, '
  'e qual o valor total desses procedimentos?',
  '[preço contratual entre 5 000 000 € e 10 000 000 €; ano 2025]',
  [S(C, 'search', '--range', 'precoContratual=5000000..10000000', '--term', 'Ano=2025', '--limit', '200', '--exact-total',
     '--fields', 'idcontrato,idprocedimento,precoContratual'),
   S(P, 'search', '--terms', 'idprocedimento={x}', '--range', 'numContratos=2..', '--sort', 'valorTotal:desc', '--limit', '50',
     '--fields', 'idprocedimento,tipoprocedimento,numContratos,valorTotal', x=J(1, 'idprocedimento'))],
  'O número de contratos por procedimento é agregado de `procedimentos`; os ids vêm do passo 1.',
  'Nenhuma além da leitura das linhas.',
  a_b20)


def a_b21(c):
    sup = rows(c, 2)
    pme = sum(1 for d in sup if d.get('pme') == 'sim')
    ents = rows(c, 3)
    return (f'{tot(c, 1)} contratos por ajuste direto em 2025 com preço exatamente 19 999,99 € ou 20 000 €; nos 20 listados, '
            f'{len(sup)} adjudicatários distintos, {pme} PME; {len(ents)} entidades, p.ex. {", ".join(e["nome"][0] for e in ents[:3])}')


Q('b21', 3, 3, 'Valores',
  'Quantos contratos por ajuste direto de um ano têm preço exatamente no limiar legal (ou um cêntimo abaixo), quantos '
  'desses adjudicatários são PME e que entidades os celebraram?',
  '[Ajuste Direto Regime Geral; preço 19 999,99 € ou 20 000 €; ano 2025; 20 contratos]',
  [S(C, 'search', '--terms', 'precoContratual=19999.99,20000', '--term', ADRG, '--term', 'Ano=2025', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'O atributo PME só existe em `fornecedores` e os nomes das entidades em `adjudicantes`.',
  'Contagem de PME; nomes distintos das entidades.',
  a_b21)


def a_b22(c):
    procs = rows(c, 1)
    ks = rows(c, 2)
    sup = {k['adjudicatarioNif'] for k in ks}
    return (f'{tot(c, 1)} consultas prévias de março de 2025 com 4+ contratos; nas listadas, {len(ks)} contratos e {len(sup)} '
            f'adjudicatários distintos; maior procedimento {procs[0]["idprocedimento"]} ({procs[0]["numContratos"]} contratos, {eur(procs[0]["valorTotal"])})')


Q('b22', 2, 2, 'Procedimento e fundamento',
  'Nas consultas prévias com primeira publicação num mês e vários contratos, quantos adjudicatários distintos houve?',
  '[Consulta Prévia; primeira publicação em 2025-03; 4 ou mais contratos]',
  [S(P, 'search', '--term', 'tipoprocedimento=Consulta Prévia', '--range', 'primeiraPublicacao=2025-03-01..2025-03-31', '--range', 'numContratos=4..',
     '--sort', 'numContratos:desc', '--limit', '20', '--exact-total', '--fields', 'idprocedimento,numContratos,valorTotal'),
   S(C, 'search', '--terms', 'idprocedimento={x}', '--limit', '300', '--fields', 'idcontrato,idprocedimento,adjudicatarioNif,precoContratual', x=J(1, 'idprocedimento'))],
  'O número de contratos e a primeira publicação são agregados de `procedimentos`; os adjudicatários estão em `contratos`.',
  'Contagem de NIF distintos nas linhas do passo 2.',
  a_b22)


def a_b23(c):
    ks = rows(c, 1)
    procs = byfield(c, 2, 'idprocedimento')
    return (f'{tot(c, 1)} contratos de 2025 com fundamento na alínea d) do n.º 1 do art. 20.º; maiores: ' +
            '; '.join(f'{k["idcontrato"]} {eur(k["precoContratual"])} (procedimento com {procs[k["idprocedimento"]]["numContratos"]} contrato(s))' for k in ks))


Q('b23', 2, 2, 'Procedimento e fundamento',
  'Quantos contratos de um ano invocam uma alínea legal (pesquisada como expressão), e quantos contratos teve o '
  'procedimento de cada um dos maiores?',
  '[expressão «Artigo 20.º, n.º 1, alínea d)» no fundamento; ano 2025; 5 maiores]',
  [S(C, 'search', '--phrase-in', 'fundamentacao=Artigo 20.º, n.º 1, alínea d)', '--term', 'Ano=2025', '--sort', 'precoContratual:desc',
     '--limit', '5', '--exact-total', '--fields', 'idcontrato,idprocedimento,precoContratual'),
   S(P, 'get', '{*k}', k=KEYS(1, 'idprocedimento'))],
  'O número de contratos por procedimento só está em `procedimentos`.',
  'Emparelha contrato e procedimento por id.',
  a_b23)


def a_b24(c):
    ks = rows(c, 1)
    desc = {d['cpv']: d['descricao'] for d in rows(c, 2)}
    ents = {d['nif']: d for d in rows(c, 3)}
    return (f'{tot(c, 1)} ajustes diretos de 2025 fundamentados em «outros»; maiores: ' +
            '; '.join(f'{eur(k["precoContratual"])} CPV {k["cpvPrincipal"]} «{desc[k["cpvPrincipal"]]}» '
                      f'({ents[k["adjudicanteNif"].split(SEP)[0]]["nome"][0]})' for k in ks[:4]))


Q('b24', 3, 3, 'Procedimento e fundamento',
  'Que objetos (descrição CPV) têm os maiores ajustes diretos de um ano com uma dada fundamentação do ajuste direto, e '
  'que entidades os celebraram?',
  '[fundamentação do ajuste direto «outros»; ano 2025; 10 maiores]',
  [S(C, 'search', '--term', 'fundamentAjusteDireto=outros', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '10',
     '--exact-total', '--fields', 'idcontrato,adjudicanteNif,cpvPrincipal,precoContratual'),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpvPrincipal')),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'A descrição canónica de cada CPV está em `cpv` e os nomes das entidades em `adjudicantes`.',
  'Emparelha contrato, descrição e entidade por código e NIF.',
  a_b24)


def a_b25(c):
    ents = byfield(c, 2, 'nif')
    types = {}
    for e in ents.values():
        t = (e.get('tipoEntidade') or ['sem tipo indicado'])[0]
        types[t] = types.get(t, 0) + 1
    return (f'{tot(c, 1)} contratos de 2024 sem redução a escrito por urgência imperiosa; {len(ents)} entidades nos 20 maiores; tipos: ' +
            ', '.join(f'{a} ({b})' for a, b in types.items()))


Q('b25', 2, 2, 'Procedimento e fundamento',
  'Que entidades invocaram uma justificação para não reduzir o contrato a escrito num ano, e de que tipo são?',
  '[justificação com as palavras «urgência imperiosa»; ano 2024; 20 maiores]',
  [S(C, 'search', '--text-in', 'justifNReducEscrContrato=urgência imperiosa', '--term', 'Ano=2024', '--sort', 'precoContratual:desc',
     '--limit', '20', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'O tipo de entidade só existe em `adjudicantes`.',
  'Contagem por `tipoEntidade` dos documentos do passo 2.',
  a_b25)


def a_b26(c):
    procs = rows(c, 1)
    ks = rows(c, 2)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 3)}
    out = []
    for p in procs:
        e = ents.get(p['adjudicanteNif'].split(SEP)[0], '?')
        n = sum(1 for k in ks if k['idprocedimento'] == p['idprocedimento'])
        out.append(f'{p["tipoprocedimento"]}: {e} ({n} contrato(s), {eur(p["valorTotal"])})')
    return '; '.join(out)


Q('b26', 4, 4, 'Procedimento e fundamento',
  'Que entidades usaram os tipos de procedimento mais raros, que contratos resultaram, de que valor, e os adjudicatários são PME?',
  '[Parceria para a inovação; Diálogo concorrencial; Concurso de ideias simplificado]',
  [S(P, 'search', '--terms', 'tipoprocedimento=Parceria para a inovação,Diálogo concorrencial,Concurso de ideias simplificado',
     '--sort', 'tipoprocedimento:asc', '--limit', '20', '--fields', 'idprocedimento,tipoprocedimento,adjudicanteNif,valorTotal'),
   S(C, 'search', '--terms', 'idprocedimento={x}', '--limit', '50', '--fields', 'idcontrato,idprocedimento,adjudicatarioNif,precoContratual', x=J(1, 'idprocedimento')),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'search', '--terms', 'nif={x}', '--limit', '50', '--fields', 'nif,pme', x=J(2, 'adjudicatarioNif'))],
  'Procedimentos, contratos, nomes de entidade e atributo PME estão em quatro índices; os passos 2 e 3 dependem do passo 1, '
  'o 4 do passo 2.',
  'Contagem de contratos por procedimento nas linhas do passo 2; contagem de PME.',
  plus(a_b26, lambda c: f'; adjudicatários PME {pme_of(c, 4)}'))


def a_b27(c):
    ks = rows(c, 1)
    an = byfield(c, 2, 'IDAnuncio')
    pr = [int(an[k['IDAnuncio']]['PrazoPropostas']) for k in ks if k['IDAnuncio'] in an and an[k['IDAnuncio']]['PrazoPropostas']]
    return (f'{tot(c, 1)} contratos de concursos urgentes em 2025; prazos de propostas dos anúncios dos 10 maiores: '
            f'mín. {min(pr)}, mediana {statistics.median(pr)}, máx. {max(pr)} dias')


Q('b27', 2, 2, 'Prazos e datas',
  'Nos contratos resultantes de concursos urgentes de um ano, que prazo para apresentação de propostas deram os anúncios?',
  '[tipo de anúncio «Anúncio de concurso urgente»; ano 2025; 10 maiores contratos]',
  [S(C, 'search', '--term', 'TipoAnuncio=Anúncio de concurso urgente', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '10',
     '--exact-total', '--fields', 'idcontrato,IDAnuncio,precoContratual'),
   S(A, 'search', '--terms', 'IDAnuncio={x}', '--fields', 'IDAnuncio,PrazoPropostas,dataPublicacao,DataLimitePropostas', x=J(1, 'IDAnuncio'))],
  'O prazo de propostas só existe em `anuncios`.',
  'Mínimo, mediana e máximo dos prazos devolvidos.',
  a_b27)


def a_b28(c):
    ks = rows(c, 1)
    desc = {d['nuts']: d['descricao'] for d in rows(c, 2)}
    reg = {}
    for k in ks:
        for x in _split(k['nuts']):
            reg[x] = reg.get(x, 0) + 1
    return (f'{tot(c, 1)} contratos PRR (art. 2.º, DL 78/2022) com NUTS indicada; regiões dos 20 maiores: ' +
            ', '.join(f'{desc.get(a, a)} ({b})' for a, b in sorted(reg.items(), key=lambda x: -x[1])[:5]))


Q('b28', 2, 2, 'Procedimento e fundamento',
  'Em que regiões foram executados os maiores contratos celebrados ao abrigo de uma medida especial de contratação?',
  '[TipoMEC «Plano de Recuperação e Resiliência (PRR) – artigo 2º da Lei n.º30/2021, DL n.º 78/2022»; NUTS indicada; 20 maiores]',
  [S(C, 'search', '--term', 'TipoMEC=Plano de Recuperação e Resiliência (PRR) – artigo 2º da Lei n.º30/2021, DL n.º 78/2022',
     '--terms', 'nutsNivel1=PT1,PT2,PT3', '--sort', 'precoContratual:desc', '--limit', '20', '--exact-total', '--fields', 'idcontrato,nuts,precoContratual'),
   S(N, 'get', '{*k}', k=KEYS(1, 'nuts'))],
  'A descrição das regiões está em `nuts`.',
  'Contagem de contratos por NUTS (campo multivalor).',
  a_b28)


def a_b29(c):
    cps = rows(c, 1)
    ks = rows(c, 2)
    ents = {d['nif']: d for d in rows(c, 3)}
    return (f'{len(cps)} códigos CPV com «refeições» ({", ".join(d["cpv"] for d in cps[:4])}…); {tot(c, 2)} contratos de 2025; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])} '
                      f'[{(ents[k["adjudicanteNif"].split(SEP)[0]].get("nutsDesc") or ["sem NUTS"])[0]}]' for k in ks[:3]))


Q('b29', 3, 3, 'Objeto (CPV)',
  'Que códigos CPV descrevem um dado tipo de bem ou serviço, quantos contratos de um ano os usaram e onde estão sediadas '
  'as entidades dos maiores?',
  '[palavra «refeições» na descrição CPV; ano 2025; 5 maiores]',
  [S(K, 'search', '--text-in', 'descricao=refeições', '--sort', 'numContratos:desc', '--limit', '50', '--fields', 'cpv,descricao,numContratos'),
   S(C, 'search', '--terms', 'cpv={x}', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,adjudicanteNome,precoContratual', x=J(1, 'cpv')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Vocabulário CPV (`cpv`), contratos (`contratos`) e sede das entidades (`adjudicantes`) em cadeia.',
  'Nenhuma além da leitura das linhas.',
  a_b29)


def a_b30(c):
    cps = rows(c, 1)
    ks = rows(c, 2)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 3)}
    return (f'CPV mais usados como complementares: ' + ', '.join(f'{d["cpv"]} «{d["descricao"]}» ({d["numComplementar"]})' for d in cps[:3]) +
            f'; {tot(c, 2)} contratos em que surgem só como complementares; entidades recentes: ' +
            ', '.join(sorted({ents.get(x, "?") for k in ks for x in _split(k["adjudicanteNif"])})[:4]))


Q('b30', 3, 3, 'Objeto (CPV)',
  'Que códigos CPV são mais usados como código complementar e que entidades os usam sem os declarar como principal?',
  '[os 5 primeiros por `numComplementar`; 20 contratos mais recentes]',
  [S(K, 'search', '--sort', 'numComplementar:desc', '--limit', '5', '--fields', 'cpv,descricao,numComplementar,numPrincipal'),
   S(C, 'search', '--terms', 'cpv={x}', '--not', '--terms', 'cpvPrincipal={x}', '--sort', 'dataPublicacao:desc', '--limit', '20',
     '--exact-total', '--fields', 'idcontrato,adjudicanteNif,cpv,cpvPrincipal', x=J(1, 'cpv')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Agregado complementar (`cpv`), contratos e nomes de entidade estão em três índices encadeados.',
  'Nomes distintos das entidades dos contratos devolvidos.',
  a_b30)


def a_b31(c):
    ks = rows(c, 1)
    desc = {d['cpv']: d['descricao'] for d in rows(c, 2)}
    codes = sorted({x for k in ks for x in _split(k['cpv'])})
    return (f'{tot(c, 1)} contratos de 2025 com «software» na descrição CPV mas não no objeto; CPV dos 10 maiores: ' +
            '; '.join(f'{x} «{desc[x]}»' for x in codes[:5]))


Q('b31', 2, 2, 'Objeto (CPV)',
  'Que contratos de um ano têm um termo na descrição do CPV mas não no objeto do contrato, e que códigos CPV são?',
  '[«software» em `cpvDescricao` e não em `objectoContrato`; ano 2025; 10 maiores]',
  [S(C, 'search', '--text-in', 'cpvDescricao=software', '--not', '--text-in', 'objectoContrato=software', '--term', 'Ano=2025',
     '--sort', 'precoContratual:desc', '--limit', '10', '--exact-total', '--fields', 'idcontrato,cpv,precoContratual'),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpv'))],
  'A descrição canónica dos códigos está em `cpv`.',
  'Códigos distintos nas linhas devolvidas.',
  a_b31)


def a_b32(c):
    cps = rows(c, 1)
    an = rows(c, 2)
    ents = rows(c, 3)
    return (f'CPV da categoria 33141: {", ".join(d["cpv"] for d in cps)}; {tot(c, 2)} anúncios desde 2025; maiores preços base: ' +
            '; '.join(f'{a["IDAnuncio"]} {eur(a["PrecoBase"])} ({a["designacaoEntidade"].split(SEP)[0]})' for a in an) +
            f'; sede das entidades: {", ".join(sorted({e["nutsDesc"].split(SEP)[0] or "sem NUTS" for e in ents}))}')


Q('b32', 3, 3, 'Anúncios',
  'Que anúncios publicados desde uma data usaram os códigos CPV mais frequentes de uma categoria, com que preço base, e '
  'onde estão sediadas as entidades?',
  '[categoria CPV 33141 (material médico descartável); 5 códigos mais frequentes; desde 2025-01-01; 5 maiores preços base]',
  [S(K, 'search', '--term', 'cpvCategoria=33141', '--sort', 'numContratos:desc', '--limit', '5', '--fields', 'cpv,descricao,numContratos'),
   S(A, 'search', '--terms', 'cpv={x}', '--range', 'dataPublicacao=2025-01-01..', '--sort', 'PrecoBase:desc', '--limit', '5', '--exact-total',
     '--fields', 'IDAnuncio,PrecoBase,designacaoEntidade,nifEntidade', x=J(1, 'cpv')),
   S(AD, 'search', '--terms', 'nif={x}', '--fields', 'nif,nome,nutsDesc', x=J(2, 'nifEntidade'))],
  'Frequência por código (`cpv`), anúncios (`anuncios`) e sede (`adjudicantes`) em cadeia.',
  'Nenhuma além da leitura das linhas.',
  a_b32)


def a_b33(c):
    out = []
    for k, d in enumerate(rows(c, 1)):
        eco = cnt(c, 2 + k)
        out.append(f'{d["cpv"]} «{d["descricao"]}»: {eco}/{d["numPrincipal"]} = {pct(eco, int(d["numPrincipal"]))}')
    return 'contratos ecológicos por CPV principal — ' + '; '.join(out)


Q('b33', 3, 2, 'Sustentabilidade',
  'Nos dois códigos CPV principais mais usados de uma divisão, que proporção dos contratos foi marcada como ecológica?',
  '[divisão CPV 45 (construção); os 2 primeiros por `numPrincipal`]',
  [S(K, 'search', '--term', 'cpvDivisao=45', '--sort', 'numPrincipal:desc', '--limit', '2', '--fields', 'cpv,descricao,numPrincipal')] +
  [S(C, 'count', '--term', 'cpvPrincipal={x}', '--term', 'ContratEcologico=sim', x=ONE(1, 'cpv', k, label=f'<{k + 1}.º CPV do passo 1>')) for k in range(2)],
  'Uma contagem por código; o denominador é o agregado `numPrincipal` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Rácio contagem/`numPrincipal`.',
  a_b33)


def a_b34(c):
    lo = rows(c, 1)
    k = rows(c, 2)[0]
    ms = rows(c, 3)
    return (f'local {lo[0]["local"]} ({lo[0]["numContratos"]} contratos, {eur(lo[0]["valorTotal"])}); maior contrato de 2025: '
            f'{k["idcontrato"]} {k["adjudicanteNome"]} {eur(k["precoContratual"])}; {len(ms)} modificação(ões)')


Q('b34', 3, 3, 'Localização',
  'Que local de execução corresponde exatamente a um concelho, qual o maior contrato aí executado num ano e quantas '
  'modificações teve?',
  '[concelho «Lagos» (termo exato: a pesquisa por palavras também apanharia «Lagoa», mesma raiz); ano 2025]',
  [S(L, 'search', '--term', 'concelho=Lagos', '--fields', 'local,numContratos,valorTotal'),
   S(C, 'search', '--term', 'localExecucao={x}', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '1',
     '--fields', 'idcontrato,adjudicanteNome,precoContratual', x=ONE(1, 'local')),
   S(M, 'search', '--term', 'idcontrato={x}', '--limit', '100', '--fields', 'idcontrato,sequencia', x=ONE(2, 'idcontrato'))],
  'Partes do local (`locais`), contrato (`contratos`) e modificações (`modificacoes`) em cadeia.',
  'Nenhuma além da leitura das linhas.',
  a_b34, zero_ok='zero modificações é uma resposta válida («nenhuma»).')


def a_b35(c):
    lo = rows(c, 1)
    top = sorted(lo, key=lambda d: -int(d['numContratos']))[:2]
    return (f'{len(lo)} concelhos na Madeira (de {lo[0]["concelho"]} a {lo[-1]["concelho"]}); empreitadas de 2025: ' +
            '; '.join(f'{d["concelho"]} {cnt(c, 2 + k)}' for k, d in enumerate(top)))


def _mad(k):
    return FN(f'<{k + 1}.º concelho com mais contratos do passo 1>',
              lambda c: sorted(rows(c, 1), key=lambda d: -int(d['numContratos']))[k]['local'])


Q('b35', 3, 2, 'Localização',
  'Que concelhos tem uma região autónoma (por ordem alfabética), e quantas empreitadas de um ano se executaram nos dois '
  'com mais contratos?',
  '[distrito «Região Autónoma da Madeira» (pesquisa por expressão, sem o local só de distrito); empreitadas de obras públicas; ano 2025]',
  [S(L, 'search', '--phrase-in', 'distrito=Região Autónoma da Madeira', '--not', '--key', 'Portugal, Região Autónoma da Madeira',
     '--sort', 'concelho:asc', '--limit', '20', '--fields', 'local,concelho,numContratos')] +
  [S(C, 'count', '--term', 'localExecucao={x}', '--term', EMP, '--term', 'Ano=2025', x=_mad(k)) for k in range(2)],
  'Uma contagem por concelho; a lista de concelhos vem de `locais`. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Escolhe os dois concelhos de maior `numContratos` nas linhas do passo 1.',
  a_b35)


def a_b36(c):
    reg = [r for r in rows(c, 1) if int(r['numFornecedores']) > int(r['numAdjudicantes'])]
    sup = rows(c, 2)
    return (f'NUTS com mais fornecedores do que adjudicantes sediados: ' +
            ', '.join(f'{r["nuts"]} {r["descricao"]} ({r["numFornecedores"]} vs {r["numAdjudicantes"]})' for r in reg[:3]) +
            f'; maiores PME sediadas em {reg[0]["descricao"]}: ' + ', '.join(f'{nif(s["nif"])} {eur(s["valorTotal"])}' for s in sup[:3]))


Q('b36', 2, 2, 'PME',
  'Que regiões NUTS têm mais fornecedores sediados do que entidades adjudicantes, e quais as PME de maior valor na primeira?',
  '[regiões com 500 ou mais fornecedores sediados; 5 PME]',
  [S(N, 'search', '--range', 'numFornecedores=500..', '--sort', 'numFornecedores:desc', '--limit', '10', '--fields', 'nuts,descricao,numAdjudicantes,numFornecedores'),
   S(F, 'search', '--term', 'nutsCod={x}', '--term', 'pme=sim', '--sort', 'valorTotal:desc', '--limit', '5', '--fields', 'nif,valorTotal,numContratos',
     x=FN('<primeira NUTS do passo 1 com numFornecedores > numAdjudicantes>',
          lambda c: next(r['nuts'] for r in rows(c, 1) if int(r['numFornecedores']) > int(r['numAdjudicantes']))))],
  'Os contadores por região estão em `nuts`; o atributo PME em `fornecedores`.',
  'Comparação numFornecedores > numAdjudicantes por linha.',
  a_b36)


def a_b37(c):
    reg = rows(c, 1)
    ks = rows(c, 2)
    return (f'NUTS das regiões autónomas: {", ".join(r["nuts"] for r in reg)}; {tot(c, 2)} contratos de compra conjunta aí executados; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks[:3]))


Q('b37', 2, 2, 'Compras centralizadas e conjuntas',
  'Quantos contratos de compra conjunta foram executados nas regiões cuja descrição NUTS contém uma expressão, e quais os maiores?',
  '[expressão «Região Autónoma» na descrição NUTS; compra conjunta = sim]',
  [S(N, 'search', '--phrase-in', 'descricao=Região Autónoma', '--limit', '20', '--fields', 'nuts,descricao'),
   S(C, 'search', '--terms', 'nuts={x}', '--term', 'compraConjunta=sim', '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNome,precoContratual', x=J(1, 'nuts'))],
  'As descrições NUTS estão em `nuts`; os contratos em `contratos`.',
  'Nenhuma além da leitura das linhas.',
  a_b37)


def a_b38(c):
    lo = rows(c, 1)
    k = rows(c, 2)[0]
    sup = rows(c, 3)
    return (f'locais fora de Portugal com mais contratos: ' + ', '.join(f'{d["local"]} ({d["numContratos"]})' for d in lo[:4]) +
            f'; maior contrato em {lo[0]["local"]}: {k["idcontrato"]} {eur(k["precoContratual"])}; entidade: ' +
            ', '.join(f'{d["nome"][0]} [{(d.get("nutsDesc") or ["sem NUTS"])[0]}, {d.get("tipoEntidade", ["tipo n/i"])[0]}]' for d in sup))


Q('b38', 3, 3, 'Localização',
  'Que locais de execução fora do país concentram mais contratos, qual o maior contrato executado no primeiro e que '
  'entidade o celebrou (sede e tipo)?',
  '[nível 1 do local diferente de «Portugal»]',
  [S(L, 'search', '--not', '--term', 'localNivel1=Portugal', '--sort', 'numContratos:desc', '--limit', '10', '--fields', 'local,numContratos,valorTotal'),
   S(C, 'search', '--term', 'localExecucao={x}', '--sort', 'precoContratual:desc', '--limit', '1',
     '--fields', 'idcontrato,adjudicanteNif,precoContratual', x=ONE(1, 'local')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Agregados por local (`locais`), contrato (`contratos`) e entidade (`adjudicantes`) em cadeia.',
  'Nenhuma além da leitura das linhas.',
  a_b38)


def a_b39(c):
    ks = rows(c, 2)
    return (f'{len(rows(c, 1))} entidades sediadas nos Açores excluídas; {tot(c, 2)} contratos executados nos Açores por entidades de fora; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks))


Q('b39', 2, 2, 'Localização',
  'Que entidades sediadas fora de uma região executam lá contratos, e quais os maiores?',
  '[região NUTS de nível 1 PT2 (Açores) / local «Portugal, Região Autónoma dos Açores»]',
  [S(AD, 'search', '--term', 'nutsNivel1=PT2', '--limit', '1000', '--fields', 'nif'),
   S(C, 'search', '--term', 'localNivel2=Portugal, Região Autónoma dos Açores', '--not', '--terms', 'adjudicanteNif={x}', '--sort', 'precoContratual:desc',
     '--limit', '5', '--exact-total', '--fields', 'idcontrato,adjudicanteNome,precoContratual', x=J(1, 'nif'))],
  'A sede das entidades só está em `adjudicantes`.',
  'Nenhuma além da leitura das linhas.',
  a_b39)


def a_b40(c):
    an = byfield(c, 1, 'IDAnuncio')
    d = [days(an[k['IDAnuncio']]['dataPublicacao'], k['dataCelebracaoContrato']) for k in rows(c, 2)
         if k['IDAnuncio'] in an and k.get('dataCelebracaoContrato')]
    return (f'{tot(c, 1)} anúncios de procedimento em 2024; {len(d)} contratos ligados; dias entre anúncio e celebração: '
            f'mín. {min(d)}, mediana {statistics.median(d)}, máx. {max(d)}')


Q('b40', 2, 2, 'Prazos e datas',
  'Quanto tempo mediou entre a publicação dos anúncios de procedimento de uma entidade num ano e a celebração dos '
  'contratos resultantes?',
  '[Município de Coimbra, NIF 506415082; «Anúncio de procedimento»; ano 2024]',
  [S(A, 'search', '--term', 'nifEntidade=506415082', '--term', 'tipoActo=Anúncio de procedimento', '--range', 'dataPublicacao=2024-01-01..2024-12-31',
     '--limit', '100', '--exact-total', '--fields', 'IDAnuncio,dataPublicacao'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '200', '--fields', 'idcontrato,IDAnuncio,dataCelebracaoContrato', x=J(1, 'IDAnuncio'))],
  'A data do anúncio está em `anuncios`, a da celebração em `contratos`.',
  'Diferença em dias por contrato; mínimo, mediana, máximo.',
  a_b40)

# ---- Batch 3 -----------------------------------------------------------------


def a_c01(c):
    ks = rows(c, 1)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 2)}
    sup = rows(c, 3)
    cps = rows(c, 4)
    return (f'{tot(c, 1)} contratos de 2025 ao abrigo de acordos-quadro de combustíveis; {len(ents)} entidades nos 20 maiores '
            f'(p.ex. {", ".join(list(ents.values())[:3])}); fornecedores: {", ".join(nif(d["nif"]) + " PME " + d.get("pme", "s/i") for d in sup)}; '
            f'CPV: {"; ".join(d["cpv"] + " «" + d["descricao"] + "»" for d in cps)}')


Q('c01', 4, 4, 'Acordos-quadro',
  'Que entidades compraram ao abrigo de acordos-quadro de um dado tipo de bem num ano, a que fornecedores (PME?) e com que CPV?',
  '[«Ao abrigo de acordo-quadro (art.º 259.º)»; descrição do acordo-quadro com «combustíveis»; ano 2025; 20 maiores]',
  [S(C, 'search', '--term', 'tipoprocedimento=Ao abrigo de acordo-quadro (art.º 259.º)', '--text-in', 'DescrAcordoQuadro=combustíveis',
     '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,numAcordoQuadro,adjudicanteNif,adjudicatarioNif,precoContratual,cpvPrincipal'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpvPrincipal'))],
  'Os passos 2 a 4 dependem do passo 1 e consultam três índices diferentes.',
  'Nenhuma além da leitura das linhas.',
  a_c01)


def a_c02(c):
    ms = rows(c, 1)
    ks = {d['idcontrato']: d for d in rows(c, 2)}
    ents = {d['nif']: d for d in rows(c, 3)}
    sup = {d['nif']: d for d in rows(c, 4)}
    procs = {d['idprocedimento']: d for d in rows(c, 5)}
    out = []
    for m in ms[:4]:
        k = ks[m['idcontrato']]
        e = ents.get(k['adjudicanteNif'][0], {})
        s = sup.get(k['adjudicatarioNif'][0], {}) if k.get('adjudicatarioNif') else {}
        p = procs.get(k['idprocedimento'], {})
        out.append(f'{m["idcontrato"]}/{m["sequencia"]}: {eur(m["modifContratoPrecoAlterado"])} vs contratual {eur(k["precoContratual"])} '
                   f'({e.get("nome", ["?"])[0]}, {(e.get("tipoEntidade") or ["tipo n/i"])[0]}; adjudicatário {nif(s.get("nif", "-"))} PME {s.get("pme", "s/i")}; '
                   f'{p.get("tipoprocedimento", "?")})')
    return f'{tot(c, 1)} modificações de 2024 por trabalhos complementares; maiores: ' + '; '.join(out)


Q('c02', 5, 5, 'Modificações',
  'Quais as maiores modificações de um ano com um dado fundamento, face ao preço contratual original, e quem são a '
  'entidade, o adjudicatário (PME?) e o tipo de procedimento de cada contrato?',
  '[fundamento «Trabalhos Complementares (art. 370.º do CCP)»; ano 2024; 10 maiores por preço alterado]',
  [S(M, 'search', '--term', 'modifContratoFundamento=Trabalhos Complementares (art. 370.º do CCP)', '--term', 'Ano=2024',
     '--sort', 'modifContratoPrecoAlterado:desc', '--limit', '10', '--exact-total', '--fields', 'idcontrato,sequencia,modifContratoPrecoAlterado'),
   S(C, 'get', '{*k}', k=KEYS(1, 'idcontrato')),
   S(AD, 'get', '{*k}', k=FN('--key <adjudicanteNif dos contratos do passo 2> …', lambda c: [t for x in vals(c, 2, 'adjudicanteNif') for t in ('--key', x)])),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif')),
   S(P, 'get', '{*k}', k=KEYS(2, 'idprocedimento'))],
  'O contrato depende do passo 1; entidade, adjudicatário e procedimento dependem do passo 2 e estão em três índices.',
  'Comparação preço alterado vs preço contratual por contrato.',
  a_c02)


def a_c03(c):
    m = rows(c, 1)[0]
    k = rows(c, 2)[0]
    return (f'modificação {m["idcontrato"]}/{m["sequencia"]} ({m["modifContratoTipoAto"]}, {m["modifContratoData"]}): '
            f'«{m["modifContratoFundamento"]}», preço alterado {eur(m["modifContratoPrecoAlterado"])}; contrato de {k["adjudicanteNome"][0]} '
            f'com preço contratual {eur(k["precoContratual"])} (rácio {float(m["modifContratoPrecoAlterado"]) / float(k["precoContratual"]):.1f}×)')


Q('c03', 2, 2, 'Modificações',
  'Qual o conteúdo de uma modificação identificada pela chave composta contrato/sequência, e como se compara com o '
  'preço contratual original?',
  '[modificação 7682444/2 (a de maior preço alterado do índice)]',
  [S(M, 'get', '--key', '7682444/2'),
   S(C, 'get', '--key', '7682444')],
  'Modificação e contrato estão em índices diferentes.',
  'Rácio preço alterado / preço contratual.',
  a_c03)


def a_c04(c):
    ks = rows(c, 1)
    ms = rows(c, 2)
    last = {}
    for m in ms:
        if m['idcontrato'] not in last:
            last[m['idcontrato']] = m
    sup = {d['nif']: d for d in rows(c, 3)}
    return ('; '.join(f'{k["idcontrato"]} ({k["numModificacoes"]} modif.; última: «{last[k["idcontrato"]]["modifContratoFundamento"]}» em '
                      f'{last[k["idcontrato"]]["modifContratoData"]}; adjudicatário {nif(k["adjudicatarioNif"])} PME '
                      f'{sup.get(k["adjudicatarioNif"], {}).get("pme", "s/i")})' for k in ks))


Q('c04', 3, 3, 'Modificações',
  'Quais os contratos de uma entidade com mais modificações, qual o fundamento da última modificação de cada um e o '
  'adjudicatário é PME?',
  '[Município de Lisboa, NIF 500051070; 3 contratos]',
  [S(C, 'search', '--term', 'adjudicanteNif=500051070', '--sort', 'numModificacoes:desc', '--limit', '3',
     '--fields', 'idcontrato,numModificacoes,adjudicatarioNif,precoContratual'),
   S(M, 'search', '--terms', 'idcontrato={x}', '--sort', 'sequencia:desc', '--limit', '200',
     '--fields', 'idcontrato,sequencia,modifContratoFundamento,modifContratoData', x=J(1, 'idcontrato')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'Modificações e atributo PME estão em índices diferentes, ambos a partir do passo 1.',
  'Primeira linha por contrato na ordem `sequencia` decrescente = última modificação.',
  a_c04)


def a_c05(c):
    ms = rows(c, 1)
    ks = {d['idcontrato']: d for d in rows(c, 2)}
    ents = {d['nif']: d for d in rows(c, 3)}
    return (f'{tot(c, 1)} modificações não reduzidas a escrito por urgência imperiosa; maiores: ' +
            '; '.join(f'{eur(m["modifContratoPrecoAlterado"])} em {m["idcontrato"]} ({ents[ks[m["idcontrato"]]["adjudicanteNif"][0]]["nome"][0]})' for m in ms[:4]))


Q('c05', 3, 3, 'Modificações',
  'Que modificações não foram reduzidas a escrito com uma dada justificação, e de que contratos e entidades são?',
  '[justificação com a expressão «urgência imperiosa»; 10 maiores por preço alterado]',
  [S(M, 'search', '--phrase-in', 'modifJustNContratEscrito=urgência imperiosa', '--sort', 'modifContratoPrecoAlterado:desc', '--limit', '10',
     '--exact-total', '--fields', 'idcontrato,sequencia,modifContratoPrecoAlterado'),
   S(C, 'get', '{*k}', k=KEYS(1, 'idcontrato')),
   S(AD, 'get', '{*k}', k=FN('--key <adjudicanteNif dos contratos do passo 2> …', lambda c: [t for x in vals(c, 2, 'adjudicanteNif') for t in ('--key', x)]))],
  'Cadeia modificação → contrato → entidade, um índice por passo.',
  'Emparelhamento por ids.',
  a_c05)


def a_c06(c):
    ms = rows(c, 1)
    ks = {d['idcontrato']: d for d in rows(c, 2)}
    ents = {d['nif']: d for d in rows(c, 3)}
    types = {}
    for m in ms:
        types[m['modifContratoTipoAto']] = types.get(m['modifContratoTipoAto'], 0) + 1
    return (f'{tot(c, 1)} modificações por decisão judicial ou arbitral; nas 20 mais recentes: {types}; entidades: ' +
            ', '.join(sorted({ents[ks[m["idcontrato"]]["adjudicanteNif"][0]]["nome"][0] for m in ms})[:5]))


Q('c06', 3, 3, 'Modificações',
  'Que contratos foram modificados por decisão judicial ou arbitral, e por que entidades foram celebrados?',
  '[tipo de ato «Decisão Judicial» ou «Decisão Arbitral»; 20 mais recentes]',
  [S(M, 'search', '--terms', 'modifContratoTipoAto=Decisão Judicial,Decisão Arbitral', '--sort', 'modifContratoData:desc', '--limit', '20',
     '--exact-total', '--fields', 'idcontrato,sequencia,modifContratoTipoAto,modifContratoData'),
   S(C, 'get', '{*k}', k=KEYS(1, 'idcontrato')),
   S(AD, 'get', '{*k}', k=FN('--key <adjudicanteNif dos contratos do passo 2> …', lambda c: [t for x in vals(c, 2, 'adjudicanteNif') for t in ('--key', x)]))],
  'Cadeia modificação → contrato → entidade, um índice por passo.',
  'Contagem por tipo de ato; nomes distintos das entidades.',
  a_c06)


def a_c07(c):
    ms = rows(c, 1)
    ks = {d['idcontrato']: d for d in rows(c, 2)}
    out = []
    for m in ms[:4]:
        k = ks[m['idcontrato']]
        out.append(f'{m["idcontrato"]}: {m["modifPrazoExecucao"]} dias após modificação vs {k.get("prazoExecucao", "sem prazo")} no contrato ({k["adjudicanteNome"][0]})')
    return f'{tot(c, 1)} modificações com prazo ≥ 3650 dias; ' + '; '.join(out)


Q('c07', 2, 2, 'Modificações',
  'Em que contratos uma modificação fixou um prazo de execução muito longo, e qual era o prazo original?',
  '[prazo modificado ≥ 3650 dias (o marcador 99999999 não é indexado); 10 maiores]',
  [S(M, 'search', '--range', 'modifPrazoExecucao=3650..', '--sort', 'modifPrazoExecucao:desc', '--limit', '10', '--exact-total',
     '--fields', 'idcontrato,sequencia,modifPrazoExecucao'),
   S(C, 'get', '{*k}', k=KEYS(1, 'idcontrato'))],
  'O prazo original está em `contratos`.',
  'Comparação prazo modificado vs prazo original.',
  a_c07,
  notes=['`modifPrazoExecucao` e `prazoExecucao` não indexam o marcador `99999999`: o intervalo aberto não o apanha.'])


def a_c08(c):
    n = len(rows(c, 1))
    rev, allm = cnt(c, 2), cnt(c, 3)
    return (f'{tot(c, 1)} contratos de obras de estradas de 2022 com modificações; nos {n} listados, {allm} modificações, das quais '
            f'{rev} por revisão extraordinária de preços ({pct(rev, allm)})')


Q('c08', 3, 2, 'Modificações',
  'Nos contratos de uma categoria CPV de um ano que foram modificados, que parte das modificações teve por fundamento a '
  'revisão extraordinária de preços?',
  '[categoria CPV 45233 (obras de estradas); ano 2022; fundamento «Revisão de preços Extraordinária (D.L. n.º 36/2022, 20-05)»]',
  [S(C, 'search', '--term', 'cpvCategoria=45233', '--term', 'Ano=2022', '--range', 'numModificacoes=1..', '--limit', '500', '--exact-total',
     '--fields', 'idcontrato'),
   S(M, 'count', '--terms', 'idcontrato={x}', '--term', 'modifContratoFundamento=Revisão de preços Extraordinária (D.L. n.º 36/2022, 20-05)', x=J(1, 'idcontrato')),
   S(M, 'count', '--terms', 'idcontrato={x}', x=J(1, 'idcontrato'))],
  'O CPV está em `contratos`; as modificações em `modificacoes`; duas contagens com critérios diferentes.',
  'Rácio entre as duas contagens.',
  a_c08)


def a_c09(c):
    ks = rows(c, 1)
    ms = rows(c, 2)
    fund = {}
    for m in ms:
        fund[m['modifContratoFundamento']] = fund.get(m['modifContratoFundamento'], 0) + 1
    return (f'{len(ks)} contratos com 50+ modificações ({", ".join(k["idcontrato"] + " " + k["adjudicanteNome"] for k in ks[:3])}…); '
            f'fundamentos das modificações de ordem ≥ 50: ' + '; '.join(f'{a} ({b})' for a, b in sorted(fund.items(), key=lambda x: -x[1])[:3]))


Q('c09', 2, 2, 'Modificações',
  'Que contratos acumularam um número muito elevado de modificações, e com que fundamentos foram feitas as modificações '
  'a partir dessa ordem?',
  '[50 ou mais modificações; sequência ≥ 50]',
  [S(C, 'search', '--range', 'numModificacoes=50..', '--sort', 'numModificacoes:desc', '--limit', '20', '--fields', 'idcontrato,adjudicanteNome,numModificacoes'),
   S(M, 'search', '--terms', 'idcontrato={x}', '--range', 'sequencia=50..', '--limit', '500', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(1, 'idcontrato'))],
  'O contador está em `contratos`; os fundamentos em `modificacoes`.',
  'Contagem dos fundamentos nas linhas devolvidas.',
  a_c09)


def a_c10(c):
    ks = rows(c, 1)
    ms = rows(c, 2)
    ents = {d['nif']: d for d in rows(c, 3)}
    fund = {}
    for m in ms:
        fund[m['modifContratoFundamento']] = fund.get(m['modifContratoFundamento'], 0) + 1
    return (f'{tot(c, 1)} contratos com «covid» nas causas de alteração do preço; fundamentos das modificações dos 20 maiores: ' +
            '; '.join(f'{a} ({b})' for a, b in sorted(fund.items(), key=lambda x: -x[1])[:2]) +
            f'; entidades: {", ".join(sorted({e["nome"][0] for e in ents.values()})[:4])}')


Q('c10', 3, 3, 'Modificações',
  'Que contratos referem uma dada causa nas razões de alteração do preço, com que fundamentos legais foram modificados e '
  'por que entidades?',
  '[palavra «covid» em `CausasAlteraPreco`; 20 maiores]',
  [S(C, 'search', '--text-in', 'CausasAlteraPreco=covid', '--sort', 'precoContratual:desc', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,precoContratual'),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '300', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(1, 'idcontrato')),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'Modificações e entidades estão em índices diferentes, ambos a partir do passo 1.',
  'Contagem de fundamentos nas linhas do passo 2.',
  a_c10,
  notes=['`CausasAlteraPreco` não indexa os marcadores `n/a`, `N/A`, `.`, `-`.'])


def a_c11(c):
    ents = rows(c, 2)
    cps = rows(c, 3)
    return (f'{tot(c, 1)} sistemas de aquisição dinâmicos anunciados por {len(ents)} entidades (p.ex. {", ".join(e["nome"].split(SEP)[0] for e in ents[:3])}); '
            f'objetos mais frequentes no país: ' + '; '.join(f'{d["cpv"]} «{d["descricao"]}»' for d in cps[:3]))


Q('c11', 3, 3, 'Anúncios',
  'Que entidades anunciaram sistemas de aquisição dinâmicos e para que objetos (descrição CPV)?',
  '[sistema de aquisição dinâmico = sim; todos os anúncios (até 200)]',
  [S(A, 'search', '--term', 'sistemaAquisicaoDinamico=sim', '--sort', 'dataPublicacao:desc', '--limit', '200', '--exact-total',
     '--fields', 'IDAnuncio,nifEntidade,cpv'),
   S(AD, 'search', '--terms', 'nif={x}', '--sort', 'nome:asc', '--limit', '200', '--fields', 'nif,nome', x=J(1, 'nifEntidade')),
   S(K, 'search', '--terms', 'cpv={x}', '--sort', 'numContratos:desc', '--limit', '200', '--fields', 'cpv,descricao,numContratos', x=J(1, 'cpv'))],
  'Entidades e vocabulário CPV estão em índices diferentes, ambos a partir do passo 1.',
  'Nenhuma além da leitura das linhas.',
  a_c11)


def a_c12(c):
    an = rows(c, 1)
    ks = rows(c, 2)
    sup = rows(c, 3)
    zero = sum(1 for k in ks if float(k['precoContratual']) == 0)
    return (f'{tot(c, 1)} anúncios de acordo-quadro em 2025; dos 20 de maior preço base resultaram {tot(c, 2)} contratos, '
            f'{zero} deles com preço contratual 0 (acordo-quadro sem valor próprio); adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}')


Q('c12', 3, 3, 'Acordos-quadro',
  'Dos anúncios de acordos-quadro de um ano com maior preço base, que contratos resultaram e quantos adjudicatários são PME?',
  '[acordo-quadro = sim no anúncio; ano 2025; 20 maiores preços base]',
  [S(A, 'search', '--term', 'AcordoQuadro=sim', '--term', 'Ano=2025', '--sort', 'PrecoBase:desc', '--limit', '20', '--exact-total', '--fields', 'IDAnuncio,PrecoBase'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '100', '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Cadeia anúncio → contrato → fornecedor, um índice por passo.',
  'Contagem dos contratos com preço 0 e de PME.',
  a_c12)


def a_c13(c):
    an = rows(c, 1)
    ks = rows(c, 2)
    procs = {d['idprocedimento']: d for d in rows(c, 3)}
    return (f'{tot(c, 1)} anúncios com um lote ≥ 50 M€; {len(ks)} contratos resultantes dos 15 maiores; procedimentos: ' +
            '; '.join(f'{p["idprocedimento"]} ({p["tipoprocedimento"]}, {p["numContratos"]} contratos, {eur(p["valorTotal"])})' for p in list(procs.values())[:3]))


Q('c13', 4, 4, 'Anúncios',
  'Que anúncios tinham pelo menos um lote acima de um valor base, que contratos e procedimentos deles resultaram, e por que entidades?',
  '[valor base de lote ≥ 50 000 000 € (campo multivalor); 15 maiores preços base]',
  [S(A, 'search', '--range', 'loteValorBase=50000000..', '--sort', 'PrecoBase:desc', '--limit', '15', '--exact-total', '--fields', 'IDAnuncio,PrecoBase,loteValorBase'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '100', '--fields', 'idcontrato,IDAnuncio,idprocedimento,adjudicanteNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(P, 'get', '{*k}', k=KEYS(2, 'idprocedimento')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Cadeia anúncio → contrato → (procedimento, entidade); os passos 3 e 4 são índices diferentes.',
  'Nenhuma além da leitura das linhas.',
  plus(a_c13, lambda c: '; entidades: ' + ', '.join(sorted({d['nome'][0] for d in rows(c, 4)})[:4])))


def a_c14(c):
    ks = rows(c, 2)
    sup = rows(c, 3)
    return (f'{tot(c, 1)} anúncios com fator de sustentabilidade ambiental ou social; {len(ks)} contratos resultantes; '
            f'adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}; total {eur(sum(float(k["precoContratual"]) for k in ks))}')


Q('c14', 3, 3, 'Sustentabilidade',
  'Que contratos resultaram de anúncios com um fator de adjudicação de sustentabilidade, e os adjudicatários são PME?',
  '[fator de aquisição «Sustentabilidade ambiental ou social» no anúncio]',
  [S(A, 'search', '--term', 'criterioAquisicaoTipoFator=Sustentabilidade ambiental ou social', '--limit', '100', '--exact-total', '--fields', 'IDAnuncio'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '200', '--fields', 'idcontrato,adjudicatarioNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Cadeia anúncio → contrato → fornecedor, um índice por passo.',
  'Soma dos preços; contagem de PME.',
  a_c14)


def a_c15(c):
    an = rows(c, 1)
    ents = rows(c, 2)
    return (f'{tot(c, 1)} anúncios no DR n.º 75 de 2026; {len(ents)} entidades distintas: ' +
            ', '.join(f'{e["nome"].split(SEP)[0]} [{e["nutsDesc"].split(SEP)[0] or "sem NUTS"}]' for e in ents[:5]))


Q('c15', 2, 2, 'Anúncios',
  'Que entidades publicaram anúncios num dado número do Diário da República, e onde estão sediadas?',
  '[DR n.º 75 (numDR), ano 2026]',
  [S(A, 'search', '--term', 'numDR=75', '--term', 'Ano=2026', '--limit', '200', '--exact-total', '--fields', 'IDAnuncio,nifEntidade'),
   S(AD, 'search', '--terms', 'nif={x}', '--sort', 'nome:asc', '--limit', '200', '--fields', 'nif,nome,nutsDesc', x=J(1, 'nifEntidade'))],
  'A sede das entidades está em `adjudicantes`.',
  'Nenhuma além da leitura das linhas.',
  a_c15)


def a_c16(c):
    an = rows(c, 1)
    ks = rows(c, 2)
    sup = rows(c, 3)
    return (f'{tot(c, 1)} anúncios de 2025 sobre requalificação de escolas; {len(ks)} contratos resultantes ({eur(sum(float(k["precoContratual"]) for k in ks))}); '
            f'adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}')


Q('c16', 3, 3, 'Anúncios',
  'Que contratos resultaram dos anúncios de um ano sobre um tema (pesquisa livre), e os adjudicatários são PME?',
  '[palavras «requalificação escola» nos campos de texto do anúncio; ano 2025]',
  [S(A, 'search', '--text', 'requalificação escola', '--term', 'Ano=2025', '--sort', '_score', '--limit', '100', '--exact-total', '--fields', 'IDAnuncio,descricaoAnuncio'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '200', '--fields', 'idcontrato,adjudicatarioNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Cadeia anúncio → contrato → fornecedor, um índice por passo.',
  'Soma dos preços; contagem de PME.',
  a_c16)


def a_c17(c):
    ks = rows(c, 2)
    procs = rows(c, 3)
    ents = {d['nif']: d for d in rows(c, 4)}
    return (f'{tot(c, 1)} anúncios de concurso limitado por prévia qualificação em 2024; {len(ks)} contratos dos 30 maiores; '
            f'procedimentos: {", ".join(sorted({p["tipoprocedimento"] for p in procs}))}; entidades: {", ".join(sorted({e["nome"][0] for e in ents.values()})[:4])}')


Q('c17', 5, 5, 'Procedimento e fundamento',
  'Dos anúncios de um modelo de procedimento num ano, que contratos resultaram, com que tipo de procedimento ficou '
  'registado cada um, por que entidades e com que adjudicatários (PME?)?',
  '[modelo de anúncio «Concurso limitado por prévia qualificação»; ano 2024; 30 maiores preços base]',
  [S(A, 'search', '--term', 'modeloAnuncio=Concurso limitado por prévia qualificação', '--term', 'Ano=2024', '--sort', 'PrecoBase:desc',
     '--limit', '30', '--exact-total', '--fields', 'IDAnuncio,PrecoBase'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '200', '--fields', 'idcontrato,idprocedimento,adjudicanteNif,adjudicatarioNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(P, 'get', '{*k}', k=KEYS(2, 'idprocedimento')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Os passos 3 a 5 dependem dos contratos do passo 2 e são índices diferentes.',
  'Tipos de procedimento distintos, nomes distintos das entidades, contagem de PME.',
  plus(a_c17, lambda c: f'; adjudicatários PME {pme_of(c, 5)}'))


def a_c18(c):
    a = rows(c, 1)[0]
    ks = rows(c, 2)
    return (f'anúncio {a["IDAnuncio"]} ({a["nAnuncio"]}, {a["dataPublicacao"]}, {a["designacaoEntidade"]}); {len(ks)} contrato(s): ' +
            '; '.join(f'{k["idcontrato"]} {eur(k["precoContratual"])}' for k in ks))


Q('c18', 2, 2, 'Anúncios',
  'Que anúncio corresponde a um código INCM e que contratos o citam pelo mesmo código?',
  '[código INCM 419675763]',
  [S(A, 'search', '--term', 'IdIncm=419675763', '--fields', 'IDAnuncio,nAnuncio,dataPublicacao,designacaoEntidade'),
   S(C, 'search', '--term', 'idINCM=419675763', '--fields', 'idcontrato,IDAnuncio,precoContratual')],
  'Anúncio e contratos estão em índices diferentes; o código INCM é a chave comum.',
  'Confirma que o `IDAnuncio` dos contratos coincide com o do anúncio.',
  a_c18)


def a_c19(c):
    a = rows(c, 1)[0]
    ks = rows(c, 2)
    return (f'anúncio n.º 27585/2025 ({a["IDAnuncio"]}, prazo {a["PrazoPropostas"]} dias, preço base {eur(a["PrecoBase"]) if a["PrecoBase"] else "não indicado"}); '
            f'{len(ks)} contrato(s), total {eur(sum(float(k["precoContratual"]) for k in ks))}; publicação: {a["url"]}')


Q('c19', 2, 2, 'Anúncios',
  'Que prazo e preço base teve um anúncio identificado pelo seu número no Diário da República, onde está publicado, e '
  'que contratos resultaram?',
  '[anúncio n.º 27585/2025]',
  [S(A, 'search', '--term', 'nAnuncio=27585/2025', '--fields', 'IDAnuncio,PrazoPropostas,PrecoBase,url'),
   S(C, 'search', '--term', 'nAnuncio=27585/2025', '--fields', 'idcontrato,precoContratual')],
  'Anúncio e contratos estão em índices diferentes.',
  'Soma dos preços contratuais.',
  a_c19)


def a_c20(c):
    ks = rows(c, 1)
    ents = {d['nif']: d for d in rows(c, 2)}
    desc = {d['nuts']: d['descricao'] for d in rows(c, 3)}
    reg = {}
    for e in ents.values():
        for x in e.get('nutsCod') or []:
            reg[desc.get(x, x)] = reg.get(desc.get(x, x), 0) + 1
    return (f'{tot(c, 1)} contratos de 2025 ecológicos e com critérios materiais; sede das {len(ents)} entidades dos 20 maiores: ' +
            ', '.join(f'{a} ({b})' for a, b in sorted(reg.items(), key=lambda x: -x[1])[:4]))


Q('c20', 4, 4, 'Sustentabilidade',
  'Onde estão sediadas as entidades que celebraram os maiores contratos ecológicos com critérios materiais num ano, e os '
  'adjudicatários são PME?',
  '[contrato ecológico = sim e critérios materiais = sim; ano 2025; 20 maiores]',
  [S(C, 'search', '--term', 'ContratEcologico=sim', '--term', 'CritMateriais=sim', '--term', 'Ano=2025', '--sort', 'precoContratual:desc',
     '--limit', '20', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(N, 'get', '{*k}', k=FN('--key <nutsCod das entidades do passo 2> …', lambda c: [t for x in vals(c, 2, 'nutsCod') for t in ('--key', x)])),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'Cadeia contrato → entidade → região, mais o atributo PME em `fornecedores`; um índice por passo.',
  'Contagem de entidades por região e de PME.',
  plus(a_c20, lambda c: f'; adjudicatários PME {pme_of(c, 4)}'))


def a_c21(c):
    ks = rows(c, 1)
    sup = rows(c, 2)
    return (f'{tot(c, 1)} contratos com o fator «Sustentabilidade ambiental ou social»; {len(sup)} adjudicatários nos 30 maiores, '
            f'{sum(1 for d in sup if d.get("pme") == "sim")} PME; maior: {eur(ks[0]["precoContratual"])} ({ks[0]["adjudicanteNome"]})')


Q('c21', 2, 2, 'Sustentabilidade',
  'Quem ganhou os contratos adjudicados com um fator de sustentabilidade, e quantos desses adjudicatários são PME?',
  '[fator de adjudicação «Sustentabilidade ambiental ou social»; 30 maiores]',
  [S(C, 'search', '--term', 'criterioAdjudicacaoTipoFator=Sustentabilidade ambiental ou social', '--sort', 'precoContratual:desc', '--limit', '30',
     '--exact-total', '--fields', 'idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'O atributo PME só está em `fornecedores`.',
  'Contagem de PME.',
  a_c21)


def a_c22(c):
    n, all_, eco = tot(c, 1), cnt(c, 2), cnt(c, 3)
    return (f'{n} anúncios com critério ambiental em 2023 originaram {all_} contratos, dos quais {eco} marcados como ecológicos '
            f'({pct(eco, all_)})')


Q('c22', 3, 2, 'Sustentabilidade',
  'Dos contratos resultantes de anúncios com critério ambiental de um ano, quantos ficaram marcados como contrato ecológico?',
  '[critério ambiental = sim; ano 2023]',
  [S(A, 'search', '--term', 'CriterAmbient=sim', '--term', 'Ano=2023', '--limit', '3000', '--exact-total', '--fields', 'IDAnuncio'),
   S(C, 'count', '--terms', 'IDAnuncio={x}', x=J(1, 'IDAnuncio')),
   S(C, 'count', '--terms', 'IDAnuncio={x}', '--term', 'ContratEcologico=sim', x=J(1, 'IDAnuncio'))],
  'Os ids vêm de `anuncios`; duas contagens com critérios diferentes em `contratos`.',
  'Rácio ecológicos/total.',
  a_c22)


def a_c23(c):
    ks = rows(c, 1)
    ents = {d['nif']: d for d in rows(c, 2)}
    cps = rows(c, 3)
    return (f'{tot(c, 1)} contratos com o fator «Circularidade»; entidades: {", ".join(sorted({e["nome"][0] for e in ents.values()})[:4])}; '
            f'CPV: {"; ".join(d["descricao"] for d in cps[:4])}')


Q('c23', 3, 3, 'Sustentabilidade',
  'Que entidades adjudicaram contratos com um fator de circularidade, e o que compraram (descrição CPV)?',
  '[fator de adjudicação «Circularidade»]',
  [S(C, 'search', '--term', 'criterioAdjudicacaoTipoFator=Circularidade', '--sort', 'precoContratual:desc', '--limit', '50', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,cpvPrincipal,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpvPrincipal'))],
  'Entidades e CPV estão em índices diferentes, ambos a partir do passo 1.',
  'Nomes e descrições distintos.',
  a_c23)


def a_c24(c):
    sup = rows(c, 2)
    return (f'{tot(c, 1)} contratos de construção em 2025; nos 20 maiores, {len(sup)} adjudicatários distintos, '
            f'{sum(1 for d in sup if d.get("pme") == "sim")} PME')


Q('c24', 3, 3, 'PME',
  'Nos maiores contratos de uma divisão CPV num ano, quantos adjudicatários distintos são PME e que objetos (descrição CPV) têm?',
  '[divisão CPV 45; ano 2025; 20 maiores]',
  [S(C, 'search', '--term', 'cpvDivisao=45', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,adjudicatarioNif,cpvPrincipal,precoContratual'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpvPrincipal'))],
  'O atributo PME está em `fornecedores` e a descrição dos códigos em `cpv`, ambos a partir do passo 1.',
  'Contagem de PME entre os adjudicatários distintos.',
  plus(a_c24, lambda c: '; CPV: ' + '; '.join(d['descricao'] for d in rows(c, 3)[:4])))


def a_c25(c):
    sup = rows(c, 1)
    ks = rows(c, 2)
    return (f'{tot(c, 1)} fornecedores sem indicação de PME; os 20 de maior valor ganharam {tot(c, 2)} contratos em 2025; maiores: ' +
            '; '.join(f'{"+".join(nif(x) for x in _split(k["adjudicatarioNif"]))} ← {k["adjudicanteNome"]} {eur(k["precoContratual"])}' for k in ks[:3]))


Q('c25', 2, 2, 'PME',
  'Quantos contratos de um ano ganharam os fornecedores de maior valor sem indicação de PME, e com que entidades?',
  '[fornecedores com `pme` diferente de «sim» (sem indicação); 20 de maior `valorTotal`; ano 2025]',
  [S(F, 'search', '--not', '--term', 'pme=sim', '--sort', 'valorTotal:desc', '--limit', '20', '--exact-total', '--fields', 'nif,valorTotal'),
   S(C, 'search', '--terms', 'adjudicatarioNif={x}', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total',
     '--fields', 'idcontrato,adjudicatarioNif,adjudicanteNome,precoContratual', x=J(1, 'nif'))],
  'O atributo PME está em `fornecedores`; os contratos em `contratos`.',
  'Nenhuma além da leitura das linhas.',
  a_c25)


def a_c26(c):
    ks = rows(c, 1)
    ents = {d['nif']: d for d in rows(c, 2)}
    return (f'{tot(c, 1)} contratos de 2025 por procedimento centralizado; maiores entidades: ' +
            '; '.join(f'{ents[k["adjudicanteNif"].split(SEP)[0]]["nome"][0]} ({(ents[k["adjudicanteNif"].split(SEP)[0]].get("nutsDesc") or ["sem NUTS"])[0]}) {eur(k["precoContratual"])}' for k in ks[:4]))


Q('c26', 3, 3, 'Compras centralizadas e conjuntas',
  'Que entidades celebraram os maiores contratos resultantes de procedimentos centralizados num ano, onde estão sediadas, '
  'e os adjudicatários são PME?',
  '[procedimento centralizado = sim; ano 2025; 15 maiores]',
  [S(C, 'search', '--term', 'ProcedimentoCentralizado=sim', '--term', 'Ano=2025', '--sort', 'precoContratual:desc', '--limit', '15', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'A sede das entidades só está em `adjudicantes` e o atributo PME em `fornecedores`, ambos a partir do passo 1.',
  'Contagem de PME.',
  plus(a_c26, lambda c: f'; adjudicatários PME {pme_of(c, 3)}'))


def a_c27(c):
    all_, conj = cnt(c, 2), cnt(c, 3)
    return (f'{len(rows(c, 1))} empresas públicas; {all_} contratos em 2025, dos quais {conj} por compra conjunta ({pct(conj, all_)})')


Q('c27', 3, 2, 'Compras centralizadas e conjuntas',
  'Que proporção dos contratos de um ano celebrados pelas entidades de um dado tipo resultou de compra conjunta?',
  '[tipo de entidade «Empresa pública»; ano 2025]',
  [S(AD, 'search', '--term', 'tipoEntidade=Empresa pública', '--limit', '200', '--fields', 'nif'),
   S(C, 'count', '--terms', 'adjudicanteNif={x}', '--term', 'Ano=2025', x=J(1, 'nif')),
   S(C, 'count', '--terms', 'adjudicanteNif={x}', '--term', 'Ano=2025', '--term', 'compraConjunta=sim', x=J(1, 'nif'))],
  'O tipo de entidade só existe em `adjudicantes`; duas contagens com critérios diferentes.',
  'Rácio entre as duas contagens.',
  a_c27)


def a_c28(c):
    an = rows(c, 1)
    ents = rows(c, 2)
    ks = rows(c, 3)
    return (f'{tot(c, 1)} anúncios subscritos por ambas, juntando {len(ents)} entidades de {len({e["nutsCod"] for e in ents})} regiões; '
            f'{tot(c, 3)} contratos resultantes, total listado {eur(sum(float(k["precoContratual"]) for k in ks))}')


Q('c28', 3, 3, 'Compras centralizadas e conjuntas',
  'Que anúncios conjuntos subscreveram duas entidades, que outras entidades os subscreveram (e de que regiões), e que '
  'contratos resultaram?',
  '[Águas do Algarve, NIF 505176300, e AgdA – Águas Públicas do Alentejo, NIF 509133843; todos os anúncios conjuntos (até 100)]',
  [S(A, 'search', '--term', 'nifEntidade=505176300', '--term', 'nifEntidade=509133843', '--sort', 'dataPublicacao:desc', '--limit', '100',
     '--exact-total', '--fields', 'IDAnuncio,nifEntidade,dataPublicacao'),
   S(AD, 'search', '--terms', 'nif={x}', '--limit', '100', '--fields', 'nif,nome,nutsCod', x=J(1, 'nifEntidade')),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '200', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,precoContratual', x=J(1, 'IDAnuncio'))],
  'Entidades e contratos estão em índices diferentes, ambos a partir do passo 1.',
  'Contagem de entidades e regiões distintas; soma dos preços.',
  a_c28,
  notes=['Duas cláusulas `--term` no mesmo campo multivalor exigem ambos os valores no mesmo documento.'])


def a_c29(c):
    procs = rows(c, 1)
    ks = rows(c, 2)
    sup = {k['adjudicatarioNif'] for k in ks}
    return (f'procedimentos ao abrigo de acordo-quadro (art.º 258.º) com mais contratos: ' +
            ', '.join(f'{p["idprocedimento"]} ({p["numContratos"]})' for p in procs) +
            f'; {len(ks)} contratos, {len(sup)} adjudicatários distintos')


Q('c29', 3, 3, 'Acordos-quadro',
  'Nos procedimentos ao abrigo de acordo-quadro com mais contratos, quantos adjudicatários distintos houve e quantos são PME?',
  '[«Ao abrigo de acordo-quadro (art.º 258.º)»; 3 procedimentos com mais contratos]',
  [S(P, 'search', '--term', 'tipoprocedimento=Ao abrigo de acordo-quadro (art.º 258.º)', '--sort', 'numContratos:desc', '--limit', '3',
     '--fields', 'idprocedimento,numContratos,valorTotal'),
   S(C, 'search', '--terms', 'idprocedimento={x}', '--limit', '1000', '--fields', 'idcontrato,adjudicatarioNif', x=J(1, 'idprocedimento')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Agregado por procedimento (`procedimentos`), adjudicatários (`contratos`) e atributo PME (`fornecedores`) em cadeia.',
  'Contagem de NIF distintos e de PME.',
  plus(a_c29, lambda c: f'; adjudicatários PME {pme_of(c, 3)}'))


def a_c30(c):
    aq = vals(c, 1, 'numAcordoQuadro')
    procs = rows(c, 2)
    ents = rows(c, 3)
    return (f'{tot(c, 1)} contratos ao abrigo do acordo-quadro de viagens e alojamento; {len(aq)} números de acordo-quadro distintos nos 50 '
            f'mais recentes; {len(procs)} correspondem a um `idprocedimento` indexado ({", ".join(p["idprocedimento"] + " " + p["tipoprocedimento"] for p in procs[:2])}); '
            f'{len(ents)} entidades compradoras distintas')


Q('c30', 3, 3, 'Acordos-quadro',
  'Que contratos foram celebrados ao abrigo de um acordo-quadro descrito por uma expressão, que procedimento formou o '
  'acordo-quadro e que entidades o usaram?',
  '[expressão «Viagens e Alojamento» na descrição do acordo-quadro; 50 contratos mais recentes]',
  [S(C, 'search', '--phrase-in', 'DescrAcordoQuadro=Viagens e Alojamento', '--sort', 'dataPublicacao:desc', '--limit', '50', '--exact-total',
     '--fields', 'idcontrato,numAcordoQuadro,adjudicanteNif'),
   S(P, 'search', '--terms', 'idprocedimento={x}', '--fields', 'idprocedimento,tipoprocedimento,adjudicanteNif', x=J(1, 'numAcordoQuadro')),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'Procedimento formador e entidades estão em índices diferentes, ambos a partir do passo 1.',
  'Conta números de acordo-quadro distintos e quantos existem como `idprocedimento`.',
  a_c30,
  notes=['Ligação observada (não declarada no esquema): `numAcordoQuadro` contém, nestes contratos, o `idprocedimento` '
         'do procedimento que formou o acordo-quadro.'])

# ---- Batch 4 -----------------------------------------------------------------


def _first_collective(i, field):
    return FN(f'<primeiro {field} de pessoa coletiva do passo {i}>',
              lambda c: next(x for x in vals(c, i, field) if collective(x)))


def a_d01(c):
    v = next(r for r in rows(c, 1) if collective(r['nif']))
    inv, won = cnt(c, 2), cnt(c, 3)
    f = rows(c, 4)[0]
    return (f'{nif(v["nif"])}: {v["numConvites"]} convites (agregado) / {inv} contratos com convite; ganhou {won} desses ({pct(won, inv)}); '
            f'como fornecedor: {f["numContratos"]} contratos, {eur(f["valorTotal"])}, PME {f.get("pme", "s/i")}')


Q('d01', 4, 3, 'Concorrência',
  'Para a entidade mais convidada a apresentar proposta, em quantos contratos foi convidada, quantos desses ganhou e '
  'qual o seu historial como fornecedora?',
  '[a entidade (pessoa coletiva) com maior `numConvites`]',
  [S(V, 'search', '--sort', 'numConvites:desc', '--limit', '5', '--fields', 'nif,numConvites,primeiroAno,ultimoAno'),
   S(C, 'count', '--term', 'convidadaNif={x}', x=_first_collective(1, 'nif')),
   S(C, 'count', '--term', 'convidadaNif={x}', '--term', 'adjudicatarioNif={x}', x=_first_collective(1, 'nif')),
   S(F, 'get', '{*k}', k=FN('--key <NIF do passo 1>', lambda c: ['--key', next(x for x in vals(c, 1, 'nif') if collective(x))]))],
  'Duas contagens com critérios diferentes (um `count` dá um número) e o historial em `fornecedores`.',
  'Taxa de sucesso = ganhos/convidada; compara o agregado `numConvites` com a contagem.',
  a_d01)


def a_d02(c):
    inv = vals(c, 1, 'convidadaNif')
    vz = rows(c, 2)
    fz = rows(c, 3)
    return (f'{tot(c, 1)} consultas prévias do Município de Sintra em 2025; nas 30 mais recentes, {len(inv)} entidades convidadas; '
            f'mais convidadas no país: ' + ', '.join(f'{nif(r["nif"])} ({r["numConvites"]})' for r in vz[:3]) +
            f'; {len(fz)} já adjudicatárias, PME primeiro: ' + ', '.join(f'{nif(r["nif"])} PME {r["pme"] or "s/i"}' for r in fz[:3]))


Q('d02', 3, 3, 'Concorrência',
  'Que entidades convidou uma entidade adjudicante nas suas consultas prévias de um ano, quão convidadas são no país e '
  'quais já foram adjudicatárias (PME primeiro)?',
  '[Município de Sintra, NIF 500051062; Consulta Prévia; ano 2025; 30 mais recentes]',
  [S(C, 'search', '--term', 'adjudicanteNif=500051062', '--term', 'tipoprocedimento=Consulta Prévia', '--term', 'Ano=2025',
     '--sort', 'dataPublicacao:desc', '--limit', '30', '--exact-total', '--fields', 'idcontrato,convidadaNif'),
   S(V, 'search', '--terms', 'nif={x}', '--sort', 'numConvites:desc', '--limit', '200', '--fields', 'nif,numConvites', x=J(1, 'convidadaNif')),
   S(F, 'search', '--terms', 'nif={x}', '--sort', 'pme:desc,valorTotal:desc', '--limit', '200', '--fields', 'nif,pme,valorTotal', x=J(1, 'convidadaNif'))],
  'Convites (agregados) e historial de adjudicação estão em índices diferentes, ambos a partir do passo 1.',
  'Nenhuma além da leitura das linhas.',
  a_d02)


def a_d03(c):
    new = rows(c, 1)
    won = rows(c, 3)
    return (f'{tot(c, 1)} entidades convidadas pela primeira vez em 2025–2026 com 20+ convites; nas 50 maiores, {cnt(c, 2)} contratos de 2026 '
            f'com convite; {len(won)} já foram adjudicatárias (PME {sum(1 for r in won if r["pme"] == "sim")})')


Q('d03', 3, 3, 'Concorrência',
  'Quantas entidades começaram a ser convidadas recentemente com muitos convites, em quantos contratos do ano corrente '
  'foram convidadas, e quantas já ganharam contratos?',
  '[primeiro ano de convite 2025 ou 2026; 20 ou mais convites; ano dos contratos 2026; 50 com mais convites]',
  [S(V, 'search', '--terms', 'primeiroAno=2025,2026', '--range', 'numConvites=20..', '--sort', 'numConvites:desc', '--limit', '50', '--exact-total',
     '--fields', 'nif,numConvites'),
   S(C, 'count', '--terms', 'convidadaNif={x}', '--term', 'Ano=2026', x=J(1, 'nif')),
   S(F, 'search', '--terms', 'nif={x}', '--limit', '50', '--fields', 'nif,pme,numContratos', x=J(1, 'nif'))],
  'Convites em `convidadas`, contratos em `contratos`, adjudicação em `fornecedores`; os passos 2 e 3 dependem do passo 1.',
  'Contagem de PME nas linhas do passo 3.',
  a_d03)


def a_d04(c):
    allc, won = tot(c, 1), cnt(c, 2)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 3)}
    return (f'empresa 503040630 em {allc} contratos (adjudicatária ou concorrente); ganhou {won} ({pct(won, allc)}); '
            f'entidades dos 20 mais recentes: {", ".join(sorted(set(ents.values()))[:5])}')


Q('d04', 3, 2, 'Concorrência',
  'Em quantos contratos uma empresa foi adjudicatária ou concorrente, qual a sua taxa de sucesso e com que entidades '
  'concorreu recentemente?',
  '[empresa com NIF 503040630 (pessoa coletiva, frequente nos concursos do Município de Lisboa)]',
  [S(C, 'search', '--should', '--term', 'adjudicatarioNif=503040630', '--should', '--term', 'concorrenteNif=503040630', '--sort', 'dataPublicacao:desc',
     '--limit', '20', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,dataPublicacao'),
   S(C, 'count', '--term', 'adjudicatarioNif=503040630'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'A disjunção (`--should` sem cláusulas obrigatórias) e a contagem de vitórias são critérios diferentes; os nomes das '
  'entidades estão em `adjudicantes`.',
  'Taxa de sucesso = vitórias/participações.',
  a_d04,
  notes=['`--should` só filtra quando não há cláusulas obrigatórias; com um `--term` obrigatório passaria a mero reforço de relevância.'])


def a_d05(c):
    ks = rows(c, 1)
    ents = {d['nif']: d for d in rows(c, 2)}
    return (f'{tot(c, 1)} contratos com a expressão «manutenção de elevadores»; mais relevantes (reforço «preventiva»): ' +
            '; '.join(f'{k["idcontrato"]} {ents[k["adjudicanteNif"].split(SEP)[0]]["nome"][0]} '
                      f'[{(ents[k["adjudicanteNif"].split(SEP)[0]].get("nutsDesc") or ["sem NUTS"])[0]}]' for k in ks[:4]))


Q('d05', 2, 2, 'Objeto (CPV)',
  'Quais os contratos mais relevantes sobre um serviço (expressão em qualquer campo de texto), favorecendo os que '
  'mencionam um aspeto, e onde estão sediadas as entidades?',
  '[expressão «manutenção de elevadores»; reforço opcional «manutenção preventiva» no objeto; 10 mais relevantes]',
  [S(C, 'search', '--phrase', 'manutenção de elevadores', '--should', '--phrase-in', 'objectoContrato=manutenção preventiva',
     '--sort', '_score', '--limit', '10', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,objectoContrato'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif'))],
  'A sede das entidades só está em `adjudicantes`.',
  'Nenhuma além da leitura das linhas.',
  a_d05,
  notes=['Aqui `--should` não filtra (há uma cláusula obrigatória): só reordena por `_score`.'])


def a_d06(c):
    out = []
    for k, d in enumerate(rows(c, 1)):
        n = cnt(c, 2 + k)
        out.append(f'{d["cpv"]} «{d["descricao"]}»: {n}/{d["numContratos"]} = {pct(n, int(d["numContratos"]))}')
    return 'contratos por procedimento centralizado — ' + '; '.join(out)


Q('d06', 3, 2, 'Compras centralizadas e conjuntas',
  'Nos dois códigos CPV mais usados de um grupo, que proporção dos contratos resultou de procedimentos centralizados?',
  '[grupo CPV 909 (serviços de limpeza e saneamento); os 2 primeiros por `numContratos`]',
  [S(K, 'search', '--term', 'cpvGrupo=909', '--sort', 'numContratos:desc', '--limit', '2', '--fields', 'cpv,descricao,numContratos')] +
  [S(C, 'count', '--term', 'cpv={x}', '--term', 'ProcedimentoCentralizado=sim', x=ONE(1, 'cpv', k, label=f'<{k + 1}.º CPV do passo 1>')) for k in range(2)],
  'Uma contagem por código; o denominador é o agregado `numContratos` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.',
  'Rácio contagem/`numContratos`.',
  a_d06)


def a_d07(c):
    reg = rows(c, 1)
    return (f'códigos com essas descrições: {", ".join(r["nuts"] + " " + r["descricao"] for r in reg)}; {cnt(c, 2)} contratos de 2025 executados nelas')


Q('d07', 2, 2, 'Localização',
  'Que códigos NUTS (versões 2013 e 2024) têm uma dada designação, e quantos contratos de um ano foram executados nessas regiões?',
  '[designações exatas «Algarve» e «Douro»; ano 2025]',
  [S(N, 'search', '--terms', 'descricao=Algarve,Douro', '--sort', 'nuts:asc', '--fields', 'nuts,descricao,numContratos'),
   S(C, 'count', '--terms', 'nuts={x}', '--term', 'Ano=2025', x=J(1, 'nuts'))],
  'Os códigos de cada designação estão em `nuts`; a contagem em `contratos`.',
  'Nenhuma.',
  a_d07)


def a_d08(c):
    locs = rows(c, 1)
    ks = rows(c, 2)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 3)}
    return (f'«lagos» também apanha {", ".join(d["local"] for d in locs)} (mesma raiz); {tot(c, 2)} contratos nesses locais; maiores: ' +
            '; '.join(f'{ents[k["adjudicanteNif"].split(SEP)[0]]} {eur(k["precoContratual"])} ({k["localExecucao"]})' for k in ks[:3]))


def _lag(k):
    return ONE(1, 'local', k, label=f'<{k + 1}.º local do passo 1>')


Q('d08', 3, 3, 'Localização',
  'Que locais de execução encontra uma pesquisa por palavra num nome de concelho (com o efeito do stemmer), quantos '
  'contratos somam e quem celebrou os maiores?',
  '[palavra «lagos» no concelho (encontra Lagos e Lagoa); os 3 locais devolvidos]',
  [S(L, 'search', '--text-in', 'concelho=lagos', '--sort', 'local:asc', '--fields', 'local,numContratos'),
   S(C, 'search', '--should', '--term', 'localExecucao={a}', '--should', '--term', 'localExecucao={b}', '--should', '--term', 'localExecucao={c}',
     '--sort', 'precoContratual:desc', '--limit', '5', '--exact-total', '--fields', 'idcontrato,adjudicanteNif,localExecucao,precoContratual',
     a=_lag(0), b=_lag(1), c=_lag(2)),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Locais, contratos e nomes de entidade estão em três índices; os locais têm vírgulas e por isso são disjunção de '
  '`--should --term` (não `--terms`).',
  'Nenhuma além da leitura das linhas.',
  a_d08)


def a_d09(c):
    ks = rows(c, 1)
    ms = {m['idcontrato']: m for m in rows(c, 2)}
    return '; '.join(f'{k["idcontrato"]} ({k["numModificacoes"]} modif.): 1.ª em {ms[k["idcontrato"]]["modifContratoData"]}, '
                     f'«{ms[k["idcontrato"]]["modifContratoFundamento"]}», {ms[k["idcontrato"]]["modifContratoTipoAto"]}' for k in ks)


def _k1(k):
    return FN(f'<idcontrato n.º {k + 1} do passo 1>/1', lambda c: rows(c, 1)[k]['idcontrato'] + '/1')


Q('d09', 2, 2, 'Modificações',
  'Para os contratos de um ano com mais modificações, quando e com que fundamento foi feita a primeira modificação?',
  '[ano 2025; 3 contratos; chave composta `idcontrato/1`]',
  [S(C, 'search', '--term', 'Ano=2025', '--sort', 'numModificacoes:desc', '--limit', '3', '--fields', 'idcontrato,numModificacoes'),
   S(M, 'search', '--should', '--key', '{a}', '--should', '--key', '{b}', '--should', '--key', '{c}', '--fields', 'all', a=_k1(0), b=_k1(1), c=_k1(2))],
  'O contador está em `contratos`; a modificação n.º 1 em `modificacoes`, pedida pelas três chaves compostas num só pedido.',
  'Emparelha por `idcontrato`.',
  a_d09)


def a_d10(c):
    ks = rows(c, 1)
    ms = rows(c, 2)
    sup = rows(c, 3)
    fund = {}
    for m in ms:
        fund[m['modifContratoFundamento']] = fund.get(m['modifContratoFundamento'], 0) + 1
    return (f'{tot(c, 1)} contratos celebrados a 31/12/2024 e depois modificados; {len(ms)} modificações; fundamento mais frequente: '
            f'{max(fund.items(), key=lambda x: x[1])[0]}; adjudicatários PME {sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}')


Q('d10', 3, 3, 'Prazos e datas',
  'Dos contratos celebrados num dia concreto que foram depois modificados, com que fundamentos e por que adjudicatários (PME?)?',
  '[data de celebração 31/12/2024 (formato DD/MM/AAAA)]',
  [S(C, 'search', '--term', 'dataCelebracaoContrato=31/12/2024', '--range', 'numModificacoes=1..', '--limit', '50', '--exact-total',
     '--fields', 'idcontrato,adjudicatarioNif,numModificacoes'),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '300', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(1, 'idcontrato')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'Modificações e atributo PME estão em índices diferentes, ambos a partir do passo 1.',
  'Fundamento mais frequente; contagem de PME.',
  a_d10)


def a_d11(c):
    ks = rows(c, 2)
    return (f'{len(rows(c, 1))} entidades com nível 3 PT16E; {tot(c, 2)} contratos publicados a 29 e 30/12/2025; maiores: ' +
            '; '.join(f'{k["adjudicanteNome"]} {eur(k["precoContratual"])} ({k["dataPublicacao"]})' for k in ks[:3]))


Q('d11', 2, 2, 'Prazos e datas',
  'Quantos contratos publicaram as entidades de uma região nos últimos dias úteis de um ano, e quais os maiores?',
  '[entidades com NUTS de nível 3 PT16E (Região de Coimbra); publicação a 2025-12-29 ou 2025-12-30]',
  [S(AD, 'search', '--term', 'nutsNivel3=PT16E', '--limit', '500', '--fields', 'nif'),
   S(C, 'search', '--terms', 'adjudicanteNif={x}', '--terms', 'dataPublicacao=2025-12-29,2025-12-30', '--sort', 'precoContratual:desc', '--limit', '5',
     '--exact-total', '--fields', 'idcontrato,adjudicanteNome,precoContratual,dataPublicacao', x=J(1, 'nif'))],
  'A sede das entidades só está em `adjudicantes`.',
  'Nenhuma além da leitura das linhas.',
  a_d11)


def a_d12(c):
    procs = rows(c, 1)
    ks = rows(c, 2)
    sup = rows(c, 3)
    span = [days(p['primeiraPublicacao'], p['ultimaPublicacao']) for p in procs]
    return (f'{tot(c, 1)} procedimentos iniciados em 2022 com publicações até 2025 ou depois; nos 5 com mais contratos, '
            f'{max(span)} dias entre a primeira e a última publicação (máx.); {len(ks)} contratos; adjudicatários PME '
            f'{sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}')


Q('d12', 4, 4, 'Prazos e datas',
  'Que procedimentos iniciados num ano continuaram a publicar contratos anos depois, quantos contratos tiveram, com que '
  'adjudicatários e por que entidades?',
  '[primeira publicação em 2022; última publicação a partir de 2025-01-01; 5 com mais contratos]',
  [S(P, 'search', '--range', 'primeiraPublicacao=2022-01-01..2022-12-31', '--range', 'ultimaPublicacao=2025-01-01..', '--sort', 'numContratos:desc',
     '--limit', '5', '--exact-total', '--fields', 'idprocedimento,numContratos,primeiraPublicacao,ultimaPublicacao'),
   S(C, 'search', '--terms', 'idprocedimento={x}', '--limit', '1000', '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif', x=J(1, 'idprocedimento')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif')),
   S(AD, 'get', '{*k}', k=KEYS(2, 'adjudicanteNif'))],
  'Cadeia procedimento → contratos → (fornecedores, entidades); os passos 3 e 4 são índices diferentes.',
  'Diferença em dias entre as datas agregadas do procedimento; contagem de PME.',
  plus(a_d12, lambda c: '; entidades: ' + ', '.join(sorted({d['nome'][0] for d in rows(c, 4)})[:4])))


def a_d13(c):
    ms = rows(c, 1)
    ks = {k['idcontrato']: k for k in rows(c, 2)}
    lag = [days(m['modifContratoData'], m['modifDataPublicacao']) for m in ms]
    return (f'{tot(c, 1)} modificações publicadas em dezembro de 2025 com data anterior a 2024; atraso das 5 mais antigas: '
            f'{min(lag)} a {max(lag)} dias; entidades: {", ".join(sorted({ks[m["idcontrato"]]["adjudicanteNome"] for m in ms if m["idcontrato"] in ks}))}')


def _mk(k):
    return FN(f'<idcontrato n.º {k + 1} do passo 1>', lambda c: rows(c, 1)[k]['idcontrato'])


Q('d13', 2, 2, 'Prazos e datas',
  'Que modificações foram publicadas muito depois de feitas, com que atraso, e de que entidades são os contratos?',
  '[publicadas em 2025-12; data da modificação até 2023-12-31; 5 mais antigas]',
  [S(M, 'search', '--range', 'modifDataPublicacao=2025-12-01..2025-12-31', '--range', 'modifContratoData=..2023-12-31', '--sort', 'modifContratoData:asc',
     '--limit', '5', '--exact-total', '--fields', 'idcontrato,sequencia,modifContratoData,modifDataPublicacao'),
   S(C, 'search', '--should', '--key', '{a}', '--should', '--key', '{b}', '--should', '--key', '{c}', '--should', '--key', '{d}', '--should', '--key', '{e}',
     '--fields', 'idcontrato,adjudicanteNome,precoContratual', a=_mk(0), b=_mk(1), c=_mk(2), d=_mk(3), e=_mk(4))],
  'As datas da modificação estão em `modificacoes`; a entidade em `contratos` (cinco chaves numa disjunção, um pedido).',
  'Atraso = data de publicação − data da modificação.',
  a_d13)


def a_d14(c):
    ks = rows(c, 2)
    procs = rows(c, 3)
    return (f'{tot(c, 1)} anúncios com prazo-limite a 30/06/2025; {len(ks)} contratos resultantes ({eur(sum(float(k["precoContratual"]) for k in ks))}); '
            f'tipos de procedimento: {", ".join(sorted({p["tipoprocedimento"] for p in procs}))}')


Q('d14', 4, 4, 'Prazos e datas',
  'Que contratos e procedimentos resultaram dos anúncios cujo prazo para propostas terminava num dado dia, e os adjudicatários são PME?',
  '[data-limite de propostas 2025-06-30]',
  [S(A, 'search', '--term', 'DataLimitePropostas=2025-06-30', '--limit', '200', '--exact-total', '--fields', 'IDAnuncio'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '300', '--fields', 'idcontrato,idprocedimento,adjudicatarioNif,precoContratual', x=J(1, 'IDAnuncio')),
   S(P, 'get', '{*k}', k=KEYS(2, 'idprocedimento')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Cadeia anúncio → contrato → (procedimento, fornecedor); os passos 3 e 4 são índices diferentes.',
  'Soma dos preços; tipos distintos; contagem de PME.',
  plus(a_d14, lambda c: f'; adjudicatários PME {pme_of(c, 4)}'))


def a_d15(c):
    ks = rows(c, 1)
    sup = rows(c, 2)
    return (f'{tot(c, 1)} contratos da ULS de Coimbra fechados em 2025; nos 20 com fecho mais tardio, {len(sup)} adjudicatários, '
            f'{sum(1 for d in sup if d.get("pme") == "sim")} PME; último fecho {ks[0]["dataFechoContrato"]}')


Q('d15', 2, 2, 'Prazos e datas',
  'Que contratos de uma entidade foram concluídos num ano, e os adjudicatários são PME?',
  '[Unidade Local de Saúde de Coimbra, NIF 510103448; data de fecho em 2025; 20 mais tardios]',
  [S(C, 'search', '--term', 'adjudicanteNif=510103448', '--range', 'dataFechoContrato=2025-01-01..2025-12-31', '--sort', 'dataFechoContrato:desc',
     '--limit', '20', '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,dataFechoContrato'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'O atributo PME só está em `fornecedores`.',
  'Contagem de PME.',
  a_d15)


def a_d16(c):
    ks = rows(c, 1)
    ents = {d['nif']: d for d in rows(c, 2)}
    return (f'{tot(c, 1)} empreitadas de 2023–2025 com prazo superior a 5 anos; maiores prazos: ' +
            '; '.join(f'{k["prazoExecucao"]} dias, {ents[k["adjudicanteNif"].split(SEP)[0]]["nome"][0]}' for k in ks[:4]))


Q('d16', 3, 3, 'Prazos e datas',
  'Que entidades celebraram empreitadas com prazos de execução muito longos num período, quais os prazos, e os '
  'adjudicatários são PME?',
  '[empreitadas de obras públicas; prazo ≥ 1826 dias (o marcador 99999999 não é indexado); anos 2023 a 2025]',
  [S(C, 'search', '--term', EMP, '--range', 'prazoExecucao=1826..', '--range', 'Ano=2023..2025', '--sort', 'prazoExecucao:desc', '--limit', '10',
     '--exact-total', '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,prazoExecucao'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif'))],
  'Os nomes das entidades estão em `adjudicantes` e o atributo PME em `fornecedores`, ambos a partir do passo 1.',
  'Contagem de PME.',
  plus(a_d16, lambda c: f'; adjudicatários PME {pme_of(c, 3)}'),
  notes=['O intervalo aberto em `prazoExecucao` não apanha o marcador `99999999`, que não é indexado.'])


def a_d17(c):
    ks = rows(c, 1)
    sup = rows(c, 2)
    return (f'{tot(c, 1)} contratos resolvidos pelo Município de Lisboa publicados em 2021–2023; os 10 maiores envolvem {len(sup)} adjudicatário(s) '
            f'(PME {sum(1 for d in sup if d.get("pme") == "sim")}), que ganharam depois {cnt(c, 3)} contratos à mesma entidade desde 2024')


Q('d17', 3, 2, 'Quem fornece',
  'Que contratos resolveu uma entidade, quem eram os adjudicatários e continuaram a ganhar-lhe contratos depois?',
  '[Município de Lisboa, NIF 500051070; tipo de fim «Resolução do Contrato»; publicados em 2021–2023; 10 maiores; contratos desde 2024-01-01]',
  [S(C, 'search', '--term', 'adjudicanteNif=500051070', '--term', 'tipoFimContrato=Resolução do Contrato', '--range', 'dataPublicacao=2021-01-01..2023-12-31',
     '--sort', 'precoContratual:desc', '--limit', '10', '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,dataPublicacao,precoContratual'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(C, 'count', '--terms', 'adjudicatarioNif={x}', '--term', 'adjudicanteNif=500051070', '--range', 'dataPublicacao=2024-01-01..', x=J(1, 'adjudicatarioNif'))],
  'O atributo PME está em `fornecedores`; a contagem posterior é um critério diferente do passo 1.',
  'Contagem de PME.',
  a_d17, zero_ok='zero contratos posteriores é uma resposta válida («não voltaram a ganhar»).')


def a_d18(c):
    ks = rows(c, 1)
    ents = {d['nif']: d['nome'][0] for d in rows(c, 2)}
    sup = rows(c, 3)
    return (f'{tot(c, 1)} contratos com ocorrências; entidades: {", ".join(sorted(set(ents.values())))}; adjudicatários PME '
            f'{sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}; total {eur(sum(float(k["precoContratual"]) for k in ks))}')


Q('d18', 4, 4, 'Quem compra',
  'Que contratos registam ocorrências, que entidades os celebraram, quem foram os adjudicatários e foram modificados?',
  '[ocorrências = sim]',
  [S(C, 'search', '--term', 'ocorrencias=sim', '--sort', 'precoContratual:desc', '--limit', '20', '--exact-total',
     '--fields', 'idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual'),
   S(AD, 'get', '{*k}', k=KEYS(1, 'adjudicanteNif')),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '100', '--fields', 'idcontrato,sequencia,modifContratoFundamento', x=J(1, 'idcontrato'))],
  'Entidades, fornecedores e modificações estão em três índices, todos a partir do passo 1.',
  'Soma dos preços; contagem de PME e de modificações.',
  plus(a_d18, lambda c: f'; {tot(c, 4)} modificação(ões)'))


def a_d19(c):
    an = byfield(c, 1, 'IDAnuncio')
    ks = rows(c, 2)
    over = [k for k in ks if k['IDAnuncio'] in an and an[k['IDAnuncio']]['PrecoBase']
            and float(k['precoContratual']) > float(an[k['IDAnuncio']]['PrecoBase'])]
    ratios = [float(k['precoContratual']) / float(an[k['IDAnuncio']]['PrecoBase']) for k in ks
              if k['IDAnuncio'] in an and an[k['IDAnuncio']]['PrecoBase'] and float(an[k['IDAnuncio']]['PrecoBase']) > 0]
    return (f'{tot(c, 1)} anúncios do Município do Porto em 2024 com preço base; {len(ks)} contratos ligados; mediana contratual/base '
            f'{statistics.median(ratios):.2f}; {len(over)} acima do preço base anunciado')


Q('d19', 2, 2, 'Valores',
  'Nos anúncios de uma entidade num ano, como se compara o preço contratual dos contratos resultantes com o preço base '
  'anunciado, e quantos o excederam?',
  '[Município do Porto, NIF 501306099; ano 2024; preço base preenchido]',
  [S(A, 'search', '--term', 'nifEntidade=501306099', '--term', 'Ano=2024', '--range', 'PrecoBase=..', '--limit', '200', '--exact-total',
     '--fields', 'IDAnuncio,PrecoBase'),
   S(C, 'search', '--terms', 'IDAnuncio={x}', '--limit', '300', '--fields', 'idcontrato,IDAnuncio,precoContratual', x=J(1, 'IDAnuncio'))],
  'O preço base anunciado está em `anuncios`; o contratual em `contratos`.',
  'Rácio contratual/base por contrato (mediana) e contagem dos que excedem.',
  a_d19,
  notes=['`--range PrecoBase=..` exclui o marcador `0` (ausente), que não é indexado.'])


def a_d20(c):
    ks = rows(c, 1)
    sup = rows(c, 2)
    cps = rows(c, 3)
    locs = rows(c, 4)
    return (f'{tot(c, 1)} contratos de restauração executados na Região de Coimbra em 2024–2025; adjudicatários PME '
            f'{sum(1 for d in sup if d.get("pme") == "sim")}/{len(sup)}; CPV: {"; ".join(d["descricao"] for d in cps[:3])}; '
            f'locais: {", ".join(d["local"] + " (" + str(d["numContratos"]) + ")" for d in locs[:3])}')


Q('d20', 5, 5, 'Objeto (CPV)',
  'Que contratos de um grupo CPV foram executados numa NUTS III em dois anos, por que adjudicatários (PME?), com que '
  'códigos CPV, em que concelhos e com quantas modificações?',
  '[grupo CPV 553 (serviços de restauração); NUTS III de execução PT16E; anos 2024 e 2025; ordenados por local]',
  [S(C, 'search', '--term', 'cpvGrupo=553', '--term', 'nutsNivel3=PT16E', '--terms', 'Ano=2024,2025', '--sort', 'localExecucao:asc,precoContratual:desc',
     '--limit', '50', '--exact-total', '--fields', 'idcontrato,adjudicatarioNif,cpv,localExecucao,precoContratual'),
   S(F, 'get', '{*k}', k=KEYS(1, 'adjudicatarioNif')),
   S(K, 'get', '{*k}', k=KEYS(1, 'cpv')),
   S(L, 'get', '{*k}', k=KEYS(1, 'localExecucao')),
   S(M, 'search', '--terms', 'idcontrato={x}', '--limit', '200', '--fields', 'idcontrato,sequencia', x=J(1, 'idcontrato'))],
  'Os passos 2 a 5 dependem do passo 1 e consultam quatro índices diferentes.',
  'Contagem de PME e de modificações.',
  plus(a_d20, lambda c: f'; {tot(c, 5)} modificação(ões)'))


def a_d21(c):
    procs = rows(c, 1)
    ks = rows(c, 2)
    sup = {x for k in ks for x in _split(k['adjudicatarioNif'])}
    return (f'{tot(c, 1)} procedimentos de vigilância com preço base ≥ 1 M€; nos 10 maiores, {len(ks)} contratos e {len(sup)} adjudicatários distintos; '
            f'maior: {procs[0]["idprocedimento"]} base {eur(procs[0]["precoBaseProcedimento"])} vs adjudicado {eur(procs[0]["valorTotal"])}')


Q('d21', 3, 3, 'Valores',
  'Nos procedimentos sobre um serviço com preço base acima de um valor, quanto foi adjudicado face ao preço base, a '
  'quantos adjudicatários, e quantos são PME?',
  '[palavra «vigilância» no objeto do procedimento; preço base ≥ 1 000 000 €; 10 maiores preços base]',
  [S(P, 'search', '--text-in', 'objectoContrato=vigilância', '--range', 'precoBaseProcedimento=1000000..', '--sort', 'precoBaseProcedimento:desc',
     '--limit', '10', '--exact-total', '--fields', 'idprocedimento,precoBaseProcedimento,valorTotal,numContratos'),
   S(C, 'search', '--terms', 'idprocedimento={x}', '--limit', '500', '--fields', 'idcontrato,adjudicatarioNif', x=J(1, 'idprocedimento')),
   S(F, 'get', '{*k}', k=KEYS(2, 'adjudicatarioNif'))],
  'Preço base e valor agregado (`procedimentos`), adjudicatários (`contratos`) e atributo PME (`fornecedores`) em cadeia.',
  'Contagem de NIF distintos e de PME; comparação base vs adjudicado.',
  plus(a_d21, lambda c: f'; adjudicatários PME {pme_of(c, 3)}'))


def a_d22(c):
    cps = rows(c, 1)
    ks = rows(c, 2)
    return (f'{len(cps)} códigos com «serviços de limpeza»; {tot(c, 2)} contratos de limpeza com preço exatamente 75 000 € '
            f'(limiar da consulta prévia); primeiros por código: ' + '; '.join(f'{k["cpvPrincipal"]} {k["adjudicanteNome"]} ({k["tipoprocedimento"]})' for k in ks[:3]))


Q('d22', 2, 2, 'Valores',
  'Quantos contratos de um tipo de serviço têm preço exatamente igual a um limiar legal, e por que procedimento foram feitos?',
  '[expressão «serviços de limpeza» na descrição CPV; preço 75 000 € (limiar da consulta prévia para bens e serviços)]',
  [S(K, 'search', '--phrase-in', 'descricao=serviços de limpeza', '--limit', '50', '--fields', 'cpv,descricao'),
   S(C, 'search', '--terms', 'cpvPrincipal={x}', '--term', 'precoContratual=75000', '--sort', 'cpvPrincipal:asc,dataPublicacao:desc', '--limit', '10',
     '--exact-total', '--fields', 'idcontrato,cpvPrincipal,adjudicanteNome,tipoprocedimento', x=J(1, 'cpv'))],
  'O vocabulário CPV está em `cpv`; os contratos em `contratos`.',
  'Nenhuma além da leitura das linhas.',
  a_d22)

# @@QUESTIONS@@

# @@END-QUESTIONS@@

# --------------------------------------------------------------------------
# Runner
# --------------------------------------------------------------------------


def check_read_only(argv):
    """Refuses any argv that is not a read-only `rucene-base` request."""
    if argv[2] not in READ_ONLY or WRITE_FLAGS.intersection(argv[3:]):
        raise PlanError(f'refused non read-only request: {argv[1]} {argv[2]}')


def shown(argv):
    parts = ['rucene-base', argv[1], argv[2], '$R'] + argv[4:]
    return mask(' '.join(a if re.fullmatch(r'[\w.,:=/@%+-]+', a) else shlex.quote(a)
                         for a in parts))


def run_question(q, binary, root):
    ctx, steps_out, ok, err = [], [], True, ''
    for s in q['steps']:
        try:
            argv = s.argv(ctx, binary, root)
            check_read_only(argv)
        except PlanError as e:
            ok, err = False, f'passo {len(ctx) + 1}: {e}'
            break
        except Exception as e:  # noqa: BLE001 - recorded as a failure
            ok, err = False, f'passo {len(ctx) + 1}: erro ao preencher: {e!r}'
            break
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=900)
        res = Res(s, argv, proc)
        ctx.append(res)
        steps_out.append(dict(cmd=shown(argv), exit=res.exit, hits=res.total,
                              exact=res.exact, stderr=mask(res.stderr[:400])))
        if res.exit != 0:
            ok, err = False, f'passo {len(ctx)}: exit {res.exit}: {mask(res.stderr[:300])}'
            break
    answer = ''
    if ok:
        try:
            answer = q['ans'](ctx) or ''
        except Exception as e:  # noqa: BLE001 - recorded as a failure
            ok, err = False, f'erro na combinação: {e!r}'
        if ok and not answer:
            ok, err = False, 'resposta vazia'
    return dict(id=q['id'], ok=ok, err=err, steps=steps_out, answer=mask(answer))


# --------------------------------------------------------------------------
# Schema (from crates/rucene-cli-base/src/entities/*.rs) and coverage
# --------------------------------------------------------------------------

FIELD_RE = re.compile(r'F::new\(\s*"(\w+)",\s*(\w+)\s*\)((?:\s*\.\w+\((?:[^()]|\([^()]*\))*\))*)'
                      r'|F::derived\(\s*"(\w+)"\s*\)')


def _strings(text):
    return re.findall(r'"([^"]*)"', text)


def load_schemas(src=CRATE / 'src'):
    """Field kinds, flags and default text fields of every index, read from the
    entity schema declarations (the same source `rucene-base <entity> help`
    prints)."""
    consts = {m.group(1): _strings(m.group(2)) for m in
              re.finditer(r'pub const (\w+): &\[&str\] = &\[([^\]]*)\];',
                          (src / 'entities.rs').read_text(encoding='utf-8'))}
    schemas = {}
    for e in ENTITIES:
        text = (src / 'entities' / f'{e}.rs').read_text(encoding='utf-8')
        body = text[text.index('fields: &['):text.index('key: &[')]
        keys = _strings(re.search(r'key: &\[([^\]]*)\]', text).group(1))
        defaults = _strings(re.search(r'text_fields: &\[([^\]]*)\]', text).group(1))
        fields = {}
        for m in FIELD_RE.finditer(body):
            if m.group(4):
                fields[m.group(4)] = dict(kind='keyword', derived=True, key=False, multi=False, absent='')
                continue
            name, kind, chain = m.group(1), m.group(2).lower(), m.group(3) or ''
            absent = ''
            am = re.search(r'\.absent\(([^()]*)\)', chain)
            if am:
                arg = am.group(1).strip()
                absent = ', '.join(consts[arg] if arg in consts else _strings(arg))
            fields[name] = dict(kind=kind, derived=False, key=name in keys,
                                multi='.multi()' in chain, absent=absent)
        schemas[e] = dict(fields=fields, defaults=defaults)
    return schemas


OPS = ['term', 'terms', 'range', 'text', 'text-in', 'phrase', 'phrase-in', 'key', 'get',
       'sort', 'fields']
SUPPORTED = {
    'exact': {'term', 'terms', 'key', 'get'},
    'keyword': {'term', 'terms', 'sort', 'key', 'get'},
    'flag': {'term', 'terms', 'sort'},
    'name': {'text', 'text-in', 'phrase', 'phrase-in', 'term', 'terms', 'sort'},
    'text': {'text', 'text-in', 'phrase', 'phrase-in'},
    'int': {'term', 'terms', 'range', 'sort', 'key', 'get'},
    'double': {'term', 'terms', 'range', 'sort'},
    'date': {'term', 'terms', 'range', 'sort'},
    'stored': {'fields'},
}
for _k in SUPPORTED:
    SUPPORTED[_k].add('fields')
KINDS = list(SUPPORTED)


def step_ops(step, schemas):
    """(field, kind, op, modifier) tuples of one step; field '' for global
    options."""
    sch = schemas[step.entity]
    out = []
    keyfields = [f for f, d in sch['fields'].items() if d['key']]
    if step.sub == 'count':
        out.append(('', '', 'count', ''))
    if step.sub == 'get':
        for f in keyfields:
            out.append((f, sch['fields'][f]['kind'], 'get', 'composta' if len(keyfields) > 1 else ''))
        return out
    args, i, mod = step.args, 0, ''
    while i < len(args):
        a = args[i]
        if a in ('--should', '--not', '--must'):
            mod = a[2:]
            i += 1
            continue
        if a in ('--text', '--phrase'):
            for f in sch['defaults']:
                out.append((f, sch['fields'][f]['kind'], a[2:], mod))
            out.append(('*', '', a[2:] + ' (campos por omissão)', mod))
            mod, i = '', i + 2
            continue
        if a in ('--text-in', '--phrase-in', '--term', '--terms', '--range'):
            f = args[i + 1].split('=', 1)[0]
            out.append((f, sch['fields'][f]['kind'], a[2:], mod))
            mod, i = '', i + 2
            continue
        if a == '--key':
            for f in keyfields:
                out.append((f, sch['fields'][f]['kind'], 'key', mod))
            mod, i = '', i + 2
            continue
        if a == '--sort':
            for part in args[i + 1].split(','):
                f = part.split(':')[0]
                if f in ('_score', '_doc'):
                    out.append(('', '', 'sort ' + f, ''))
                else:
                    out.append((f, sch['fields'][f]['kind'], 'sort', ''))
            i += 2
            continue
        if a == '--fields':
            out.append(('', '', 'fields all' if args[i + 1] == 'all' else 'fields', ''))
            names = ([f for f, d in sch['fields'].items() if not d['derived']] if args[i + 1] == 'all'
                     else args[i + 1].split(','))
            for f in names:
                out.append((f, sch['fields'][f]['kind'], 'fields', ''))
            i += 2
            continue
        if a == '--exact-total':
            out.append(('', '', 'exact-total', ''))
            i += 1
            continue
        if a == '--limit':
            i += 2
            continue
        raise ValueError(f'argumento desconhecido {a!r} em {step.display()}')
    return out


def describe_ops(q, schemas):
    """`índice.campo` (tipo[, multivalor][, nível derivado][, chave][, ausente=…]): operações;
    os campos só impressos resumem-se numa linha de saída por índice."""
    parts, outputs, globals_ = {}, {}, []
    for s in q['steps']:
        sch = schemas[s.entity]['fields']
        for f, kind, op, mod in step_ops(s, schemas):
            if f == '*':
                continue
            if f == '':
                if op != 'fields' and f'{s.entity}: {op}' not in globals_:
                    globals_.append(f'{s.entity}: {op}')
                continue
            if op == 'fields':
                lst = outputs.setdefault(s.entity, [])
                tag = f'{f} ({kind})'
                if tag not in lst:
                    lst.append(tag)
                continue
            d = sch[f]
            tags = [kind]
            if d['multi']:
                tags.append('multivalor')
            if d['derived']:
                tags.append('nível derivado')
            if d['key']:
                tags.append('chave')
            if d['absent']:
                tags.append('ausente=' + d['absent'].replace('`', ''))
            key = f'`{s.entity}.{f}` ({", ".join(tags)})'
            opname = op if not mod else f'--{mod} {op}'
            if mod == 'composta':
                opname = 'get (chave composta)'
            parts.setdefault(key, [])
            if opname not in parts[key]:
                parts[key].append(opname)
    lines = [f'{k}: {", ".join(v)}' for k, v in parts.items()]
    for e, lst in outputs.items():
        lines.append(f'saída `--fields` de `{e}`: ' + ', '.join(lst))
    if globals_:
        lines.append('opções: ' + '; '.join(globals_))
    return lines


def coverage(qs, schemas):
    idx_q, idx_r = {e: 0 for e in ENTITIES}, {e: 0 for e in ENTITIES}
    kind_op = {k: {} for k in KINDS}
    extra = {}
    derived, multi, absent = {}, {}, {}
    for q in qs:
        seen_idx, seen_ko, seen_x, seen_d, seen_m, seen_a = set(), set(), set(), set(), set(), set()
        for s in q['steps']:
            idx_r[s.entity] += 1
            seen_idx.add(s.entity)
            sch = schemas[s.entity]['fields']
            for f, kind, op, mod in step_ops(s, schemas):
                if f == '':
                    seen_x.add(op)
                    continue
                if f == '*':
                    seen_x.add(op if not mod else f'--{mod} {op}')
                    continue
                base = op if op in OPS else op
                seen_ko.add((kind, base))
                if mod in ('should', 'not'):
                    seen_x.add(f'--{mod} ({kind} {base})')
                if mod == 'composta':
                    seen_x.add('get com chave composta (modificacoes)')
                d = sch[f]
                if d['derived']:
                    seen_d.add(f'{s.entity}.{f}')
                if d['multi']:
                    seen_m.add(f'{s.entity}.{f}')
                if d['absent'] and (base == 'range' or kind == 'text'):
                    seen_a.add(f'{s.entity}.{f}')
        for e in seen_idx:
            idx_q[e] += 1
        for k, o in seen_ko:
            kind_op[k][o] = kind_op[k].get(o, 0) + 1
        for x in seen_x:
            extra[x] = extra.get(x, 0) + 1
        for x in seen_d:
            derived[x] = derived.get(x, 0) + 1
        for x in seen_m:
            multi[x] = multi.get(x, 0) + 1
        for x in seen_a:
            absent[x] = absent.get(x, 0) + 1
    return idx_q, idx_r, kind_op, extra, derived, multi, absent


# --------------------------------------------------------------------------
# Deliverable
# --------------------------------------------------------------------------

MANUAL_GAPS = [
    'flag × terms: as flags valem `sim`/`nao` e estão preenchidas em todos os documentos de `contratos` e `anuncios` '
    '(em `fornecedores.pme` só existe `sim` ou ausência); `--terms sim,nao` equivale a «todos» ou a `--term`, sem pergunta '
    'pertinente.',
    'sort `_doc`: ordem interna dos documentos no índice, sem significado para nenhuma pergunta do domínio.',
]

UNCOVERED_REASON = {
    ('exact', 'key'): '`--key` só se aplica ao campo-chave; coberto onde a chave é exata.',
    ('keyword', 'get'): 'chave keyword (cpv, locais, nuts): ver `get` nessas entidades.',
    ('keyword', 'key'): 'chave keyword (cpv, locais, nuts): ver `--key` nessas entidades.',
    ('int', 'key'): 'só `modificacoes.sequencia` é parte de chave; coberto pelo `get` composto.',
    ('int', 'get'): 'só `modificacoes.sequencia` é parte de chave; coberto pelo `get` composto.',
}


def ordered(qs):
    """Publication order: R descending, then I descending, then declaration order."""
    return sorted(qs, key=lambda q: (-q['r'], -q['i'], QUESTIONS.index(q)))


def write_md(qs, results, schemas, cmdline, when):
    res = {r['id']: r for r in results}
    passed = sum(1 for r in results if r['ok'])
    order = ordered(qs)
    rdist = {}
    for q in order:
        rdist[q['r']] = rdist.get(q['r'], 0) + 1
    L_ = []
    w = L_.append
    w('# Portal BASE: 100 perguntas multi-índice para o `rucene-base`')
    w('')
    w('Perguntas sobre transparência da contratação pública que o conjunto de dados Portal BASE '
      'responde através dos dez índices do CLI `rucene-base` (crate `crates/rucene-cli-base`). '
      'Cada pergunta precisa de dados de pelo menos dois índices diferentes.')
    w('')
    w('## Métrica de complexidade')
    w('')
    w('* **R** — número de pedidos ao CLI no plano mínimo: cada invocação '
      '`rucene-base <entidade> search|count|get` é um pedido contra um índice.')
    w('* **I** — número de índices distintos usados pelo plano.')
    w('* **Regra de agregação em lote:** uma expansão sobre uma lista de resultados conta como '
      '**um** pedido quando o CLI a aceita num só comando (`--terms CAMPO=V1,V2,...`, '
      '`get --key K1 --key K2 ...`); um `count` por valor é um pedido por valor. Os planos '
      'são limitados (os N primeiros ou uma lista curta fixa) e não têm pedidos supérfluos: cada secção diz '
      'porque os passos não se fundem.')
    w('* Ordenação: R decrescente; empate por I decrescente.')
    w('* `$R` = `crates/rucene-cli-base/data/base/index` (raiz por omissão, com os dez índices, 2,1 GB); '
      '`rucene-base` = `target/release/rucene-base`. Só comandos de leitura (`search`, `count`, `get`).')
    w(f'* Planos executáveis: `{SCRIPT}` (corre todos os planos a partir da raiz do repositório).')
    w('* Combinação no cliente: apenas aritmética pequena sobre as linhas devolvidas '
      '(diferenças, rácios, comparações, interseções de listas curtas), declarada em cada pergunta.')
    w('* Dados pessoais: nenhum nome nem NIF de pessoa singular aparece neste documento '
      '(NIF começado por 1–4, 7 ou 8 é mascarado; nomes de fornecedores e convidados não são pedidos).')
    w('')
    w('## Validação')
    w('')
    w(f'* Comando: `{cmdline}`')
    w(f'* Data: {when}')
    w(f'* Resultado: **{passed}/{len(qs)}** perguntas validadas (todos os passos com exit 0 e '
      'resposta não vazia, ou zero declarado significativo).')
    w('* Distribuição de R: ' + ', '.join(f'R={r}: {n}' for r, n in sorted(rdist.items(), reverse=True)))
    w('')
    w('## Resumo')
    w('')
    w('| # | Id | Pergunta | R | I | Índices | Categoria |')
    w('| ---: | --- | --- | ---: | ---: | --- | --- |')
    for n, q in enumerate(order, 1):
        idx = []
        for s in q['steps']:
            if s.entity not in idx:
                idx.append(s.entity)
        w(f'| {n} | `{q["id"]}` | {q["q"]} | {q["r"]} | {q["i"]} | {", ".join(idx)} | {q["cat"]} |')
    w('')
    w('## Perguntas')
    for n, q in enumerate(order, 1):
        r = res.get(q['id'], dict(ok=False, steps=[], answer='', err='não executada'))
        idx = []
        for s in q['steps']:
            if s.entity not in idx:
                idx.append(s.entity)
        w('')
        w(f'### {n}. {q["q"]}')
        w('')
        w(f'**Instanciação:** {q["inst"]}  ')
        w(f'**Id:** `{q["id"]}` · **R = {q["r"]}, I = {q["i"]}** ({", ".join(idx)}) · {q["cat"]}')
        w('')
        w('**Plano:**')
        w('')
        for k, s in enumerate(q['steps'], 1):
            st = r['steps'][k - 1] if k - 1 < len(r['steps']) else None
            tail = f' — exit {st["exit"]}, {st["hits"]} resultado(s)' if st else ''
            w(f'{k}. `{s.display()}`{tail}')
        w('')
        w(f'**Porque R não é menor:** {q["why"]}')
        w('')
        w(f'**Combinação no cliente:** {q["comb"]}')
        w('')
        w('**Tipos de campo e operações:**')
        w('')
        for line in describe_ops(q, schemas):
            w(f'* {line}')
        for note in q['notes']:
            w(f'* {note}')
        w('')
        status = 'validada' if r['ok'] else 'FALHOU: ' + r.get('err', '')
        w(f'**Resposta ({status}):** {r["answer"]}')
        if q['zero_ok']:
            w('')
            w(f'*Zero significativo:* {q["zero_ok"]}')
    idx_q, idx_r, kind_op, extra, derived, multi, absent = coverage(qs, schemas)
    w('')
    w('## Matriz de cobertura')
    w('')
    w('### Índice × perguntas')
    w('')
    w('| Índice | Perguntas | Pedidos |')
    w('| --- | ---: | ---: |')
    for e in ENTITIES:
        w(f'| `{e}` | {idx_q[e]} | {idx_r[e]} |')
    w('')
    w('### Tipo de campo × operação (número de perguntas)')
    w('')
    w('Células vazias: operação que o tipo não suporta. `0`: suportada e não usada.')
    w('')
    w('| Tipo | ' + ' | '.join(OPS) + ' |')
    w('| --- |' + ' ---: |' * len(OPS))
    gaps = []
    for k in KINDS:
        cells = []
        for o in OPS:
            if o in SUPPORTED[k]:
                v = kind_op[k].get(o, 0)
                cells.append(str(v))
                if v == 0:
                    gaps.append((k, o))
            else:
                cells.append('')
        w(f'| {k} | ' + ' | '.join(cells) + ' |')
    w('')
    w('Notas: `text`/`phrase` contam os campos por omissão do índice (tipos text e name); '
      '`key` é `--key` em `search`/`count`; `get` conta o campo-chave.')
    w('')
    w('### Opções e modificadores (número de perguntas)')
    w('')
    w('| Opção | Perguntas |')
    w('| --- | ---: |')
    for x in sorted(extra):
        w(f'| {x} | {extra[x]} |')
    w('')
    w('### Níveis hierárquicos derivados (número de perguntas)')
    w('')
    w(', '.join(f'`{x}`: {derived[x]}' for x in sorted(derived)))
    w('')
    w('### Campos multivalor usados (número de perguntas)')
    w('')
    w(', '.join(f'`{x}`: {multi[x]}' for x in sorted(multi)))
    w('')
    w('### Semântica de ausência: intervalos e texto sobre campos com marcador (número de perguntas)')
    w('')
    w(', '.join(f'`{x}`: {absent[x]}' for x in sorted(absent)))
    w('')
    w('### Operações não cobertas')
    w('')
    for k, o in gaps:
        if (k, o) == ('flag', 'terms'):
            continue
        w(f'* {k} × {o}: {UNCOVERED_REASON.get((k, o), "sem pergunta pertinente no conjunto.")}')
    for g in MANUAL_GAPS:
        w(f'* {g}')
    w('')
    return '\n'.join(L_)


def resolve(path):
    p = Path(path)
    return p if p.is_absolute() else REPO / p


class UsageError(Exception):
    pass


def check_declarations():
    """Declared R and I must equal the plan; ids must be unique."""
    ids = [q['id'] for q in QUESTIONS]
    if len(ids) != len(set(ids)):
        raise ValueError('repeated question ids')
    for q in QUESTIONS:
        if (q['r'], q['i']) != (plan_r(q), plan_i(q)):
            raise ValueError(f'{q["id"]}: declared R/I {q["r"]}/{q["i"]} != plan {plan_r(q)}/{plan_i(q)}')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--binary', default=DEFAULT_BINARY,
                        help=f'the rucene-base binary (default {DEFAULT_BINARY}, relative to the repository)')
    parser.add_argument('--root', default=DEFAULT_ROOT,
                        help=f'root of the ten indices, read only (default {DEFAULT_ROOT})')
    parser.add_argument('--only', help='comma-separated question ids (default: all)')
    parser.add_argument('--workers', type=int, default=4, help='processes at once, 1 to 4 (default 4)')
    parser.add_argument('--write-md', action='store_true',
                        help=f'regenerate {DOCUMENT.relative_to(REPO)} from a full run')
    args = parser.parse_args(argv)
    try:
        binary, root = resolve(args.binary), resolve(args.root)
        if not binary.is_file():
            raise UsageError(f'--binary {binary} is not a file')
        if not root.is_dir():
            raise UsageError(f'--root {root} is not a directory')
        if not 1 <= args.workers <= 4:
            raise UsageError('--workers must be between 1 and 4')
        known = [q['id'] for q in QUESTIONS]
        only = None
        if args.only:
            only = [x for x in args.only.split(',') if x]
            unknown = [x for x in only if x not in known]
            if unknown or not only:
                raise UsageError(f'--only: unknown question id(s) {",".join(unknown)}')
            if args.write_md:
                raise UsageError('--write-md needs a full run (no --only)')
    except UsageError as exc:
        print(f'perguntas_portal_base: error: {exc}', file=sys.stderr)
        return 2
    check_declarations()
    schemas = load_schemas()
    for q in QUESTIONS:
        for s in q['steps']:
            step_ops(s, schemas)  # validates field names against the schemas
    todo = [q for q in ordered(QUESTIONS) if only is None or q['id'] in only]
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as ex:
        results = list(ex.map(lambda q: run_question(q, binary, root), todo))
    for q, r in zip(todo, results):
        mark = 'OK ' if r['ok'] else 'ERR'
        print(f'{mark} {q["id"]:>4} R={q["r"]} I={q["i"]} '
              f'{r["answer"][:150] if r["ok"] else r["err"][:200]}')
    passed = sum(r['ok'] for r in results)
    print(f'{passed}/{len(todo)} questions passed')
    if args.write_md:
        when = datetime.datetime.now().strftime('%Y-%m-%d %H:%M')
        md = write_md(QUESTIONS, results, schemas,
                      f'python3 -I {SCRIPT} --write-md', when)
        DOCUMENT.write_text(md, encoding='utf-8')
        print(f'wrote {DOCUMENT.relative_to(REPO)}')
    return 0 if passed == len(todo) else 1


if __name__ == '__main__':
    sys.exit(main())
