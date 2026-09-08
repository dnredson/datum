# Stage 87 — local secret resolution

Stage 87 connects the node-local resolver contract to governed
SmartSentinel container execution.

The cloud config contains only an allowlist mapping an opaque
`secret_ref` to a local environment-variable name. It does not contain
secret values.

Before a container is created, SmartSentinel:

1. validates that every artifact reference has an allowlisted binding;
2. reads the value only from the local process environment;
3. blocks execution if a binding is missing or empty;
4. passes only the variable name to Docker (`--env NAME`), with the value
   inherited through the local process environment;
5. never includes the value in reports, status messages, or command arguments.

Stopped containers that require secrets are not started, because Docker
cannot change their environment without recreation. No secret is resolved
during dry-run/preflight, and missing local material remains fail-closed.

The next validation should run without setting the eight cloud secret
variables and confirm `execution_status=blocked_missing_secret`,
`host_mutation_performed=false`, and
`side_effects_performed=false`.
