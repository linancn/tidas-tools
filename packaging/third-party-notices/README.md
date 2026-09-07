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

These exports are reviewable source inputs to TIDAS #185. The Rust report and
objc2 explanation can contain references that require additional original
terms; neither is silently classified as complete standalone license text.
Executable/build binding, complete retained terms, native archive integration
and archive verification remain required before publication qualification.
