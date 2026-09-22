# Phase 113A — final closure pass 6.1 validation commands (post diff-hygiene recovery)

```text
# --- focused DDeploy filters (report separately, never combined) ---
cargo test --locked --lib core::ddeploy:: -q            # dserver: 181 passed
cargo test --locked --lib api::ddeploy:: -q              # dserver: 24 passed

# --- Phase-113 focused contract/authority tests ---
cargo test --locked --lib canonical_dforward -- -q       # dserver: 7 passed
cargo test --locked --lib canonical_dmap_v2 -- -q         # dserver: 9 passed
cargo test --locked --lib dforward_projection -- -q       # dserver: 11 passed
cargo test --locked --lib dcode_authorization -- -q       # dserver: 20 passed
cargo test --locked --lib dcall -- -q                     # dserver: 73 passed
cargo test --locked --lib canonical_dforward -- -q        # DATUM mirror: 7 passed
cargo test --locked --lib canonical_dmap_v2 -- -q          # DATUM mirror: 7 passed

# --- Phase-113 fixture checkers ---
python -B scripts/check_phase113a_contracts.py
python -B scripts/check_phase113a_contracts_selftest.py

# --- full broad suites (run once each, not just focused subsets) ---
cargo test --locked                                       # dserver: 1317+1 passed
cargo check --locked && cargo check --locked --tests       # dserver

cargo test --locked --lib                                  # DATUM: 1024 passed, 1 ignored (platform)
cargo test --locked --tests -- --test-threads=1             # DATUM: 481 passed, 7 ignored (see below)
cargo check --locked && cargo check --locked --tests        # DATUM

# --- Phase-112 real cross-node D-Call subprocess integration proof ---
$env:DATUM_PROOF_PYTHON='C:/Users/dnred/AppData/Local/Python/pythoncore-3.14-64/python.exe'
cargo test --locked --features dcall-test-support --test dcall_two_node_integration -- --ignored --test-threads=1
# required closure target: 7 passed, 0 failed -- not waived

# --- all 11 root checkers ---
python -B scripts/check_dcall_default_provenance.py
python -B scripts/check_canonical_datum_contracts_v1.py
python -B scripts/check_canonical_datum_contracts_v1_selftest.py
python -B scripts/check_ddeploy_contracts_v1.py
python -B scripts/check_ddeploy_contracts_v1_selftest.py
python -B scripts/check_datum_contract_fixtures.py
python -B scripts/check_datum_contract_fixtures_selftest.py
python -B scripts/check_real_catalog_oci_pinning.py
python -B scripts/check_real_catalog_oci_pinning_selftest.py
python -B scripts/check_phase113a_contracts.py
python -B scripts/check_phase113a_contracts_selftest.py

# --- docs and diff hygiene ---
python -m mkdocs build --strict
git diff --check

# --- formatting: only the 6 genuinely-new Phase-113A files are asserted clean ---
cargo fmt --check -- dserver/src/core/canonical_dforward.rs
cargo fmt --check -- dserver/src/core/canonical_dmap_v2.rs
cargo fmt --check -- dserver/src/core/dforward_projection.rs
cargo fmt --check -- dserver/src/core/ddeploy_v2.rs
cargo fmt --check -- DATUM/src/agent/canonical_dforward.rs
cargo fmt --check -- DATUM/src/agent/canonical_dmap_v2.rs
```

All commands above were actually run against the current worktree during Pass
6.1, after diagnosing and recovering from the `cargo fmt` overreach incident
documented in `test-results.txt` and `PHASE113A_REVIEW.md`. 39 of the 42
files carrying real Phase-113A content plus accidental churn were
successfully cleaned in Pass 6.1; 3 files (`dserver/src/core/ddeploy.rs`,
`dserver/src/api/ddeploy.rs`, `DATUM/src/dcall/delivery/tests/endpoint_authority.rs`)
remain functionally correct but not diff-clean -- see `test-results.txt` for
the exact hunk-level detail. The three canonical DDeploy focused filters are
always reported as three separate numbers, never summed or combined (see the
Pass 4.1 test-count reconciliation note in `test-results.txt`).
Repository-wide `cargo fmt --check` is not asserted clean in this pass --
only the 6 genuinely-new files are individually checked -- because of the
disclosed 3-file incident residue above.
