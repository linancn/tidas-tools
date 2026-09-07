# Retained upstream license evidence

These sources supplement the exact locked crate archives that omit standalone
license files. Each record binds the crate version/checksum, its packaged VCS
revision, the immutable upstream document and the retained text digest. The Git
blob object hash was independently checked while retrieving the document.

`upstream-license-text` is a retained license text. `upstream-licensing-notice`
is the upstream project's licensing explanation, including its references and
caveats; it must not be silently treated as a complete standalone license text
or an assertion about copyright holders.

`tidas-dist cargo-notices` reads the locked, target-filtered Cargo graph and
verifies each registry archive against `Cargo.lock` before collecting material.
It walks normal and build dependency edges from the `tidas` root, excludes
development edges, and retains declared target kinds and resolved features.
Normal/build scopes describe dependency edges, not linked executable contents;
in particular, a normal proc-macro dependency is still a host tool.

`tidas-dist native-notices` retains every installed target port's original
`share/<port>/copyright`, the vcpkg status inventory, and the explicitly supplied
Rust sysroot's project terms and library notice report. Both exporters write
deterministic, content-addressed text files and JSON into a fresh directory.
Missing material, changed checksums, unsupported inputs and symlink traversal
in native/toolchain material fail collection.

The pinned POSIX `libiconv` port can install only a CMake wrapper for the system
library. When its copyright file is absent, collection requires the exact
installed wrapper/metadata file inventory and retains the inventory and wrapper
as build evidence. Additional files, missing inventory or missing files fail;
Windows and other packages cannot use this case. This record does not claim to
retain a compiled library's license.

`referenced-terms.json` pins canonical license/exception text to the immutable
SPDX License List Data v3.28.0 commit. Each retained file has a verified Git
blob identity, SHA-256 and byte length. The original reference text, including
any template notation, is preserved verbatim; it is not filled in with guessed
copyright holders or substituted for upstream attribution.

The complete `tidas-dist notices` producer retains these terms separately from
the objc2 licensing explanation and from original Rust project/library notices.
It also retains license/copyright files from the actual installed `rust-src`
library tree, including compiler-builtins and libm. The package verifier checks
canonical term records against the reviewed source-pinned catalog, so replacing
terms and recomputing local hashes does not qualify them.

The executable-bound bundle includes the locked Cargo inputs, original native
notices, source/toolchain evidence and the complete retained text inventory.
It is carried by distribution-manifest v2 and checked before archive smoke.
See [the packaging contract](../README.md) for scope, provenance and commands.

`rust-project-licenses.json` supplies original Rust project terms for the exact
Rust 1.98.1 compiler commit. Their bytes were checked both against the immutable
Rust source Git blobs and the compiler archive authenticated by Rust's official
release manifest. These are the original compiler-project files omitted from
rustup's installed component tree, not generic replacement license templates.
The complete producer selects them only when both project terms are absent
from the sysroot and the observed compiler identity matches exactly. It still
requires the installed library report and original rust-src material.
