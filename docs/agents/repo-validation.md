---
title: tidas-tools Validation Guide
docType: guide
scope: repo
status: active
authoritative: false
owner: tidas-tools
language: en
whenToUse:
  - when a tidas-tools change is ready for local validation
  - when selecting proof for a Rust domain, executable asset, release, or automation change
  - when writing PR validation notes
whenToUpdate:
  - when canonical checks, supported platforms, scale budgets, or release proof changes
checkPaths:
  - docs/agents/repo-validation.md
  - AGENTS.md
  - .docpact/config.yaml
  - Cargo.toml
  - Cargo.lock
  - crates/**
  - contracts/**
  - assets/**
  - packaging/**
  - migration/**
  - .github/workflows/**
  - .githooks/pre-push
  - scripts/**
lastReviewedAt: 2026-09-07
lastReviewedCommit: d89c14c5bde28729f80ed1a6d960878ed66217cb
lastReviewedNote: "Reviewed for tidas-tools #185: executable-bound native notice bundles retain locked Cargo/native/Rust source evidence, original notices and source-pinned reference terms. Distribution-manifest v2 and package/verify enforce the complete inventory and byte bindings; release jobs collect notices in the native build environment. Source scopes remain distinct from linkage, and public runtime, supported platforms and release authorization are unchanged."
related:
  - ../../AGENTS.md
  - ../../.docpact/config.yaml
  - ./repo-architecture.md
  - ./cli-contract.md
  - ../../README.md
---

# Validation guide

## Default baseline

Run this for every non-documentation change:

```bash
scripts/audit-rust-only.sh
cargo run --locked -p tidas-assets --bin tidas-asset-lock -- check
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
scripts/sync-rust-package-assets.sh check
scripts/publish-crates.sh check
```

The asset command checks both the paired English/Chinese schema contract and
the complete executable-asset byte lock. The pre-push hook adds strict
Docpact. Pull requests run the product matrix on Linux x86_64/ARM64, macOS
Apple Silicon, and Windows x86_64. macOS Intel and Windows ARM64 are
intentionally absent; the POSIX installer rejects macOS Intel before any
download.

Windows qualification uses `CARGO_BUILD_TARGET=x86_64-pc-windows-msvc`,
target-specific `-C target-feature=+crt-static` and vcpkg's
`x64-windows-static` triplet. The release job reads the built PE's dependency
and import tables with the selected Visual Studio DUMPBIN, rejects unbundled
VC runtime/XML DLLs, then performs repeated package/checksum verification and
the extracted archive smoke. A smoke on a developer runner alone cannot prove
closure: v0.2.1's `VCRUNTIME140.dll` import was masked by installed VC tools.
The corrected artifact must remove that dependency rather than require a user
to install a redistributable or copy a development-machine DLL.

## Validation matrix

| Change | Minimum local proof | Higher-risk proof |
| --- | --- | --- |
| CLI, contracts, or shared runtime | baseline; root and affected command help; deterministic JSON/version/completion; report/stdout separation; usage and exit-class tests | configuration precedence, cancellation, bounded queues, memory accounting, spool determinism, and all affected JSON Schema contracts |
| conversion | focused conversion + CLI tests; both directions; representative category round-trips; schema-order/XSD proof with scrambled JSON members; envelope and projection-recovery sidecars; source-semantic hash proof; tree hash; symlink, invalid XML, cancellation, budget, rollback | run the local package twice, validate every projected XML document, recover every adapted TIDAS fragment, compare tree hashes, and record wall time/RSS |
| import | all supported format fixtures; native target validation; deterministic package/mapping/bundle hashes; malformed/unsupported input, cancellation, budget, atomic publication | large exchange/issue-spool fixture with wall time/RSS and cross-root determinism |
| export | focused crate/CLI tests; report schema; secret redaction; unsafe paths; cancellation/budget; version suffixes; deterministic ZIP; atomic replacement | disposable local PostgreSQL and S3-compatible fixtures twice, comparing archive bytes and membership |
| release | closure/order/round-trip golden fixtures; missing/inexact reference failure; four deterministic ZIPs; native validation; cancellation/budget; atomic directory publication | run the local 237 MiB package twice, compare all four archives, and record wall time/RSS |
| validation/batch/references | compile every bundled schema/XSD root offline; schema and semantic fixtures including internal keyrefs; complete TIDAS projection/XSD/recovery proof; explicit schema-only diagnostic behavior; oversized rejected-instance event below the 1 MiB frame ceiling; bounded issue spool; batch preflight/drift/final-event hash; extraction schema/roles | local large-package validation twice, recording native time, projection/XSD/recovery time, peak RSS, cancellation, and spool hash |
| assets | baseline asset check; representative `git check-attr eol`; schema-local-reference and translation-parity tests | regenerate locks only after reviewing every changed path/hash; compare fingerprints twice |
| XML/XSD/XSLT | focused `tidas-xml` and validation tests; resolver/security tests; four-platform CI | representative production schemas/stylesheets and static-release dependency inspection |
| native distribution | focused `tidas-dist`; package twice; archive/checksum equality; extract and run version/help/JSON/ruleset; installer syntax and hermetic installer contract tests | four release jobs, clean-machine archive execution, runtime dependency inspection, SBOM and attestation |
| crates.io | sync check; public-set qualification; verify exact version set and `tidas-dist` exclusion; script syntax | inspect each `.crate`; source install; registry absent/existing checksum simulations without a real token |
| release request or final migration marker | shell syntax; tamper/append-only validation; actionlint; strict Docpact | simulate modification/multiple-file/target/tag/ancestry conflicts and confirm exact-tag workflow dispatch |
| governed docs only | strict Docpact config validation and enforced lint | one focused route rendering for the changed intent |

Scale proofs must run locally first. The canonical large package is outside
Git:

```text
/Users/biao/Code/lca-workspace/lca-workspace/_test_data/tidas-package-open_data-1784707539957.zip
```

Acceptance limits:

- native schema validation: at most 60 seconds
- unzip/hash/parse/validate/spool, excluding remote download: at most 3 minutes
- peak RSS: at most 512 MiB
- issue-detail memory must remain bounded even near 1.07 million issues

## PR validation note

Record:

1. exact baseline commands and results;
2. focused CLI/domain probes and input formats;
3. scale wall time, peak RSS, output/spool hashes, and repeat count when run;
4. package/archive/clean-machine proof when distribution changes;
5. whether the owned asset paths require a downstream `tidas-sdk` refresh;
6. any cross-platform proof left to CI.

Do not claim a deferred cross-platform job or external connector test as a
local pass.

For notice input collection, run `cargo test --locked -p tidas-dist` and the
internal exporters against the actual locked Cargo registry archives, native
installation and Rust sysroot. Regressions must cover tampered crate and
supplement bytes, missing native/toolchain material, symlink traversal,
repeatable output and preservation of an existing output on failure. Record
the selected target and package counts as source-input evidence. Then generate
the executable-bound bundle from a clean committed source tree with the actual
static-build environment and Rust source/documentation components. Package
twice and run archive verification and smoke. Tests must reject a foreign
binary, omitted dependency records, altered canonical terms even with updated
local hashes, and deleted archive notices even with an updated outer checksum.
Distribution-manifest v2 and the complete notice tree must qualify on all four
native jobs before a versioned release. Source-only exports do not prove that
native packaging or publication has completed.

## Local Docpact push gate

```bash
./scripts/install-git-hooks.sh
```

The hook runs strict Docpact, the Rust-only audit, both asset locks, format,
clippy, and all workspace tests. Override an unusual comparison only with an
explicit `DOCPACT_BASE_REF`.
