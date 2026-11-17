# Portal BASE: 100 perguntas multi-índice para o `rucene-base`

Perguntas sobre transparência da contratação pública que o conjunto de dados Portal BASE responde através dos dez índices do CLI `rucene-base` (crate `crates/rucene-cli-base`). Cada pergunta precisa de dados de pelo menos dois índices diferentes.

## Métrica de complexidade

* **R** — número de pedidos ao CLI no plano mínimo: cada invocação `rucene-base <entidade> search|count|get` é um pedido contra um índice.
* **I** — número de índices distintos usados pelo plano.
* **Regra de agregação em lote:** uma expansão sobre uma lista de resultados conta como **um** pedido quando o CLI a aceita num só comando (`--terms CAMPO=V1,V2,...`, `get --key K1 --key K2 ...`); um `count` por valor é um pedido por valor. Os planos são limitados (os N primeiros ou uma lista curta fixa) e não têm pedidos supérfluos: cada secção diz porque os passos não se fundem.
* Ordenação: R decrescente; empate por I decrescente.
* `$R` = `crates/rucene-cli-base/data/base/index` (raiz por omissão, com os dez índices, 2,1 GB); `rucene-base` = `target/release/rucene-base`. Só comandos de leitura (`search`, `count`, `get`).
* Planos executáveis: `crates/rucene-cli-base/scripts/perguntas_portal_base.py` (corre todos os planos a partir da raiz do repositório).
* Combinação no cliente: apenas aritmética pequena sobre as linhas devolvidas (diferenças, rácios, comparações, interseções de listas curtas), declarada em cada pergunta.
* Dados pessoais: nenhum nome nem NIF de pessoa singular aparece neste documento (NIF começado por 1–4, 7 ou 8 é mascarado; nomes de fornecedores e convidados não são pedidos).

## Validação

* Comando: `python3 -I crates/rucene-cli-base/scripts/perguntas_portal_base.py --write-md`
* Data: 2026-11-17 14:38
* Resultado: **100/100** perguntas validadas (todos os passos com exit 0 e resposta não vazia, ou zero declarado significativo).
* Distribuição de R: R=8: 1, R=7: 3, R=6: 4, R=5: 4, R=4: 10, R=3: 41, R=2: 37

## Resumo

| # | Id | Pergunta | R | I | Índices | Categoria |
| ---: | --- | --- | ---: | ---: | --- | --- |
| 1 | `h01` | Qual a empreitada de maior valor celebrada num ano por uma entidade de âmbito municipal sediada numa NUTS III, e qual o seu percurso completo: anúncio, procedimento, adjudicatário, modificações, local de execução e CPV? | 8 | 8 | adjudicantes, contratos, anuncios, procedimentos, fornecedores, modificacoes, locais, cpv | Quem compra |
| 2 | `h05` | Que anúncios de alteração publicou uma entidade num ano, que anúncio anterior alteraram e com que prazo-limite, que contratos resultaram, por que procedimentos, com que modificações, adjudicatários e CPV? | 7 | 6 | anuncios, contratos, procedimentos, modificacoes, fornecedores, cpv | Anúncios |
| 3 | `h02` | Que percentagem dos contratos de um ano foi marcada como contrato ecológico em cada uma das três NUTS III com mais contratos registados? | 7 | 2 | nuts, contratos | Sustentabilidade |
| 4 | `h03` | Nos três concelhos de um distrito com mais contratos executados, quantos contratos de um ano foram feitos por ajuste direto e quantos por concurso público? | 7 | 2 | locais, contratos | Procedimento e fundamento |
| 5 | `h06` | Dos anúncios com critério ambiental de um ano para obras, de maior preço base, que contratos resultaram, quantos ficaram marcados como ecológicos, por que procedimentos, com que adjudicatários (PME?), modificações e CPV? | 6 | 6 | anuncios, contratos, procedimentos, fornecedores, modificacoes, cpv | Sustentabilidade |
| 6 | `h07` | Numa NUTS II, qual a NUTS III com maior valor contratado, que entidades aí sediadas mais contrataram, quais os seus maiores contratos, com que adjudicatários (PME?), onde foram executados e com quantas modificações? | 6 | 6 | nuts, adjudicantes, contratos, fornecedores, locais, modificacoes | Localização |
| 7 | `h08` | Quais os contratos de uma entidade com mais modificações, com que fundamentos foram modificados, de que procedimento e anúncio vieram, quem os executa (PME?) e qual o seu CPV? | 6 | 6 | contratos, modificacoes, procedimentos, anuncios, fornecedores, cpv | Modificações |
| 8 | `b16` | Nos cinco municípios com mais contratos, que proporção dos contratos não tem o preço base do procedimento preenchido? | 6 | 2 | adjudicantes, contratos | Valores |
| 9 | `c02` | Quais as maiores modificações de um ano com um dado fundamento, face ao preço contratual original, e quem são a entidade, o adjudicatário (PME?) e o tipo de procedimento de cada contrato? | 5 | 5 | modificacoes, contratos, adjudicantes, fornecedores, procedimentos | Modificações |
| 10 | `c17` | Dos anúncios de um modelo de procedimento num ano, que contratos resultaram, com que tipo de procedimento ficou registado cada um, por que entidades e com que adjudicatários (PME?)? | 5 | 5 | anuncios, contratos, procedimentos, adjudicantes, fornecedores | Procedimento e fundamento |
| 11 | `d20` | Que contratos de um grupo CPV foram executados numa NUTS III em dois anos, por que adjudicatários (PME?), com que códigos CPV, em que concelhos e com quantas modificações? | 5 | 5 | contratos, fornecedores, cpv, locais, modificacoes | Objeto (CPV) |
| 12 | `h04` | No concurso público com mais propostas de uma entidade num ano, quem foram os concorrentes, quantos já ganharam contratos ou foram convidados, e qual o historial do vencedor junto dessa entidade? | 5 | 3 | contratos, fornecedores, convidadas | Concorrência |
| 13 | `b03` | Quais os maiores contratos por ajuste direto acima de um valor, celebrados num ano pelos municípios de uma NUTS II, a que NUTS III pertence cada município, e os adjudicatários são PME? | 4 | 4 | adjudicantes, contratos, nuts, fornecedores | Procedimento e fundamento |
| 14 | `b08` | Quais os maiores contratos ganhos por PME sediadas numa NUTS III, quantos foram celebrados com entidades sediadas fora dessa região, e o que foi contratado (descrição CPV)? | 4 | 4 | fornecedores, contratos, adjudicantes, cpv | PME |
| 15 | `b26` | Que entidades usaram os tipos de procedimento mais raros, que contratos resultaram, de que valor, e os adjudicatários são PME? | 4 | 4 | procedimentos, contratos, adjudicantes, fornecedores | Procedimento e fundamento |
| 16 | `c01` | Que entidades compraram ao abrigo de acordos-quadro de um dado tipo de bem num ano, a que fornecedores (PME?) e com que CPV? | 4 | 4 | contratos, adjudicantes, fornecedores, cpv | Acordos-quadro |
| 17 | `c13` | Que anúncios tinham pelo menos um lote acima de um valor base, que contratos e procedimentos deles resultaram, e por que entidades? | 4 | 4 | anuncios, contratos, procedimentos, adjudicantes | Anúncios |
| 18 | `c20` | Onde estão sediadas as entidades que celebraram os maiores contratos ecológicos com critérios materiais num ano, e os adjudicatários são PME? | 4 | 4 | contratos, adjudicantes, nuts, fornecedores | Sustentabilidade |
| 19 | `d12` | Que procedimentos iniciados num ano continuaram a publicar contratos anos depois, quantos contratos tiveram, com que adjudicatários e por que entidades? | 4 | 4 | procedimentos, contratos, fornecedores, adjudicantes | Prazos e datas |
| 20 | `d14` | Que contratos e procedimentos resultaram dos anúncios cujo prazo para propostas terminava num dado dia, e os adjudicatários são PME? | 4 | 4 | anuncios, contratos, procedimentos, fornecedores | Prazos e datas |
| 21 | `d18` | Que contratos registam ocorrências, que entidades os celebraram, quem foram os adjudicatários e foram modificados? | 4 | 4 | contratos, adjudicantes, fornecedores, modificacoes | Quem compra |
| 22 | `d01` | Para a entidade mais convidada a apresentar proposta, em quantos contratos foi convidada, quantos desses ganhou e qual o seu historial como fornecedora? | 4 | 3 | convidadas, contratos, fornecedores | Concorrência |
| 23 | `b01` | Que entidades classificadas com um dado tipo de entidade mais gastaram num ano em contratos de uma divisão CPV, quantos contratos foram ao todo, e os adjudicatários dos maiores são PME? | 3 | 3 | adjudicantes, contratos, fornecedores | Quem compra |
| 24 | `b04` | Que entidades celebraram, num ano, contratos acima de um valor com um dado fundamento legal, onde estão sediadas e os adjudicatários são PME? | 3 | 3 | contratos, adjudicantes, fornecedores | Procedimento e fundamento |
| 25 | `b14` | No procedimento com mais contratos lançado por uma entidade, que adjudicatário recebeu a maior fatia do valor total, é PME e quanto ganhou no total? | 3 | 3 | procedimentos, contratos, fornecedores | Valores |
| 26 | `b17` | Qual a região NUTS III com maior valor contratado, quais os maiores contratos aí executados num ano e os seus adjudicatários são PME? | 3 | 3 | nuts, contratos, fornecedores | Localização |
| 27 | `b18` | Quais os concelhos de um distrito com maior valor contratado, qual o maior contrato executado no primeiro e quantas modificações teve? | 3 | 3 | locais, contratos, modificacoes | Localização |
| 28 | `b19` | Qual o código CPV com maior valor dentro de uma classe, quais os maiores contratos de um ano com esse CPV principal e os adjudicatários são PME? | 3 | 3 | cpv, contratos, fornecedores | Objeto (CPV) |
| 29 | `b21` | Quantos contratos por ajuste direto de um ano têm preço exatamente no limiar legal (ou um cêntimo abaixo), quantos desses adjudicatários são PME e que entidades os celebraram? | 3 | 3 | contratos, fornecedores, adjudicantes | Valores |
| 30 | `b24` | Que objetos (descrição CPV) têm os maiores ajustes diretos de um ano com uma dada fundamentação do ajuste direto, e que entidades os celebraram? | 3 | 3 | contratos, cpv, adjudicantes | Procedimento e fundamento |
| 31 | `b29` | Que códigos CPV descrevem um dado tipo de bem ou serviço, quantos contratos de um ano os usaram e onde estão sediadas as entidades dos maiores? | 3 | 3 | cpv, contratos, adjudicantes | Objeto (CPV) |
| 32 | `b30` | Que códigos CPV são mais usados como código complementar e que entidades os usam sem os declarar como principal? | 3 | 3 | cpv, contratos, adjudicantes | Objeto (CPV) |
| 33 | `b32` | Que anúncios publicados desde uma data usaram os códigos CPV mais frequentes de uma categoria, com que preço base, e onde estão sediadas as entidades? | 3 | 3 | cpv, anuncios, adjudicantes | Anúncios |
| 34 | `b34` | Que local de execução corresponde exatamente a um concelho, qual o maior contrato aí executado num ano e quantas modificações teve? | 3 | 3 | locais, contratos, modificacoes | Localização |
| 35 | `b38` | Que locais de execução fora do país concentram mais contratos, qual o maior contrato executado no primeiro e que entidade o celebrou (sede e tipo)? | 3 | 3 | locais, contratos, adjudicantes | Localização |
| 36 | `c04` | Quais os contratos de uma entidade com mais modificações, qual o fundamento da última modificação de cada um e o adjudicatário é PME? | 3 | 3 | contratos, modificacoes, fornecedores | Modificações |
| 37 | `c05` | Que modificações não foram reduzidas a escrito com uma dada justificação, e de que contratos e entidades são? | 3 | 3 | modificacoes, contratos, adjudicantes | Modificações |
| 38 | `c06` | Que contratos foram modificados por decisão judicial ou arbitral, e por que entidades foram celebrados? | 3 | 3 | modificacoes, contratos, adjudicantes | Modificações |
| 39 | `c10` | Que contratos referem uma dada causa nas razões de alteração do preço, com que fundamentos legais foram modificados e por que entidades? | 3 | 3 | contratos, modificacoes, adjudicantes | Modificações |
| 40 | `c11` | Que entidades anunciaram sistemas de aquisição dinâmicos e para que objetos (descrição CPV)? | 3 | 3 | anuncios, adjudicantes, cpv | Anúncios |
| 41 | `c12` | Dos anúncios de acordos-quadro de um ano com maior preço base, que contratos resultaram e quantos adjudicatários são PME? | 3 | 3 | anuncios, contratos, fornecedores | Acordos-quadro |
| 42 | `c14` | Que contratos resultaram de anúncios com um fator de adjudicação de sustentabilidade, e os adjudicatários são PME? | 3 | 3 | anuncios, contratos, fornecedores | Sustentabilidade |
| 43 | `c16` | Que contratos resultaram dos anúncios de um ano sobre um tema (pesquisa livre), e os adjudicatários são PME? | 3 | 3 | anuncios, contratos, fornecedores | Anúncios |
| 44 | `c23` | Que entidades adjudicaram contratos com um fator de circularidade, e o que compraram (descrição CPV)? | 3 | 3 | contratos, adjudicantes, cpv | Sustentabilidade |
| 45 | `c24` | Nos maiores contratos de uma divisão CPV num ano, quantos adjudicatários distintos são PME e que objetos (descrição CPV) têm? | 3 | 3 | contratos, fornecedores, cpv | PME |
| 46 | `c26` | Que entidades celebraram os maiores contratos resultantes de procedimentos centralizados num ano, onde estão sediadas, e os adjudicatários são PME? | 3 | 3 | contratos, adjudicantes, fornecedores | Compras centralizadas e conjuntas |
| 47 | `c28` | Que anúncios conjuntos subscreveram duas entidades, que outras entidades os subscreveram (e de que regiões), e que contratos resultaram? | 3 | 3 | anuncios, adjudicantes, contratos | Compras centralizadas e conjuntas |
| 48 | `c29` | Nos procedimentos ao abrigo de acordo-quadro com mais contratos, quantos adjudicatários distintos houve e quantos são PME? | 3 | 3 | procedimentos, contratos, fornecedores | Acordos-quadro |
| 49 | `c30` | Que contratos foram celebrados ao abrigo de um acordo-quadro descrito por uma expressão, que procedimento formou o acordo-quadro e que entidades o usaram? | 3 | 3 | contratos, procedimentos, adjudicantes | Acordos-quadro |
| 50 | `d02` | Que entidades convidou uma entidade adjudicante nas suas consultas prévias de um ano, quão convidadas são no país e quais já foram adjudicatárias (PME primeiro)? | 3 | 3 | contratos, convidadas, fornecedores | Concorrência |
| 51 | `d03` | Quantas entidades começaram a ser convidadas recentemente com muitos convites, em quantos contratos do ano corrente foram convidadas, e quantas já ganharam contratos? | 3 | 3 | convidadas, contratos, fornecedores | Concorrência |
| 52 | `d08` | Que locais de execução encontra uma pesquisa por palavra num nome de concelho (com o efeito do stemmer), quantos contratos somam e quem celebrou os maiores? | 3 | 3 | locais, contratos, adjudicantes | Localização |
| 53 | `d10` | Dos contratos celebrados num dia concreto que foram depois modificados, com que fundamentos e por que adjudicatários (PME?)? | 3 | 3 | contratos, modificacoes, fornecedores | Prazos e datas |
| 54 | `d16` | Que entidades celebraram empreitadas com prazos de execução muito longos num período, quais os prazos, e os adjudicatários são PME? | 3 | 3 | contratos, adjudicantes, fornecedores | Prazos e datas |
| 55 | `d21` | Nos procedimentos sobre um serviço com preço base acima de um valor, quanto foi adjudicado face ao preço base, a quantos adjudicatários, e quantos são PME? | 3 | 3 | procedimentos, contratos, fornecedores | Valores |
| 56 | `b33` | Nos dois códigos CPV principais mais usados de uma divisão, que proporção dos contratos foi marcada como ecológica? | 3 | 2 | cpv, contratos | Sustentabilidade |
| 57 | `b35` | Que concelhos tem uma região autónoma (por ordem alfabética), e quantas empreitadas de um ano se executaram nos dois com mais contratos? | 3 | 2 | locais, contratos | Localização |
| 58 | `c08` | Nos contratos de uma categoria CPV de um ano que foram modificados, que parte das modificações teve por fundamento a revisão extraordinária de preços? | 3 | 2 | contratos, modificacoes | Modificações |
| 59 | `c22` | Dos contratos resultantes de anúncios com critério ambiental de um ano, quantos ficaram marcados como contrato ecológico? | 3 | 2 | anuncios, contratos | Sustentabilidade |
| 60 | `c27` | Que proporção dos contratos de um ano celebrados pelas entidades de um dado tipo resultou de compra conjunta? | 3 | 2 | adjudicantes, contratos | Compras centralizadas e conjuntas |
| 61 | `d04` | Em quantos contratos uma empresa foi adjudicatária ou concorrente, qual a sua taxa de sucesso e com que entidades concorreu recentemente? | 3 | 2 | contratos, adjudicantes | Concorrência |
| 62 | `d06` | Nos dois códigos CPV mais usados de um grupo, que proporção dos contratos resultou de procedimentos centralizados? | 3 | 2 | cpv, contratos | Compras centralizadas e conjuntas |
| 63 | `d17` | Que contratos resolveu uma entidade, quem eram os adjudicatários e continuaram a ganhar-lhe contratos depois? | 3 | 2 | contratos, fornecedores | Quem fornece |
| 64 | `b02` | Que entidades com uma dada área de atividade compraram por ajuste direto produtos de uma classe CPV, e quais os contratos mais recentes? | 2 | 2 | adjudicantes, contratos | Procedimento e fundamento |
| 65 | `b05` | Que entidades sediadas numa região celebraram contratos ao abrigo de um regime jurídico regional num ano (listadas por ordem alfabética)? | 2 | 2 | adjudicantes, contratos | Procedimento e fundamento |
| 66 | `b06` | Quais dos maiores compradores públicos também aparecem como adjudicatários, e quanto receberam como fornecedores face ao que compraram? | 2 | 2 | adjudicantes, fornecedores | Quem compra |
| 67 | `b07` | Que entidades lançaram anúncios de um modelo de procedimento pouco frequente desde uma data, e em que regiões estão sediadas? | 2 | 2 | anuncios, adjudicantes | Anúncios |
| 68 | `b09` | Dos fornecedores com atividade contínua entre dois anos e mais contratos, quantos contratos celebraram com uma dada entidade e quais os maiores? | 2 | 2 | fornecedores, contratos | Quem fornece |
| 69 | `b10` | Quem ganhou os contratos de uma entidade cujo objeto contém uma dada expressão, e esses adjudicatários são PME? | 2 | 2 | contratos, fornecedores | PME |
| 70 | `b11` | Das entidades mais convidadas a apresentar proposta, quantas nunca foram adjudicatárias e quais ganham menos contratos por convite? | 2 | 2 | convidadas, fornecedores | Concorrência |
| 71 | `b12` | Em que regiões NUTS estão sediados os fornecedores de maior valor total adjudicado (entre os que têm sede conhecida)? | 2 | 2 | fornecedores, nuts | Quem fornece |
| 72 | `b13` | Que empresas foram indicadas como subcontratadas nas modificações por cessão da posição contratual e subcontratação, e quantas já foram adjudicatárias e são PME? | 2 | 2 | modificacoes, fornecedores | Modificações |
| 73 | `b15` | Nos contratos concluídos de uma entidade com preço total efetivo registado, quais excederam o preço contratual, em quanto, e quem foram os adjudicatários? | 2 | 2 | contratos, fornecedores | Valores |
| 74 | `b20` | Entre os contratos de um intervalo de valor celebrados num ano, quais pertencem a procedimentos com vários contratos, e qual o valor total desses procedimentos? | 2 | 2 | contratos, procedimentos | Valores |
| 75 | `b22` | Nas consultas prévias com primeira publicação num mês e vários contratos, quantos adjudicatários distintos houve? | 2 | 2 | procedimentos, contratos | Procedimento e fundamento |
| 76 | `b23` | Quantos contratos de um ano invocam uma alínea legal (pesquisada como expressão), e quantos contratos teve o procedimento de cada um dos maiores? | 2 | 2 | contratos, procedimentos | Procedimento e fundamento |
| 77 | `b25` | Que entidades invocaram uma justificação para não reduzir o contrato a escrito num ano, e de que tipo são? | 2 | 2 | contratos, adjudicantes | Procedimento e fundamento |
| 78 | `b27` | Nos contratos resultantes de concursos urgentes de um ano, que prazo para apresentação de propostas deram os anúncios? | 2 | 2 | contratos, anuncios | Prazos e datas |
| 79 | `b28` | Em que regiões foram executados os maiores contratos celebrados ao abrigo de uma medida especial de contratação? | 2 | 2 | contratos, nuts | Procedimento e fundamento |
| 80 | `b31` | Que contratos de um ano têm um termo na descrição do CPV mas não no objeto do contrato, e que códigos CPV são? | 2 | 2 | contratos, cpv | Objeto (CPV) |
| 81 | `b36` | Que regiões NUTS têm mais fornecedores sediados do que entidades adjudicantes, e quais as PME de maior valor na primeira? | 2 | 2 | nuts, fornecedores | PME |
| 82 | `b37` | Quantos contratos de compra conjunta foram executados nas regiões cuja descrição NUTS contém uma expressão, e quais os maiores? | 2 | 2 | nuts, contratos | Compras centralizadas e conjuntas |
| 83 | `b39` | Que entidades sediadas fora de uma região executam lá contratos, e quais os maiores? | 2 | 2 | adjudicantes, contratos | Localização |
| 84 | `b40` | Quanto tempo mediou entre a publicação dos anúncios de procedimento de uma entidade num ano e a celebração dos contratos resultantes? | 2 | 2 | anuncios, contratos | Prazos e datas |
| 85 | `c03` | Qual o conteúdo de uma modificação identificada pela chave composta contrato/sequência, e como se compara com o preço contratual original? | 2 | 2 | modificacoes, contratos | Modificações |
| 86 | `c07` | Em que contratos uma modificação fixou um prazo de execução muito longo, e qual era o prazo original? | 2 | 2 | modificacoes, contratos | Modificações |
| 87 | `c09` | Que contratos acumularam um número muito elevado de modificações, e com que fundamentos foram feitas as modificações a partir dessa ordem? | 2 | 2 | contratos, modificacoes | Modificações |
| 88 | `c15` | Que entidades publicaram anúncios num dado número do Diário da República, e onde estão sediadas? | 2 | 2 | anuncios, adjudicantes | Anúncios |
| 89 | `c18` | Que anúncio corresponde a um código INCM e que contratos o citam pelo mesmo código? | 2 | 2 | anuncios, contratos | Anúncios |
| 90 | `c19` | Que prazo e preço base teve um anúncio identificado pelo seu número no Diário da República, onde está publicado, e que contratos resultaram? | 2 | 2 | anuncios, contratos | Anúncios |
| 91 | `c21` | Quem ganhou os contratos adjudicados com um fator de sustentabilidade, e quantos desses adjudicatários são PME? | 2 | 2 | contratos, fornecedores | Sustentabilidade |
| 92 | `c25` | Quantos contratos de um ano ganharam os fornecedores de maior valor sem indicação de PME, e com que entidades? | 2 | 2 | fornecedores, contratos | PME |
| 93 | `d05` | Quais os contratos mais relevantes sobre um serviço (expressão em qualquer campo de texto), favorecendo os que mencionam um aspeto, e onde estão sediadas as entidades? | 2 | 2 | contratos, adjudicantes | Objeto (CPV) |
| 94 | `d07` | Que códigos NUTS (versões 2013 e 2024) têm uma dada designação, e quantos contratos de um ano foram executados nessas regiões? | 2 | 2 | nuts, contratos | Localização |
| 95 | `d09` | Para os contratos de um ano com mais modificações, quando e com que fundamento foi feita a primeira modificação? | 2 | 2 | contratos, modificacoes | Modificações |
| 96 | `d11` | Quantos contratos publicaram as entidades de uma região nos últimos dias úteis de um ano, e quais os maiores? | 2 | 2 | adjudicantes, contratos | Prazos e datas |
| 97 | `d13` | Que modificações foram publicadas muito depois de feitas, com que atraso, e de que entidades são os contratos? | 2 | 2 | modificacoes, contratos | Prazos e datas |
| 98 | `d15` | Que contratos de uma entidade foram concluídos num ano, e os adjudicatários são PME? | 2 | 2 | contratos, fornecedores | Prazos e datas |
| 99 | `d19` | Nos anúncios de uma entidade num ano, como se compara o preço contratual dos contratos resultantes com o preço base anunciado, e quantos o excederam? | 2 | 2 | anuncios, contratos | Valores |
| 100 | `d22` | Quantos contratos de um tipo de serviço têm preço exatamente igual a um limiar legal, e por que procedimento foram feitos? | 2 | 2 | cpv, contratos | Valores |

## Perguntas

### 1. Qual a empreitada de maior valor celebrada num ano por uma entidade de âmbito municipal sediada numa NUTS III, e qual o seu percurso completo: anúncio, procedimento, adjudicatário, modificações, local de execução e CPV?

**Instanciação:** [NUTS III PT11A – Área Metropolitana do Porto; nome com «município»; empreitadas de obras públicas; ano 2024]  
**Id:** `h01` · **R = 8, I = 8** (adjudicantes, contratos, anuncios, procedimentos, fornecedores, modificacoes, locais, cpv) · Quem compra

**Plano:**

1. `rucene-base adjudicantes search $R --term nutsCod=PT11A --text-in nome=município --limit 100 --fields nif,nome` — exit 0, 20 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term 'tipoContrato=Empreitadas de obras públicas' --term Ano=2024 --sort precoContratual:desc --limit 1 --fields idcontrato,idprocedimento,IDAnuncio,adjudicanteNif,adjudicatarioNif,precoContratual,precoBaseProcedimento,cpvPrincipal,localExecucao` — exit 0, 957 resultado(s)
3. `rucene-base anuncios search $R --term 'IDAnuncio=<IDAnuncio do passo 2>' --fields IDAnuncio,dataPublicacao,modeloAnuncio,PrazoPropostas,PrecoBase,CriterAmbient` — exit 0, 1 resultado(s)
4. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 1 resultado(s)
5. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 1 resultado(s)
6. `rucene-base modificacoes search $R --term 'idcontrato=<idcontrato do passo 2>' --fields idcontrato,sequencia,modifContratoFundamento,modifContratoPrecoAlterado` — exit 0, 5 resultado(s)
7. `rucene-base locais get $R --key <localExecucao do passo 2> …` — exit 0, 1 resultado(s)
8. `rucene-base cpv get $R --key <cpvPrincipal do passo 2> …` — exit 0, 1 resultado(s)

**Porque R não é menor:** O passo 2 depende dos NIF do passo 1 (a sede da entidade só existe em `adjudicantes`); os passos 3 a 8 dependem do contrato do passo 2 e cada um consulta um índice diferente, pelo que não se fundem.

**Combinação no cliente:** Escolhe-se a linha única do passo 2; poupança face ao preço base = (base − contratual)/base; o nome da entidade vem do passo 1.

**Tipos de campo e operações:**

* `adjudicantes.nutsCod` (keyword, multivalor): term
* `adjudicantes.nome` (name, multivalor): text-in
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.tipoContrato` (keyword, multivalor): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `anuncios.IDAnuncio` (exact, chave): term
* `procedimentos.idprocedimento` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* `modificacoes.idcontrato` (exact, chave): term
* `locais.local` (keyword, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `adjudicantes`: nif (exact), nome (name)
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), IDAnuncio (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double), precoBaseProcedimento (double), cpvPrincipal (keyword), localExecucao (keyword)
* saída `--fields` de `anuncios`: IDAnuncio (exact), dataPublicacao (date), modeloAnuncio (keyword), PrazoPropostas (int), PrecoBase (double), CriterAmbient (flag)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name), modifContratoPrecoAlterado (double)

**Resposta (validada):** contrato 11040286 de CMPH - DomusSocial - Empresa de Habitação e Manutenção do Município do Porto, EM: 15 498 945,61 €, 10,2 % abaixo do preço base; anúncio 378105 (Concurso público, 2024-04-01, prazo de propostas 43 dias, preço base anunciado 17 250 000,00 €, critério ambiental nao); procedimento 7073513 (Concurso público, 1 contrato(s), 15 498 945,61 €); adjudicatário 507391160 (PME: sim, 183 contratos); 5 modificação(ões); local Portugal, Porto, Porto (172519 contratos no total); CPV 45211000-9 «Construção de edifícios de vários fogos e de moradias independentes»

### 2. Que anúncios de alteração publicou uma entidade num ano, que anúncio anterior alteraram e com que prazo-limite, que contratos resultaram, por que procedimentos, com que modificações, adjudicatários e CPV?

**Instanciação:** [Infraestruturas de Portugal, NIF 503933813; tipo de ato «Anúncio de Alteração»; ano 2025; os 15 primeiros por data]  
**Id:** `h05` · **R = 7, I = 6** (anuncios, contratos, procedimentos, modificacoes, fornecedores, cpv) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term 'tipoActo=Anúncio de Alteração' --term nifEntidade=503933813 --term Ano=2025 --sort dataPublicacao:asc --limit 15 --fields IDAnuncio,anteriorIDAnuncio,dataPublicacao,DataLimitePropostas,PrecoBase` — exit 0, 113 resultado(s)
2. `rucene-base anuncios search $R --terms 'IDAnuncio=<anteriorIDAnuncio do passo 1>' --limit 50 --fields IDAnuncio,DataLimitePropostas,PrazoPropostas,PrecoBase` — exit 0, 15 resultado(s)
3. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio e anteriorIDAnuncio do passo 1>' --limit 50 --fields idcontrato,IDAnuncio,idprocedimento,adjudicatarioNif,precoContratual,cpvPrincipal` — exit 0, 9 resultado(s)
4. `rucene-base procedimentos get $R --key <idprocedimento do passo 3> …` — exit 0, 9 resultado(s)
5. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 3>' --limit 100 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 2 resultado(s)
6. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 3> …` — exit 0, 8 resultado(s)
7. `rucene-base cpv get $R --key <cpvPrincipal do passo 3> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** O passo 2 lê os anúncios anteriores cujo id só se conhece no passo 1; o passo 3 precisa dos ids de ambos; os passos 4 a 7 dependem dos contratos do passo 3 e são índices diferentes.

**Combinação no cliente:** Emparelha cada alteração com o anúncio anterior (prazo-limite antes→depois); soma do preço contratual dos contratos.

**Tipos de campo e operações:**

* `anuncios.tipoActo` (keyword): term
* `anuncios.nifEntidade` (exact, multivalor, ausente=-): term
* `anuncios.Ano` (int): term
* `anuncios.dataPublicacao` (date): sort
* `anuncios.IDAnuncio` (exact, chave): terms
* `contratos.IDAnuncio` (exact): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `modificacoes.idcontrato` (exact, chave): terms
* `fornecedores.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), anteriorIDAnuncio (exact), dataPublicacao (date), DataLimitePropostas (date), PrecoBase (double), PrazoPropostas (int)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), idprocedimento (exact), adjudicatarioNif (exact), precoContratual (double), cpvPrincipal (keyword)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)

**Resposta (validada):** 15 anúncios de alteração; 15 ligados ao anterior, p.ex. 381902←400107 (prazo-limite 2025-02-12→2025-04-10); 398246←400164 (prazo-limite 2025-01-31→2025-02-05); 400417←401965 (prazo-limite 2025-02-28→2025-02-28); 9 contrato(s) resultante(s) somando 37 326 507,00 €; procedimentos: 7117410 (Concurso público), 7383244 (Concurso público), 7417890 (Concurso público), 7444048 (Concurso público), 7444049 (Concurso público), 7468717 (Concurso público), 7432794 (Concurso público), 7474586 (Concurso público), 7431300 (Concurso público); 2 modificação(ões); adjudicatários 500553408 PME sim, 503038113 PME sim, 514363762 PME sim, 500257760 PME sim, 501672796 PME sim, 515823368 PME sim, 502001364 PME sim, 513320717 PME sim; CPV 45234100-7 «Construção de vias férreas», 34946220-8 «Agulhas, cróssimas, alavancas para comando de agulhas e outros dispositivos para mudança de vias», 45221119-9 «Obras de renovação de pontes», 31440000-2 «Baterias», 71335000-5 «Estudos de engenharia»

### 3. Que percentagem dos contratos de um ano foi marcada como contrato ecológico em cada uma das três NUTS III com mais contratos registados?

**Instanciação:** [ano 2025; as três NUTS com maior `nuts.numContratos`]  
**Id:** `h02` · **R = 7, I = 2** (nuts, contratos) · Sustentabilidade

**Plano:**

1. `rucene-base nuts search $R --sort numContratos:desc --limit 3 --fields nuts,descricao,numContratos` — exit 0, 288 resultado(s)
2. `rucene-base contratos count $R --term 'nuts=<1.ª NUTS do passo 1>' --term Ano=2025 --term ContratEcologico=sim` — exit 0, 1523 resultado(s)
3. `rucene-base contratos count $R --term 'nuts=<1.ª NUTS do passo 1>' --term Ano=2025` — exit 0, 66796 resultado(s)
4. `rucene-base contratos count $R --term 'nuts=<2.ª NUTS do passo 1>' --term Ano=2025 --term ContratEcologico=sim` — exit 0, 569 resultado(s)
5. `rucene-base contratos count $R --term 'nuts=<2.ª NUTS do passo 1>' --term Ano=2025` — exit 0, 44609 resultado(s)
6. `rucene-base contratos count $R --term 'nuts=<3.ª NUTS do passo 1>' --term Ano=2025 --term ContratEcologico=sim` — exit 0, 167 resultado(s)
7. `rucene-base contratos count $R --term 'nuts=<3.ª NUTS do passo 1>' --term Ano=2025` — exit 0, 11501 resultado(s)

**Porque R não é menor:** Sem agregação no índice, cada contagem (ecológicos e total por região) é um `count` próprio: 3 regiões × 2 contagens + 1 pedido para escolher as regiões. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Rácio ecológicos/total por região.

**Tipos de campo e operações:**

* `nuts.numContratos` (int): sort
* `contratos.nuts` (keyword, multivalor): term
* `contratos.Ano` (int): term
* `contratos.ContratEcologico` (flag): term
* saída `--fields` de `nuts`: nuts (keyword), descricao (name), numContratos (int)
* opções: contratos: count

**Resposta (validada):** contratos ecológicos em 2025 — PT170 Área Metropolitana de Lisboa: 1523/66796 = 2,3 %; PT11A Área Metropolitana do Porto: 569/44609 = 1,3 %; PT150 Algarve: 167/11501 = 1,5 %

### 4. Nos três concelhos de um distrito com mais contratos executados, quantos contratos de um ano foram feitos por ajuste direto e quantos por concurso público?

**Instanciação:** [distrito «Portugal, Évora» (excluindo o local só de distrito); ano 2025]  
**Id:** `h03` · **R = 7, I = 2** (locais, contratos) · Procedimento e fundamento

**Plano:**

1. `rucene-base locais search $R --term 'localNivel2=Portugal, Évora' --not --key 'Portugal, Évora' --sort numContratos:desc --limit 3 --fields local,concelho,numContratos` — exit 0, 14 resultado(s)
2. `rucene-base contratos count $R --term 'localNivel3=<1.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Ajuste Direto Regime Geral'` — exit 0, 661 resultado(s)
3. `rucene-base contratos count $R --term 'localNivel3=<1.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Concurso público'` — exit 0, 364 resultado(s)
4. `rucene-base contratos count $R --term 'localNivel3=<2.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Ajuste Direto Regime Geral'` — exit 0, 81 resultado(s)
5. `rucene-base contratos count $R --term 'localNivel3=<2.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Concurso público'` — exit 0, 6 resultado(s)
6. `rucene-base contratos count $R --term 'localNivel3=<3.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Ajuste Direto Regime Geral'` — exit 0, 154 resultado(s)
7. `rucene-base contratos count $R --term 'localNivel3=<3.º local do passo 1>' --term Ano=2025 --term 'tipoprocedimento=Concurso público'` — exit 0, 12 resultado(s)

**Porque R não é menor:** Uma contagem por concelho e por tipo de procedimento (o CLI não agrega por valor) e um pedido para escolher os concelhos; os locais têm vírgulas, por isso nem `--terms` os aceitaria. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Percentagem de ajuste direto no total dos dois tipos.

**Tipos de campo e operações:**

* `locais.localNivel2` (keyword, nível derivado): term
* `locais.local` (keyword, chave): --not key
* `locais.numContratos` (int): sort
* `contratos.localNivel3` (keyword, nível derivado): term
* `contratos.Ano` (int): term
* `contratos.tipoprocedimento` (keyword): term
* saída `--fields` de `locais`: local (keyword), concelho (name), numContratos (int)
* opções: contratos: count

**Resposta (validada):** contratos executados em 2025 — Évora: ajuste direto 661, concurso público 364 (ajuste direto = 64,5 %); Portel: ajuste direto 81, concurso público 6 (ajuste direto = 93,1 %); Vila Viçosa: ajuste direto 154, concurso público 12 (ajuste direto = 92,8 %)

### 5. Dos anúncios com critério ambiental de um ano para obras, de maior preço base, que contratos resultaram, quantos ficaram marcados como ecológicos, por que procedimentos, com que adjudicatários (PME?), modificações e CPV?

**Instanciação:** [critério ambiental = sim; divisão CPV 45; ano 2023; os 20 anúncios de maior preço base]  
**Id:** `h06` · **R = 6, I = 6** (anuncios, contratos, procedimentos, fornecedores, modificacoes, cpv) · Sustentabilidade

**Plano:**

1. `rucene-base anuncios search $R --term CriterAmbient=sim --term cpvDivisao=45 --term Ano=2023 --sort PrecoBase:desc --limit 20 --fields IDAnuncio,PrecoBase,designacaoEntidade` — exit 0, 333 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 100 --fields idcontrato,IDAnuncio,idprocedimento,adjudicatarioNif,precoContratual,ContratEcologico,cpv` — exit 0, 4 resultado(s)
3. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 4 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 4 resultado(s)
5. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 2>' --limit 200 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 33 resultado(s)
6. `rucene-base cpv get $R --key <cpv do passo 2> …` — exit 0, 4 resultado(s)

**Porque R não é menor:** Cada passo a partir do 2 depende de ids do passo anterior ou do passo 2, e cada um consulta um índice diferente.

**Combinação no cliente:** Contagem de contratos com `ContratEcologico=sim` e de adjudicatários PME nas linhas devolvidas; soma dos preços.

**Tipos de campo e operações:**

* `anuncios.CriterAmbient` (flag): term
* `anuncios.cpvDivisao` (keyword, nível derivado): term
* `anuncios.Ano` (int): term
* `anuncios.PrecoBase` (double, ausente=0): sort
* `contratos.IDAnuncio` (exact): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* `modificacoes.idcontrato` (exact, chave): terms
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double), designacaoEntidade (name)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), idprocedimento (exact), adjudicatarioNif (exact), precoContratual (double), ContratEcologico (flag), cpv (keyword)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)

**Resposta (validada):** 20 anúncios; 4 contratos resultantes (3 marcados ecológicos), total 413 967 985,06 €; 4 procedimentos; adjudicatários PME 4/4; 33 modificações; CPV: 45234111-7 «Construção de via férrea urbana»; 45214700-7 «Construção de residências universitárias»; 45212212-5 «Construção de piscinas»; 45215100-8 «Obras de construção de edifícios relacionados com serviços de saúde»

### 6. Numa NUTS II, qual a NUTS III com maior valor contratado, que entidades aí sediadas mais contrataram, quais os seus maiores contratos, com que adjudicatários (PME?), onde foram executados e com quantas modificações?

**Instanciação:** [NUTS II PT16 – Centro; 3 entidades; 5 contratos]  
**Id:** `h07` · **R = 6, I = 6** (nuts, adjudicantes, contratos, fornecedores, locais, modificacoes) · Localização

**Plano:**

1. `rucene-base nuts search $R --term nutsNivel2=PT16 --sort valorTotal:desc --limit 5 --fields nuts,descricao,numContratos,valorTotal` — exit 0, 20 resultado(s)
2. `rucene-base adjudicantes search $R --term 'nutsCod=<NUTS III de maior valor do passo 1>' --sort valorTotal:desc --limit 3 --fields nif,nome,valorTotal` — exit 0, 243 resultado(s)
3. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 2>' --sort precoContratual:desc --limit 5 --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual,localExecucao` — exit 0, 17495 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 3> …` — exit 0, 4 resultado(s)
5. `rucene-base locais get $R --key <localExecucao do passo 3> …` — exit 0, 2 resultado(s)
6. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 3>' --limit 200 --fields idcontrato,sequencia` — exit 0, 91 resultado(s)

**Porque R não é menor:** Cadeia estrita: região → entidades (sede só em `adjudicantes`) → contratos → três enriquecimentos em índices diferentes.

**Combinação no cliente:** Escolhe a primeira NUTS do passo 1; conta PME nas linhas do passo 4.

**Tipos de campo e operações:**

* `nuts.nutsNivel2` (keyword, nível derivado): term
* `nuts.valorTotal` (double): sort
* `adjudicantes.nutsCod` (keyword, multivalor): term
* `adjudicantes.valorTotal` (double): sort
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* `locais.local` (keyword, chave): get
* `modificacoes.idcontrato` (exact, chave): terms
* saída `--fields` de `nuts`: nuts (keyword), descricao (name), numContratos (int), valorTotal (double)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double), localExecucao (keyword)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int)

**Resposta (validada):** NUTS III de maior valor no Centro: PT16E Região de Coimbra (2 347 619 543,18 €); entidades de maior valor aí: Município de Coimbra, Instituto Português de Oncologia de Coimbra Francisco Gentil, EPE, Município da Figueira da Foz; 5 maiores contratos: 10927705 35 688 997,20 €, 1026182 23 711 683,78 €, 6983173 22 699 530,83 €, 10294019 17 522 980,35 €, 7105657 13 613 090,64 €; adjudicatários PME 4/4; locais: Portugal, Coimbra, Coimbra, Portugal; 91 modificações

### 7. Quais os contratos de uma entidade com mais modificações, com que fundamentos foram modificados, de que procedimento e anúncio vieram, quem os executa (PME?) e qual o seu CPV?

**Instanciação:** [Infraestruturas de Portugal, NIF 503933813; os 5 contratos com mais modificações]  
**Id:** `h08` · **R = 6, I = 6** (contratos, modificacoes, procedimentos, anuncios, fornecedores, cpv) · Modificações

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=503933813 --range numModificacoes=1.. --sort numModificacoes:desc --limit 5 --fields idcontrato,idprocedimento,IDAnuncio,adjudicatarioNif,precoContratual,cpvPrincipal,numModificacoes` — exit 0, 1004 resultado(s)
2. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --limit 500 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 162 resultado(s)
3. `rucene-base procedimentos get $R --key <idprocedimento do passo 1> …` — exit 0, 5 resultado(s)
4. `rucene-base anuncios search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --fields IDAnuncio,PrazoPropostas,PrecoBase` — exit 0, 5 resultado(s)
5. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 5 resultado(s)
6. `rucene-base cpv get $R --key <cpvPrincipal do passo 1> …` — exit 0, 2 resultado(s)

**Porque R não é menor:** Os passos 2 a 6 dependem dos ids do passo 1 e cada um consulta um índice diferente.

**Combinação no cliente:** Contagem dos fundamentos nas linhas de modificação devolvidas (lista limitada aos 5 contratos).

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.numModificacoes` (int): range, sort
* `modificacoes.idcontrato` (exact, chave): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `anuncios.IDAnuncio` (exact, chave): terms
* `fornecedores.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), IDAnuncio (exact), adjudicatarioNif (exact), precoContratual (double), cpvPrincipal (keyword), numModificacoes (int)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrazoPropostas (int), PrecoBase (double)

**Resposta (validada):** contratos 6465619 (40 modif.), 5579421 (37 modif.), 9158683 (30 modif.), 7806711 (28 modif.), 6814977 (27 modif.); fundamentos mais frequentes: Trabalhos Complementares (art. 370.º do CCP) e Trabalhos a Menos (art. 379.º) (75); Trabalhos Complementares (art. 370.º do CCP) (48); procedimentos: Concurso público, Concurso público, Concurso público, Concurso público, Concurso público; 5 anúncio(s) de origem (prazos: 90 dias, 91 dias, 8 dias, 11 dias, 75 dias); adjudicatários PME 4/5; CPV: Construção de vias férreas; Obras de construção total ou parcial e de engenharia civil

### 8. Nos cinco municípios com mais contratos, que proporção dos contratos não tem o preço base do procedimento preenchido?

**Instanciação:** [nome com «município»; os 5 primeiros por `adjudicantes.numContratos`]  
**Id:** `b16` · **R = 6, I = 2** (adjudicantes, contratos) · Valores

**Plano:**

1. `rucene-base adjudicantes search $R --text-in nome=município --sort numContratos:desc --limit 5 --fields nif,nome,numContratos` — exit 0, 368 resultado(s)
2. `rucene-base contratos count $R --term 'adjudicanteNif=<1.º NIF do passo 1>' --not --range precoBaseProcedimento=..` — exit 0, 178 resultado(s)
3. `rucene-base contratos count $R --term 'adjudicanteNif=<2.º NIF do passo 1>' --not --range precoBaseProcedimento=..` — exit 0, 33 resultado(s)
4. `rucene-base contratos count $R --term 'adjudicanteNif=<3.º NIF do passo 1>' --not --range precoBaseProcedimento=..` — exit 0, 40 resultado(s)
5. `rucene-base contratos count $R --term 'adjudicanteNif=<4.º NIF do passo 1>' --not --range precoBaseProcedimento=..` — exit 0, 3148 resultado(s)
6. `rucene-base contratos count $R --term 'adjudicanteNif=<5.º NIF do passo 1>' --not --range precoBaseProcedimento=..` — exit 0, 231 resultado(s)

**Porque R não é menor:** Uma contagem por município (o CLI não agrega por valor); o denominador é o agregado `numContratos` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Rácio contagem/`numContratos` por município.

**Tipos de campo e operações:**

* `adjudicantes.nome` (name, multivalor): text-in
* `adjudicantes.numContratos` (int): sort
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.precoBaseProcedimento` (double, ausente=0): --not range
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), numContratos (int)
* opções: contratos: count
* `--not --range precoBaseProcedimento=..` conta os documentos sem valor: o marcador `0` não é indexado.

**Resposta (validada):** contratos sem preço base preenchido — Município de Lisboa: 178/18286 = 1,0 %; Município de Oeiras: 33/9372 = 0,4 %; Município de Sintra: 40/8876 = 0,5 %; CMPH - DomusSocial - Empresa de Habitação e Manutenção do Município do Porto, EM: 3148/8197 = 38,4 %; Município de Loulé: 231/8017 = 2,9 %

### 9. Quais as maiores modificações de um ano com um dado fundamento, face ao preço contratual original, e quem são a entidade, o adjudicatário (PME?) e o tipo de procedimento de cada contrato?

**Instanciação:** [fundamento «Trabalhos Complementares (art. 370.º do CCP)»; ano 2024; 10 maiores por preço alterado]  
**Id:** `c02` · **R = 5, I = 5** (modificacoes, contratos, adjudicantes, fornecedores, procedimentos) · Modificações

**Plano:**

1. `rucene-base modificacoes search $R --term 'modifContratoFundamento=Trabalhos Complementares (art. 370.º do CCP)' --term Ano=2024 --sort modifContratoPrecoAlterado:desc --limit 10 --exact-total --fields idcontrato,sequencia,modifContratoPrecoAlterado` — exit 0, 4812 resultado(s)
2. `rucene-base contratos get $R --key <idcontrato do passo 1> …` — exit 0, 5 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif dos contratos do passo 2> …` — exit 0, 2 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 6 resultado(s)
5. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** O contrato depende do passo 1; entidade, adjudicatário e procedimento dependem do passo 2 e estão em três índices.

**Combinação no cliente:** Comparação preço alterado vs preço contratual por contrato.

**Tipos de campo e operações:**

* `modificacoes.modifContratoFundamento` (name): term
* `modificacoes.Ano` (int): term
* `modificacoes.modifContratoPrecoAlterado` (double): sort
* `contratos.idcontrato` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* `procedimentos.idprocedimento` (exact, chave): get
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoPrecoAlterado (double)
* opções: modificacoes: exact-total

**Resposta (validada):** 4812 modificações de 2024 por trabalhos complementares; maiores: 11063930/2: 251 889 536,00 € vs contratual 2 450 000,00 € (Centro Social Paroquial de Santo Antão, tipo n/i; adjudicatário 510866123 PME sim; Concurso público); 11154786/4: 112 062 555,70 € vs contratual 110 742 152,27 € (Infraestruturas de Portugal, SA, Empresa pública; adjudicatário 500023875 PME sim; Concurso público); 11154786/3: 111 614 491,21 € vs contratual 110 742 152,27 € (Infraestruturas de Portugal, SA, Empresa pública; adjudicatário 500023875 PME sim; Concurso público); 11154786/1: 110 849 905,64 € vs contratual 110 742 152,27 € (Infraestruturas de Portugal, SA, Empresa pública; adjudicatário 500023875 PME sim; Concurso público)

### 10. Dos anúncios de um modelo de procedimento num ano, que contratos resultaram, com que tipo de procedimento ficou registado cada um, por que entidades e com que adjudicatários (PME?)?

**Instanciação:** [modelo de anúncio «Concurso limitado por prévia qualificação»; ano 2024; 30 maiores preços base]  
**Id:** `c17` · **R = 5, I = 5** (anuncios, contratos, procedimentos, adjudicantes, fornecedores) · Procedimento e fundamento

**Plano:**

1. `rucene-base anuncios search $R --term 'modeloAnuncio=Concurso limitado por prévia qualificação' --term Ano=2024 --sort PrecoBase:desc --limit 30 --exact-total --fields IDAnuncio,PrecoBase` — exit 0, 465 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 200 --fields idcontrato,idprocedimento,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 10 resultado(s)
3. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 8 resultado(s)
4. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 8 resultado(s)
5. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 12 resultado(s)

**Porque R não é menor:** Os passos 3 a 5 dependem dos contratos do passo 2 e são índices diferentes.

**Combinação no cliente:** Tipos de procedimento distintos, nomes distintos das entidades, contagem de PME.

**Tipos de campo e operações:**

* `anuncios.modeloAnuncio` (keyword): term
* `anuncios.Ano` (int): term
* `anuncios.PrecoBase` (double, ausente=0): sort
* `contratos.IDAnuncio` (exact): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double)
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: anuncios: exact-total

**Resposta (validada):** 465 anúncios de concurso limitado por prévia qualificação em 2024; 10 contratos dos 30 maiores; procedimentos: Concurso limitado por prévia qualificação; entidades: Associação de Municípios para a Gestão Sustentável de Resíduos do Grande Porto - LIPOR, Direção-Geral da Educação, MUNICÍPIO DE VILA NOVA DE FAMALICÃO, Município de Esposende; adjudicatários PME 10/12

### 11. Que contratos de um grupo CPV foram executados numa NUTS III em dois anos, por que adjudicatários (PME?), com que códigos CPV, em que concelhos e com quantas modificações?

**Instanciação:** [grupo CPV 553 (serviços de restauração); NUTS III de execução PT16E; anos 2024 e 2025; ordenados por local]  
**Id:** `d20` · **R = 5, I = 5** (contratos, fornecedores, cpv, locais, modificacoes) · Objeto (CPV)

**Plano:**

1. `rucene-base contratos search $R --term cpvGrupo=553 --term nutsNivel3=PT16E --terms Ano=2024,2025 --sort localExecucao:asc,precoContratual:desc --limit 50 --exact-total --fields idcontrato,adjudicatarioNif,cpv,localExecucao,precoContratual` — exit 0, 47 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 29 resultado(s)
3. `rucene-base cpv get $R --key <cpv do passo 1> …` — exit 0, 7 resultado(s)
4. `rucene-base locais get $R --key <localExecucao do passo 1> …` — exit 0, 16 resultado(s)
5. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --limit 200 --fields idcontrato,sequencia` — exit 0, 6 resultado(s)

**Porque R não é menor:** Os passos 2 a 5 dependem do passo 1 e consultam quatro índices diferentes.

**Combinação no cliente:** Contagem de PME e de modificações.

**Tipos de campo e operações:**

* `contratos.cpvGrupo` (keyword, nível derivado): term
* `contratos.nutsNivel3` (keyword, nível derivado): term
* `contratos.Ano` (int): terms
* `contratos.localExecucao` (keyword, multivalor): sort
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* `locais.local` (keyword, chave): get
* `modificacoes.idcontrato` (exact, chave): terms
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), cpv (keyword), localExecucao (keyword), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int)
* opções: contratos: exact-total

**Resposta (validada):** 47 contratos de restauração executados na Região de Coimbra em 2024–2025; adjudicatários PME 29/29; CPV: Serviços de mesa e de restauração; Serviços de confecção de refeições; Serviços de provimento de refeições; locais: Portugal (192464), Portugal, Coimbra, Figueira da Foz (10518), Portugal, Coimbra, Coimbra (78810); 6 modificação(ões)

### 12. No concurso público com mais propostas de uma entidade num ano, quem foram os concorrentes, quantos já ganharam contratos ou foram convidados, e qual o historial do vencedor junto dessa entidade?

**Instanciação:** [Município de Lisboa, NIF 500051070; concurso público; ano 2024]  
**Id:** `h04` · **R = 5, I = 3** (contratos, fornecedores, convidadas) · Concorrência

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=500051070 --term 'tipoprocedimento=Concurso público' --term Ano=2024 --sort numPropostas:desc --limit 1 --fields idcontrato,numPropostas,precoContratual,concorrenteNif,adjudicatarioNif` — exit 0, 164 resultado(s)
2. `rucene-base fornecedores search $R --terms 'nif=<concorrenteNif do passo 1>' --limit 100 --fields nif,pme,numContratos` — exit 0, 21 resultado(s)
3. `rucene-base convidadas search $R --terms 'nif=<concorrenteNif do passo 1>' --limit 100 --fields nif,numConvites` — exit 0, 20 resultado(s)
4. `rucene-base contratos count $R --term 'concorrenteNif=<adjudicatarioNif do passo 1>'` — exit 0, 99 resultado(s)
5. `rucene-base contratos count $R --term 'adjudicatarioNif=<adjudicatarioNif do passo 1>' --term adjudicanteNif=500051070` — exit 0, 3 resultado(s)

**Porque R não é menor:** Os passos 2 e 3 são índices diferentes; os passos 4 e 5 são contagens com critérios diferentes sobre o NIF vencedor do passo 1 (um `count` devolve um só número).

**Combinação no cliente:** Interseção da lista de concorrentes com as linhas dos passos 2 e 3; contagem de PME.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.tipoprocedimento` (keyword): term
* `contratos.Ano` (int): term
* `contratos.numPropostas` (int): sort
* `fornecedores.nif` (exact, chave): terms
* `convidadas.nif` (exact, chave): terms
* `contratos.concorrenteNif` (exact, multivalor, ausente=-): term
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): term
* saída `--fields` de `contratos`: idcontrato (exact), numPropostas (int), precoContratual (double), concorrenteNif (exact), adjudicatarioNif (exact)
* saída `--fields` de `fornecedores`: nif (exact), pme (flag), numContratos (int)
* saída `--fields` de `convidadas`: nif (exact), numConvites (int)
* opções: contratos: count
* NIF de concorrentes e vencedor só são mostrados quando são de pessoa coletiva.

**Resposta (validada):** contrato 10669306 (22 propostas, 125 000,00 €); 22 concorrentes: 21 já foram adjudicatários (PME: 21), 20 constam de `convidadas`; vencedor 516196448: concorreu em 99 contratos no total e ganhou 3 contratos ao Município de Lisboa

### 13. Quais os maiores contratos por ajuste direto acima de um valor, celebrados num ano pelos municípios de uma NUTS II, a que NUTS III pertence cada município, e os adjudicatários são PME?

**Instanciação:** [NUTS II PT18 – Alentejo; nome com «município»; Ajuste Direto Regime Geral; ≥ 20 000 €; ano 2025]  
**Id:** `b03` · **R = 4, I = 4** (adjudicantes, contratos, nuts, fornecedores) · Procedimento e fundamento

**Plano:**

1. `rucene-base adjudicantes search $R --term nutsNivel2=PT18 --text-in nome=município --limit 100 --fields nif,nome,nutsCod` — exit 0, 57 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term 'tipoprocedimento=Ajuste Direto Regime Geral' --range precoContratual=20000.. --term Ano=2025 --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 410 resultado(s)
3. `rucene-base nuts get $R --key <nutsCod das entidades do passo 1 presentes no passo 2> …` — exit 0, 4 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 9 resultado(s)

**Porque R não é menor:** Sede (`adjudicantes`), contratos, descrição NUTS e atributo PME estão em quatro índices; os passos 3 e 4 só conhecem as chaves depois do passo 2.

**Combinação no cliente:** Emparelha cada contrato com o `nutsCod` da sua entidade (passo 1) e com a descrição (passo 3); contagem de PME.

**Tipos de campo e operações:**

* `adjudicantes.nutsNivel2` (keyword, nível derivado): term
* `adjudicantes.nome` (name, multivalor): text-in
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.tipoprocedimento` (keyword): term
* `contratos.precoContratual` (double): range, sort
* `contratos.Ano` (int): term
* `nuts.nuts` (keyword, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), nutsCod (keyword)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 410 contratos ≥ 20 000 € por ajuste direto em 2025; maiores: Município de Évora (Alentejo Central) 2 235 568,79 €; Município de Alpiarça (Lezíria do Tejo) 1 742 545,79 €; Município da Golegã (Lezíria do Tejo) 1 170 744,00 €; Município de Marvão (Alto Alentejo) 909 686,26 €; Município da Golegã (Lezíria do Tejo) 893 507,77 €; adjudicatários PME 9/9

### 14. Quais os maiores contratos ganhos por PME sediadas numa NUTS III, quantos foram celebrados com entidades sediadas fora dessa região, e o que foi contratado (descrição CPV)?

**Instanciação:** [NUTS III PT16E – Região de Coimbra; PME = sim; 10 maiores contratos]  
**Id:** `b08` · **R = 4, I = 4** (fornecedores, contratos, adjudicantes, cpv) · PME

**Plano:**

1. `rucene-base fornecedores search $R --term pme=sim --term nutsCod=PT16E --limit 500 --fields nif` — exit 0, 43 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicatarioNif=<nif do passo 1>' --sort precoContratual:desc --limit 10 --fields idcontrato,adjudicatarioNif,adjudicanteNif,cpvPrincipal,precoContratual` — exit 0, 472 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 6 resultado(s)
4. `rucene-base cpv get $R --key <cpvPrincipal do passo 2> …` — exit 0, 6 resultado(s)

**Porque R não é menor:** PME e sede do fornecedor estão em `fornecedores`, a sede da entidade em `adjudicantes`, a descrição CPV em `cpv`; os passos 3 e 4 dependem do passo 2.

**Combinação no cliente:** Compara o `nutsCod` de cada entidade com PT16E.

**Tipos de campo e operações:**

* `fornecedores.pme` (flag): term
* `fornecedores.nutsCod` (keyword, multivalor): term
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): terms
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `fornecedores`: nif (exact)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), adjudicanteNif (exact), cpvPrincipal (keyword), precoContratual (double)

**Resposta (validada):** 43 PME sediadas na Região de Coimbra; dos 10 maiores contratos, 9 foram com entidades sediadas fora da região: 503222364→Direção-Geral da Educação [PT170] 9 970 000,00 €; 501121528→ARTE - Agência para a Reforma Tecnológica do Estado [PT1A0] 1 494 818,08 €; 503004405→Município de Águeda [PT16D] 1 207 547,17 €; 508796806→ARTE - Agência para a Reforma Tecnológica do Estado [PT1A0] 771 809,28 €; CPV: Pacote de software para ensino; Serviços de desenvolvimento experimental e de investigação; Serviços de tratamento e eliminação de resíduos e lixos não-perigosos

### 15. Que entidades usaram os tipos de procedimento mais raros, que contratos resultaram, de que valor, e os adjudicatários são PME?

**Instanciação:** [Parceria para a inovação; Diálogo concorrencial; Concurso de ideias simplificado]  
**Id:** `b26` · **R = 4, I = 4** (procedimentos, contratos, adjudicantes, fornecedores) · Procedimento e fundamento

**Plano:**

1. `rucene-base procedimentos search $R --terms 'tipoprocedimento=Parceria para a inovação,Diálogo concorrencial,Concurso de ideias simplificado' --sort tipoprocedimento:asc --limit 20 --fields idprocedimento,tipoprocedimento,adjudicanteNif,valorTotal` — exit 0, 6 resultado(s)
2. `rucene-base contratos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --limit 50 --fields idcontrato,idprocedimento,adjudicatarioNif,precoContratual` — exit 0, 6 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 6 resultado(s)
4. `rucene-base fornecedores search $R --terms 'nif=<adjudicatarioNif do passo 2>' --limit 50 --fields nif,pme` — exit 0, 5 resultado(s)

**Porque R não é menor:** Procedimentos, contratos, nomes de entidade e atributo PME estão em quatro índices; os passos 2 e 3 dependem do passo 1, o 4 do passo 2.

**Combinação no cliente:** Contagem de contratos por procedimento nas linhas do passo 2; contagem de PME.

**Tipos de campo e operações:**

* `procedimentos.tipoprocedimento` (keyword): terms, sort
* `contratos.idprocedimento` (exact): terms
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): terms
* saída `--fields` de `procedimentos`: idprocedimento (exact), tipoprocedimento (keyword), adjudicanteNif (exact), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), adjudicatarioNif (exact), precoContratual (double)
* saída `--fields` de `fornecedores`: nif (exact), pme (flag)

**Resposta (validada):** Concurso de ideias simplificado: Município de Alvaiázere (1 contrato(s), 0,00 €); Concurso de ideias simplificado: Freguesia de Anelhe (1 contrato(s), 83 422,94 €); Concurso de ideias simplificado: Secretaria Regional do Turismo, Mobilidade e Infraestruturas (1 contrato(s), 6 000,00 €); Diálogo concorrencial: Unidade Local de Saúde de Coimbra, EPE (1 contrato(s), 474 680,80 €); Parceria para a inovação: Serviços Partilhados do Ministério da Saúde, EPE (1 contrato(s), 196 500,00 €); Parceria para a inovação: Agência para o Investimento e Comércio Externo de Portugal, EPE (1 contrato(s), 1 545 437,00 €); adjudicatários PME 5/5

### 16. Que entidades compraram ao abrigo de acordos-quadro de um dado tipo de bem num ano, a que fornecedores (PME?) e com que CPV?

**Instanciação:** [«Ao abrigo de acordo-quadro (art.º 259.º)»; descrição do acordo-quadro com «combustíveis»; ano 2025; 20 maiores]  
**Id:** `c01` · **R = 4, I = 4** (contratos, adjudicantes, fornecedores, cpv) · Acordos-quadro

**Plano:**

1. `rucene-base contratos search $R --term 'tipoprocedimento=Ao abrigo de acordo-quadro (art.º 259.º)' --text-in DescrAcordoQuadro=combustíveis --term Ano=2025 --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,numAcordoQuadro,adjudicanteNif,adjudicatarioNif,precoContratual,cpvPrincipal` — exit 0, 419 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 11 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 4 resultado(s)
4. `rucene-base cpv get $R --key <cpvPrincipal do passo 1> …` — exit 0, 4 resultado(s)

**Porque R não é menor:** Os passos 2 a 4 dependem do passo 1 e consultam três índices diferentes.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `contratos.tipoprocedimento` (keyword): term
* `contratos.DescrAcordoQuadro` (text): text-in
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), numAcordoQuadro (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double), cpvPrincipal (keyword)
* opções: contratos: exact-total

**Resposta (validada):** 419 contratos de 2025 ao abrigo de acordos-quadro de combustíveis; 11 entidades nos 20 maiores (p.ex. Estado Maior da Força Aérea, Guarda Nacional Republicana, Serviços Municipalizados de Sintra (SMAS)); fornecedores: 500697370 PME sim, 500246963 PME sim, 980622212 PME sim, 502840153 PME sim; CPV: 09131100-7 «Combustíveis do tipo querosene para motores de reacção»; 09130000-9 «Petróleo e destilados»; 09100000-0 «Combustíveis»; 09134100-8 «Gasóleo»

### 17. Que anúncios tinham pelo menos um lote acima de um valor base, que contratos e procedimentos deles resultaram, e por que entidades?

**Instanciação:** [valor base de lote ≥ 50 000 000 € (campo multivalor); 15 maiores preços base]  
**Id:** `c13` · **R = 4, I = 4** (anuncios, contratos, procedimentos, adjudicantes) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --range loteValorBase=50000000.. --sort PrecoBase:desc --limit 15 --exact-total --fields IDAnuncio,PrecoBase,loteValorBase` — exit 0, 196 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 100 --fields idcontrato,IDAnuncio,idprocedimento,adjudicanteNif,precoContratual` — exit 0, 7 resultado(s)
3. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 4 resultado(s)
4. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 4 resultado(s)

**Porque R não é menor:** Cadeia anúncio → contrato → (procedimento, entidade); os passos 3 e 4 são índices diferentes.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `anuncios.loteValorBase` (double, multivalor): range
* `anuncios.PrecoBase` (double, ausente=0): sort
* `contratos.IDAnuncio` (exact): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double), loteValorBase (double)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), idprocedimento (exact), adjudicanteNif (exact), precoContratual (double)
* opções: anuncios: exact-total

**Resposta (validada):** 196 anúncios com um lote ≥ 50 M€; 7 contratos resultantes dos 15 maiores; procedimentos: 10357542 (Concurso público, 1 contratos, 1 499 000,00 €); 6866843 (Concurso público, 1 contratos, 1 661 362 811,55 €); 5678243 (Procedimento de negociação, 1 contratos, 746 042 424,94 €); entidades: CP - Comboios de Portugal, EPE, Infraestruturas de Portugal, SA, Instituto Português de Oncologia de Lisboa FG, E.P.E., TML - Transportes Metropolitanos de Lisboa, EMT, SA

### 18. Onde estão sediadas as entidades que celebraram os maiores contratos ecológicos com critérios materiais num ano, e os adjudicatários são PME?

**Instanciação:** [contrato ecológico = sim e critérios materiais = sim; ano 2025; 20 maiores]  
**Id:** `c20` · **R = 4, I = 4** (contratos, adjudicantes, nuts, fornecedores) · Sustentabilidade

**Plano:**

1. `rucene-base contratos search $R --term ContratEcologico=sim --term CritMateriais=sim --term Ano=2025 --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 91 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 10 resultado(s)
3. `rucene-base nuts get $R --key <nutsCod das entidades do passo 2> …` — exit 0, 6 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 14 resultado(s)

**Porque R não é menor:** Cadeia contrato → entidade → região, mais o atributo PME em `fornecedores`; um índice por passo.

**Combinação no cliente:** Contagem de entidades por região e de PME.

**Tipos de campo e operações:**

* `contratos.ContratEcologico` (flag): term
* `contratos.CritMateriais` (flag): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `nuts.nuts` (keyword, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 91 contratos de 2025 ecológicos e com critérios materiais; sede das 10 entidades dos 20 maiores: Grande Lisboa (3), Alto Minho (2), Área Metropolitana de Lisboa (2), Área Metropolitana do Porto (1); adjudicatários PME 14/14

### 19. Que procedimentos iniciados num ano continuaram a publicar contratos anos depois, quantos contratos tiveram, com que adjudicatários e por que entidades?

**Instanciação:** [primeira publicação em 2022; última publicação a partir de 2025-01-01; 5 com mais contratos]  
**Id:** `d12` · **R = 4, I = 4** (procedimentos, contratos, fornecedores, adjudicantes) · Prazos e datas

**Plano:**

1. `rucene-base procedimentos search $R --range primeiraPublicacao=2022-01-01..2022-12-31 --range ultimaPublicacao=2025-01-01.. --sort numContratos:desc --limit 5 --exact-total --fields idprocedimento,numContratos,primeiraPublicacao,ultimaPublicacao` — exit 0, 16 resultado(s)
2. `rucene-base contratos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --limit 1000 --fields idcontrato,adjudicanteNif,adjudicatarioNif` — exit 0, 269 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 13 resultado(s)
4. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 201 resultado(s)

**Porque R não é menor:** Cadeia procedimento → contratos → (fornecedores, entidades); os passos 3 e 4 são índices diferentes.

**Combinação no cliente:** Diferença em dias entre as datas agregadas do procedimento; contagem de PME.

**Tipos de campo e operações:**

* `procedimentos.primeiraPublicacao` (date): range
* `procedimentos.ultimaPublicacao` (date): range
* `procedimentos.numContratos` (int): sort
* `contratos.idprocedimento` (exact): terms
* `fornecedores.nif` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `procedimentos`: idprocedimento (exact), numContratos (int), primeiraPublicacao (date), ultimaPublicacao (date)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact)
* opções: procedimentos: exact-total

**Resposta (validada):** 16 procedimentos iniciados em 2022 com publicações até 2025 ou depois; nos 5 com mais contratos, 1498 dias entre a primeira e a última publicação (máx.); 269 contratos; adjudicatários PME 13/13; entidades: A. D. A. M. - Águas do Alto Minho, SA, ARS Norte - Administração Regional de Saúde do Norte, I.P., ARTE - Agência para a Reforma Tecnológica do Estado, Academia Portuguesa da História

### 20. Que contratos e procedimentos resultaram dos anúncios cujo prazo para propostas terminava num dado dia, e os adjudicatários são PME?

**Instanciação:** [data-limite de propostas 2025-06-30]  
**Id:** `d14` · **R = 4, I = 4** (anuncios, contratos, procedimentos, fornecedores) · Prazos e datas

**Plano:**

1. `rucene-base anuncios search $R --term DataLimitePropostas=2025-06-30 --limit 200 --exact-total --fields IDAnuncio` — exit 0, 122 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 300 --fields idcontrato,idprocedimento,adjudicatarioNif,precoContratual` — exit 0, 168 resultado(s)
3. `rucene-base procedimentos get $R --key <idprocedimento do passo 2> …` — exit 0, 88 resultado(s)
4. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 125 resultado(s)

**Porque R não é menor:** Cadeia anúncio → contrato → (procedimento, fornecedor); os passos 3 e 4 são índices diferentes.

**Combinação no cliente:** Soma dos preços; tipos distintos; contagem de PME.

**Tipos de campo e operações:**

* `anuncios.DataLimitePropostas` (date): term
* `contratos.IDAnuncio` (exact): terms
* `procedimentos.idprocedimento` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact)
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: anuncios: exact-total

**Resposta (validada):** 122 anúncios com prazo-limite a 30/06/2025; 168 contratos resultantes (73 872 245,21 €); tipos de procedimento: Concurso público; adjudicatários PME 125/125

### 21. Que contratos registam ocorrências, que entidades os celebraram, quem foram os adjudicatários e foram modificados?

**Instanciação:** [ocorrências = sim]  
**Id:** `d18` · **R = 4, I = 4** (contratos, adjudicantes, fornecedores, modificacoes) · Quem compra

**Plano:**

1. `rucene-base contratos search $R --term ocorrencias=sim --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 5 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 4 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 5 resultado(s)
4. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --limit 100 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 2 resultado(s)

**Porque R não é menor:** Entidades, fornecedores e modificações estão em três índices, todos a partir do passo 1.

**Combinação no cliente:** Soma dos preços; contagem de PME e de modificações.

**Tipos de campo e operações:**

* `contratos.ocorrencias` (flag): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* `modificacoes.idcontrato` (exact, chave): terms
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)
* opções: contratos: exact-total

**Resposta (validada):** 5 contratos com ocorrências; entidades: CFPIMM - Centro de Formação Profissional das Indústrias da Madeira e Mobiliário, Município de Chaves, Município de Vila Nova de Gaia, Serviços Municipalizados de Água e Saneamento do Município do Montijo; adjudicatários PME 5/5; total 1 026 826,53 €; 2 modificação(ões)

### 22. Para a entidade mais convidada a apresentar proposta, em quantos contratos foi convidada, quantos desses ganhou e qual o seu historial como fornecedora?

**Instanciação:** [a entidade (pessoa coletiva) com maior `numConvites`]  
**Id:** `d01` · **R = 4, I = 3** (convidadas, contratos, fornecedores) · Concorrência

**Plano:**

1. `rucene-base convidadas search $R --sort numConvites:desc --limit 5 --fields nif,numConvites,primeiroAno,ultimoAno` — exit 0, 50689 resultado(s)
2. `rucene-base contratos count $R --term 'convidadaNif=<primeiro nif de pessoa coletiva do passo 1>'` — exit 0, 61705 resultado(s)
3. `rucene-base contratos count $R --term 'convidadaNif=<primeiro nif de pessoa coletiva do passo 1>' --term 'adjudicatarioNif=<primeiro nif de pessoa coletiva do passo 1>'` — exit 0, 8320 resultado(s)
4. `rucene-base fornecedores get $R --key <NIF do passo 1>` — exit 0, 1 resultado(s)

**Porque R não é menor:** Duas contagens com critérios diferentes (um `count` dá um número) e o historial em `fornecedores`.

**Combinação no cliente:** Taxa de sucesso = ganhos/convidada; compara o agregado `numConvites` com a contagem.

**Tipos de campo e operações:**

* `convidadas.numConvites` (int): sort
* `contratos.convidadaNif` (exact, multivalor, ausente=-): term
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): term
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `convidadas`: nif (exact), numConvites (int), primeiroAno (int), ultimoAno (int)
* opções: contratos: count

**Resposta (validada):** 508107997: 61705 convites (agregado) / 61705 contratos com convite; ganhou 8320 desses (13,5 %); como fornecedor: 15831 contratos, 136 538 031,51 €, PME sim

### 23. Que entidades classificadas com um dado tipo de entidade mais gastaram num ano em contratos de uma divisão CPV, quantos contratos foram ao todo, e os adjudicatários dos maiores são PME?

**Instanciação:** [tipo de entidade «Autoridade local»; divisão CPV 90 (esgotos, resíduos, limpeza e ambiente); ano 2025; 5 maiores]  
**Id:** `b01` · **R = 3, I = 3** (adjudicantes, contratos, fornecedores) · Quem compra

**Plano:**

1. `rucene-base adjudicantes search $R --term 'tipoEntidade=Autoridade local' --limit 400 --fields nif` — exit 0, 265 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term cpvDivisao=90 --term Ano=2025 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual` — exit 0, 183 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** O tipo de entidade só existe em `adjudicantes` e o atributo PME em `fornecedores`; o `search --exact-total` dá total e maiores contratos num só pedido.

**Combinação no cliente:** Contagem de PME nos documentos do passo 3.

**Tipos de campo e operações:**

* `adjudicantes.tipoEntidade` (keyword, multivalor): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.cpvDivisao` (keyword, nível derivado): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `adjudicantes`: nif (exact)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 183 contratos de autoridades locais na divisão 90 em 2025; maiores: Municipio de Vila Real de Santo António 33 900 000,00 €; Município da Amadora 2 106 255,60 €; Município da Amadora 1 662 829,20 €; Município da Amadora 787 800,24 €; Câmara Municipal de Matosinhos 771 565,79 €; adjudicatários PME 5/5

### 24. Que entidades celebraram, num ano, contratos acima de um valor com um dado fundamento legal, onde estão sediadas e os adjudicatários são PME?

**Instanciação:** [fundamento «Artigo 24.º, n.º 1, alínea c) do Código dos Contratos Públicos» (urgência imperiosa); ≥ 1 000 000 €; ano 2020; 10 maiores]  
**Id:** `b04` · **R = 3, I = 3** (contratos, adjudicantes, fornecedores) · Procedimento e fundamento

**Plano:**

1. `rucene-base contratos search $R --term 'fundamentacao=Artigo 24.º, n.º 1, alínea c) do Código dos Contratos Públicos' --term Ano=2020 --range precoContratual=1000000.. --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 70 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 3 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 9 resultado(s)

**Porque R não é menor:** A sede da entidade só está em `adjudicantes` e o atributo PME em `fornecedores`; ambos dependem do passo 1.

**Combinação no cliente:** Emparelha contrato e entidade por NIF; contagem de PME.

**Tipos de campo e operações:**

* `contratos.fundamentacao` (name): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): range, sort
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 70 contratos ≥ 1 M€ por urgência imperiosa em 2020; maiores: Direção-Geral da Saúde [Grande Lisboa] 19 680 000,00 €; Portos dos Açores, SA [Região Autónoma dos Açores] 18 626 874,00 €; Secretaria Regional do Turismo, Mobilidade e Infraestruturas [Região Autónoma dos Açores] 17 500 000,00 €; Direção-Geral da Saúde [Grande Lisboa] 13 800 000,00 €; Direção-Geral da Saúde [Grande Lisboa] 11 484 105,80 €; adjudicatários dos 10 maiores PME 6/9

### 25. No procedimento com mais contratos lançado por uma entidade, que adjudicatário recebeu a maior fatia do valor total, é PME e quanto ganhou no total?

**Instanciação:** [Serviços Partilhados do Ministério da Saúde (SPMS), NIF 509540716]  
**Id:** `b14` · **R = 3, I = 3** (procedimentos, contratos, fornecedores) · Valores

**Plano:**

1. `rucene-base procedimentos search $R --term adjudicanteNif=509540716 --sort numContratos:desc --limit 1 --fields idprocedimento,numContratos,valorTotal,precoBaseProcedimento` — exit 0, 1721 resultado(s)
2. `rucene-base contratos search $R --term 'idprocedimento=<idprocedimento do passo 1>' --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicatarioNif,precoContratual` — exit 0, 70 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif da 1.ª linha do passo 2>` — exit 0, 1 resultado(s)

**Porque R não é menor:** O agregado por procedimento está em `procedimentos`, a repartição por adjudicatário em `contratos` e o historial em `fornecedores`; cada passo depende do anterior.

**Combinação no cliente:** Rácio entre o maior preço contratual e o `valorTotal` do procedimento.

**Tipos de campo e operações:**

* `procedimentos.adjudicanteNif` (exact, multivalor): term
* `procedimentos.numContratos` (int): sort
* `contratos.idprocedimento` (exact): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `procedimentos`: idprocedimento (exact), numContratos (int), valorTotal (double), precoBaseProcedimento (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** procedimento 11049488 (70 contratos, 479 856,20 €, preço base 1 413 011,82 €); maior fatia: 504615947 com 33 082,47 € = 6,9 %; esse adjudicatário: PME sim, 10398 contratos e 1 181 815 906,89 € no total

### 26. Qual a região NUTS III com maior valor contratado, quais os maiores contratos aí executados num ano e os seus adjudicatários são PME?

**Instanciação:** [exclui PTZZZ (Extra-Regio); ano 2025; 5 maiores]  
**Id:** `b17` · **R = 3, I = 3** (nuts, contratos, fornecedores) · Localização

**Plano:**

1. `rucene-base nuts search $R --not --term nuts=PTZZZ --sort valorTotal:desc --limit 1 --fields nuts,descricao,valorTotal` — exit 0, 287 resultado(s)
2. `rucene-base contratos search $R --term 'nuts=<nuts do passo 1>' --term Ano=2025 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual` — exit 0, 66796 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 6 resultado(s)

**Porque R não é menor:** O valor agregado por região está em `nuts`, os contratos em `contratos`, o atributo PME em `fornecedores`.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `nuts.nuts` (keyword, chave): --not term
* `nuts.valorTotal` (double): sort
* `contratos.nuts` (keyword, multivalor): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `nuts`: nuts (keyword), descricao (name), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** PT170 Área Metropolitana de Lisboa (15 495 481 771,10 €); 66796 contratos executados aí em 2025; maiores: 12020914 Instituto Registos Notariado, I.p. 42 000 000,00 €; 12185333 MARINHA - Direção de Abastecimento 41 400 000,00 €; 11577383 Instituto do Turismo de Portugal, IP 40 000 000,00 €; 11393908 Município de Lisboa 31 147 209,54 €; 11176543 DGV - Direcção Geral de Veterinária 29 947 140,00 €; adjudicatários PME 6/6

### 27. Quais os concelhos de um distrito com maior valor contratado, qual o maior contrato executado no primeiro e quantas modificações teve?

**Instanciação:** [distrito «Portugal, Faro»; 5 concelhos]  
**Id:** `b18` · **R = 3, I = 3** (locais, contratos, modificacoes) · Localização

**Plano:**

1. `rucene-base locais search $R --term 'localNivel2=Portugal, Faro' --sort valorTotal:desc --limit 5 --fields local,concelho,numContratos,valorTotal` — exit 0, 17 resultado(s)
2. `rucene-base contratos search $R --term 'localExecucao=<local do passo 1>' --sort precoContratual:desc --limit 1 --fields idcontrato,adjudicanteNome,precoContratual` — exit 0, 1585 resultado(s)
3. `rucene-base modificacoes search $R --term 'idcontrato=<idcontrato do passo 2>' --sort sequencia:asc --limit 100 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 18 resultado(s)

**Porque R não é menor:** Agregado por local (`locais`), contrato (`contratos`) e modificações (`modificacoes`) em cadeia.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `locais.localNivel2` (keyword, nível derivado): term
* `locais.valorTotal` (double): sort
* `contratos.localExecucao` (keyword, multivalor): term
* `contratos.precoContratual` (double): sort
* `modificacoes.idcontrato` (exact, chave): term
* `modificacoes.sequencia` (int, chave): sort
* saída `--fields` de `locais`: local (keyword), concelho (name), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)

**Resposta (validada):** concelhos de maior valor: Faro 2 038 086 483,72 €, Loulé 971 511 197,09 €, Portimão 706 124 392,48 €; maior contrato em Faro: 7778715 Infraestruturas de Portugal, SA 20 452 244,08 €, 18 modificação(ões) (última: «Cláusulas contratuais que indiquem de forma clara, precisa e inequívoca o âmbito e a natureza das eventuais modificações, bem como as condições em que podem ser aplicadas (art. 312.º, a) do CCP)»)

*Zero significativo:* zero modificações seria uma resposta válida («nenhuma»).

### 28. Qual o código CPV com maior valor dentro de uma classe, quais os maiores contratos de um ano com esse CPV principal e os adjudicatários são PME?

**Instanciação:** [classe CPV 3314 (material médico de consumo); ano 2025; 3 maiores]  
**Id:** `b19` · **R = 3, I = 3** (cpv, contratos, fornecedores) · Objeto (CPV)

**Plano:**

1. `rucene-base cpv search $R --term cpvClasse=3314 --sort valorTotal:desc --limit 1 --fields cpv,descricao,valorTotal` — exit 0, 79 resultado(s)
2. `rucene-base contratos search $R --term 'cpvPrincipal=<cpv do passo 1>' --term Ano=2025 --sort precoContratual:desc --limit 3 --exact-total --fields idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual` — exit 0, 12987 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 3 resultado(s)

**Porque R não é menor:** O valor por CPV é agregado de `cpv`, os contratos estão em `contratos`, o atributo PME em `fornecedores`.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `cpv.cpvClasse` (keyword, nível derivado): term
* `cpv.valorTotal` (double): sort
* `contratos.cpvPrincipal` (keyword): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 33140000-3 «Material médico de consumo» (1 804 363 669,67 €); 12987 contratos em 2025 com esse CPV principal; maiores: Unidade Local de Saúde de Lisboa Ocidental, EPE 3 300 000,00 €; Unidade Local de Saúde de Lisboa Ocidental, EPE 2 849 950,00 €; Unidade Local de Saúde do Tâmega e Sousa, EPE 2 181 450,00 €; adjudicatários PME 3/3

### 29. Quantos contratos por ajuste direto de um ano têm preço exatamente no limiar legal (ou um cêntimo abaixo), quantos desses adjudicatários são PME e que entidades os celebraram?

**Instanciação:** [Ajuste Direto Regime Geral; preço 19 999,99 € ou 20 000 €; ano 2025; 20 contratos]  
**Id:** `b21` · **R = 3, I = 3** (contratos, fornecedores, adjudicantes) · Valores

**Plano:**

1. `rucene-base contratos search $R --terms precoContratual=19999.99,20000 --term 'tipoprocedimento=Ajuste Direto Regime Geral' --term Ano=2025 --limit 20 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 317 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 19 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 18 resultado(s)

**Porque R não é menor:** O atributo PME só existe em `fornecedores` e os nomes das entidades em `adjudicantes`.

**Combinação no cliente:** Contagem de PME; nomes distintos das entidades.

**Tipos de campo e operações:**

* `contratos.precoContratual` (double): terms
* `contratos.tipoprocedimento` (keyword): term
* `contratos.Ano` (int): term
* `fornecedores.nif` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 317 contratos por ajuste direto em 2025 com preço exatamente 19 999,99 € ou 20 000 €; nos 20 listados, 19 adjudicatários distintos, 19 PME; 18 entidades, p.ex. Freguesia de Corroios, Município de Penalva do Castelo, Município do Seixal

### 30. Que objetos (descrição CPV) têm os maiores ajustes diretos de um ano com uma dada fundamentação do ajuste direto, e que entidades os celebraram?

**Instanciação:** [fundamentação do ajuste direto «outros»; ano 2025; 10 maiores]  
**Id:** `b24` · **R = 3, I = 3** (contratos, cpv, adjudicantes) · Procedimento e fundamento

**Plano:**

1. `rucene-base contratos search $R --term fundamentAjusteDireto=outros --term Ano=2025 --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,adjudicanteNif,cpvPrincipal,precoContratual` — exit 0, 12 resultado(s)
2. `rucene-base cpv get $R --key <cpvPrincipal do passo 1> …` — exit 0, 6 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** A descrição canónica de cada CPV está em `cpv` e os nomes das entidades em `adjudicantes`.

**Combinação no cliente:** Emparelha contrato, descrição e entidade por código e NIF.

**Tipos de campo e operações:**

* `contratos.fundamentAjusteDireto` (keyword): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `cpv.cpv` (keyword, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), cpvPrincipal (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 12 ajustes diretos de 2025 fundamentados em «outros»; maiores: 1 403 051,03 € CPV 45453100-8 «Obras de recuperação» (Santa Casa da Misericórdia de Albufeira); 885 728,34 € CPV 45000000-7 «Construção» (Sociedade Recreativa e Musical da Pedreira); 225 000,00 € CPV 92370000-5 «Técnico de som» (MUNICÍPIO DE VILA NOVA DE FAMALICÃO); 209 000,00 € CPV 92370000-5 «Técnico de som» (MUNICÍPIO DE VILA NOVA DE FAMALICÃO)

### 31. Que códigos CPV descrevem um dado tipo de bem ou serviço, quantos contratos de um ano os usaram e onde estão sediadas as entidades dos maiores?

**Instanciação:** [palavra «refeições» na descrição CPV; ano 2025; 5 maiores]  
**Id:** `b29` · **R = 3, I = 3** (cpv, contratos, adjudicantes) · Objeto (CPV)

**Plano:**

1. `rucene-base cpv search $R --text-in descricao=refeições --sort numContratos:desc --limit 50 --fields cpv,descricao,numContratos` — exit 0, 23 resultado(s)
2. `rucene-base contratos search $R --terms 'cpv=<cpv do passo 1>' --term Ano=2025 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNif,adjudicanteNome,precoContratual` — exit 0, 1749 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** Vocabulário CPV (`cpv`), contratos (`contratos`) e sede das entidades (`adjudicantes`) em cadeia.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `cpv.descricao` (name): text-in
* `cpv.numContratos` (int): sort
* `contratos.cpv` (keyword, multivalor): terms
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numContratos (int)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicanteNome (name), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 23 códigos CPV com «refeições» (55520000-1, 55524000-9, 55500000-5, 55322000-3…); 1749 contratos de 2025; maiores: Município de Lisboa 31 147 209,54 € [Grande Lisboa]; Município de Oeiras 15 237 519,80 € [Grande Lisboa]; CP - Comboios de Portugal, EPE 12 045 000,00 € [Grande Lisboa]

### 32. Que códigos CPV são mais usados como código complementar e que entidades os usam sem os declarar como principal?

**Instanciação:** [os 5 primeiros por `numComplementar`; 20 contratos mais recentes]  
**Id:** `b30` · **R = 3, I = 3** (cpv, contratos, adjudicantes) · Objeto (CPV)

**Plano:**

1. `rucene-base cpv search $R --sort numComplementar:desc --limit 5 --fields cpv,descricao,numComplementar,numPrincipal` — exit 0, 7669 resultado(s)
2. `rucene-base contratos search $R --terms 'cpv=<cpv do passo 1>' --not --terms 'cpvPrincipal=<cpv do passo 1>' --sort dataPublicacao:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,cpv,cpvPrincipal` — exit 0, 1425 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 2 resultado(s)

**Porque R não é menor:** Agregado complementar (`cpv`), contratos e nomes de entidade estão em três índices encadeados.

**Combinação no cliente:** Nomes distintos das entidades dos contratos devolvidos.

**Tipos de campo e operações:**

* `cpv.numComplementar` (int): sort
* `contratos.cpv` (keyword, multivalor): terms
* `contratos.cpvPrincipal` (keyword): --not terms
* `contratos.dataPublicacao` (date): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numComplementar (int), numPrincipal (int)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), cpv (keyword), cpvPrincipal (keyword)
* opções: contratos: exact-total

**Resposta (validada):** CPV mais usados como complementares: 98341000-5 «Serviços de alojamento» (1094), 60400000-2 «Serviços de transporte aéreo» (801), 33000000-0 «Equipamento médico, medicamentos e produtos para cuidados pessoais» (471); 1425 contratos em que surgem só como complementares; entidades recentes: A. R. M. - Águas e Resíduos da Madeira, SA, Estado Maior da Força Aérea

### 33. Que anúncios publicados desde uma data usaram os códigos CPV mais frequentes de uma categoria, com que preço base, e onde estão sediadas as entidades?

**Instanciação:** [categoria CPV 33141 (material médico descartável); 5 códigos mais frequentes; desde 2025-01-01; 5 maiores preços base]  
**Id:** `b32` · **R = 3, I = 3** (cpv, anuncios, adjudicantes) · Anúncios

**Plano:**

1. `rucene-base cpv search $R --term cpvCategoria=33141 --sort numContratos:desc --limit 5 --fields cpv,descricao,numContratos` — exit 0, 78 resultado(s)
2. `rucene-base anuncios search $R --terms 'cpv=<cpv do passo 1>' --range dataPublicacao=2025-01-01.. --sort PrecoBase:desc --limit 5 --exact-total --fields IDAnuncio,PrecoBase,designacaoEntidade,nifEntidade` — exit 0, 94 resultado(s)
3. `rucene-base adjudicantes search $R --terms 'nif=<nifEntidade do passo 2>' --fields nif,nome,nutsDesc` — exit 0, 3 resultado(s)

**Porque R não é menor:** Frequência por código (`cpv`), anúncios (`anuncios`) e sede (`adjudicantes`) em cadeia.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `cpv.cpvCategoria` (keyword, nível derivado): term
* `cpv.numContratos` (int): sort
* `anuncios.cpv` (keyword, multivalor): terms
* `anuncios.dataPublicacao` (date): range
* `anuncios.PrecoBase` (double, ausente=0): sort
* `adjudicantes.nif` (exact, chave): terms
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numContratos (int)
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double), designacaoEntidade (name), nifEntidade (exact)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), nutsDesc (text)
* opções: anuncios: exact-total

**Resposta (validada):** CPV da categoria 33141: 33141000-0, 33141200-2, 33141110-4, 33141700-7, 33141100-1; 94 anúncios desde 2025; maiores preços base: 459519 1 146 875,50 € (Unidade Local de Saúde de Gaia/Espinho, E. P. E.); 459515 850 909,47 € (Unidade Local de Saúde de Gaia/Espinho, E. P. E.); 425349 666 622,83 € (Unidade Local de Saúde do Alentejo Central, EPE); 459513 621 645,33 € (Unidade Local de Saúde de Gaia/Espinho, E. P. E.); 451604 556 500,00 € (Unidade Local de Saúde do Oeste, EPE); sede das entidades: Grande Lisboa, Área Metropolitana do Porto

### 34. Que local de execução corresponde exatamente a um concelho, qual o maior contrato aí executado num ano e quantas modificações teve?

**Instanciação:** [concelho «Lagos» (termo exato: a pesquisa por palavras também apanharia «Lagoa», mesma raiz); ano 2025]  
**Id:** `b34` · **R = 3, I = 3** (locais, contratos, modificacoes) · Localização

**Plano:**

1. `rucene-base locais search $R --term concelho=Lagos --fields local,numContratos,valorTotal` — exit 0, 1 resultado(s)
2. `rucene-base contratos search $R --term 'localExecucao=<local do passo 1>' --term Ano=2025 --sort precoContratual:desc --limit 1 --fields idcontrato,adjudicanteNome,precoContratual` — exit 0, 723 resultado(s)
3. `rucene-base modificacoes search $R --term 'idcontrato=<idcontrato do passo 2>' --limit 100 --fields idcontrato,sequencia` — exit 0, 0 resultado(s)

**Porque R não é menor:** Partes do local (`locais`), contrato (`contratos`) e modificações (`modificacoes`) em cadeia.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `locais.concelho` (name): term
* `contratos.localExecucao` (keyword, multivalor): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `modificacoes.idcontrato` (exact, chave): term
* saída `--fields` de `locais`: local (keyword), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int)

**Resposta (validada):** local Portugal, Faro, Lagos (12758 contratos, 596 432 282,09 €); maior contrato de 2025: 11822184 MUNICIPIO DE LAGOS 14 813 727,74 €; 0 modificação(ões)

*Zero significativo:* zero modificações é uma resposta válida («nenhuma»).

### 35. Que locais de execução fora do país concentram mais contratos, qual o maior contrato executado no primeiro e que entidade o celebrou (sede e tipo)?

**Instanciação:** [nível 1 do local diferente de «Portugal»]  
**Id:** `b38` · **R = 3, I = 3** (locais, contratos, adjudicantes) · Localização

**Plano:**

1. `rucene-base locais search $R --not --term localNivel1=Portugal --sort numContratos:desc --limit 10 --fields local,numContratos,valorTotal` — exit 0, 143 resultado(s)
2. `rucene-base contratos search $R --term 'localExecucao=<local do passo 1>' --sort precoContratual:desc --limit 1 --fields idcontrato,adjudicanteNif,precoContratual` — exit 0, 1011 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 1 resultado(s)

**Porque R não é menor:** Agregados por local (`locais`), contrato (`contratos`) e entidade (`adjudicantes`) em cadeia.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `locais.localNivel1` (keyword, nível derivado): --not term
* `locais.numContratos` (int): sort
* `contratos.localExecucao` (keyword, multivalor): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `locais`: local (keyword), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), precoContratual (double)

**Resposta (validada):** locais fora de Portugal com mais contratos: Espanha (1023), Bélgica (811), França (765), Alemanha (672); maior contrato em Espanha: 1906899 22 200 000,00 €; entidade: Infraestruturas de Portugal, SA [Área Metropolitana de Lisboa, Empresa pública]

### 36. Quais os contratos de uma entidade com mais modificações, qual o fundamento da última modificação de cada um e o adjudicatário é PME?

**Instanciação:** [Município de Lisboa, NIF 500051070; 3 contratos]  
**Id:** `c04` · **R = 3, I = 3** (contratos, modificacoes, fornecedores) · Modificações

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=500051070 --sort numModificacoes:desc --limit 3 --fields idcontrato,numModificacoes,adjudicatarioNif,precoContratual` — exit 0, 4759 resultado(s)
2. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --sort sequencia:desc --limit 200 --fields idcontrato,sequencia,modifContratoFundamento,modifContratoData` — exit 0, 36 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 3 resultado(s)

**Porque R não é menor:** Modificações e atributo PME estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Primeira linha por contrato na ordem `sequencia` decrescente = última modificação.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.numModificacoes` (int): sort
* `modificacoes.idcontrato` (exact, chave): terms
* `modificacoes.sequencia` (int, chave): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), numModificacoes (int), adjudicatarioNif (exact), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name), modifContratoData (date)

**Resposta (validada):** 7632662 (17 modif.; última: «Alteração anormal e imprevisível das circunstâncias (Art. 312.º, al. b) do CCP)» em 2026-10-26; adjudicatário 500197814 PME sim); 7594154 (12 modif.; última: «Trabalhos Complementares (art. 370.º do CCP)» em 2026-09-08; adjudicatário 500023875 PME sim); 7672100 (7 modif.; última: «Cláusulas contratuais que indiquem de forma clara, precisa e inequívoca o âmbito e a natureza das eventuais modificações, bem como as condições em que podem ser aplicadas (art. 312.º, a) do CCP)» em 2026-07-01; adjudicatário 500963363 PME sim)

### 37. Que modificações não foram reduzidas a escrito com uma dada justificação, e de que contratos e entidades são?

**Instanciação:** [justificação com a expressão «urgência imperiosa»; 10 maiores por preço alterado]  
**Id:** `c05` · **R = 3, I = 3** (modificacoes, contratos, adjudicantes) · Modificações

**Plano:**

1. `rucene-base modificacoes search $R --phrase-in 'modifJustNContratEscrito=urgência imperiosa' --sort modifContratoPrecoAlterado:desc --limit 10 --exact-total --fields idcontrato,sequencia,modifContratoPrecoAlterado` — exit 0, 408 resultado(s)
2. `rucene-base contratos get $R --key <idcontrato do passo 1> …` — exit 0, 6 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif dos contratos do passo 2> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** Cadeia modificação → contrato → entidade, um índice por passo.

**Combinação no cliente:** Emparelhamento por ids.

**Tipos de campo e operações:**

* `modificacoes.modifJustNContratEscrito` (name): phrase-in
* `modificacoes.modifContratoPrecoAlterado` (double): sort
* `contratos.idcontrato` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoPrecoAlterado (double)
* opções: modificacoes: exact-total

**Resposta (validada):** 408 modificações não reduzidas a escrito por urgência imperiosa; maiores: 25 787 422,12 € em 3980601 (Gestão e Obras do Porto, EM); 25 771 271,58 € em 3980601 (Gestão e Obras do Porto, EM); 19 091 935,36 € em 7288568 (Portos dos Açores, SA); 14 618 934,78 € em 6387034 (Portos dos Açores, SA)

### 38. Que contratos foram modificados por decisão judicial ou arbitral, e por que entidades foram celebrados?

**Instanciação:** [tipo de ato «Decisão Judicial» ou «Decisão Arbitral»; 20 mais recentes]  
**Id:** `c06` · **R = 3, I = 3** (modificacoes, contratos, adjudicantes) · Modificações

**Plano:**

1. `rucene-base modificacoes search $R --terms 'modifContratoTipoAto=Decisão Judicial,Decisão Arbitral' --sort modifContratoData:desc --limit 20 --exact-total --fields idcontrato,sequencia,modifContratoTipoAto,modifContratoData` — exit 0, 159 resultado(s)
2. `rucene-base contratos get $R --key <idcontrato do passo 1> …` — exit 0, 20 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif dos contratos do passo 2> …` — exit 0, 19 resultado(s)

**Porque R não é menor:** Cadeia modificação → contrato → entidade, um índice por passo.

**Combinação no cliente:** Contagem por tipo de ato; nomes distintos das entidades.

**Tipos de campo e operações:**

* `modificacoes.modifContratoTipoAto` (keyword): terms
* `modificacoes.modifContratoData` (date): sort
* `contratos.idcontrato` (exact, chave): get
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoTipoAto (keyword), modifContratoData (date)
* opções: modificacoes: exact-total

**Resposta (validada):** 159 modificações por decisão judicial ou arbitral; nas 20 mais recentes: {'Decisão Arbitral': 8, 'Decisão Judicial': 12}; entidades: ARCOR - Associação Recreativa e Cultural de Ois da Ribeira, CFPIMM - Centro de Formação Profissional das Indústrias da Madeira e Mobiliário, CMPEAE - Empresa de Águas e Energia do Município do Porto, EM, Edia - Empresa de Desenvolvimento e Infra-Estruturas do Alqueva, SA, Instituto do Emprego e da Formação Profissional, IP

### 39. Que contratos referem uma dada causa nas razões de alteração do preço, com que fundamentos legais foram modificados e por que entidades?

**Instanciação:** [palavra «covid» em `CausasAlteraPreco`; 20 maiores]  
**Id:** `c10` · **R = 3, I = 3** (contratos, modificacoes, adjudicantes) · Modificações

**Plano:**

1. `rucene-base contratos search $R --text-in CausasAlteraPreco=covid --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,precoContratual` — exit 0, 2880 resultado(s)
2. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --limit 300 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 10 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 18 resultado(s)

**Porque R não é menor:** Modificações e entidades estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Contagem de fundamentos nas linhas do passo 2.

**Tipos de campo e operações:**

* `contratos.CausasAlteraPreco` (text, ausente=n/a, N/A, ., -): text-in
* `contratos.precoContratual` (double): sort
* `modificacoes.idcontrato` (exact, chave): terms
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), precoContratual (double)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)
* opções: contratos: exact-total
* `CausasAlteraPreco` não indexa os marcadores `n/a`, `N/A`, `.`, `-`.

**Resposta (validada):** 2880 contratos com «covid» nas causas de alteração do preço; fundamentos das modificações dos 20 maiores: Trabalhos Complementares (art. 370.º do CCP) (4); Interesse público decorrentes de necessidades novas ou de uma nova ponderação das circunstâncias existentes (art. 312.º, al. b) do CCP) (3); entidades: AEVA - Associação para a Educação e Valorização da Região de Aveiro, Agência de Gestão da Tesouraria e da Dívida Pública - IGCP, EPE, INSTITUTO POLITÉCNICO DE BEJA, Instituto de Medicina Molecular

### 40. Que entidades anunciaram sistemas de aquisição dinâmicos e para que objetos (descrição CPV)?

**Instanciação:** [sistema de aquisição dinâmico = sim; todos os anúncios (até 200)]  
**Id:** `c11` · **R = 3, I = 3** (anuncios, adjudicantes, cpv) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term sistemaAquisicaoDinamico=sim --sort dataPublicacao:desc --limit 200 --exact-total --fields IDAnuncio,nifEntidade,cpv` — exit 0, 156 resultado(s)
2. `rucene-base adjudicantes search $R --terms 'nif=<nifEntidade do passo 1>' --sort nome:asc --limit 200 --fields nif,nome` — exit 0, 5 resultado(s)
3. `rucene-base cpv search $R --terms 'cpv=<cpv do passo 1>' --sort numContratos:desc --limit 200 --fields cpv,descricao,numContratos` — exit 0, 48 resultado(s)

**Porque R não é menor:** Entidades e vocabulário CPV estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `anuncios.sistemaAquisicaoDinamico` (flag): term
* `anuncios.dataPublicacao` (date): sort
* `adjudicantes.nif` (exact, chave): terms
* `adjudicantes.nome` (name, multivalor): sort
* `cpv.cpv` (keyword, chave): terms
* `cpv.numContratos` (int): sort
* saída `--fields` de `anuncios`: IDAnuncio (exact), nifEntidade (exact), cpv (keyword)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name)
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numContratos (int)
* opções: anuncios: exact-total

**Resposta (validada):** 156 sistemas de aquisição dinâmicos anunciados por 5 entidades (p.ex. Associação Empresarial de Fafe, Cabeceiras de Basto e Celorico de Basto, EAS - Empresa de Ambiente na Saúde, Unipessoal, Lda, Serviços Partilhados do Ministério da Saúde, EPE); objetos mais frequentes no país: 33600000-6 «Produtos farmacêuticos»; 33000000-0 «Equipamento médico, medicamentos e produtos para cuidados pessoais»; 33100000-1 «Equipamento médico»

### 41. Dos anúncios de acordos-quadro de um ano com maior preço base, que contratos resultaram e quantos adjudicatários são PME?

**Instanciação:** [acordo-quadro = sim no anúncio; ano 2025; 20 maiores preços base]  
**Id:** `c12` · **R = 3, I = 3** (anuncios, contratos, fornecedores) · Acordos-quadro

**Plano:**

1. `rucene-base anuncios search $R --term AcordoQuadro=sim --term Ano=2025 --sort PrecoBase:desc --limit 20 --exact-total --fields IDAnuncio,PrecoBase` — exit 0, 227 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 100 --exact-total --fields idcontrato,adjudicatarioNif,precoContratual` — exit 0, 57 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 55 resultado(s)

**Porque R não é menor:** Cadeia anúncio → contrato → fornecedor, um índice por passo.

**Combinação no cliente:** Contagem dos contratos com preço 0 e de PME.

**Tipos de campo e operações:**

* `anuncios.AcordoQuadro` (flag): term
* `anuncios.Ano` (int): term
* `anuncios.PrecoBase` (double, ausente=0): sort
* `contratos.IDAnuncio` (exact): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: anuncios: exact-total; contratos: exact-total

**Resposta (validada):** 227 anúncios de acordo-quadro em 2025; dos 20 de maior preço base resultaram 57 contratos, 57 deles com preço contratual 0 (acordo-quadro sem valor próprio); adjudicatários PME 54/55

### 42. Que contratos resultaram de anúncios com um fator de adjudicação de sustentabilidade, e os adjudicatários são PME?

**Instanciação:** [fator de aquisição «Sustentabilidade ambiental ou social» no anúncio]  
**Id:** `c14` · **R = 3, I = 3** (anuncios, contratos, fornecedores) · Sustentabilidade

**Plano:**

1. `rucene-base anuncios search $R --term 'criterioAquisicaoTipoFator=Sustentabilidade ambiental ou social' --limit 100 --exact-total --fields IDAnuncio` — exit 0, 46 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 200 --fields idcontrato,adjudicatarioNif,precoContratual` — exit 0, 173 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 52 resultado(s)

**Porque R não é menor:** Cadeia anúncio → contrato → fornecedor, um índice por passo.

**Combinação no cliente:** Soma dos preços; contagem de PME.

**Tipos de campo e operações:**

* `anuncios.criterioAquisicaoTipoFator` (keyword, multivalor): term
* `contratos.IDAnuncio` (exact): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: anuncios: exact-total

**Resposta (validada):** 46 anúncios com fator de sustentabilidade ambiental ou social; 173 contratos resultantes; adjudicatários PME 52/52; total 30 834 753,36 €

### 43. Que contratos resultaram dos anúncios de um ano sobre um tema (pesquisa livre), e os adjudicatários são PME?

**Instanciação:** [palavras «requalificação escola» nos campos de texto do anúncio; ano 2025]  
**Id:** `c16` · **R = 3, I = 3** (anuncios, contratos, fornecedores) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --text 'requalificação escola' --term Ano=2025 --sort _score --limit 100 --exact-total --fields IDAnuncio,descricaoAnuncio` — exit 0, 184 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 200 --fields idcontrato,adjudicatarioNif,precoContratual` — exit 0, 45 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 42 resultado(s)

**Porque R não é menor:** Cadeia anúncio → contrato → fornecedor, um índice por passo.

**Combinação no cliente:** Soma dos preços; contagem de PME.

**Tipos de campo e operações:**

* `anuncios.descricaoAnuncio` (text): text
* `anuncios.designacaoEntidade` (name, multivalor): text
* `anuncios.cpvDescricao` (text, multivalor): text
* `anuncios.Ano` (int): term
* `contratos.IDAnuncio` (exact): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `anuncios`: IDAnuncio (exact), descricaoAnuncio (text)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: anuncios: sort _score; anuncios: exact-total

**Resposta (validada):** 184 anúncios de 2025 sobre requalificação de escolas; 45 contratos resultantes (84 005 296,36 €); adjudicatários PME 41/42

### 44. Que entidades adjudicaram contratos com um fator de circularidade, e o que compraram (descrição CPV)?

**Instanciação:** [fator de adjudicação «Circularidade»]  
**Id:** `c23` · **R = 3, I = 3** (contratos, adjudicantes, cpv) · Sustentabilidade

**Plano:**

1. `rucene-base contratos search $R --term criterioAdjudicacaoTipoFator=Circularidade --sort precoContratual:desc --limit 50 --exact-total --fields idcontrato,adjudicanteNif,cpvPrincipal,precoContratual` — exit 0, 31 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 12 resultado(s)
3. `rucene-base cpv get $R --key <cpvPrincipal do passo 1> …` — exit 0, 18 resultado(s)

**Porque R não é menor:** Entidades e CPV estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Nomes e descrições distintos.

**Tipos de campo e operações:**

* `contratos.criterioAdjudicacaoTipoFator` (keyword, multivalor): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), cpvPrincipal (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 31 contratos com o fator «Circularidade»; entidades: Agrupamento de Escolas Dr. Ferreira da Silva, Oliveira de Azeméis, Agrupamento de Escolas de Azeitão, Setúbal, Freguesia de Pinhal Novo, Freguesia de Silveira; CPV: Obras de recuperação; Serviços de arquitectura e afins; Serviços de auditoria; Serviços de manutenção de jardins ornamentais e recreativos

### 45. Nos maiores contratos de uma divisão CPV num ano, quantos adjudicatários distintos são PME e que objetos (descrição CPV) têm?

**Instanciação:** [divisão CPV 45; ano 2025; 20 maiores]  
**Id:** `c24` · **R = 3, I = 3** (contratos, fornecedores, cpv) · PME

**Plano:**

1. `rucene-base contratos search $R --term cpvDivisao=45 --term Ano=2025 --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,adjudicatarioNif,cpvPrincipal,precoContratual` — exit 0, 16198 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 20 resultado(s)
3. `rucene-base cpv get $R --key <cpvPrincipal do passo 1> …` — exit 0, 16 resultado(s)

**Porque R não é menor:** O atributo PME está em `fornecedores` e a descrição dos códigos em `cpv`, ambos a partir do passo 1.

**Combinação no cliente:** Contagem de PME entre os adjudicatários distintos.

**Tipos de campo e operações:**

* `contratos.cpvDivisao` (keyword, nível derivado): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), cpvPrincipal (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 16198 contratos de construção em 2025; nos 20 maiores, 20 adjudicatários distintos, 19 PME; CPV: Construção de vias férreas; Locomotivas e material circulante para vias férreas e respectivas peças; Construção; Obras de construção de edifícios relacionados com alojamento

### 46. Que entidades celebraram os maiores contratos resultantes de procedimentos centralizados num ano, onde estão sediadas, e os adjudicatários são PME?

**Instanciação:** [procedimento centralizado = sim; ano 2025; 15 maiores]  
**Id:** `c26` · **R = 3, I = 3** (contratos, adjudicantes, fornecedores) · Compras centralizadas e conjuntas

**Plano:**

1. `rucene-base contratos search $R --term ProcedimentoCentralizado=sim --term Ano=2025 --sort precoContratual:desc --limit 15 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,precoContratual` — exit 0, 14519 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 11 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 11 resultado(s)

**Porque R não é menor:** A sede das entidades só está em `adjudicantes` e o atributo PME em `fornecedores`, ambos a partir do passo 1.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `contratos.ProcedimentoCentralizado` (flag): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 14519 contratos de 2025 por procedimento centralizado; maiores entidades: Instituto Nacional de Emergência Médica, IP (Grande Lisboa) 77 475 160,00 €; Unidade Local de Saúde de Santo António, EPE (Grande Lisboa) 29 416 808,00 €; Unidade Local de Saúde de Santo António, EPE (Grande Lisboa) 18 382 386,40 €; Unidade Local de Saúde de Santa Maria, EPE (Área Metropolitana de Lisboa) 17 887 765,32 €; adjudicatários PME 11/11

### 47. Que anúncios conjuntos subscreveram duas entidades, que outras entidades os subscreveram (e de que regiões), e que contratos resultaram?

**Instanciação:** [Águas do Algarve, NIF 505176300, e AgdA – Águas Públicas do Alentejo, NIF 509133843; todos os anúncios conjuntos (até 100)]  
**Id:** `c28` · **R = 3, I = 3** (anuncios, adjudicantes, contratos) · Compras centralizadas e conjuntas

**Plano:**

1. `rucene-base anuncios search $R --term nifEntidade=505176300 --term nifEntidade=509133843 --sort dataPublicacao:desc --limit 100 --exact-total --fields IDAnuncio,nifEntidade,dataPublicacao` — exit 0, 68 resultado(s)
2. `rucene-base adjudicantes search $R --terms 'nif=<nifEntidade do passo 1>' --limit 100 --fields nif,nome,nutsCod` — exit 0, 18 resultado(s)
3. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 200 --exact-total --fields idcontrato,adjudicanteNif,precoContratual` — exit 0, 32 resultado(s)

**Porque R não é menor:** Entidades e contratos estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Contagem de entidades e regiões distintas; soma dos preços.

**Tipos de campo e operações:**

* `anuncios.nifEntidade` (exact, multivalor, ausente=-): term
* `anuncios.dataPublicacao` (date): sort
* `adjudicantes.nif` (exact, chave): terms
* `contratos.IDAnuncio` (exact): terms
* saída `--fields` de `anuncios`: IDAnuncio (exact), nifEntidade (exact), dataPublicacao (date)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), nutsCod (keyword)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), precoContratual (double)
* opções: anuncios: exact-total; contratos: exact-total
* Duas cláusulas `--term` no mesmo campo multivalor exigem ambos os valores no mesmo documento.

**Resposta (validada):** 68 anúncios subscritos por ambas, juntando 18 entidades de 15 regiões; 32 contratos resultantes, total listado 1 395 403,10 €

### 48. Nos procedimentos ao abrigo de acordo-quadro com mais contratos, quantos adjudicatários distintos houve e quantos são PME?

**Instanciação:** [«Ao abrigo de acordo-quadro (art.º 258.º)»; 3 procedimentos com mais contratos]  
**Id:** `c29` · **R = 3, I = 3** (procedimentos, contratos, fornecedores) · Acordos-quadro

**Plano:**

1. `rucene-base procedimentos search $R --term 'tipoprocedimento=Ao abrigo de acordo-quadro (art.º 258.º)' --sort numContratos:desc --limit 3 --fields idprocedimento,numContratos,valorTotal` — exit 0, 8763 resultado(s)
2. `rucene-base contratos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --limit 1000 --fields idcontrato,adjudicatarioNif` — exit 0, 188 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 40 resultado(s)

**Porque R não é menor:** Agregado por procedimento (`procedimentos`), adjudicatários (`contratos`) e atributo PME (`fornecedores`) em cadeia.

**Combinação no cliente:** Contagem de NIF distintos e de PME.

**Tipos de campo e operações:**

* `procedimentos.tipoprocedimento` (keyword): term
* `procedimentos.numContratos` (int): sort
* `contratos.idprocedimento` (exact): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `procedimentos`: idprocedimento (exact), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact)

**Resposta (validada):** procedimentos ao abrigo de acordo-quadro (art.º 258.º) com mais contratos: 4847303 (80), 5157747 (55), 4835791 (53); 188 contratos, 41 adjudicatários distintos; adjudicatários PME 38/40

### 49. Que contratos foram celebrados ao abrigo de um acordo-quadro descrito por uma expressão, que procedimento formou o acordo-quadro e que entidades o usaram?

**Instanciação:** [expressão «Viagens e Alojamento» na descrição do acordo-quadro; 50 contratos mais recentes]  
**Id:** `c30` · **R = 3, I = 3** (contratos, procedimentos, adjudicantes) · Acordos-quadro

**Plano:**

1. `rucene-base contratos search $R --phrase-in 'DescrAcordoQuadro=Viagens e Alojamento' --sort dataPublicacao:desc --limit 50 --exact-total --fields idcontrato,numAcordoQuadro,adjudicanteNif` — exit 0, 19164 resultado(s)
2. `rucene-base procedimentos search $R --terms 'idprocedimento=<numAcordoQuadro do passo 1>' --fields idprocedimento,tipoprocedimento,adjudicanteNif` — exit 0, 1 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 13 resultado(s)

**Porque R não é menor:** Procedimento formador e entidades estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Conta números de acordo-quadro distintos e quantos existem como `idprocedimento`.

**Tipos de campo e operações:**

* `contratos.DescrAcordoQuadro` (text): phrase-in
* `contratos.dataPublicacao` (date): sort
* `procedimentos.idprocedimento` (exact, chave): terms
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), numAcordoQuadro (exact), adjudicanteNif (exact)
* saída `--fields` de `procedimentos`: idprocedimento (exact), tipoprocedimento (keyword), adjudicanteNif (exact)
* opções: contratos: exact-total
* Ligação observada (não declarada no esquema): `numAcordoQuadro` contém, nestes contratos, o `idprocedimento` do procedimento que formou o acordo-quadro.

**Resposta (validada):** 19164 contratos ao abrigo do acordo-quadro de viagens e alojamento; 1 números de acordo-quadro distintos nos 50 mais recentes; 1 correspondem a um `idprocedimento` indexado (5715196 Concurso limitado por prévia qualificação); 13 entidades compradoras distintas

### 50. Que entidades convidou uma entidade adjudicante nas suas consultas prévias de um ano, quão convidadas são no país e quais já foram adjudicatárias (PME primeiro)?

**Instanciação:** [Município de Sintra, NIF 500051062; Consulta Prévia; ano 2025; 30 mais recentes]  
**Id:** `d02` · **R = 3, I = 3** (contratos, convidadas, fornecedores) · Concorrência

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=500051062 --term 'tipoprocedimento=Consulta Prévia' --term Ano=2025 --sort dataPublicacao:desc --limit 30 --exact-total --fields idcontrato,convidadaNif` — exit 0, 268 resultado(s)
2. `rucene-base convidadas search $R --terms 'nif=<convidadaNif do passo 1>' --sort numConvites:desc --limit 200 --fields nif,numConvites` — exit 0, 102 resultado(s)
3. `rucene-base fornecedores search $R --terms 'nif=<convidadaNif do passo 1>' --sort pme:desc,valorTotal:desc --limit 200 --fields nif,pme,valorTotal` — exit 0, 95 resultado(s)

**Porque R não é menor:** Convites (agregados) e historial de adjudicação estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.tipoprocedimento` (keyword): term
* `contratos.Ano` (int): term
* `contratos.dataPublicacao` (date): sort
* `convidadas.nif` (exact, chave): terms
* `convidadas.numConvites` (int): sort
* `fornecedores.nif` (exact, chave): terms
* `fornecedores.pme` (flag): sort
* `fornecedores.valorTotal` (double): sort
* saída `--fields` de `contratos`: idcontrato (exact), convidadaNif (exact)
* saída `--fields` de `convidadas`: nif (exact), numConvites (int)
* saída `--fields` de `fornecedores`: nif (exact), pme (flag), valorTotal (double)
* opções: contratos: exact-total

**Resposta (validada):** 268 consultas prévias do Município de Sintra em 2025; nas 30 mais recentes, 102 entidades convidadas; mais convidadas no país: 500697370 (5236), 980079659 (5036), 503670693 (2973); 95 já adjudicatárias, PME primeiro: 500697370 PME sim, 502124083 PME sim, 980079659 PME sim

### 51. Quantas entidades começaram a ser convidadas recentemente com muitos convites, em quantos contratos do ano corrente foram convidadas, e quantas já ganharam contratos?

**Instanciação:** [primeiro ano de convite 2025 ou 2026; 20 ou mais convites; ano dos contratos 2026; 50 com mais convites]  
**Id:** `d03` · **R = 3, I = 3** (convidadas, contratos, fornecedores) · Concorrência

**Plano:**

1. `rucene-base convidadas search $R --terms primeiroAno=2025,2026 --range numConvites=20.. --sort numConvites:desc --limit 50 --exact-total --fields nif,numConvites` — exit 0, 146 resultado(s)
2. `rucene-base contratos count $R --terms 'convidadaNif=<nif do passo 1>' --term Ano=2026` — exit 0, 984 resultado(s)
3. `rucene-base fornecedores search $R --terms 'nif=<nif do passo 1>' --limit 50 --fields nif,pme,numContratos` — exit 0, 29 resultado(s)

**Porque R não é menor:** Convites em `convidadas`, contratos em `contratos`, adjudicação em `fornecedores`; os passos 2 e 3 dependem do passo 1.

**Combinação no cliente:** Contagem de PME nas linhas do passo 3.

**Tipos de campo e operações:**

* `convidadas.primeiroAno` (int): terms
* `convidadas.numConvites` (int): range, sort
* `contratos.convidadaNif` (exact, multivalor, ausente=-): terms
* `contratos.Ano` (int): term
* `fornecedores.nif` (exact, chave): terms
* saída `--fields` de `convidadas`: nif (exact), numConvites (int)
* saída `--fields` de `fornecedores`: nif (exact), pme (flag), numContratos (int)
* opções: convidadas: exact-total; contratos: count

**Resposta (validada):** 146 entidades convidadas pela primeira vez em 2025–2026 com 20+ convites; nas 50 maiores, 984 contratos de 2026 com convite; 29 já foram adjudicatárias (PME 19)

### 52. Que locais de execução encontra uma pesquisa por palavra num nome de concelho (com o efeito do stemmer), quantos contratos somam e quem celebrou os maiores?

**Instanciação:** [palavra «lagos» no concelho (encontra Lagos e Lagoa); os 3 locais devolvidos]  
**Id:** `d08` · **R = 3, I = 3** (locais, contratos, adjudicantes) · Localização

**Plano:**

1. `rucene-base locais search $R --text-in concelho=lagos --sort local:asc --fields local,numContratos` — exit 0, 3 resultado(s)
2. `rucene-base contratos search $R --should --term 'localExecucao=<1.º local do passo 1>' --should --term 'localExecucao=<2.º local do passo 1>' --should --term 'localExecucao=<3.º local do passo 1>' --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNif,localExecucao,precoContratual` — exit 0, 19715 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 2> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** Locais, contratos e nomes de entidade estão em três índices; os locais têm vírgulas e por isso são disjunção de `--should --term` (não `--terms`).

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `locais.concelho` (name): text-in
* `locais.local` (keyword, chave): sort
* `contratos.localExecucao` (keyword, multivalor): --should term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `locais`: local (keyword), numContratos (int)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), localExecucao (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** «lagos» também apanha Portugal, Faro, Lagoa, Portugal, Faro, Lagos, Portugal, Região Autónoma dos Açores, Lagoa (mesma raiz); 19715 contratos nesses locais; maiores: Infraestruturas de Portugal, SA 25 423 000,00 € (Portugal, Faro, Lagos | Portugal, Faro, Portimão | Portugal, Faro, Silves | Portugal, Faro, Lagoa); MUSAMI - Operações Municipais do Ambiente, EIM, SA 19 770 112,16 € (Portugal, Região Autónoma dos Açores, Ribeira Grande | Portugal, Região Autónoma dos Açores, Ponta Delgada | Portugal, Região Autónoma dos Açores, Vila Franca do Campo | Portugal, Região Autónoma dos Açores, Lagoa); MUNICIPIO DE LAGOS 14 813 727,74 € (Portugal, Faro, Lagos)

### 53. Dos contratos celebrados num dia concreto que foram depois modificados, com que fundamentos e por que adjudicatários (PME?)?

**Instanciação:** [data de celebração 31/12/2024 (formato DD/MM/AAAA)]  
**Id:** `d10` · **R = 3, I = 3** (contratos, modificacoes, fornecedores) · Prazos e datas

**Plano:**

1. `rucene-base contratos search $R --term dataCelebracaoContrato=31/12/2024 --range numModificacoes=1.. --limit 50 --exact-total --fields idcontrato,adjudicatarioNif,numModificacoes` — exit 0, 20 resultado(s)
2. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --limit 300 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 33 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 17 resultado(s)

**Porque R não é menor:** Modificações e atributo PME estão em índices diferentes, ambos a partir do passo 1.

**Combinação no cliente:** Fundamento mais frequente; contagem de PME.

**Tipos de campo e operações:**

* `contratos.dataCelebracaoContrato` (date): term
* `contratos.numModificacoes` (int): range
* `modificacoes.idcontrato` (exact, chave): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), numModificacoes (int)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)
* opções: contratos: exact-total

**Resposta (validada):** 20 contratos celebrados a 31/12/2024 e depois modificados; 33 modificações; fundamento mais frequente: Trabalhos Complementares (art. 370.º do CCP); adjudicatários PME 16/17

### 54. Que entidades celebraram empreitadas com prazos de execução muito longos num período, quais os prazos, e os adjudicatários são PME?

**Instanciação:** [empreitadas de obras públicas; prazo ≥ 1826 dias (o marcador 99999999 não é indexado); anos 2023 a 2025]  
**Id:** `d16` · **R = 3, I = 3** (contratos, adjudicantes, fornecedores) · Prazos e datas

**Plano:**

1. `rucene-base contratos search $R --term 'tipoContrato=Empreitadas de obras públicas' --range prazoExecucao=1826.. --range Ano=2023..2025 --sort prazoExecucao:desc --limit 10 --exact-total --fields idcontrato,adjudicanteNif,adjudicatarioNif,prazoExecucao` — exit 0, 28 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 9 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 9 resultado(s)

**Porque R não é menor:** Os nomes das entidades estão em `adjudicantes` e o atributo PME em `fornecedores`, ambos a partir do passo 1.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `contratos.tipoContrato` (keyword, multivalor): term
* `contratos.prazoExecucao` (int, ausente=99999999): range, sort
* `contratos.Ano` (int): range
* `adjudicantes.nif` (exact, chave): get
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), adjudicatarioNif (exact), prazoExecucao (int)
* opções: contratos: exact-total
* O intervalo aberto em `prazoExecucao` não apanha o marcador `99999999`, que não é indexado.

**Resposta (validada):** 28 empreitadas de 2023–2025 com prazo superior a 5 anos; maiores prazos: 17520 dias, CMPH - DomusSocial - Empresa de Habitação e Manutenção do Município do Porto, EM; 16742 dias, Câmara Municipal de Angra do Heroísmo; 16437 dias, Município de Oeiras; 10957 dias, Infraestruturas de Portugal, SA; adjudicatários PME 8/9

### 55. Nos procedimentos sobre um serviço com preço base acima de um valor, quanto foi adjudicado face ao preço base, a quantos adjudicatários, e quantos são PME?

**Instanciação:** [palavra «vigilância» no objeto do procedimento; preço base ≥ 1 000 000 €; 10 maiores preços base]  
**Id:** `d21` · **R = 3, I = 3** (procedimentos, contratos, fornecedores) · Valores

**Plano:**

1. `rucene-base procedimentos search $R --text-in objectoContrato=vigilância --range precoBaseProcedimento=1000000.. --sort precoBaseProcedimento:desc --limit 10 --exact-total --fields idprocedimento,precoBaseProcedimento,valorTotal,numContratos` — exit 0, 479 resultado(s)
2. `rucene-base contratos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --limit 500 --fields idcontrato,adjudicatarioNif` — exit 0, 65 resultado(s)
3. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 2> …` — exit 0, 12 resultado(s)

**Porque R não é menor:** Preço base e valor agregado (`procedimentos`), adjudicatários (`contratos`) e atributo PME (`fornecedores`) em cadeia.

**Combinação no cliente:** Contagem de NIF distintos e de PME; comparação base vs adjudicado.

**Tipos de campo e operações:**

* `procedimentos.objectoContrato` (text, multivalor): text-in
* `procedimentos.precoBaseProcedimento` (double, ausente=0): range, sort
* `contratos.idprocedimento` (exact): terms
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `procedimentos`: idprocedimento (exact), precoBaseProcedimento (double), valorTotal (double), numContratos (int)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact)
* opções: procedimentos: exact-total

**Resposta (validada):** 479 procedimentos de vigilância com preço base ≥ 1 M€; nos 10 maiores, 65 contratos e 12 adjudicatários distintos; maior: 6577580 base 31 290 000,00 € vs adjudicado 18 895 754,05 €; adjudicatários PME 11/12

### 56. Nos dois códigos CPV principais mais usados de uma divisão, que proporção dos contratos foi marcada como ecológica?

**Instanciação:** [divisão CPV 45 (construção); os 2 primeiros por `numPrincipal`]  
**Id:** `b33` · **R = 3, I = 2** (cpv, contratos) · Sustentabilidade

**Plano:**

1. `rucene-base cpv search $R --term cpvDivisao=45 --sort numPrincipal:desc --limit 2 --fields cpv,descricao,numPrincipal` — exit 0, 717 resultado(s)
2. `rucene-base contratos count $R --term 'cpvPrincipal=<1.º CPV do passo 1>' --term ContratEcologico=sim` — exit 0, 1046 resultado(s)
3. `rucene-base contratos count $R --term 'cpvPrincipal=<2.º CPV do passo 1>' --term ContratEcologico=sim` — exit 0, 791 resultado(s)

**Porque R não é menor:** Uma contagem por código; o denominador é o agregado `numPrincipal` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Rácio contagem/`numPrincipal`.

**Tipos de campo e operações:**

* `cpv.cpvDivisao` (keyword, nível derivado): term
* `cpv.numPrincipal` (int): sort
* `contratos.cpvPrincipal` (keyword): term
* `contratos.ContratEcologico` (flag): term
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numPrincipal (int)
* opções: contratos: count

**Resposta (validada):** contratos ecológicos por CPV principal — 45000000-7 «Construção»: 1046/19015 = 5,5 %; 45453000-7 «Obras de revisão e recuperação»: 791/12550 = 6,3 %

### 57. Que concelhos tem uma região autónoma (por ordem alfabética), e quantas empreitadas de um ano se executaram nos dois com mais contratos?

**Instanciação:** [distrito «Região Autónoma da Madeira» (pesquisa por expressão, sem o local só de distrito); empreitadas de obras públicas; ano 2025]  
**Id:** `b35` · **R = 3, I = 2** (locais, contratos) · Localização

**Plano:**

1. `rucene-base locais search $R --phrase-in 'distrito=Região Autónoma da Madeira' --not --key 'Portugal, Região Autónoma da Madeira' --sort concelho:asc --limit 20 --fields local,concelho,numContratos` — exit 0, 11 resultado(s)
2. `rucene-base contratos count $R --term 'localExecucao=<1.º concelho com mais contratos do passo 1>' --term 'tipoContrato=Empreitadas de obras públicas' --term Ano=2025` — exit 0, 68 resultado(s)
3. `rucene-base contratos count $R --term 'localExecucao=<2.º concelho com mais contratos do passo 1>' --term 'tipoContrato=Empreitadas de obras públicas' --term Ano=2025` — exit 0, 12 resultado(s)

**Porque R não é menor:** Uma contagem por concelho; a lista de concelhos vem de `locais`. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Escolhe os dois concelhos de maior `numContratos` nas linhas do passo 1.

**Tipos de campo e operações:**

* `locais.distrito` (name): phrase-in
* `locais.local` (keyword, chave): --not key
* `locais.concelho` (name): sort
* `contratos.localExecucao` (keyword, multivalor): term
* `contratos.tipoContrato` (keyword, multivalor): term
* `contratos.Ano` (int): term
* saída `--fields` de `locais`: local (keyword), concelho (name), numContratos (int)
* opções: contratos: count

**Resposta (validada):** 11 concelhos na Madeira (de Calheta a São Vicente); empreitadas de 2025: Funchal 68; Santa Cruz 12

### 58. Nos contratos de uma categoria CPV de um ano que foram modificados, que parte das modificações teve por fundamento a revisão extraordinária de preços?

**Instanciação:** [categoria CPV 45233 (obras de estradas); ano 2022; fundamento «Revisão de preços Extraordinária (D.L. n.º 36/2022, 20-05)»]  
**Id:** `c08` · **R = 3, I = 2** (contratos, modificacoes) · Modificações

**Plano:**

1. `rucene-base contratos search $R --term cpvCategoria=45233 --term Ano=2022 --range numModificacoes=1.. --limit 500 --exact-total --fields idcontrato` — exit 0, 346 resultado(s)
2. `rucene-base modificacoes count $R --terms 'idcontrato=<idcontrato do passo 1>' --term 'modifContratoFundamento=Revisão de preços Extraordinária (D.L. n.º 36/2022, 20-05)'` — exit 0, 13 resultado(s)
3. `rucene-base modificacoes count $R --terms 'idcontrato=<idcontrato do passo 1>'` — exit 0, 632 resultado(s)

**Porque R não é menor:** O CPV está em `contratos`; as modificações em `modificacoes`; duas contagens com critérios diferentes.

**Combinação no cliente:** Rácio entre as duas contagens.

**Tipos de campo e operações:**

* `contratos.cpvCategoria` (keyword, nível derivado): term
* `contratos.Ano` (int): term
* `contratos.numModificacoes` (int): range
* `modificacoes.idcontrato` (exact, chave): terms
* `modificacoes.modifContratoFundamento` (name): term
* saída `--fields` de `contratos`: idcontrato (exact)
* opções: contratos: exact-total; modificacoes: count

**Resposta (validada):** 346 contratos de obras de estradas de 2022 com modificações; nos 346 listados, 632 modificações, das quais 13 por revisão extraordinária de preços (2,1 %)

### 59. Dos contratos resultantes de anúncios com critério ambiental de um ano, quantos ficaram marcados como contrato ecológico?

**Instanciação:** [critério ambiental = sim; ano 2023]  
**Id:** `c22` · **R = 3, I = 2** (anuncios, contratos) · Sustentabilidade

**Plano:**

1. `rucene-base anuncios search $R --term CriterAmbient=sim --term Ano=2023 --limit 3000 --exact-total --fields IDAnuncio` — exit 0, 1598 resultado(s)
2. `rucene-base contratos count $R --terms 'IDAnuncio=<IDAnuncio do passo 1>'` — exit 0, 1668 resultado(s)
3. `rucene-base contratos count $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --term ContratEcologico=sim` — exit 0, 1627 resultado(s)

**Porque R não é menor:** Os ids vêm de `anuncios`; duas contagens com critérios diferentes em `contratos`.

**Combinação no cliente:** Rácio ecológicos/total.

**Tipos de campo e operações:**

* `anuncios.CriterAmbient` (flag): term
* `anuncios.Ano` (int): term
* `contratos.IDAnuncio` (exact): terms
* `contratos.ContratEcologico` (flag): term
* saída `--fields` de `anuncios`: IDAnuncio (exact)
* opções: anuncios: exact-total; contratos: count

**Resposta (validada):** 1598 anúncios com critério ambiental em 2023 originaram 1668 contratos, dos quais 1627 marcados como ecológicos (97,5 %)

### 60. Que proporção dos contratos de um ano celebrados pelas entidades de um dado tipo resultou de compra conjunta?

**Instanciação:** [tipo de entidade «Empresa pública»; ano 2025]  
**Id:** `c27` · **R = 3, I = 2** (adjudicantes, contratos) · Compras centralizadas e conjuntas

**Plano:**

1. `rucene-base adjudicantes search $R --term 'tipoEntidade=Empresa pública' --limit 200 --fields nif` — exit 0, 10 resultado(s)
2. `rucene-base contratos count $R --terms 'adjudicanteNif=<nif do passo 1>' --term Ano=2025` — exit 0, 13768 resultado(s)
3. `rucene-base contratos count $R --terms 'adjudicanteNif=<nif do passo 1>' --term Ano=2025 --term compraConjunta=sim` — exit 0, 1133 resultado(s)

**Porque R não é menor:** O tipo de entidade só existe em `adjudicantes`; duas contagens com critérios diferentes.

**Combinação no cliente:** Rácio entre as duas contagens.

**Tipos de campo e operações:**

* `adjudicantes.tipoEntidade` (keyword, multivalor): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.Ano` (int): term
* `contratos.compraConjunta` (flag): term
* saída `--fields` de `adjudicantes`: nif (exact)
* opções: contratos: count

**Resposta (validada):** 10 empresas públicas; 13768 contratos em 2025, dos quais 1133 por compra conjunta (8,2 %)

### 61. Em quantos contratos uma empresa foi adjudicatária ou concorrente, qual a sua taxa de sucesso e com que entidades concorreu recentemente?

**Instanciação:** [empresa com NIF 503040630 (pessoa coletiva, frequente nos concursos do Município de Lisboa)]  
**Id:** `d04` · **R = 3, I = 2** (contratos, adjudicantes) · Concorrência

**Plano:**

1. `rucene-base contratos search $R --should --term adjudicatarioNif=503040630 --should --term concorrenteNif=503040630 --sort dataPublicacao:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,dataPublicacao` — exit 0, 2878 resultado(s)
2. `rucene-base contratos count $R --term adjudicatarioNif=503040630` — exit 0, 872 resultado(s)
3. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 16 resultado(s)

**Porque R não é menor:** A disjunção (`--should` sem cláusulas obrigatórias) e a contagem de vitórias são critérios diferentes; os nomes das entidades estão em `adjudicantes`.

**Combinação no cliente:** Taxa de sucesso = vitórias/participações.

**Tipos de campo e operações:**

* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): --should term, term
* `contratos.concorrenteNif` (exact, multivalor, ausente=-): --should term
* `contratos.dataPublicacao` (date): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), dataPublicacao (date)
* opções: contratos: exact-total; contratos: count
* `--should` só filtra quando não há cláusulas obrigatórias; com um `--term` obrigatório passaria a mero reforço de relevância.

**Resposta (validada):** empresa 503040630 em 2878 contratos (adjudicatária ou concorrente); ganhou 872 (30,3 %); entidades dos 20 mais recentes: A. R. M. - Águas e Resíduos da Madeira, SA, Autoridade Nacional de Comunicações, CMPH - DomusSocial - Empresa de Habitação e Manutenção do Município do Porto, EM, EMEL - Empresa Municipal de Mobilidade e Estacionamento de Lisboa, EM, SA, Gebalis - Gestão do Arrendamento da Habitação Municipal de Lisboa, EM, SA

### 62. Nos dois códigos CPV mais usados de um grupo, que proporção dos contratos resultou de procedimentos centralizados?

**Instanciação:** [grupo CPV 909 (serviços de limpeza e saneamento); os 2 primeiros por `numContratos`]  
**Id:** `d06` · **R = 3, I = 2** (cpv, contratos) · Compras centralizadas e conjuntas

**Plano:**

1. `rucene-base cpv search $R --term cpvGrupo=909 --sort numContratos:desc --limit 2 --fields cpv,descricao,numContratos` — exit 0, 23 resultado(s)
2. `rucene-base contratos count $R --term 'cpv=<1.º CPV do passo 1>' --term ProcedimentoCentralizado=sim` — exit 0, 975 resultado(s)
3. `rucene-base contratos count $R --term 'cpv=<2.º CPV do passo 1>' --term ProcedimentoCentralizado=sim` — exit 0, 45 resultado(s)

**Porque R não é menor:** Uma contagem por código; o denominador é o agregado `numContratos` do passo 1. Agrupar no cliente as linhas de um único `search` seria agregação não limitada, que fica de fora.

**Combinação no cliente:** Rácio contagem/`numContratos`.

**Tipos de campo e operações:**

* `cpv.cpvGrupo` (keyword, nível derivado): term
* `cpv.numContratos` (int): sort
* `contratos.cpv` (keyword, multivalor): term
* `contratos.ProcedimentoCentralizado` (flag): term
* saída `--fields` de `cpv`: cpv (keyword), descricao (name), numContratos (int)
* opções: contratos: count

**Resposta (validada):** contratos por procedimento centralizado — 90910000-9 «Serviços de limpeza»: 975/12138 = 8,0 %; 90911200-8 «Serviços de limpeza de edifícios»: 45/3560 = 1,3 %

### 63. Que contratos resolveu uma entidade, quem eram os adjudicatários e continuaram a ganhar-lhe contratos depois?

**Instanciação:** [Município de Lisboa, NIF 500051070; tipo de fim «Resolução do Contrato»; publicados em 2021–2023; 10 maiores; contratos desde 2024-01-01]  
**Id:** `d17` · **R = 3, I = 2** (contratos, fornecedores) · Quem fornece

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=500051070 --term 'tipoFimContrato=Resolução do Contrato' --range dataPublicacao=2021-01-01..2023-12-31 --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,adjudicatarioNif,dataPublicacao,precoContratual` — exit 0, 319 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 1 resultado(s)
3. `rucene-base contratos count $R --terms 'adjudicatarioNif=<adjudicatarioNif do passo 1>' --term adjudicanteNif=500051070 --range dataPublicacao=2024-01-01..` — exit 0, 0 resultado(s)

**Porque R não é menor:** O atributo PME está em `fornecedores`; a contagem posterior é um critério diferente do passo 1.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.tipoFimContrato` (keyword): term
* `contratos.dataPublicacao` (date): range
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): terms
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), dataPublicacao (date), precoContratual (double)
* opções: contratos: exact-total; contratos: count

**Resposta (validada):** 319 contratos resolvidos pelo Município de Lisboa publicados em 2021–2023; os 10 maiores envolvem 1 adjudicatário(s) (PME 1), que ganharam depois 0 contratos à mesma entidade desde 2024

*Zero significativo:* zero contratos posteriores é uma resposta válida («não voltaram a ganhar»).

### 64. Que entidades com uma dada área de atividade compraram por ajuste direto produtos de uma classe CPV, e quais os contratos mais recentes?

**Instanciação:** [atividade «Saúde»; classe CPV 3360 (produtos farmacêuticos); Ajuste Direto Regime Geral]  
**Id:** `b02` · **R = 2, I = 2** (adjudicantes, contratos) · Procedimento e fundamento

**Plano:**

1. `rucene-base adjudicantes search $R --term atividadeEntidade=Saúde --limit 100 --fields nif,nome` — exit 0, 26 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term cpvClasse=3360 --term 'tipoprocedimento=Ajuste Direto Regime Geral' --sort dataPublicacao:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,dataPublicacao,precoContratual` — exit 0, 8924 resultado(s)

**Porque R não é menor:** A atividade da entidade só existe em `adjudicantes`; total e lista saem do mesmo `search --exact-total`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `adjudicantes.atividadeEntidade` (keyword, multivalor): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.cpvClasse` (keyword, nível derivado): term
* `contratos.tipoprocedimento` (keyword): term
* `contratos.dataPublicacao` (date): sort
* saída `--fields` de `adjudicantes`: nif (exact), nome (name)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), dataPublicacao (date), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 26 entidades de saúde; 8924 contratos de produtos farmacêuticos por ajuste direto; mais recentes: Unidade Local de Saúde de Coimbra, EPE 2026-11-12 612 840,90 €; Unidade Local de Saúde de Coimbra, EPE 2026-11-12 5 651,82 €; Unidade Local de Saúde de Coimbra, EPE 2026-11-12 714,00 €; Unidade Local de Saúde de Coimbra, EPE 2026-11-12 82,00 €; Unidade Local de Saúde de Coimbra, EPE 2026-11-12 105 198,19 €

### 65. Que entidades sediadas numa região celebraram contratos ao abrigo de um regime jurídico regional num ano (listadas por ordem alfabética)?

**Instanciação:** [NUTS PT200 – Região Autónoma dos Açores; regime «DLR n.º 27/2015/A, de 29.12»; ano 2024]  
**Id:** `b05` · **R = 2, I = 2** (adjudicantes, contratos) · Procedimento e fundamento

**Plano:**

1. `rucene-base adjudicantes search $R --term nutsCod=PT200 --limit 500 --fields nif` — exit 0, 317 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --term 'regime=DLR n.º 27/2015/A, de 29.12' --term Ano=2024 --sort adjudicanteNome:asc,precoContratual:desc --limit 5 --exact-total --fields adjudicanteNome,idcontrato,precoContratual` — exit 0, 1288 resultado(s)

**Porque R não é menor:** A sede só existe em `adjudicantes`; total e lista ordenada num só `search --exact-total`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `adjudicantes.nutsCod` (keyword, multivalor): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.regime` (keyword): term
* `contratos.Ano` (int): term
* `contratos.adjudicanteNome` (name, multivalor): sort
* `contratos.precoContratual` (double): sort
* saída `--fields` de `adjudicantes`: nif (exact)
* saída `--fields` de `contratos`: adjudicanteNome (name), idcontrato (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 1288 contratos ao abrigo do DLR 27/2015/A em 2024 por entidades sediadas nos Açores; primeiros por nome: ADI - Associação de Desenvolvimento Intergeracional 1 598 962,08 €; Agência para a Modernização Administrativa e Qualidade do Serviço ao Cidadão - RIAC 74 971,20 €; Agência para a Modernização Administrativa e Qualidade do Serviço ao Cidadão - RIAC 34 310,28 €; Agência para a Modernização Administrativa e Qualidade do Serviço ao Cidadão - RIAC 29 265,49 €; Agência para a Modernização Administrativa e Qualidade do Serviço ao Cidadão - RIAC 25 523,00 €

### 66. Quais dos maiores compradores públicos também aparecem como adjudicatários, e quanto receberam como fornecedores face ao que compraram?

**Instanciação:** [os 100 adjudicantes de maior `valorTotal`]  
**Id:** `b06` · **R = 2, I = 2** (adjudicantes, fornecedores) · Quem compra

**Plano:**

1. `rucene-base adjudicantes search $R --sort valorTotal:desc --limit 100 --fields nif,nome,valorTotal` — exit 0, 11209 resultado(s)
2. `rucene-base fornecedores search $R --terms 'nif=<nif do passo 1>' --sort valorTotal:desc --limit 10 --exact-total --fields nif,numContratos,valorTotal` — exit 0, 83 resultado(s)

**Porque R não é menor:** Comprador e fornecedor são índices distintos com a mesma chave NIF.

**Combinação no cliente:** Interseção por NIF; comparação do valor recebido com o valor comprado.

**Tipos de campo e operações:**

* `adjudicantes.valorTotal` (double): sort
* `fornecedores.nif` (exact, chave): terms
* `fornecedores.valorTotal` (double): sort
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), valorTotal (double)
* saída `--fields` de `fornecedores`: nif (exact), numContratos (int), valorTotal (double)
* opções: fornecedores: exact-total

**Resposta (validada):** 83 dos 100 maiores compradores também são adjudicatários; maiores como fornecedores: Serviço de Utilização Comum dos Hospitais recebeu 436 020 025,35 € em 2379 contratos (comprou 507 529 421,90 €); EPAL - Empresa Portuguesa das Águas Livres, SA recebeu 31 845 651,42 € em 446 contratos (comprou 523 427 181,87 €); Unidade Local de Saúde de São José, EPE recebeu 22 803 345,69 € em 28 contratos (comprou 2 537 249 950,00 €); Santa Casa da Misericórdia de Lisboa recebeu 17 719 350,03 € em 25 contratos (comprou 1 703 845 558,47 €)

### 67. Que entidades lançaram anúncios de um modelo de procedimento pouco frequente desde uma data, e em que regiões estão sediadas?

**Instanciação:** [modelo de anúncio «Concurso de concepção»; desde 2020-01-01; os 30 mais recentes]  
**Id:** `b07` · **R = 2, I = 2** (anuncios, adjudicantes) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term 'modeloAnuncio=Concurso de concepção' --range dataPublicacao=2020-01-01.. --sort dataPublicacao:desc --limit 30 --exact-total --fields IDAnuncio,nifEntidade,dataPublicacao` — exit 0, 334 resultado(s)
2. `rucene-base adjudicantes search $R --terms 'nif=<nifEntidade do passo 1>' --limit 30 --fields nif,nome,nutsCod,nutsDesc` — exit 0, 23 resultado(s)

**Porque R não é menor:** A sede das entidades só está em `adjudicantes`, a partir dos NIF do passo 1.

**Combinação no cliente:** Contagem de entidades por descrição NUTS nas linhas do passo 2.

**Tipos de campo e operações:**

* `anuncios.modeloAnuncio` (keyword): term
* `anuncios.dataPublicacao` (date): range, sort
* `adjudicantes.nif` (exact, chave): terms
* saída `--fields` de `anuncios`: IDAnuncio (exact), nifEntidade (exact), dataPublicacao (date)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), nutsCod (keyword), nutsDesc (text)
* opções: anuncios: exact-total

**Resposta (validada):** 334 concursos de conceção desde 2020; 23 entidades distintas nos 30 mais recentes; regiões: Área Metropolitana do Porto (3), Região Autónoma da Madeira (3), Área Metropolitana de Lisboa (2), Alto Minho (2), Beiras e Serra da Estrela (1)

### 68. Dos fornecedores com atividade contínua entre dois anos e mais contratos, quantos contratos celebraram com uma dada entidade e quais os maiores?

**Instanciação:** [primeiro ano 2012 e último ano 2026; os 20 primeiros por `numContratos`; Município do Porto, NIF 501306099]  
**Id:** `b09` · **R = 2, I = 2** (fornecedores, contratos) · Quem fornece

**Plano:**

1. `rucene-base fornecedores search $R --term primeiroAno=2012 --term ultimoAno=2026 --sort numContratos:desc --limit 20 --fields nif,numContratos,valorTotal` — exit 0, 1186 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicatarioNif=<nif do passo 1>' --term adjudicanteNif=501306099 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicatarioNif,precoContratual,dataPublicacao` — exit 0, 45 resultado(s)

**Porque R não é menor:** Os anos de atividade são agregados de `fornecedores`; os contratos com a entidade estão em `contratos`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `fornecedores.primeiroAno` (int): term
* `fornecedores.ultimoAno` (int): term
* `fornecedores.numContratos` (int): sort
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): terms
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.precoContratual` (double): sort
* saída `--fields` de `fornecedores`: nif (exact), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double), dataPublicacao (date)
* opções: contratos: exact-total

**Resposta (validada):** 45 contratos dos 20 fornecedores com atividade contínua 2012–2026 com o Município do Porto; maiores: 504615947+507349601+506370836 6 325 282,37 € (2021-11-24); 500697370 3 300 000,00 € (2026-08-11); 500697370 2 000 000,00 € (2012-10-31)

### 69. Quem ganhou os contratos de uma entidade cujo objeto contém uma dada expressão, e esses adjudicatários são PME?

**Instanciação:** [Município de Lisboa, NIF 500051070; expressão «refeições escolares» no objeto do contrato]  
**Id:** `b10` · **R = 2, I = 2** (contratos, fornecedores) · PME

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=500051070 --phrase-in 'objectoContrato=refeições escolares' --sort dataPublicacao:desc --limit 20 --exact-total --fields idcontrato,adjudicatarioNif,precoContratual,dataPublicacao` — exit 0, 16 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 8 resultado(s)

**Porque R não é menor:** O atributo PME só está em `fornecedores`, a partir dos NIF do passo 1.

**Combinação no cliente:** Contagem de PME nos documentos do passo 2.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.objectoContrato` (text): phrase-in
* `contratos.dataPublicacao` (date): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double), dataPublicacao (date)
* opções: contratos: exact-total

**Resposta (validada):** 16 contratos de refeições escolares do Município de Lisboa; 8 adjudicatários distintos, 6 PME: 505231212 (PME sim), 509295967 (PME sim), 500126623 (PME sim), 500574880 (PME sim), 504215965 (PME s/i), 500142858 (PME sim), 500145415 (PME sim), 510581692 (PME s/i)

### 70. Das entidades mais convidadas a apresentar proposta, quantas nunca foram adjudicatárias e quais ganham menos contratos por convite?

**Instanciação:** [as 50 entidades com maior `numConvites`]  
**Id:** `b11` · **R = 2, I = 2** (convidadas, fornecedores) · Concorrência

**Plano:**

1. `rucene-base convidadas search $R --sort numConvites:desc --limit 50 --fields nif,numConvites,primeiroAno,ultimoAno` — exit 0, 50693 resultado(s)
2. `rucene-base fornecedores search $R --terms 'nif=<nif do passo 1>' --limit 50 --fields nif,numContratos` — exit 0, 50 resultado(s)

**Porque R não é menor:** Convidadas e adjudicatárias são índices distintos com chave NIF.

**Combinação no cliente:** Diferença de conjuntos (convidadas ausentes do passo 2) e rácio `numContratos`/`numConvites` por NIF.

**Tipos de campo e operações:**

* `convidadas.numConvites` (int): sort
* `fornecedores.nif` (exact, chave): terms
* saída `--fields` de `convidadas`: nif (exact), numConvites (int), primeiroAno (int), ultimoAno (int)
* saída `--fields` de `fornecedores`: nif (exact), numContratos (int)

**Resposta (validada):** 0 das 50 entidades mais convidadas nunca foram adjudicatárias; menores rácios contratos ganhos/convites: 504362674 645/15030, 510452973 557/12196, 506037169 1218/26431, 517568020 482/9093

### 71. Em que regiões NUTS estão sediados os fornecedores de maior valor total adjudicado (entre os que têm sede conhecida)?

**Instanciação:** [os 20 fornecedores de maior `valorTotal` com `nutsCod` preenchido (nível 1 em PT1, PT2 ou PT3)]  
**Id:** `b12` · **R = 2, I = 2** (fornecedores, nuts) · Quem fornece

**Plano:**

1. `rucene-base fornecedores search $R --terms nutsNivel1=PT1,PT2,PT3 --sort valorTotal:desc --limit 20 --fields nif,nutsCod,valorTotal,pme` — exit 0, 7139 resultado(s)
2. `rucene-base nuts get $R --key <nutsCod do passo 1> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** A descrição das regiões está em `nuts`, para os códigos do passo 1.

**Combinação no cliente:** Contagem de fornecedores por código NUTS.

**Tipos de campo e operações:**

* `fornecedores.nutsNivel1` (keyword, nível derivado): terms
* `fornecedores.valorTotal` (double): sort
* `nuts.nuts` (keyword, chave): get
* saída `--fields` de `fornecedores`: nif (exact), nutsCod (keyword), valorTotal (double), pme (flag)
* `--terms nutsNivel1=PT1,PT2,PT3` serve de filtro de presença: um `nutsCod` ausente não tem nível derivado.

**Resposta (validada):** sede dos 20 fornecedores de maior valor com NUTS conhecida: PT1A0 Grande Lisboa (11), PT1C3 Alto Alentejo (5), PT112 Cávado (2), PT11D Douro (1), PT11C Tâmega e Sousa (1)

### 72. Que empresas foram indicadas como subcontratadas nas modificações por cessão da posição contratual e subcontratação, e quantas já foram adjudicatárias e são PME?

**Instanciação:** [fundamento «Cessão da posição contratual e subcontratação (art. 316.º e seguintes do CCP)»; subcontratada com «Lda» no nome; 20 mais recentes]  
**Id:** `b13` · **R = 2, I = 2** (modificacoes, fornecedores) · Modificações

**Plano:**

1. `rucene-base modificacoes search $R --term 'modifContratoFundamento=Cessão da posição contratual e subcontratação (art. 316.º e seguintes do CCP)' --text-in subcontratadaNome=lda --sort modifContratoData:desc --limit 20 --exact-total --fields idcontrato,sequencia,subcontratadaNif,modifContratoData` — exit 0, 3 resultado(s)
2. `rucene-base fornecedores search $R --terms 'nif=<subcontratadaNif do passo 1>' --limit 500 --fields nif,pme,numContratos` — exit 0, 111 resultado(s)

**Porque R não é menor:** O atributo PME e o historial de adjudicação estão em `fornecedores`, a partir dos NIF do passo 1.

**Combinação no cliente:** Interseção da lista de subcontratadas com o passo 2; contagem de PME.

**Tipos de campo e operações:**

* `modificacoes.modifContratoFundamento` (name): term
* `modificacoes.subcontratadaNome` (text, multivalor): text-in
* `modificacoes.modifContratoData` (date): sort
* `fornecedores.nif` (exact, chave): terms
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), subcontratadaNif (exact), modifContratoData (date)
* saída `--fields` de `fornecedores`: nif (exact), pme (flag), numContratos (int)
* opções: modificacoes: exact-total

**Resposta (validada):** 3 modificações por cessão/subcontratação com subcontratadas «Lda»; 167 subcontratadas distintas, 111 já foram adjudicatárias, 90 PME

### 73. Nos contratos concluídos de uma entidade com preço total efetivo registado, quais excederam o preço contratual, em quanto, e quem foram os adjudicatários?

**Instanciação:** [Município de Braga, NIF 506901173; preço total efetivo preenchido (o marcador 0 não é indexado); 20 maiores]  
**Id:** `b15` · **R = 2, I = 2** (contratos, fornecedores) · Valores

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=506901173 --range PrecoTotalEfetivo=.. --sort PrecoTotalEfetivo:desc --limit 20 --exact-total --fields idcontrato,adjudicatarioNif,precoContratual,PrecoTotalEfetivo` — exit 0, 3453 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 15 resultado(s)

**Porque R não é menor:** O atributo PME só está em `fornecedores`; os NIF vêm do passo 1.

**Combinação no cliente:** Desvio = efetivo − contratual, filtrado aos positivos nas 20 linhas.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.PrecoTotalEfetivo` (double, ausente=0): range, sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), precoContratual (double), PrecoTotalEfetivo (double)
* opções: contratos: exact-total
* `--range PrecoTotalEfetivo=..` sem limites seleciona apenas documentos com valor: o marcador `0` (ausente) não é indexado.

**Resposta (validada):** 3453 contratos com preço efetivo; 11 dos 20 maiores excederam o contratual: 7495438: +19 550 486,34 € (9900,0 %), adjudicatário 506442179 PME sim; 289800: +190 778,99 € (2,6 %), adjudicatário 501489126 PME sim; 838558: +5 381 821,00 € (9856,8 %), adjudicatário 504803336 PME s/i; 10100256: +238 288,16 € (7,4 %), adjudicatário 501323325 PME sim

### 74. Entre os contratos de um intervalo de valor celebrados num ano, quais pertencem a procedimentos com vários contratos, e qual o valor total desses procedimentos?

**Instanciação:** [preço contratual entre 5 000 000 € e 10 000 000 €; ano 2025]  
**Id:** `b20` · **R = 2, I = 2** (contratos, procedimentos) · Valores

**Plano:**

1. `rucene-base contratos search $R --range precoContratual=5000000..10000000 --term Ano=2025 --limit 200 --exact-total --fields idcontrato,idprocedimento,precoContratual` — exit 0, 290 resultado(s)
2. `rucene-base procedimentos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --range numContratos=2.. --sort valorTotal:desc --limit 50 --fields idprocedimento,tipoprocedimento,numContratos,valorTotal` — exit 0, 28 resultado(s)

**Porque R não é menor:** O número de contratos por procedimento é agregado de `procedimentos`; os ids vêm do passo 1.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `contratos.precoContratual` (double): range
* `contratos.Ano` (int): term
* `procedimentos.idprocedimento` (exact, chave): terms
* `procedimentos.numContratos` (int): range
* `procedimentos.valorTotal` (double): sort
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), precoContratual (double)
* saída `--fields` de `procedimentos`: idprocedimento (exact), tipoprocedimento (keyword), numContratos (int), valorTotal (double)
* opções: contratos: exact-total

**Resposta (validada):** 290 contratos entre 5 e 10 M€ em 2025; 28 pertencem a procedimentos com 2+ contratos, p.ex. 7448320 (Ao abrigo de acordo-quadro (art.º 259.º), 290 contratos, 231 283 077,15 €); 7436502 (Ao abrigo de acordo-quadro (art.º 259.º), 160 contratos, 198 375 875,48 €); 7437568 (Ao abrigo de acordo-quadro (art.º 259.º), 97 contratos, 115 039 138,31 €)

### 75. Nas consultas prévias com primeira publicação num mês e vários contratos, quantos adjudicatários distintos houve?

**Instanciação:** [Consulta Prévia; primeira publicação em 2025-03; 4 ou mais contratos]  
**Id:** `b22` · **R = 2, I = 2** (procedimentos, contratos) · Procedimento e fundamento

**Plano:**

1. `rucene-base procedimentos search $R --term 'tipoprocedimento=Consulta Prévia' --range primeiraPublicacao=2025-03-01..2025-03-31 --range numContratos=4.. --sort numContratos:desc --limit 20 --exact-total --fields idprocedimento,numContratos,valorTotal` — exit 0, 67 resultado(s)
2. `rucene-base contratos search $R --terms 'idprocedimento=<idprocedimento do passo 1>' --limit 300 --fields idcontrato,idprocedimento,adjudicatarioNif,precoContratual` — exit 0, 137 resultado(s)

**Porque R não é menor:** O número de contratos e a primeira publicação são agregados de `procedimentos`; os adjudicatários estão em `contratos`.

**Combinação no cliente:** Contagem de NIF distintos nas linhas do passo 2.

**Tipos de campo e operações:**

* `procedimentos.tipoprocedimento` (keyword): term
* `procedimentos.primeiraPublicacao` (date): range
* `procedimentos.numContratos` (int): range, sort
* `contratos.idprocedimento` (exact): terms
* saída `--fields` de `procedimentos`: idprocedimento (exact), numContratos (int), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), adjudicatarioNif (exact), precoContratual (double)
* opções: procedimentos: exact-total

**Resposta (validada):** 67 consultas prévias de março de 2025 com 4+ contratos; nas listadas, 137 contratos e 80 adjudicatários distintos; maior procedimento 7492028 (13 contratos, 40 719,69 €)

### 76. Quantos contratos de um ano invocam uma alínea legal (pesquisada como expressão), e quantos contratos teve o procedimento de cada um dos maiores?

**Instanciação:** [expressão «Artigo 20.º, n.º 1, alínea d)» no fundamento; ano 2025; 5 maiores]  
**Id:** `b23` · **R = 2, I = 2** (contratos, procedimentos) · Procedimento e fundamento

**Plano:**

1. `rucene-base contratos search $R --phrase-in 'fundamentacao=Artigo 20.º, n.º 1, alínea d)' --term Ano=2025 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,idprocedimento,precoContratual` — exit 0, 44928 resultado(s)
2. `rucene-base procedimentos get $R --key <idprocedimento do passo 1> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** O número de contratos por procedimento só está em `procedimentos`.

**Combinação no cliente:** Emparelha contrato e procedimento por id.

**Tipos de campo e operações:**

* `contratos.fundamentacao` (name): phrase-in
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `procedimentos.idprocedimento` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), idprocedimento (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 44928 contratos de 2025 com fundamento na alínea d) do n.º 1 do art. 20.º; maiores: 11653910 1 248 945,00 € (procedimento com 1 contrato(s)); 11623241 985 976,12 € (procedimento com 1 contrato(s)); 11878953 741 896,00 € (procedimento com 1 contrato(s)); 11549010 740 000,00 € (procedimento com 1 contrato(s)); 11254923 731 800,00 € (procedimento com 1 contrato(s))

### 77. Que entidades invocaram uma justificação para não reduzir o contrato a escrito num ano, e de que tipo são?

**Instanciação:** [justificação com as palavras «urgência imperiosa»; ano 2024; 20 maiores]  
**Id:** `b25` · **R = 2, I = 2** (contratos, adjudicantes) · Procedimento e fundamento

**Plano:**

1. `rucene-base contratos search $R --text-in 'justifNReducEscrContrato=urgência imperiosa' --term Ano=2024 --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,adjudicanteNif,precoContratual` — exit 0, 4351 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 11 resultado(s)

**Porque R não é menor:** O tipo de entidade só existe em `adjudicantes`.

**Combinação no cliente:** Contagem por `tipoEntidade` dos documentos do passo 2.

**Tipos de campo e operações:**

* `contratos.justifNReducEscrContrato` (text, multivalor): text-in
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 4351 contratos de 2024 sem redução a escrito por urgência imperiosa; 11 entidades nos 20 maiores; tipos: Organismo de direito público, controlado por uma autoridade da administração central (1), sem tipo indicado (7), Empresa pública (2), Autoridade local (1)

### 78. Nos contratos resultantes de concursos urgentes de um ano, que prazo para apresentação de propostas deram os anúncios?

**Instanciação:** [tipo de anúncio «Anúncio de concurso urgente»; ano 2025; 10 maiores contratos]  
**Id:** `b27` · **R = 2, I = 2** (contratos, anuncios) · Prazos e datas

**Plano:**

1. `rucene-base contratos search $R --term 'TipoAnuncio=Anúncio de concurso urgente' --term Ano=2025 --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,IDAnuncio,precoContratual` — exit 0, 1325 resultado(s)
2. `rucene-base anuncios search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --fields IDAnuncio,PrazoPropostas,dataPublicacao,DataLimitePropostas` — exit 0, 10 resultado(s)

**Porque R não é menor:** O prazo de propostas só existe em `anuncios`.

**Combinação no cliente:** Mínimo, mediana e máximo dos prazos devolvidos.

**Tipos de campo e operações:**

* `contratos.TipoAnuncio` (keyword): term
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `anuncios.IDAnuncio` (exact, chave): terms
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), precoContratual (double)
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrazoPropostas (int), dataPublicacao (date), DataLimitePropostas (date)
* opções: contratos: exact-total

**Resposta (validada):** 1325 contratos de concursos urgentes em 2025; prazos de propostas dos anúncios dos 10 maiores: mín. 2, mediana 5.0, máx. 21 dias

### 79. Em que regiões foram executados os maiores contratos celebrados ao abrigo de uma medida especial de contratação?

**Instanciação:** [TipoMEC «Plano de Recuperação e Resiliência (PRR) – artigo 2º da Lei n.º30/2021, DL n.º 78/2022»; NUTS indicada; 20 maiores]  
**Id:** `b28` · **R = 2, I = 2** (contratos, nuts) · Procedimento e fundamento

**Plano:**

1. `rucene-base contratos search $R --term 'TipoMEC=Plano de Recuperação e Resiliência (PRR) – artigo 2º da Lei n.º30/2021, DL n.º 78/2022' --terms nutsNivel1=PT1,PT2,PT3 --sort precoContratual:desc --limit 20 --exact-total --fields idcontrato,nuts,precoContratual` — exit 0, 43 resultado(s)
2. `rucene-base nuts get $R --key <nuts do passo 1> …` — exit 0, 4 resultado(s)

**Porque R não é menor:** A descrição das regiões está em `nuts`.

**Combinação no cliente:** Contagem de contratos por NUTS (campo multivalor).

**Tipos de campo e operações:**

* `contratos.TipoMEC` (keyword): term
* `contratos.nutsNivel1` (keyword, nível derivado): terms
* `contratos.precoContratual` (double): sort
* `nuts.nuts` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), nuts (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 43 contratos PRR (art. 2.º, DL 78/2022) com NUTS indicada; regiões dos 20 maiores: Área Metropolitana de Lisboa (13), Área Metropolitana do Porto (9), Região de Coimbra (2), Alto Minho (1)

### 80. Que contratos de um ano têm um termo na descrição do CPV mas não no objeto do contrato, e que códigos CPV são?

**Instanciação:** [«software» em `cpvDescricao` e não em `objectoContrato`; ano 2025; 10 maiores]  
**Id:** `b31` · **R = 2, I = 2** (contratos, cpv) · Objeto (CPV)

**Plano:**

1. `rucene-base contratos search $R --text-in cpvDescricao=software --not --text-in objectoContrato=software --term Ano=2025 --sort precoContratual:desc --limit 10 --exact-total --fields idcontrato,cpv,precoContratual` — exit 0, 6076 resultado(s)
2. `rucene-base cpv get $R --key <cpv do passo 1> …` — exit 0, 7 resultado(s)

**Porque R não é menor:** A descrição canónica dos códigos está em `cpv`.

**Combinação no cliente:** Códigos distintos nas linhas devolvidas.

**Tipos de campo e operações:**

* `contratos.cpvDescricao` (text, multivalor): text-in
* `contratos.objectoContrato` (text): --not text-in
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* `cpv.cpv` (keyword, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), cpv (keyword), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 6076 contratos de 2025 com «software» na descrição CPV mas não no objeto; CPV dos 10 maiores: 48100000-9 «Pacote de software para sectores específicos»; 48218000-9 «Pacote de software para gestão de licenças»; 48300000-1 «Pacote de software para criação de documentos, desenho, imagens, gestão de tarefas e produtividade»; 48900000-7 «Pacote de software e sistemas informáticos diversos»; 51000000-9 «Serviços de instalação (excepto software)»

### 81. Que regiões NUTS têm mais fornecedores sediados do que entidades adjudicantes, e quais as PME de maior valor na primeira?

**Instanciação:** [regiões com 500 ou mais fornecedores sediados; 5 PME]  
**Id:** `b36` · **R = 2, I = 2** (nuts, fornecedores) · PME

**Plano:**

1. `rucene-base nuts search $R --range numFornecedores=500.. --sort numFornecedores:desc --limit 10 --fields nuts,descricao,numAdjudicantes,numFornecedores` — exit 0, 4 resultado(s)
2. `rucene-base fornecedores search $R --term 'nutsCod=<primeira NUTS do passo 1 com numFornecedores > numAdjudicantes>' --term pme=sim --sort valorTotal:desc --limit 5 --fields nif,valorTotal,numContratos` — exit 0, 1007 resultado(s)

**Porque R não é menor:** Os contadores por região estão em `nuts`; o atributo PME em `fornecedores`.

**Combinação no cliente:** Comparação numFornecedores > numAdjudicantes por linha.

**Tipos de campo e operações:**

* `nuts.numFornecedores` (int): range, sort
* `fornecedores.nutsCod` (keyword, multivalor): term
* `fornecedores.pme` (flag): term
* `fornecedores.valorTotal` (double): sort
* saída `--fields` de `nuts`: nuts (keyword), descricao (name), numAdjudicantes (int), numFornecedores (int)
* saída `--fields` de `fornecedores`: nif (exact), valorTotal (double), numContratos (int)

**Resposta (validada):** NUTS com mais fornecedores do que adjudicantes sediados: PT1A0 Grande Lisboa (3316 vs 131), PT11A Área Metropolitana do Porto (1782 vs 501), PT192 Região de Coimbra (603 vs 48); maiores PME sediadas em Grande Lisboa: 500697370 1 837 938 691,38 €, 980245974 1 664 977 845,19 €, 500126623 1 242 465 775,09 €

### 82. Quantos contratos de compra conjunta foram executados nas regiões cuja descrição NUTS contém uma expressão, e quais os maiores?

**Instanciação:** [expressão «Região Autónoma» na descrição NUTS; compra conjunta = sim]  
**Id:** `b37` · **R = 2, I = 2** (nuts, contratos) · Compras centralizadas e conjuntas

**Plano:**

1. `rucene-base nuts search $R --phrase-in 'descricao=Região Autónoma' --limit 20 --fields nuts,descricao` — exit 0, 6 resultado(s)
2. `rucene-base contratos search $R --terms 'nuts=<nuts do passo 1>' --term compraConjunta=sim --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,precoContratual` — exit 0, 232 resultado(s)

**Porque R não é menor:** As descrições NUTS estão em `nuts`; os contratos em `contratos`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `nuts.descricao` (name): phrase-in
* `contratos.nuts` (keyword, multivalor): terms
* `contratos.compraConjunta` (flag): term
* `contratos.precoContratual` (double): sort
* saída `--fields` de `nuts`: nuts (keyword), descricao (name)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** NUTS das regiões autónomas: PT2, PT20, PT200, PT3, PT30, PT300; 232 contratos de compra conjunta aí executados; maiores: ENEREEM - Energias Renováveis, Lda 19 652 950,00 €; EEM - Empresa de Electricidade da Madeira, SA 13 160 600,00 €; Direção Regional das Obras Públicas 6 840 000,00 €

### 83. Que entidades sediadas fora de uma região executam lá contratos, e quais os maiores?

**Instanciação:** [região NUTS de nível 1 PT2 (Açores) / local «Portugal, Região Autónoma dos Açores»]  
**Id:** `b39` · **R = 2, I = 2** (adjudicantes, contratos) · Localização

**Plano:**

1. `rucene-base adjudicantes search $R --term nutsNivel1=PT2 --limit 1000 --fields nif` — exit 0, 364 resultado(s)
2. `rucene-base contratos search $R --term 'localNivel2=Portugal, Região Autónoma dos Açores' --not --terms 'adjudicanteNif=<nif do passo 1>' --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,precoContratual` — exit 0, 7437 resultado(s)

**Porque R não é menor:** A sede das entidades só está em `adjudicantes`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `adjudicantes.nutsNivel1` (keyword, nível derivado): term
* `contratos.localNivel2` (keyword, nível derivado): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): --not terms
* `contratos.precoContratual` (double): sort
* saída `--fields` de `adjudicantes`: nif (exact)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 364 entidades sediadas nos Açores excluídas; 7437 contratos executados nos Açores por entidades de fora; maiores: Gabinete do Secretário de Estado das Infraestruturas 62 499 722,00 €; Teramb-Empresa Municipal de Gestão e Valorização Ambiental da Ilha Terceira, EM 29 711 265,00 €; Secretaria Regional da Saúde e Segurança Social 14 280 000,00 €; Navegação Aérea de Portugal - Nav Portugal, EPE 14 061 273,00 €; Secretaria Regional do Ambiente e do Mar 14 025 000,01 €

### 84. Quanto tempo mediou entre a publicação dos anúncios de procedimento de uma entidade num ano e a celebração dos contratos resultantes?

**Instanciação:** [Município de Coimbra, NIF 506415082; «Anúncio de procedimento»; ano 2024]  
**Id:** `b40` · **R = 2, I = 2** (anuncios, contratos) · Prazos e datas

**Plano:**

1. `rucene-base anuncios search $R --term nifEntidade=506415082 --term 'tipoActo=Anúncio de procedimento' --range dataPublicacao=2024-01-01..2024-12-31 --limit 100 --exact-total --fields IDAnuncio,dataPublicacao` — exit 0, 42 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 200 --fields idcontrato,IDAnuncio,dataCelebracaoContrato` — exit 0, 55 resultado(s)

**Porque R não é menor:** A data do anúncio está em `anuncios`, a da celebração em `contratos`.

**Combinação no cliente:** Diferença em dias por contrato; mínimo, mediana, máximo.

**Tipos de campo e operações:**

* `anuncios.nifEntidade` (exact, multivalor, ausente=-): term
* `anuncios.tipoActo` (keyword): term
* `anuncios.dataPublicacao` (date): range
* `contratos.IDAnuncio` (exact): terms
* saída `--fields` de `anuncios`: IDAnuncio (exact), dataPublicacao (date)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), dataCelebracaoContrato (date)
* opções: anuncios: exact-total

**Resposta (validada):** 42 anúncios de procedimento em 2024; 55 contratos ligados; dias entre anúncio e celebração: mín. 24, mediana 109, máx. 722

### 85. Qual o conteúdo de uma modificação identificada pela chave composta contrato/sequência, e como se compara com o preço contratual original?

**Instanciação:** [modificação 7682444/2 (a de maior preço alterado do índice)]  
**Id:** `c03` · **R = 2, I = 2** (modificacoes, contratos) · Modificações

**Plano:**

1. `rucene-base modificacoes get $R --key 7682444/2` — exit 0, 1 resultado(s)
2. `rucene-base contratos get $R --key 7682444` — exit 0, 1 resultado(s)

**Porque R não é menor:** Modificação e contrato estão em índices diferentes.

**Combinação no cliente:** Rácio preço alterado / preço contratual.

**Tipos de campo e operações:**

* `modificacoes.idcontrato` (exact, chave): get (chave composta)
* `modificacoes.sequencia` (int, chave): get (chave composta)
* `contratos.idcontrato` (exact, chave): get

**Resposta (validada):** modificação 7682444/2 (Acordo entre as partes, 2021-10-01): «Trabalhos Complementares (art. 370.º do CCP)», preço alterado 44 460 392 666,00 €; contrato de Santa Casa da Misericórdia de Castro Marim com preço contratual 4 248 997,26 € (rácio 10463.7×)

### 86. Em que contratos uma modificação fixou um prazo de execução muito longo, e qual era o prazo original?

**Instanciação:** [prazo modificado ≥ 3650 dias (o marcador 99999999 não é indexado); 10 maiores]  
**Id:** `c07` · **R = 2, I = 2** (modificacoes, contratos) · Modificações

**Plano:**

1. `rucene-base modificacoes search $R --range modifPrazoExecucao=3650.. --sort modifPrazoExecucao:desc --limit 10 --exact-total --fields idcontrato,sequencia,modifPrazoExecucao` — exit 0, 310 resultado(s)
2. `rucene-base contratos get $R --key <idcontrato do passo 1> …` — exit 0, 5 resultado(s)

**Porque R não é menor:** O prazo original está em `contratos`.

**Combinação no cliente:** Comparação prazo modificado vs prazo original.

**Tipos de campo e operações:**

* `modificacoes.modifPrazoExecucao` (int, ausente=99999999): range, sort
* `contratos.idcontrato` (exact, chave): get
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifPrazoExecucao (int)
* opções: modificacoes: exact-total
* `modifPrazoExecucao` e `prazoExecucao` não indexam o marcador `99999999`: o intervalo aberto não o apanha.

**Resposta (validada):** 310 modificações com prazo ≥ 3650 dias; 7748218: 19484962 dias após modificação vs 194667 no contrato (Câmara Municipal de Angra do Heroísmo); 802149: 52823 dias após modificação vs 41500 no contrato (Universidade de Évora); 1519779: 41949 dias após modificação vs 30000 no contrato (Universidade de Évora); 10298761: 36524 dias após modificação vs 36524 no contrato (Instituto das Florestas e Conservação da Natureza)

### 87. Que contratos acumularam um número muito elevado de modificações, e com que fundamentos foram feitas as modificações a partir dessa ordem?

**Instanciação:** [50 ou mais modificações; sequência ≥ 50]  
**Id:** `c09` · **R = 2, I = 2** (contratos, modificacoes) · Modificações

**Plano:**

1. `rucene-base contratos search $R --range numModificacoes=50.. --sort numModificacoes:desc --limit 20 --fields idcontrato,adjudicanteNome,numModificacoes` — exit 0, 7 resultado(s)
2. `rucene-base modificacoes search $R --terms 'idcontrato=<idcontrato do passo 1>' --range sequencia=50.. --limit 500 --fields idcontrato,sequencia,modifContratoFundamento` — exit 0, 209 resultado(s)

**Porque R não é menor:** O contador está em `contratos`; os fundamentos em `modificacoes`.

**Combinação no cliente:** Contagem dos fundamentos nas linhas devolvidas.

**Tipos de campo e operações:**

* `contratos.numModificacoes` (int): range, sort
* `modificacoes.idcontrato` (exact, chave): terms
* `modificacoes.sequencia` (int, chave): range
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), numModificacoes (int)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name)

**Resposta (validada):** 7 contratos com 50+ modificações (7626845 Metro do Porto, SA, 6983173 Instituto Português de Oncologia de Coimbra Francisco Gentil, EPE, 11449321 Universidade do Porto - Faculdade de Engenharia…); fundamentos das modificações de ordem ≥ 50: Trabalhos Complementares (art. 370.º do CCP) (142); Revisão Ordinária de Preços (45); Trabalhos a menos (Art. 379.º do CCP) (11)

### 88. Que entidades publicaram anúncios num dado número do Diário da República, e onde estão sediadas?

**Instanciação:** [DR n.º 75 (numDR), ano 2026]  
**Id:** `c15` · **R = 2, I = 2** (anuncios, adjudicantes) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term numDR=75 --term Ano=2026 --limit 200 --exact-total --fields IDAnuncio,nifEntidade` — exit 0, 173 resultado(s)
2. `rucene-base adjudicantes search $R --terms 'nif=<nifEntidade do passo 1>' --sort nome:asc --limit 200 --fields nif,nome,nutsDesc` — exit 0, 115 resultado(s)

**Porque R não é menor:** A sede das entidades está em `adjudicantes`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `anuncios.numDR` (exact): term
* `anuncios.Ano` (int): term
* `adjudicantes.nif` (exact, chave): terms
* `adjudicantes.nome` (name, multivalor): sort
* saída `--fields` de `anuncios`: IDAnuncio (exact), nifEntidade (exact)
* saída `--fields` de `adjudicantes`: nif (exact), nome (name), nutsDesc (text)
* opções: anuncios: exact-total

**Resposta (validada):** 173 anúncios no DR n.º 75 de 2026; 115 entidades distintas: A. R. M. - Águas e Resíduos da Madeira, SA [Região Autónoma da Madeira], ADC - Águas da Covilhã, EM [Beira Baixa], ARTE - Agência para a Reforma Tecnológica do Estado [Grande Lisboa], Agência Portuguesa do Ambiente, IP [Grande Lisboa], Algar - Valorização e Tratamentos de Resíduos Sólidos, SA [Algarve]

### 89. Que anúncio corresponde a um código INCM e que contratos o citam pelo mesmo código?

**Instanciação:** [código INCM 419675763]  
**Id:** `c18` · **R = 2, I = 2** (anuncios, contratos) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term IdIncm=419675763 --fields IDAnuncio,nAnuncio,dataPublicacao,designacaoEntidade` — exit 0, 1 resultado(s)
2. `rucene-base contratos search $R --term idINCM=419675763 --fields idcontrato,IDAnuncio,precoContratual` — exit 0, 1 resultado(s)

**Porque R não é menor:** Anúncio e contratos estão em índices diferentes; o código INCM é a chave comum.

**Combinação no cliente:** Confirma que o `IDAnuncio` dos contratos coincide com o do anúncio.

**Tipos de campo e operações:**

* `anuncios.IdIncm` (exact, ausente=-1): term
* `contratos.idINCM` (exact, ausente=-1): term
* saída `--fields` de `anuncios`: IDAnuncio (exact), nAnuncio (exact), dataPublicacao (date), designacaoEntidade (name)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), precoContratual (double)

**Resposta (validada):** anúncio 427153 (27585/2025, 2025-10-20, Escola Profissional Amar Terra Verde, Lda); 1 contrato(s): 12026866 120 310,00 €

### 90. Que prazo e preço base teve um anúncio identificado pelo seu número no Diário da República, onde está publicado, e que contratos resultaram?

**Instanciação:** [anúncio n.º 27585/2025]  
**Id:** `c19` · **R = 2, I = 2** (anuncios, contratos) · Anúncios

**Plano:**

1. `rucene-base anuncios search $R --term nAnuncio=27585/2025 --fields IDAnuncio,PrazoPropostas,PrecoBase,url` — exit 0, 1 resultado(s)
2. `rucene-base contratos search $R --term nAnuncio=27585/2025 --fields idcontrato,precoContratual` — exit 0, 1 resultado(s)

**Porque R não é menor:** Anúncio e contratos estão em índices diferentes.

**Combinação no cliente:** Soma dos preços contratuais.

**Tipos de campo e operações:**

* `anuncios.nAnuncio` (exact): term
* `contratos.nAnuncio` (exact): term
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrazoPropostas (int), PrecoBase (double), url (stored)
* saída `--fields` de `contratos`: idcontrato (exact), precoContratual (double)

**Resposta (validada):** anúncio n.º 27585/2025 (427153, prazo 6 dias, preço base 120 311,74 €); 1 contrato(s), total 120 310,00 €; publicação: https://files.diariodarepublica.pt/cp_hora/2025/10/202/419675763.pdf

### 91. Quem ganhou os contratos adjudicados com um fator de sustentabilidade, e quantos desses adjudicatários são PME?

**Instanciação:** [fator de adjudicação «Sustentabilidade ambiental ou social»; 30 maiores]  
**Id:** `c21` · **R = 2, I = 2** (contratos, fornecedores) · Sustentabilidade

**Plano:**

1. `rucene-base contratos search $R --term 'criterioAdjudicacaoTipoFator=Sustentabilidade ambiental ou social' --sort precoContratual:desc --limit 30 --exact-total --fields idcontrato,adjudicanteNome,adjudicatarioNif,precoContratual` — exit 0, 356 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 15 resultado(s)

**Porque R não é menor:** O atributo PME só está em `fornecedores`.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `contratos.criterioAdjudicacaoTipoFator` (keyword, multivalor): term
* `contratos.precoContratual` (double): sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), adjudicatarioNif (exact), precoContratual (double)
* opções: contratos: exact-total

**Resposta (validada):** 356 contratos com o fator «Sustentabilidade ambiental ou social»; 15 adjudicatários nos 30 maiores, 15 PME; maior: 3 040 968,00 € (Instituto da Segurança Social, IP)

### 92. Quantos contratos de um ano ganharam os fornecedores de maior valor sem indicação de PME, e com que entidades?

**Instanciação:** [fornecedores com `pme` diferente de «sim» (sem indicação); 20 de maior `valorTotal`; ano 2025]  
**Id:** `c25` · **R = 2, I = 2** (fornecedores, contratos) · PME

**Plano:**

1. `rucene-base fornecedores search $R --not --term pme=sim --sort valorTotal:desc --limit 20 --exact-total --fields nif,valorTotal` — exit 0, 46083 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicatarioNif=<nif do passo 1>' --term Ano=2025 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicatarioNif,adjudicanteNome,precoContratual` — exit 0, 8 resultado(s)

**Porque R não é menor:** O atributo PME está em `fornecedores`; os contratos em `contratos`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `fornecedores.pme` (flag): --not term
* `fornecedores.valorTotal` (double): sort
* `contratos.adjudicatarioNif` (exact, multivalor, ausente=-): terms
* `contratos.Ano` (int): term
* `contratos.precoContratual` (double): sort
* saída `--fields` de `fornecedores`: nif (exact), valorTotal (double)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), adjudicanteNome (name), precoContratual (double)
* opções: fornecedores: exact-total; contratos: exact-total

**Resposta (validada):** 46083 fornecedores sem indicação de PME; os 20 de maior valor ganharam 8 contratos em 2025; maiores: 518400581 ← Infraestruturas de Portugal, SA 1 661 362 811,55 €; 502876581+504363131 ← Associação de Municípios para a Gestão Sustentável de Resíduos do Grande Porto - LIPOR 267 141 637,80 €; 514643200 ← Metro do Porto, SA 18 874 410,63 €

### 93. Quais os contratos mais relevantes sobre um serviço (expressão em qualquer campo de texto), favorecendo os que mencionam um aspeto, e onde estão sediadas as entidades?

**Instanciação:** [expressão «manutenção de elevadores»; reforço opcional «manutenção preventiva» no objeto; 10 mais relevantes]  
**Id:** `d05` · **R = 2, I = 2** (contratos, adjudicantes) · Objeto (CPV)

**Plano:**

1. `rucene-base contratos search $R --phrase 'manutenção de elevadores' --should --phrase-in 'objectoContrato=manutenção preventiva' --sort _score --limit 10 --exact-total --fields idcontrato,adjudicanteNif,objectoContrato` — exit 0, 5256 resultado(s)
2. `rucene-base adjudicantes get $R --key <adjudicanteNif do passo 1> …` — exit 0, 7 resultado(s)

**Porque R não é menor:** A sede das entidades só está em `adjudicantes`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `contratos.objectoContrato` (text): phrase, --should phrase-in
* `contratos.descContrato` (text): phrase
* `contratos.adjudicanteNome` (name, multivalor): phrase
* `contratos.adjudicatarioNome` (name, multivalor): phrase
* `contratos.cpvDescricao` (text, multivalor): phrase
* `adjudicantes.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNif (exact), objectoContrato (text)
* opções: contratos: sort _score; contratos: exact-total
* Aqui `--should` não filtra (há uma cláusula obrigatória): só reordena por `_score`.

**Resposta (validada):** 5256 contratos com a expressão «manutenção de elevadores»; mais relevantes (reforço «preventiva»): 492693 IMPRENSA NACIONAL - CASA DA MOEDA SA [Grande Lisboa]; 8579851 FREGUESIA DE SANTA CLARA [sem NUTS]; 14663849 Município de Alcobaça [Oeste]; 15191826 Município de Anadia [Região de Aveiro]

### 94. Que códigos NUTS (versões 2013 e 2024) têm uma dada designação, e quantos contratos de um ano foram executados nessas regiões?

**Instanciação:** [designações exatas «Algarve» e «Douro»; ano 2025]  
**Id:** `d07` · **R = 2, I = 2** (nuts, contratos) · Localização

**Plano:**

1. `rucene-base nuts search $R --terms descricao=Algarve,Douro --sort nuts:asc --fields nuts,descricao,numContratos` — exit 0, 4 resultado(s)
2. `rucene-base contratos count $R --terms 'nuts=<nuts do passo 1>' --term Ano=2025` — exit 0, 19695 resultado(s)

**Porque R não é menor:** Os códigos de cada designação estão em `nuts`; a contagem em `contratos`.

**Combinação no cliente:** Nenhuma.

**Tipos de campo e operações:**

* `nuts.descricao` (name): terms
* `nuts.nuts` (keyword, chave): sort
* `contratos.nuts` (keyword, multivalor): terms
* `contratos.Ano` (int): term
* saída `--fields` de `nuts`: nuts (keyword), descricao (name), numContratos (int)
* opções: contratos: count

**Resposta (validada):** códigos com essas descrições: PT117 Douro, PT11D Douro, PT15 Algarve, PT150 Algarve; 19695 contratos de 2025 executados nelas

### 95. Para os contratos de um ano com mais modificações, quando e com que fundamento foi feita a primeira modificação?

**Instanciação:** [ano 2025; 3 contratos; chave composta `idcontrato/1`]  
**Id:** `d09` · **R = 2, I = 2** (contratos, modificacoes) · Modificações

**Plano:**

1. `rucene-base contratos search $R --term Ano=2025 --sort numModificacoes:desc --limit 3 --fields idcontrato,numModificacoes` — exit 0, 130761 resultado(s)
2. `rucene-base modificacoes search $R --should --key '<idcontrato n.º 1 do passo 1>/1' --should --key '<idcontrato n.º 2 do passo 1>/1' --should --key '<idcontrato n.º 3 do passo 1>/1' --fields all` — exit 0, 3 resultado(s)

**Porque R não é menor:** O contador está em `contratos`; a modificação n.º 1 em `modificacoes`, pedida pelas três chaves compostas num só pedido.

**Combinação no cliente:** Emparelha por `idcontrato`.

**Tipos de campo e operações:**

* `contratos.Ano` (int): term
* `contratos.numModificacoes` (int): sort
* `modificacoes.idcontrato` (exact, chave): --should key
* `modificacoes.sequencia` (int, chave): --should key
* saída `--fields` de `contratos`: idcontrato (exact), numModificacoes (int)
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoFundamento (name), modifContratoTipoAto (keyword), modifContratoData (date), modifDataPublicacao (date), modifContratoPrecoAlterado (double), modifPrazoExecucao (int), Ano (int), subcontratadaNif (exact), subcontratadaNome (text), modifJustNContratEscrito (name)
* opções: modificacoes: fields all

**Resposta (validada):** 11449321 (76 modif.): 1.ª em 2025-07-31, «Trabalhos Complementares (art. 370.º do CCP)», Acordo entre as partes; 11248782 (30 modif.): 1.ª em 2025-07-21, «Trabalhos Complementares (art. 370.º do CCP)», Acordo entre as partes; 11269413 (23 modif.): 1.ª em 2025-11-07, «Trabalhos Complementares (art. 370.º do CCP)», Acto Administrativo

### 96. Quantos contratos publicaram as entidades de uma região nos últimos dias úteis de um ano, e quais os maiores?

**Instanciação:** [entidades com NUTS de nível 3 PT16E (Região de Coimbra); publicação a 2025-12-29 ou 2025-12-30]  
**Id:** `d11` · **R = 2, I = 2** (adjudicantes, contratos) · Prazos e datas

**Plano:**

1. `rucene-base adjudicantes search $R --term nutsNivel3=PT16E --limit 500 --fields nif` — exit 0, 243 resultado(s)
2. `rucene-base contratos search $R --terms 'adjudicanteNif=<nif do passo 1>' --terms dataPublicacao=2025-12-29,2025-12-30 --sort precoContratual:desc --limit 5 --exact-total --fields idcontrato,adjudicanteNome,precoContratual,dataPublicacao` — exit 0, 104 resultado(s)

**Porque R não é menor:** A sede das entidades só está em `adjudicantes`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `adjudicantes.nutsNivel3` (keyword, nível derivado): term
* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): terms
* `contratos.dataPublicacao` (date): terms
* `contratos.precoContratual` (double): sort
* saída `--fields` de `adjudicantes`: nif (exact)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double), dataPublicacao (date)
* opções: contratos: exact-total

**Resposta (validada):** 243 entidades com nível 3 PT16E; 104 contratos publicados a 29 e 30/12/2025; maiores: Município da Mealhada 720 499,00 € (2025-12-30); INOVA, Empresa de Desenvolvimento Económico e Social de Cantanhede, EM-SA 234 133,85 € (2025-12-30); Universidade Politécnica de Coimbra 191 047,50 € (2025-12-29)

### 97. Que modificações foram publicadas muito depois de feitas, com que atraso, e de que entidades são os contratos?

**Instanciação:** [publicadas em 2025-12; data da modificação até 2023-12-31; 5 mais antigas]  
**Id:** `d13` · **R = 2, I = 2** (modificacoes, contratos) · Prazos e datas

**Plano:**

1. `rucene-base modificacoes search $R --range modifDataPublicacao=2025-12-01..2025-12-31 --range modifContratoData=..2023-12-31 --sort modifContratoData:asc --limit 5 --exact-total --fields idcontrato,sequencia,modifContratoData,modifDataPublicacao` — exit 0, 43 resultado(s)
2. `rucene-base contratos search $R --should --key '<idcontrato n.º 1 do passo 1>' --should --key '<idcontrato n.º 2 do passo 1>' --should --key '<idcontrato n.º 3 do passo 1>' --should --key '<idcontrato n.º 4 do passo 1>' --should --key '<idcontrato n.º 5 do passo 1>' --fields idcontrato,adjudicanteNome,precoContratual` — exit 0, 4 resultado(s)

**Porque R não é menor:** As datas da modificação estão em `modificacoes`; a entidade em `contratos` (cinco chaves numa disjunção, um pedido).

**Combinação no cliente:** Atraso = data de publicação − data da modificação.

**Tipos de campo e operações:**

* `modificacoes.modifDataPublicacao` (date): range
* `modificacoes.modifContratoData` (date): range, sort
* `contratos.idcontrato` (exact, chave): --should key
* saída `--fields` de `modificacoes`: idcontrato (exact), sequencia (int), modifContratoData (date), modifDataPublicacao (date)
* saída `--fields` de `contratos`: idcontrato (exact), adjudicanteNome (name), precoContratual (double)
* opções: modificacoes: exact-total

**Resposta (validada):** 43 modificações publicadas em dezembro de 2025 com data anterior a 2024; atraso das 5 mais antigas: 1391 a 2028 dias; entidades: IAPMEI, I.P., Agência para a Competitividade e Inovação, I.P., Município de Coimbra, Município de Santo Tirso, Ordem Dos Fisioterapeutas

### 98. Que contratos de uma entidade foram concluídos num ano, e os adjudicatários são PME?

**Instanciação:** [Unidade Local de Saúde de Coimbra, NIF 510103448; data de fecho em 2025; 20 mais tardios]  
**Id:** `d15` · **R = 2, I = 2** (contratos, fornecedores) · Prazos e datas

**Plano:**

1. `rucene-base contratos search $R --term adjudicanteNif=510103448 --range dataFechoContrato=2025-01-01..2025-12-31 --sort dataFechoContrato:desc --limit 20 --exact-total --fields idcontrato,adjudicatarioNif,dataFechoContrato` — exit 0, 588 resultado(s)
2. `rucene-base fornecedores get $R --key <adjudicatarioNif do passo 1> …` — exit 0, 9 resultado(s)

**Porque R não é menor:** O atributo PME só está em `fornecedores`.

**Combinação no cliente:** Contagem de PME.

**Tipos de campo e operações:**

* `contratos.adjudicanteNif` (exact, multivalor, ausente=-): term
* `contratos.dataFechoContrato` (date): range, sort
* `fornecedores.nif` (exact, chave): get
* saída `--fields` de `contratos`: idcontrato (exact), adjudicatarioNif (exact), dataFechoContrato (date)
* opções: contratos: exact-total

**Resposta (validada):** 588 contratos da ULS de Coimbra fechados em 2025; nos 20 com fecho mais tardio, 9 adjudicatários, 9 PME; último fecho 2025-12-31

### 99. Nos anúncios de uma entidade num ano, como se compara o preço contratual dos contratos resultantes com o preço base anunciado, e quantos o excederam?

**Instanciação:** [Município do Porto, NIF 501306099; ano 2024; preço base preenchido]  
**Id:** `d19` · **R = 2, I = 2** (anuncios, contratos) · Valores

**Plano:**

1. `rucene-base anuncios search $R --term nifEntidade=501306099 --term Ano=2024 --range PrecoBase=.. --limit 200 --exact-total --fields IDAnuncio,PrecoBase` — exit 0, 135 resultado(s)
2. `rucene-base contratos search $R --terms 'IDAnuncio=<IDAnuncio do passo 1>' --limit 300 --fields idcontrato,IDAnuncio,precoContratual` — exit 0, 176 resultado(s)

**Porque R não é menor:** O preço base anunciado está em `anuncios`; o contratual em `contratos`.

**Combinação no cliente:** Rácio contratual/base por contrato (mediana) e contagem dos que excedem.

**Tipos de campo e operações:**

* `anuncios.nifEntidade` (exact, multivalor, ausente=-): term
* `anuncios.Ano` (int): term
* `anuncios.PrecoBase` (double, ausente=0): range
* `contratos.IDAnuncio` (exact): terms
* saída `--fields` de `anuncios`: IDAnuncio (exact), PrecoBase (double)
* saída `--fields` de `contratos`: idcontrato (exact), IDAnuncio (exact), precoContratual (double)
* opções: anuncios: exact-total
* `--range PrecoBase=..` exclui o marcador `0` (ausente), que não é indexado.

**Resposta (validada):** 135 anúncios do Município do Porto em 2024 com preço base; 176 contratos ligados; mediana contratual/base 0.44; 0 acima do preço base anunciado

### 100. Quantos contratos de um tipo de serviço têm preço exatamente igual a um limiar legal, e por que procedimento foram feitos?

**Instanciação:** [expressão «serviços de limpeza» na descrição CPV; preço 75 000 € (limiar da consulta prévia para bens e serviços)]  
**Id:** `d22` · **R = 2, I = 2** (cpv, contratos) · Valores

**Plano:**

1. `rucene-base cpv search $R --phrase-in 'descricao=serviços de limpeza' --limit 50 --fields cpv,descricao` — exit 0, 27 resultado(s)
2. `rucene-base contratos search $R --terms 'cpvPrincipal=<cpv do passo 1>' --term precoContratual=75000 --sort cpvPrincipal:asc,dataPublicacao:desc --limit 10 --exact-total --fields idcontrato,cpvPrincipal,adjudicanteNome,tipoprocedimento` — exit 0, 11 resultado(s)

**Porque R não é menor:** O vocabulário CPV está em `cpv`; os contratos em `contratos`.

**Combinação no cliente:** Nenhuma além da leitura das linhas.

**Tipos de campo e operações:**

* `cpv.descricao` (name): phrase-in
* `contratos.cpvPrincipal` (keyword): terms, sort
* `contratos.precoContratual` (double): term
* `contratos.dataPublicacao` (date): sort
* saída `--fields` de `cpv`: cpv (keyword), descricao (name)
* saída `--fields` de `contratos`: idcontrato (exact), cpvPrincipal (keyword), adjudicanteNome (name), tipoprocedimento (keyword)
* opções: contratos: exact-total

**Resposta (validada):** 27 códigos com «serviços de limpeza»; 11 contratos de limpeza com preço exatamente 75 000 € (limiar da consulta prévia); primeiros por código: 90470000-2 SIMTEJO Saneamento Integrado dos Municípios do Tejo e Trancão, S.A. (Ajuste Direto Regime Geral); 90610000-6 União das Freguesias de Custóias, Leça do Balio e Guifões (Consulta Prévia); 90611000-3 União das Freguesias de Alvito (São Pedro e São Martinho) e Couto (Consulta Prévia)

## Matriz de cobertura

### Índice × perguntas

| Índice | Perguntas | Pedidos |
| --- | ---: | ---: |
| `contratos` | 91 | 115 |
| `procedimentos` | 17 | 17 |
| `anuncios` | 21 | 22 |
| `fornecedores` | 48 | 48 |
| `modificacoes` | 22 | 23 |
| `convidadas` | 5 | 5 |
| `adjudicantes` | 42 | 42 |
| `cpv` | 19 | 19 |
| `locais` | 9 | 9 |
| `nuts` | 10 | 10 |

### Tipo de campo × operação (número de perguntas)

Células vazias: operação que o tipo não suporta. `0`: suportada e não usada.

| Tipo | term | terms | range | text | text-in | phrase | phrase-in | key | get | sort | fields |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| exact | 23 | 51 |  |  |  |  |  | 2 | 55 |  | 93 |
| keyword | 45 | 11 |  |  |  |  |  | 2 | 16 | 5 | 41 |
| flag | 15 | 0 |  |  |  |  |  |  |  | 1 | 8 |
| name | 5 | 1 |  | 1 | 5 | 1 | 5 |  |  | 4 | 43 |
| text |  |  |  | 1 | 6 | 1 | 3 |  |  |  | 6 |
| int | 41 | 2 | 10 |  |  |  |  | 1 | 1 | 28 | 50 |
| double | 1 | 1 | 8 |  |  |  |  |  |  | 50 | 69 |
| date | 2 | 1 | 8 |  |  |  |  |  |  | 15 | 20 |
| stored |  |  |  |  |  |  |  |  |  |  | 1 |

Notas: `text`/`phrase` contam os campos por omissão do índice (tipos text e name); `key` é `--key` em `search`/`count`; `get` conta o campo-chave.

### Opções e modificadores (número de perguntas)

| Opção | Perguntas |
| --- | ---: |
| --not (double range) | 1 |
| --not (exact terms) | 1 |
| --not (flag term) | 1 |
| --not (keyword key) | 2 |
| --not (keyword term) | 2 |
| --not (keyword terms) | 1 |
| --not (text text-in) | 1 |
| --should (exact key) | 2 |
| --should (exact term) | 1 |
| --should (int key) | 1 |
| --should (keyword term) | 1 |
| --should (text phrase-in) | 1 |
| count | 15 |
| exact-total | 70 |
| fields | 99 |
| fields all | 1 |
| get com chave composta (modificacoes) | 1 |
| phrase (campos por omissão) | 1 |
| sort _score | 2 |
| text (campos por omissão) | 1 |

### Níveis hierárquicos derivados (número de perguntas)

`adjudicantes.nutsNivel1`: 1, `adjudicantes.nutsNivel2`: 1, `adjudicantes.nutsNivel3`: 1, `anuncios.cpvDivisao`: 1, `contratos.cpvCategoria`: 1, `contratos.cpvClasse`: 1, `contratos.cpvDivisao`: 2, `contratos.cpvGrupo`: 1, `contratos.localNivel2`: 1, `contratos.localNivel3`: 1, `contratos.nutsNivel1`: 1, `contratos.nutsNivel3`: 1, `cpv.cpvCategoria`: 1, `cpv.cpvClasse`: 1, `cpv.cpvDivisao`: 1, `cpv.cpvGrupo`: 1, `fornecedores.nutsNivel1`: 1, `locais.localNivel1`: 1, `locais.localNivel2`: 2, `nuts.nutsNivel2`: 1

### Campos multivalor usados (número de perguntas)

`adjudicantes.atividadeEntidade`: 1, `adjudicantes.nome`: 11, `adjudicantes.nutsCod`: 6, `adjudicantes.nutsDesc`: 3, `adjudicantes.tipoEntidade`: 2, `anuncios.anteriorIDAnuncio`: 1, `anuncios.cpv`: 2, `anuncios.cpvDescricao`: 1, `anuncios.criterioAquisicaoTipoFator`: 1, `anuncios.designacaoEntidade`: 4, `anuncios.loteValorBase`: 1, `anuncios.nifEntidade`: 8, `contratos.adjudicanteNif`: 42, `contratos.adjudicanteNome`: 17, `contratos.adjudicatarioNif`: 42, `contratos.adjudicatarioNome`: 1, `contratos.concorrenteNif`: 2, `contratos.convidadaNif`: 3, `contratos.cpv`: 6, `contratos.cpvDescricao`: 2, `contratos.criterioAdjudicacaoTipoFator`: 2, `contratos.justifNReducEscrContrato`: 1, `contratos.localExecucao`: 8, `contratos.nuts`: 5, `contratos.tipoContrato`: 3, `fornecedores.nutsCod`: 3, `modificacoes.subcontratadaNif`: 2, `modificacoes.subcontratadaNome`: 2, `procedimentos.adjudicanteNif`: 3, `procedimentos.objectoContrato`: 1

### Semântica de ausência: intervalos e texto sobre campos com marcador (número de perguntas)

`anuncios.PrecoBase`: 1, `contratos.CausasAlteraPreco`: 1, `contratos.PrecoTotalEfetivo`: 1, `contratos.prazoExecucao`: 1, `contratos.precoBaseProcedimento`: 1, `modificacoes.modifPrazoExecucao`: 1, `procedimentos.precoBaseProcedimento`: 1

### Operações não cobertas

* flag × terms: as flags valem `sim`/`nao` e estão preenchidas em todos os documentos de `contratos` e `anuncios` (em `fornecedores.pme` só existe `sim` ou ausência); `--terms sim,nao` equivale a «todos» ou a `--term`, sem pergunta pertinente.
* sort `_doc`: ordem interna dos documentos no índice, sem significado para nenhuma pergunta do domínio.
