pub fn host_id(hostname: &str) -> String {
    format!("host:{}", hostname)
}

pub fn container_id(engine: &str, container_id: &str) -> String {
    format!("container:{}:{}", engine, container_id)
}

pub fn process_id(scope: &str, name: &str) -> String {
    // scope examples: "host:<hostname>" or "container:docker:<id>"
    format!("process:{}:{}", scope, name)
}

pub fn check_id(target_id: &str, check_type: &str, name: &str) -> String {
    format!("check:{}:{}:{}", target_id, check_type, name)
}
