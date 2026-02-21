pub fn host_id(hostname: &str) -> String {
    format!("host:{}", hostname)
}

pub fn container_id(engine: &str, container_id: &str) -> String {
    format!("container:{}:{}", engine, container_id)
}

// Mantido por compatibilidade (antigo)
pub fn process_id(scope: &str, name: &str) -> String {
    // scope examples: "host:<hostname>" or "container:docker:<id>"
    format!("process:{}:{}", scope, name)
}

// Novo: processo consolidado (estável)
pub fn process_group_id(os: &str, name_norm: &str) -> String {
    format!("process:{}:{}", os, name_norm)
}

// Novo: conexão (ping) estável
pub fn connection_id(from_host_id: &str, target_norm: &str) -> String {
    format!("connection:{}:{}", from_host_id, target_norm)
}

pub fn check_id(target_id: &str, check_type: &str, name: &str) -> String {
    format!("check:{}:{}:{}", target_id, check_type, name)
}

// Helper para IDs
pub fn sanitize_id_component(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' })
        .collect()
}
