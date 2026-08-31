# I38 — Regression Validation

- date: `2026-08-31`
- checkpoint: `I38/144`
- status: `PASS`

Observed on the validated I38 branch:

```text
cargo fmt --manifest-path DATUM/Cargo.toml -- --check: PASS
cargo test --manifest-path DATUM/Cargo.toml: 426 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
cargo build --manifest-path DATUM/Cargo.toml: PASS
```

The build completed in the dev profile and emitted 43 DATUM warnings. They were non-blocking for the I38 regression gate.

The authoritative I38 live evidence is recorded in `docs/I38_SENTINEL_DEPLOYMENT_INBOX_FINAL.md`.