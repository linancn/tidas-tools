# Native package-manager metadata

`tidas-dist metadata` generates Homebrew and Winget manifests from the
SHA-256 sidecars of the same four archives published by the release workflow.
The package-manager paths never rebuild `tidas`.

For a completed `v0.1.0` artifact set:

```bash
cargo run --locked -p tidas-dist -- metadata \
  --release-base-url \
  https://github.com/tiangong-lca/tidas-tools/releases/download/v0.1.0 \
  --artifacts-dir dist/artifacts \
  --output-dir dist/package-metadata
```

The output is:

- `homebrew/tidas.rb`, ready to copy into an approved `homebrew-*` tap;
- the three `winget/TianGong.Tidas*.yaml` manifests, ready for `winget
  validate` and a separately approved community submission.

The release workflow validates and uploads these generated files with every
tag. Creating an external tap repository or submitting to `microsoft/winget-pkgs`
is intentionally a separate, human-approved publication action.

## Internal notice input collection

The repository-internal `tidas-dist` tool can export source material for native
notice review without changing or publishing the product executable:

```bash
cargo run --locked -p tidas-dist -- cargo-notices \
  --target aarch64-apple-darwin --output-dir dist/cargo-notice-inputs
cargo run --locked -p tidas-dist -- native-notices \
  --target aarch64-apple-darwin \
  --vcpkg-installed /absolute/path/to/vcpkg/installed \
  --rust-sysroot /absolute/path/to/rust/sysroot \
  --output-dir dist/native-notice-inputs
```

Use the actual build's vcpkg installation and Rust sysroot. The supported
target determines the vcpkg triplet, including `x64-windows-static` on Windows.
Output directories must be new; failures preserve existing outputs. Collection
uses locked Cargo archive checksums and retained upstream supplements, installed
native copyright material, and original Rust library notices. Each JSON export
identifies its scope and text hashes; host paths are not emitted.

These commands collect source inputs. The complete packaging path below binds
those inputs to an executable and retains the additional referenced terms.
See [the retained source evidence](third-party-notices/README.md).

## Executable-bound native notices

All CI and release jobs use Rust 1.98.1, which is also the declared source
build minimum. The release job installs `rust-src` and `rust-docs`, builds the exact native
executable with pinned static XML inputs, then runs the internal `notices`
command in the same build environment:

```bash
cargo run --locked --release -p tidas-dist -- notices \
  --binary "$EXECUTABLE" --target "$TARGET" \
  --vcpkg-root "$RUNNER_TEMP/vcpkg" --output-dir dist/notices
cargo run --locked --release -p tidas-dist -- package \
  --binary "$EXECUTABLE" --license LICENSE --notices-dir dist/notices \
  --target "$TARGET" --output-dir dist/first
```

`notices` requires clean committed source and vcpkg checkouts, the source-pinned
vcpkg baseline, the static XML build environment (including Windows static CRT),
the actual Rust sysroot and the selected target's compiled library inventory.
Rustup does not copy the compiler archive's root LICENSE-MIT/LICENSE-APACHE
files into its installed sysroot. For that layout, the producer requires
original project terms retained for the exact observed compiler release and
commit; unknown compiler identities and partial installed terms fail.
The `tidas.native-notice-bundle.v1` manifest binds the executable digest and
length, source commit, Cargo.lock, installed native package/feature set and
Rust compiler identity. Original text files and source evidence are retained
with exact byte lengths and SHA-256 digests. Publication stages in a sibling
directory and preserves existing outputs on failure.

Cargo normal/build edges and declared target kinds describe resolved source
inputs; normal proc-macro dependencies and vcpkg helpers remain build inputs.
Rust library notices retain a conservative source superset, including original
compiler-builtins/libm material. The target library inventory identifies the
installed compilation inputs; it does not claim every installed library was
linked. This distinction avoids treating development or host tools as shipped
runtime components. Official release build provenance establishes the
source-to-binary relationship.

`package` requires a verified notice bundle matching its exact executable,
target, version and project license. The resulting
`tidas.distribution-manifest.v2` adds `third_party_notices` with the byte length
and SHA-256 of
`share/licenses/tidas/third-party-notices/notice-manifest.json`. The same
directory contains all listed source evidence and `texts/<sha256>.txt` files.
The public product remains the single `bin/tidas` (or `bin/tidas.exe`).

`verify` checks the complete archive inventory, the nested manifest, all text
hashes, retained Cargo/native package sets, toolchain evidence and canonical
reference-term digests before optional smoke execution. Missing, changed,
foreign or unlisted material fails verification, including after a caller
recomputes the outer archive checksum. Archive extraction rejects links,
duplicate/unsafe paths, unexpected roots and oversized inventories. Legacy v1
archives do not satisfy the new notice requirement; existing immutable releases
are not rewritten.
