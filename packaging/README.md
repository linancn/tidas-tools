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

These commands collect source inputs. They do not establish which code is
linked into an executable, resolve every notice's referenced terms, or qualify
a native archive for redistribution. Archive integration and executable-bound
verification remain work under #185. See
[the retained source evidence](third-party-notices/README.md).
