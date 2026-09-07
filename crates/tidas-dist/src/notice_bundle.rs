//! Executable-bound native notice bundles. Official build provenance remains
//! the authority for the relationship between a source commit and its binary.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::notices::{CargoNoticePackage, NativeNoticePackage, NoticeText};
use crate::{DistError, notices};

pub const MANIFEST: &str = "notice-manifest.json";
pub const SCHEMA: &str = "tidas.native-notice-bundle.v1";
const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDigest {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSource {
    pub repository: String,
    pub commit: String,
    pub cargo_lock_sha256: String,
    pub vcpkg_commit: String,
    pub vcpkg_triplet: String,
    pub rustc_commit: String,
    pub rustc_release: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeBundleManifestV1 {
    pub schema_version: String,
    pub product: String,
    pub version: String,
    pub target: String,
    pub executable: FileDigest,
    pub source: BuildSource,
    pub cargo_packages: Vec<CargoNoticePackage>,
    pub native_packages: Vec<NativeNoticePackage>,
    pub rust_library_texts: Vec<NoticeText>,
    pub files: BTreeMap<String, FileDigest>,
}

pub struct CollectRequest<'a> {
    pub binary: &'a Path,
    pub target: &'a str,
    pub version: &'a str,
    pub vcpkg_root: &'a Path,
    pub output_dir: &'a Path,
}

pub fn collect(request: &CollectRequest<'_>) -> Result<NoticeBundleManifestV1, DistError> {
    crate::validate_target(request.target)?;
    require_new_output(request.output_dir)?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    clean_source(&workspace)?;
    let commit = git_head(&workspace)?;
    let executable = file_digest(request.binary)?;
    let vcpkg_root = request.vcpkg_root.canonicalize()?;
    clean_source(&vcpkg_root)?;
    validate_native_build_inputs(&vcpkg_root, request.target)?;
    let vcpkg_commit = git_head(&vcpkg_root)?;
    let vcpkg_manifest = read_bounded(&workspace.join("packaging/vcpkg/vcpkg.json"), MAX_FILE)?;
    let native_manifest: serde_json::Value = serde_json::from_slice(&vcpkg_manifest)?;
    if native_manifest["builtin-baseline"] != vcpkg_commit {
        return Err(invalid(
            "vcpkg checkout does not match the source-pinned baseline",
        ));
    }
    let triplet = triplet(request.target)?;
    let installed = vcpkg_root.join("installed");
    let version_output = command_output(Command::new("rustc").args(["--version", "--verbose"]))?;
    let sysroot_output = command_output(Command::new("rustc").args(["--print", "sysroot"]))?;
    let sysroot = Path::new(sysroot_output.trim()).canonicalize()?;
    let (cargo, lock) = notices::collect_current_cargo_notice_inputs(request.target)?;
    let native = notices::collect_vcpkg_notice_inputs(&installed, triplet)?;
    let rust = notices::collect_rust_distribution_notice_inputs(
        &sysroot,
        &workspace.join("packaging/third-party-notices"),
    )?;
    let status = read_bounded(&installed.join("vcpkg/status"), MAX_FILE)?;
    if digest(&status).sha256 != native.installed_status_sha256 {
        return Err(invalid("native installation changed during collection"));
    }
    let mut contents = BTreeMap::new();
    source_inventory_files(&mut contents, &sysroot, request.target, &cargo.packages)?;
    for texts in [cargo.texts, native.texts, rust.contents] {
        for (hash, bytes) in texts {
            insert_file(&mut contents, format!("texts/{hash}.txt"), bytes)?;
        }
    }
    insert_file(
        &mut contents,
        "evidence/Cargo.lock".to_owned(),
        lock.clone(),
    )?;
    insert_file(
        &mut contents,
        "evidence/vcpkg-status.txt".to_owned(),
        status,
    )?;
    insert_file(
        &mut contents,
        "evidence/vcpkg-manifest.json".to_owned(),
        vcpkg_manifest,
    )?;
    insert_file(
        &mut contents,
        "evidence/rustc-version.txt".to_owned(),
        version_output.as_bytes().to_vec(),
    )?;
    insert_file(
        &mut contents,
        "README.txt".to_owned(),
        bundle_explanation().as_bytes().to_vec(),
    )?;
    let manifest = NoticeBundleManifestV1 {
        schema_version: SCHEMA.to_owned(),
        product: "tidas".to_owned(),
        version: request.version.to_owned(),
        target: request.target.to_owned(),
        executable,
        source: BuildSource {
            repository: "https://github.com/tiangong-lca/tidas-tools".to_owned(),
            commit,
            cargo_lock_sha256: digest(&lock).sha256,
            vcpkg_commit,
            vcpkg_triplet: triplet.to_owned(),
            rustc_commit: rustc_field(&version_output, "commit-hash")?.to_owned(),
            rustc_release: rustc_field(&version_output, "release")?.to_owned(),
        },
        cargo_packages: cargo.packages,
        native_packages: native.packages,
        rust_library_texts: rust.texts,
        files: contents
            .iter()
            .map(|(path, bytes)| (path.clone(), digest(bytes)))
            .collect(),
    };
    clean_source(&workspace)?;
    if git_head(&workspace)? != manifest.source.commit
        || file_digest(request.binary)? != manifest.executable
    {
        return Err(invalid(
            "source or executable changed during notice collection",
        ));
    }
    write_bundle(request, &manifest, &contents)?;
    Ok(manifest)
}

fn write_bundle(
    request: &CollectRequest<'_>,
    manifest: &NoticeBundleManifestV1,
    contents: &BTreeMap<String, Vec<u8>>,
) -> Result<(), DistError> {
    let parent = request
        .output_dir
        .parent()
        .ok_or_else(|| invalid("notice output has no parent"))?;
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    for (path, bytes) in contents {
        let destination = stage.path().join(path);
        fs::create_dir_all(
            destination
                .parent()
                .ok_or_else(|| invalid("invalid bundle path"))?,
        )?;
        fs::write(destination, bytes)?;
    }
    let mut bytes = serde_json::to_vec_pretty(manifest)?;
    bytes.push(b'\n');
    fs::write(stage.path().join(MANIFEST), bytes)?;
    verify(
        stage.path(),
        request.binary,
        request.target,
        request.version,
    )?;
    require_new_output(request.output_dir)?;
    fs::rename(stage.path(), request.output_dir)?;
    Ok(())
}

pub fn verify(
    root: &Path,
    binary: &Path,
    target: &str,
    version: &str,
) -> Result<NoticeBundleManifestV1, DistError> {
    crate::validate_target(target)?;
    let manifest: NoticeBundleManifestV1 =
        serde_json::from_slice(&read_bounded(&root.join(MANIFEST), MAX_FILE)?)?;
    if manifest.schema_version != SCHEMA
        || manifest.product != "tidas"
        || manifest.target != target
        || manifest.version != version
        || manifest.executable != file_digest(binary)?
    {
        return Err(invalid(
            "notice bundle does not match the executable, target, version or schema",
        ));
    }
    let mut expected: BTreeSet<_> = manifest.files.keys().cloned().collect();
    expected.insert(MANIFEST.to_owned());
    let mut actual = BTreeSet::new();
    let mut total = 0_u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink()
            || (!entry.file_type().is_file() && !entry.file_type().is_dir())
        {
            return Err(invalid("non-regular notice bundle member"));
        }
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| invalid("bundle path escaped root"))?;
        crate::validate_relative(path)?;
        let path = crate::portable(path)?;
        if !expected.contains(&path) || !actual.insert(path.clone()) || actual.len() > 8192 {
            return Err(invalid("unexpected or oversized notice bundle inventory"));
        }
        let bytes = read_bounded(entry.path(), MAX_FILE)?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or(DistError::SizeOverflow)?;
        if total > MAX_TOTAL || (path != MANIFEST && manifest.files[&path] != digest(&bytes)) {
            return Err(invalid(
                "notice bundle member digest, length or bound mismatch",
            ));
        }
    }
    if actual != expected {
        return Err(invalid("notice bundle is missing required files"));
    }
    validate_source(root, &manifest)?;
    validate_packages(&manifest)?;
    Ok(manifest)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RustTargetLibraries {
    target: String,
    libraries: BTreeMap<String, FileDigest>,
}

fn source_inventory_files(
    contents: &mut BTreeMap<String, Vec<u8>>,
    sysroot: &Path,
    target: &str,
    packages: &[CargoNoticePackage],
) -> Result<(), DistError> {
    insert_file(
        contents,
        "evidence/cargo-packages.json".to_owned(),
        serde_json::to_vec_pretty(packages)?,
    )?;
    let libdir = sysroot.join("lib/rustlib").join(target).join("lib");
    let mut libraries = BTreeMap::new();
    for entry in fs::read_dir(libdir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("Rust target library name is not UTF-8"))?;
        if Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
        {
            libraries.insert(name, file_digest(&entry.path())?);
        }
    }
    let record = RustTargetLibraries {
        target: target.to_owned(),
        libraries,
    };
    validate_rust_target_libraries(&record, target)?;
    insert_file(
        contents,
        "evidence/rust-target-libraries.json".to_owned(),
        serde_json::to_vec_pretty(&record)?,
    )
}

fn validate_rust_target_libraries(
    record: &RustTargetLibraries,
    target: &str,
) -> Result<(), DistError> {
    if record.target != target
        || record.libraries.len() > 256
        || record.libraries.iter().any(|(name, digest)| {
            !Path::new(&name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
                || name.contains(['/', '\\'])
                || digest.bytes == 0
                || !hex_id(&digest.sha256, 64)
        })
    {
        return Err(invalid("invalid Rust target library inventory"));
    }
    for name in ["libstd-", "libcore-", "liballoc-", "libcompiler_builtins-"] {
        if !record.libraries.keys().any(|file| file.starts_with(name)) {
            return Err(invalid("Rust target library inventory is incomplete"));
        }
    }
    Ok(())
}

fn validate_source(root: &Path, manifest: &NoticeBundleManifestV1) -> Result<(), DistError> {
    let source = &manifest.source;
    if source.repository != "https://github.com/tiangong-lca/tidas-tools"
        || !hex_id(&source.commit, 40)
        || !hex_id(&source.vcpkg_commit, 40)
        || !hex_id(&source.rustc_commit, 40)
        || source.vcpkg_triplet != triplet(&manifest.target)?
    {
        return Err(invalid("invalid native notice source identity"));
    }
    let lock = read_bounded(&root.join("evidence/Cargo.lock"), MAX_FILE)?;
    if digest(&lock).sha256 != source.cargo_lock_sha256 {
        return Err(invalid("notice Cargo lock differs from its source binding"));
    }
    let vcpkg: serde_json::Value = serde_json::from_slice(&read_bounded(
        &root.join("evidence/vcpkg-manifest.json"),
        MAX_FILE,
    )?)?;
    if vcpkg["builtin-baseline"] != source.vcpkg_commit {
        return Err(invalid("native baseline differs from source evidence"));
    }
    let rust = read_bounded(&root.join("evidence/rustc-version.txt"), MAX_FILE)?;
    let rust = std::str::from_utf8(&rust).map_err(|_| invalid("rustc evidence is not UTF-8"))?;
    if rustc_field(rust, "commit-hash")? != source.rustc_commit
        || rustc_field(rust, "release")? != source.rustc_release
    {
        return Err(invalid("Rust toolchain differs from source evidence"));
    }
    for path in [
        "README.txt",
        "evidence/Cargo.lock",
        "evidence/vcpkg-status.txt",
        "evidence/vcpkg-manifest.json",
        "evidence/rustc-version.txt",
        "evidence/cargo-packages.json",
        "evidence/rust-target-libraries.json",
    ] {
        if !manifest.files.contains_key(path) {
            return Err(invalid("notice manifest omits required source evidence"));
        }
    }
    validate_source_inventories(root, manifest, &lock)
}

fn validate_source_inventories(
    root: &Path,
    manifest: &NoticeBundleManifestV1,
    lock: &[u8],
) -> Result<(), DistError> {
    let packages: Vec<CargoNoticePackage> = serde_json::from_slice(&read_bounded(
        &root.join("evidence/cargo-packages.json"),
        MAX_FILE,
    )?)?;
    if packages != manifest.cargo_packages {
        return Err(invalid(
            "Cargo notice package set differs from collected source evidence",
        ));
    }
    let checksums = notices::lock_checksums(
        std::str::from_utf8(lock).map_err(|_| invalid("Cargo lock is not UTF-8"))?,
    )?;
    for package in &packages {
        if let Some(checksum) = &package.registry_checksum
            && checksums.get(&(
                package.name.clone(),
                package.version.clone(),
                "registry+https://github.com/rust-lang/crates.io-index".to_owned(),
            )) != Some(checksum)
        {
            return Err(invalid(
                "Cargo notice archive differs from retained Cargo.lock",
            ));
        }
    }
    let status = read_bounded(&root.join("evidence/vcpkg-status.txt"), MAX_FILE)?;
    let status = std::str::from_utf8(&status)
        .map_err(|_| invalid("native status is not UTF-8"))?
        .replace("\r\n", "\n");
    let installed = notices::vcpkg_status_packages(&status, &manifest.source.vcpkg_triplet)?;
    let packages: BTreeMap<_, _> = manifest
        .native_packages
        .iter()
        .map(|package| {
            (
                package.name.clone(),
                (
                    package.version.clone(),
                    package.features.iter().cloned().collect(),
                ),
            )
        })
        .collect();
    if installed != packages {
        return Err(invalid(
            "native notice package set differs from installed source evidence",
        ));
    }
    let libraries: RustTargetLibraries = serde_json::from_slice(&read_bounded(
        &root.join("evidence/rust-target-libraries.json"),
        MAX_FILE,
    )?)?;
    validate_rust_target_libraries(&libraries, &manifest.target)
}

fn validate_packages(manifest: &NoticeBundleManifestV1) -> Result<(), DistError> {
    let mut cargo = BTreeSet::new();
    for package in &manifest.cargo_packages {
        if !cargo.insert((&package.name, &package.version))
            || package.scopes.is_empty()
            || package
                .scopes
                .iter()
                .any(|scope| scope != "rust-normal" && scope != "rust-build")
            || package.declared_license.is_empty()
        {
            return Err(invalid("invalid Cargo notice package scope or identity"));
        }
        if package
            .registry_checksum
            .as_ref()
            .is_some_and(|hash| !hex_id(hash, 64))
        {
            return Err(invalid("invalid locked Cargo archive digest"));
        }
        verify_texts(&package.texts, &manifest.files)?;
        if package
            .texts
            .iter()
            .any(|text| text.kind == "licensing-notice")
            && !package.texts.iter().any(|text| {
                text.kind == "referenced-license-terms" && text.license_id.as_deref() == Some("MIT")
            })
        {
            return Err(invalid(
                "upstream licensing explanation is missing its referenced terms",
            ));
        }
    }
    if !manifest.cargo_packages.iter().any(|package| {
        package.name == "tidas"
            && package.version == manifest.version
            && package.registry_checksum.is_none()
    }) || cargo.len() > 4096
    {
        return Err(invalid("notice bundle has no matching tidas source root"));
    }
    validate_native_and_rust(manifest)
}

fn validate_native_and_rust(manifest: &NoticeBundleManifestV1) -> Result<(), DistError> {
    let mut native = BTreeSet::new();
    for package in &manifest.native_packages {
        if !native.insert(package.name.as_str())
            || package.triplet != manifest.source.vcpkg_triplet
            || package.version.is_empty()
        {
            return Err(invalid("invalid native notice package identity"));
        }
        verify_texts(&package.texts, &manifest.files)?;
        let adapter = package.scope == "vcpkg-installed-build-adapter"
            && package.name == "libiconv"
            && !package.triplet.contains("windows")
            && package
                .texts
                .iter()
                .any(|text| text.kind == "build-input-inventory")
            && package
                .texts
                .iter()
                .any(|text| text.kind == "build-adapter-source");
        let notices = package.scope == "vcpkg-target-build-input"
            && package
                .texts
                .iter()
                .any(|text| text.kind == "native-upstream-notices");
        if !adapter && !notices {
            return Err(invalid(
                "native package is missing original notices or adapter evidence",
            ));
        }
    }
    if !native.contains("libxml2") || !native.contains("libxslt") {
        return Err(invalid("native XML dependencies are missing from notices"));
    }
    verify_texts(&manifest.rust_library_texts, &manifest.files)?;
    for kind in [
        "rust-project-license",
        "rust-library-notice-report",
        "rust-library-source-license-or-notice",
    ] {
        if !manifest
            .rust_library_texts
            .iter()
            .any(|text| text.kind == kind)
        {
            return Err(invalid("Rust library notice scope is incomplete"));
        }
    }
    for identifier in [
        "MIT",
        "Apache-2.0",
        "Unicode-3.0",
        "BSD-2-Clause",
        "LLVM-exception",
    ] {
        if !manifest
            .rust_library_texts
            .iter()
            .any(|text| text.license_id.as_deref() == Some(identifier))
        {
            return Err(invalid("Rust library referenced terms are missing"));
        }
    }
    Ok(())
}

fn verify_texts(
    texts: &[NoticeText],
    files: &BTreeMap<String, FileDigest>,
) -> Result<(), DistError> {
    if texts.is_empty() || texts.len() > 4096 {
        return Err(invalid("empty or oversized package notice material"));
    }
    for text in texts {
        verify_reference_term(text)?;
        let expected = FileDigest {
            sha256: text.sha256.clone(),
            bytes: text.bytes,
        };
        if text.bytes == 0
            || !hex_id(&text.sha256, 64)
            || text.source_path.is_empty()
            || files.get(&format!("texts/{}.txt", text.sha256)) != Some(&expected)
        {
            return Err(invalid(
                "notice text is absent from the complete file inventory",
            ));
        }
    }
    Ok(())
}

fn verify_reference_term(text: &NoticeText) -> Result<(), DistError> {
    if text.kind != "referenced-license-terms" {
        if text.license_id.is_some() {
            return Err(invalid(
                "license identifier attached to a non-reference notice",
            ));
        }
        return Ok(());
    }
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../packaging/third-party-notices/referenced-terms.json"
    ))?;
    let identifier = text
        .license_id
        .as_deref()
        .ok_or_else(|| invalid("reference terms lack their identifier"))?;
    let terms = catalog["terms"]
        .as_array()
        .ok_or_else(|| invalid("invalid source-pinned reference terms"))?;
    if !terms.iter().any(|term| {
        term["license_id"] == identifier
            && term["sha256"] == text.sha256
            && term["bytes"] == text.bytes
            && term["source_path"] == text.source_path
            && term["source_url"].as_str() == text.source_url.as_deref()
    }) {
        return Err(invalid(
            "reference terms differ from the reviewed source-pinned text",
        ));
    }
    Ok(())
}

pub(crate) fn file_digest(path: &Path) -> Result<FileDigest, DistError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("expected a regular notice or executable file"));
    }
    Ok(FileDigest {
        sha256: crate::sha256_file(path)?,
        bytes: metadata.len(),
    })
}

fn digest(bytes: &[u8]) -> FileDigest {
    FileDigest {
        sha256: crate::hex(Sha256::digest(bytes)),
        bytes: bytes.len() as u64,
    }
}

fn insert_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    path: String,
    bytes: Vec<u8>,
) -> Result<(), DistError> {
    if bytes.len() as u64 > MAX_FILE || files.get(&path).is_some_and(|previous| previous != &bytes)
    {
        return Err(invalid("oversized or conflicting notice bundle input"));
    }
    files.insert(path, bytes);
    Ok(())
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, DistError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(invalid("notice evidence is not a bounded regular file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(invalid("notice evidence changed while reading"));
    }
    Ok(bytes)
}

fn command_output(command: &mut Command) -> Result<String, DistError> {
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .ok_or_else(|| invalid("build tool output unavailable"))?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() as u64 > MAX_FILE {
        let _ = child.kill();
        let _ = child.wait();
        return Err(invalid("build tool output failed or exceeded its bound"));
    }
    if !child.wait()?.success() {
        return Err(invalid("build source inspection failed"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("build source output is not UTF-8"))
}

fn clean_source(root: &Path) -> Result<(), DistError> {
    if !command_output(Command::new("git").current_dir(root).args([
        "status",
        "--porcelain",
        "--untracked-files=normal",
    ]))?
    .trim()
    .is_empty()
    {
        return Err(invalid(
            "native notice collection requires a clean committed source tree",
        ));
    }
    Ok(())
}

fn git_head(root: &Path) -> Result<String, DistError> {
    let value = command_output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "HEAD"]),
    )?;
    let value = value.trim();
    if !hex_id(value, 40) {
        return Err(invalid("invalid source Git commit"));
    }
    Ok(value.to_owned())
}

fn rustc_field<'a>(output: &'a str, key: &str) -> Result<&'a str, DistError> {
    output
        .lines()
        .filter_map(|line| line.split_once(": "))
        .find_map(|(name, value)| (name == key && !value.is_empty()).then_some(value))
        .ok_or_else(|| invalid("Rust toolchain identity is incomplete"))
}

fn hex_id(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn triplet(target: &str) -> Result<&'static str, DistError> {
    match target {
        "x86_64-unknown-linux-gnu" => Ok("x64-linux"),
        "aarch64-unknown-linux-gnu" => Ok("arm64-linux"),
        "aarch64-apple-darwin" => Ok("arm64-osx"),
        "x86_64-pc-windows-msvc" => Ok("x64-windows-static"),
        _ => Err(DistError::UnsupportedTarget(target.to_owned())),
    }
}

fn validate_native_build_inputs(vcpkg_root: &Path, target: &str) -> Result<(), DistError> {
    let variable = |name: &str| {
        std::env::var(name)
            .map_err(|_| invalid("native notice collection requires the release build environment"))
    };
    let selected_triplet = triplet(target)?;
    if target == "x86_64-pc-windows-msvc" {
        if variable("VCPKGRS_TRIPLET")? != selected_triplet
            || Path::new(&variable("VCPKG_ROOT")?).canonicalize()? != vcpkg_root
            || variable("CARGO_BUILD_TARGET")? != target
            || variable("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS")?
                != "-C target-feature=+crt-static"
        {
            return Err(invalid(
                "native notice inputs differ from the Windows static-CRT build environment",
            ));
        }
    } else if variable("LIBXML2_STATIC")? != "1"
        || variable("LIBXSLT_STATIC")? != "1"
        || Path::new(&variable("PKG_CONFIG_PATH")?).canonicalize()?
            != vcpkg_root
                .join("installed")
                .join(selected_triplet)
                .join("lib/pkgconfig")
                .canonicalize()?
    {
        return Err(invalid(
            "native notice inputs differ from the static XML build environment",
        ));
    }
    Ok(())
}

fn require_new_output(output: &Path) -> Result<(), DistError> {
    match fs::symlink_metadata(output) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err(invalid("notice bundle output already exists")),
    }
}

fn invalid(message: &str) -> DistError {
    DistError::Notice(message.to_owned())
}

fn bundle_explanation() -> &'static str {
    "TIDAS native third-party notice material\n\nThe manifest binds retained original texts to the exact executable digest, source commit, Cargo.lock, native installation and Rust toolchain. Official release build provenance establishes the source-to-binary relationship.\n\nCargo normal/build scopes and target kinds describe resolved source inputs; proc macros and build helpers are not claimed to be linked runtime libraries. Native inputs include installed target ports and any explicitly evidenced system-library adapter. Rust library source notices are a conservative source superset; target-specific or development paths can be included without claiming their code is shipped.\n\nOriginal licensing explanations, original copyright notices, full source license files and referenced canonical terms are distinct records. Canonical terms retain upstream template notation verbatim; copyright holders are not invented or filled in. Consult the separately retained original notices for attribution and caveats.\n"
}
