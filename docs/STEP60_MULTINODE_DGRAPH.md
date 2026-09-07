# Implementando etapa 60/100 — D-Graph multinode real

A etapa 60 adiciona a validação de metadados multinode ao D-Graph operacional, sem alterar o contrato legado.

Um D-Graph multinode pode declarar em `metadata.node_descriptors` um descritor para cada nó:

- `node_id`;
- `continuum_role`;
- recursos com `resource_id`, `resource_kind` e atributos;
- relações continuam em `metadata.relations`.

A publicação é rejeitada quando:

- falta descritor para algum nó;
- existe descritor duplicado;
- o descritor referencia um nó inexistente;
- o papel do nó está vazio;
- um recurso está incompleto;
- há identidade de recurso duplicada.

O validador não cria telemetria, não executa deploy e não altera o estado de nenhum host. D-Graphs antigos sem `node_descriptors` continuam compatíveis.

Exemplo de forma aceita:

```json
{
  "schema_version": "0.1.0",
  "dgraph_id": "dgraph:continuum-1",
  "revision": 5,
  "project_id": "monitor-f49c8",
  "nodes": [
    {"node_id": "fog-01", "desired_services": []},
    {"node_id": "fog-02", "desired_services": []},
    {"node_id": "cloud-01", "desired_services": []}
  ],
  "metadata": {
    "node_descriptors": [
      {
        "node_id": "fog-01",
        "continuum_role": "fog",
        "resources": [
          {"resource_id": "host:fog-01", "resource_kind": "host"}
        ]
      }
    ],
    "relations": []
  }
}
```

A publicação de uma topologia real de três nós será feita separadamente, após validar os recursos e serviços existentes no laboratório.
