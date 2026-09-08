# Etapa 75/100 — endurecimento da fronteira de inferência externa

A etapa 75 fecha as garantias de segurança da fronteira criada na etapa 74.

## Requisições emitidas

O dserver mantém uma capacidade limitada de requisições de inferência emitidas pelo próprio processo. A resposta só é analisada quando o `request_id` pertence a um envelope previamente entregue pelo endpoint:

`GET /api/v1/alternative-paths/inference-request`

Isso vincula a resposta ao contexto exato que foi entregue à IA. Uma resposta com um ID inventado, expirado ou emitido por outro processo é rejeitada.

A capacidade é limitada a 256 requisições para evitar crescimento ilimitado de memória.

## Topologia

Cada salto consecutivo de uma projeção é conferido contra as relações observadas no contexto emitido:

- relações configuradas;
- relações observadas;
- arestas de conectividade entre peers;
- arestas de conectividade medidas.

Assim, uma IA não pode retornar um caminho composto apenas por nós conhecidos, mas por uma ligação inexistente no grafo observado.

## Efeitos colaterais

A validação agora exige `side_effects_performed=false`:

- no nível superior da resposta;
- dentro do objeto `execution`;
- em `changes_applied`;
- em `execution_allowed`;
- no modo `proposal_only`.

Qualquer contradição retorna HTTP 400.

## Resultado

A resposta aceita continua sendo normalizada como:

`datum_validated_ai_what_if_projection`

Ela permanece apenas uma proposta. Não é persistida, executada, implantada ou aplicada ao D-Graph.
