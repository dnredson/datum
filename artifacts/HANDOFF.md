# DATUM — ponto de retomada em 2026-09-09

Objetivo: um artefato por componente, com arquivos e pré-requisitos versionados,
seguido de deploy governado end-to-end. Esta rodada foi interrompida a pedido do
usuário para economizar cota; salvar o progresso não equivale a aprovar o deploy.

## Estado confirmado

- Base de desenvolvimento: `f4684520dc57bf0ef45f7c984d6873fe90a1abf6` (PR #103).
- Repositório original `C:\Users\dnred\OneDrive\projetos\PD` preservado, incluindo
  alterações existentes. Trabalho novo em checkout isolado, branch
  `codex/artifacts-fog-bundle`. Não fazer reset/clean no original.
- DServer do laboratório: `http://192.168.10.101:8080`, projeto `monitor-f49c8`.
- Último D-Graph observado: `dgraph:o2-mqtt-role-1`, revisão 14. Consultar novamente
  antes de qualquer alteração; preservar todos os outros nós e serviços.
- SSH previamente validado: fog `vboxuser@192.168.10.103`, cloud
  `vboxuser@192.168.10.108`, mist `dener@192.168.10.109`. Chaves e secrets não estão
  no Git. A versão web não deve presumir acesso à LAN ou ao desktop.
- Mist: Gateway Bridge 4.1.2 ARM64, imagem por digest, configuração EU868 montada;
  reconcile inicial implantou, segundo convergiu sem mutação. Snapshot publicado.
- Canary Semtech UDP -> MQTT passou: ACK, gateway ID, timestamp e contadores
  Protobuf conferidos. Uma execução de 31 ms, sem pretensão de benchmark.
- Isso NÃO valida uplink LoRaWAN completo, ChirpStack server, SMA ou armazenamento.

## O que foi salvo

- Catálogo movido para `artifacts/catalog/` com os 21 artefatos existentes.
- Configuração/perfil/teste do mist e evidências sanitizadas em `artifacts/evidence/`.
- Preparação da fog: PostgreSQL, Redis e ChirpStack, volumes, rede, configuração,
  referências de secrets e argumentos nativos. Mosquitto ganhou arquivo explícito.
- Propagação de `runtime.container.command` pelo DServer e pelo agente, execução
  sem shell do host e detecção de divergência quando o comando é não vazio.
- `prepare.py` materializa um bundle local, mantém secrets fora do repositório,
  preserva secrets existentes e recusa sobrescrita de arquivos alterados.
- README com exemplo, pré-requisitos, ordem de execução e limites.

## Validação desta rodada

- DServer: 615 testes passaram, incluindo propagação/validação de command.
- DATUM: 10 testes passaram nos contratos mist, service artifact e deployment
  reconciliation (inclui argumentos literais e rejeição de NUL/limites).
- Python: 3 testes passaram; 1 teste de symlink foi ignorado por falta de
  privilégio Windows. O teste verifica idempotência, configuração, mappings,
  dependências, secrets e preservação de arquivos alterados.
- O resultado do teste adicional de contrato fog está em `evidence/validation.json`.
- Nenhum novo deploy da fog foi executado nesta rodada.

## Próxima rodada, na web

1. Abrir esta branch/PR e revisar o diff. Não reconstruir tudo do zero.
2. Revisar configuração regional EU868, inicialização PostgreSQL e contratos
   de saúde contra as versões exatas das imagens. A configuração regional atual
   é mínima e ainda não foi validada com um servidor ChirpStack em execução.
3. Fixar digests das imagens da fog após validação, atualizando também mappings.
4. Completar testes de integração de command/drift no caminho operacional real.
   Comando vazio mantém default da imagem e não força remoção de override anterior.
5. Rever `prepare.py` no Linux e a política de permissões; o teste de symlinks
   ficou pendente. Evitar sobrescrever bundles ou regenerar senha de volume existente.
6. Atualizar DServer e SmartSentinel antes de usar os novos campos. Preparar uma
   proposta de D-Graph e autorização específica; obter aprovação do usuário antes
   da rodada de deploy na LAN. A alteração do broker pode interromper clientes.
7. Executar PostgreSQL/Redis/ChirpStack, verificar saúde e segunda reconciliação.
8. Depois completar REST API opcional, cloud/SMA e simulação de uplink até persistência.

O catálogo cloud foi preservado, mas os bundles completos de cloud e dispositivos
ainda não estão implementados. Não tratar `scenario.json` como snapshot do estado
atual. Não integrar o PR automaticamente: há validações operacionais pendentes.

No mist o checkout usado foi `/home/dener/datum-mist-f468452`; no fog e cloud
existem checkouts antigos e alterações locais. Inspecioná-los antes de atualizar.
Para testes com VMs privadas, retornar ao desktop ou usar executor com acesso
explicitamente configurado à LAN; não publicar senhas/chaves no chat web ou Git.
