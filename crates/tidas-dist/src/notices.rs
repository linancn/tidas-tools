//! Source-bound notice material. Collection is not a declaration of distribution readiness.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::DistError;

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";
const MAX_TEXT: u64 = 2 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeText {
    pub sha256: String,
    pub bytes: u64,
    pub kind: String,
    pub source_path: String,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CargoNoticePackage {
    pub name: String,
    pub version: String,
    pub registry_checksum: Option<String>,
    pub declared_license: String,
    pub scopes: Vec<String>,
    pub target_kinds: Vec<String>,
    pub features: Vec<String>,
    pub texts: Vec<NoticeText>,
}

#[derive(Debug)]
pub struct CargoNoticeInputs {
    pub packages: Vec<CargoNoticePackage>,
    pub texts: BTreeMap<String, Vec<u8>>,
    retained_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNoticePackage {
    pub name: String,
    pub version: String,
    pub scope: String,
    pub triplet: String,
    pub features: Vec<String>,
    pub texts: Vec<NoticeText>,
}

#[derive(Debug)]
pub struct NativeNoticeInputs {
    pub installed_status_sha256: String,
    pub packages: Vec<NativeNoticePackage>,
    pub texts: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug)]
pub struct RustLibraryNoticeInputs {
    pub texts: Vec<NoticeText>,
    pub contents: BTreeMap<String, Vec<u8>>,
}

/// Retains the toolchain's original project terms and standard-library notice
/// report. The report's distinction between notices and referenced terms stays intact.
pub fn collect_rust_library_notice_inputs(
    sysroot: &Path,
) -> Result<RustLibraryNoticeInputs, DistError> {
    let sysroot = sysroot.canonicalize()?;
    let choose = |paths: &[&str]| -> Result<String, DistError> {
        paths
            .iter()
            .find(|path| sysroot.join(path).is_file())
            .map(|path| (*path).to_owned())
            .ok_or_else(|| invalid("actual Rust toolchain library notice material is missing"))
    };
    let files = [
        (
            choose(&[
                "LICENSE-MIT",
                "share/doc/rust/LICENSE-MIT",
                "share/doc/rustc/LICENSE-MIT",
            ])?,
            "rust-project-license",
        ),
        (
            choose(&[
                "LICENSE-APACHE",
                "share/doc/rust/LICENSE-APACHE",
                "share/doc/rustc/LICENSE-APACHE",
            ])?,
            "rust-project-license",
        ),
        (
            choose(&[
                "share/doc/rustc/COPYRIGHT-library.html",
                "share/doc/rust/COPYRIGHT-library.html",
                "share/doc/rust/html/COPYRIGHT-library.html",
                "COPYRIGHT-library.html",
            ])?,
            "rust-library-notice-report",
        ),
    ];
    let mut collected = CargoNoticeInputs {
        packages: Vec::new(),
        texts: BTreeMap::new(),
        retained_bytes: 0,
    };
    let mut texts = Vec::new();
    for (file, kind) in files {
        texts.push(retain(
            &mut collected,
            read_contained_file(&sysroot, &file, MAX_TEXT)?,
            kind,
            file,
            None,
        )?);
    }
    Ok(RustLibraryNoticeInputs {
        texts,
        contents: collected.texts,
    })
}

type NativeInstalledPackages = BTreeMap<String, (String, BTreeSet<String>)>;

fn vcpkg_status_packages(
    status: &str,
    triplet: &str,
) -> Result<NativeInstalledPackages, DistError> {
    let mut packages: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();
    let mut features: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for paragraph in status.split("\n\n") {
        let mut fields = BTreeMap::new();
        for line in paragraph.lines() {
            if line.starts_with(' ') || line.trim().is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| invalid("malformed vcpkg status paragraph"))?;
            if fields.insert(key, value.trim()).is_some() {
                return Err(invalid("duplicate vcpkg status field"));
            }
        }
        if fields.get("Architecture") != Some(&triplet)
            || fields.get("Status") != Some(&"install ok installed")
        {
            continue;
        }
        let name = *fields
            .get("Package")
            .ok_or_else(|| invalid("vcpkg status package name missing"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(invalid("invalid native package name"));
        }
        if let Some(feature) = fields.get("Feature") {
            features
                .entry(name.to_owned())
                .or_default()
                .insert((*feature).to_owned());
        } else {
            let version = fields
                .get("Version")
                .or_else(|| fields.get("Version-Semver"))
                .ok_or_else(|| invalid("vcpkg installed package version missing"))?;
            let version = match fields.get("Port-Version") {
                Some(port) => format!("{version}#{port}"),
                None => (*version).to_owned(),
            };
            if packages
                .insert(name.to_owned(), (version, BTreeSet::new()))
                .is_some()
            {
                return Err(invalid("duplicate installed vcpkg package"));
            }
        }
    }
    if packages.is_empty() || packages.len() > 1024 {
        return Err(invalid("native notice scope is empty or oversized"));
    }
    for (name, selected) in features {
        packages
            .get_mut(&name)
            .ok_or_else(|| invalid("vcpkg feature has no installed core package"))?
            .1
            .extend(selected);
    }
    Ok(packages)
}

/// Retains every installed target-port notice. Scope is build input, not a
/// claim that every installed port contributes code to the final binary.
pub fn collect_vcpkg_notice_inputs(
    installed: &Path,
    triplet: &str,
) -> Result<NativeNoticeInputs, DistError> {
    if triplet.is_empty()
        || !triplet
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(invalid("invalid vcpkg triplet"));
    }
    let installed = installed.canonicalize()?;
    let status_bytes = read_contained_file(&installed, "vcpkg/status", 16 * 1024 * 1024)?;
    let status = std::str::from_utf8(&status_bytes)
        .map_err(|_| invalid("vcpkg status is not UTF-8"))?
        .replace("\r\n", "\n");
    let packages = vcpkg_status_packages(&status, triplet)?;
    let mut material = CargoNoticeInputs {
        packages: Vec::new(),
        texts: BTreeMap::new(),
        retained_bytes: 0,
    };
    let mut records = Vec::new();
    for (name, (version, features)) in packages {
        let (scope, texts) =
            native_package_material(&mut material, &installed, triplet, &name, &version)?;
        records.push(NativeNoticePackage {
            name,
            version,
            scope: scope.to_owned(),
            triplet: triplet.to_owned(),
            features: features.into_iter().collect(),
            texts,
        });
    }
    Ok(NativeNoticeInputs {
        installed_status_sha256: hash(&status_bytes),
        packages: records,
        texts: material.texts,
    })
}

fn native_package_material(
    material: &mut CargoNoticeInputs,
    installed: &Path,
    triplet: &str,
    name: &str,
    version: &str,
) -> Result<(&'static str, Vec<NoticeText>), DistError> {
    let relative = format!("{triplet}/share/{name}/copyright");
    match read_contained_file(installed, &relative, MAX_TEXT) {
        Ok(bytes) => Ok((
            "vcpkg-target-build-input",
            vec![retain(
                material,
                bytes,
                "native-upstream-notices",
                relative,
                None,
            )?],
        )),
        Err(DistError::Io(error))
            if error.kind() == std::io::ErrorKind::NotFound
                && name == "libiconv"
                && ["arm64-osx", "arm64-linux", "x64-linux"].contains(&triplet) =>
        {
            Ok((
                "vcpkg-installed-build-adapter",
                iconv_adapter_material(material, installed, triplet, version)?,
            ))
        }
        Err(error) => Err(error),
    }
}

// The pinned POSIX port can use the system iconv implementation and install
// only a CMake adapter. Absence of a copyright file alone never establishes
// this case: require its exact installed inventory and retain that evidence.
fn iconv_adapter_material(
    material: &mut CargoNoticeInputs,
    installed: &Path,
    triplet: &str,
    version: &str,
) -> Result<Vec<NoticeText>, DistError> {
    let version = version.split('#').next().unwrap_or_default();
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
    {
        return Err(invalid("invalid iconv adapter version"));
    }
    let list = format!("vcpkg/info/libiconv_{version}_{triplet}.list");
    let bytes = read_contained_file(installed, &list, MAX_TEXT)?;
    let lines: Vec<_> = std::str::from_utf8(&bytes)
        .map_err(|_| invalid("native file inventory is not UTF-8"))?
        .lines()
        .collect();
    let actual: BTreeSet<_> = lines.iter().copied().collect();
    let expected: BTreeSet<_> = [
        "",
        "share/",
        "share/iconv/",
        "share/iconv/vcpkg-cmake-wrapper.cmake",
        "share/libiconv/",
        "share/libiconv/vcpkg.spdx.json",
        "share/libiconv/vcpkg_abi_info.txt",
    ]
    .iter()
    .map(|path| format!("{triplet}/{path}"))
    .collect();
    if actual.len() != lines.len() || actual != expected.iter().map(String::as_str).collect() {
        return Err(invalid(
            "iconv adapter inventory contains missing or unexpected files",
        ));
    }
    for path in actual.iter().filter(|path| !path.ends_with('/')) {
        read_contained_file(installed, path, MAX_TEXT)?;
    }
    let wrapper = format!("{triplet}/share/iconv/vcpkg-cmake-wrapper.cmake");
    Ok(vec![
        retain(material, bytes, "build-input-inventory", list, None)?,
        retain(
            material,
            read_contained_file(installed, &wrapper, MAX_TEXT)?,
            "build-adapter-source",
            wrapper,
            None,
        )?,
    ])
}

/// Exports original native/toolchain material for review. The explicit input
/// roots are not evidence that a particular executable was built from them.
pub fn write_native_notice_inputs(
    installed: &Path,
    sysroot: &Path,
    target: &str,
    output: &Path,
) -> Result<usize, DistError> {
    crate::validate_target(target)?;
    let triplet = match target {
        "x86_64-unknown-linux-gnu" => "x64-linux",
        "aarch64-unknown-linux-gnu" => "arm64-linux",
        "aarch64-apple-darwin" => "arm64-osx",
        "x86_64-pc-windows-msvc" => "x64-windows-static",
        _ => return Err(DistError::UnsupportedTarget(target.to_owned())),
    };
    let native = collect_vcpkg_notice_inputs(installed, triplet)?;
    let rust = collect_rust_library_notice_inputs(sysroot)?;
    let status = read_contained_file(&installed.canonicalize()?, "vcpkg/status", 16 * 1024 * 1024)?;
    if hash(&status) != native.installed_status_sha256 {
        return Err(invalid("vcpkg status changed during notice collection"));
    }
    let mut material = CargoNoticeInputs {
        packages: Vec::new(),
        texts: native.texts,
        retained_bytes: 0,
    };
    material.retained_bytes = material.texts.values().map(Vec::len).sum();
    for (digest, bytes) in rust.contents {
        retain(&mut material, bytes, "retained-input", digest, None)?;
    }
    let status_record = retain(
        &mut material,
        status,
        "build-input-inventory",
        "vcpkg/status".to_owned(),
        None,
    )?;
    let record = serde_json::json!({
        "schema_version":"tidas.native-notice-inputs.v1",
        "scope":"installed native target and Rust library source inputs; not an executable-bound complete notice bundle",
        "target":target,
        "vcpkg":{"triplet":triplet,"installed_status":status_record,"packages":native.packages},
        "rust_library":{"texts":rust.texts},
    });
    write_notice_inputs(
        output,
        "native-notice-inputs.json",
        &record,
        &material.texts,
    )?;
    Ok(native.packages.len())
}

/// Collects reviewable Cargo source inputs; this is not a complete native bundle.
pub fn write_current_cargo_notice_inputs(target: &str, output: &Path) -> Result<usize, DistError> {
    crate::validate_target(target)?;
    if output.exists() {
        return Err(invalid("notice input output already exists"));
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let lock_bytes = read_file(&workspace.join("Cargo.lock"), 16 * 1024 * 1024)?;
    let lock = std::str::from_utf8(&lock_bytes).map_err(|_| invalid("Cargo lock is not UTF-8"))?;
    lock_checksums(lock)?;
    let mut child = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--filter-platform",
            target,
            "--manifest-path",
        ])
        .arg(workspace.join("crates/tidas-cli/Cargo.toml"))
        .current_dir(&workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut metadata_bytes = Vec::new();
    let maximum = 16 * 1024 * 1024;
    let read = child
        .stdout
        .take()
        .ok_or_else(|| invalid("Cargo metadata stdout unavailable"))?
        .take(maximum + 1)
        .read_to_end(&mut metadata_bytes);
    if let Err(error) = read {
        let _ = child.kill();
        let _ = child.wait();
        return Err(DistError::Io(error));
    }
    if metadata_bytes.len() as u64 > maximum {
        let _ = child.kill();
        let _ = child.wait();
        return Err(invalid("Cargo metadata exceeds its byte bound"));
    }
    if !child.wait()?.success() {
        return Err(invalid("locked Cargo metadata command failed"));
    }
    let metadata: Value = serde_json::from_slice(&metadata_bytes)?;
    let collected = collect_cargo_notice_inputs(
        &workspace,
        &metadata,
        lock,
        Some(&workspace.join("packaging/third-party-notices")),
    )?;
    if read_file(&workspace.join("Cargo.lock"), 16 * 1024 * 1024)? != lock_bytes {
        return Err(invalid("Cargo lock changed during notice collection"));
    }
    let record = serde_json::json!({
        "schema_version":"tidas.cargo-notice-inputs.v1",
        "scope":"resolved normal/build source inputs; not a binary SBOM or complete native notice bundle",
        "target":target,"cargo_lock_sha256":hash(&lock_bytes),"packages":collected.packages,
    });
    write_notice_inputs(
        output,
        "cargo-notice-inputs.json",
        &record,
        &collected.texts,
    )?;
    Ok(collected.packages.len())
}

fn write_notice_inputs(
    output: &Path,
    filename: &str,
    record: &Value,
    texts: &BTreeMap<String, Vec<u8>>,
) -> Result<(), DistError> {
    if output.exists() {
        return Err(invalid("notice input output already exists"));
    }
    let parent = output
        .parent()
        .ok_or_else(|| invalid("notice output has no parent"))?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    fs::create_dir(staging.path().join("texts"))?;
    for (digest, bytes) in texts {
        fs::write(
            staging.path().join("texts").join(format!("{digest}.txt")),
            bytes,
        )?;
    }
    let mut bytes = serde_json::to_vec_pretty(record)?;
    bytes.push(b'\n');
    fs::write(staging.path().join(filename), bytes)?;
    if output.exists() {
        return Err(invalid("notice output appeared during collection"));
    }
    fs::rename(staging.path(), output)?;
    Ok(())
}

fn invalid(message: &str) -> DistError {
    DistError::Notice(message.to_owned())
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, DistError> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid("missing string in source metadata"))
}

fn values<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], DistError> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| invalid("missing source metadata array"))
}

fn hash(bytes: &[u8]) -> String {
    crate::hex(Sha256::digest(bytes))
}

fn read_file(path: &Path, maximum: u64) -> Result<Vec<u8>, DistError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(invalid("notice input must be a bounded regular file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > maximum {
        return Err(invalid("notice input changed while reading"));
    }
    Ok(bytes)
}

fn read_contained_file(root: &Path, path: &str, maximum: u64) -> Result<Vec<u8>, DistError> {
    relative(Path::new(path))?;
    let mut current = root.to_path_buf();
    for component in Path::new(path).components() {
        current.push(component);
        if fs::symlink_metadata(&current)?.file_type().is_symlink() {
            return Err(invalid("notice input traverses a symlink"));
        }
    }
    read_file(&current, maximum)
}

fn relative(path: &Path) -> Result<String, DistError> {
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(invalid("notice paths must be contained relative paths"));
    }
    let value = path
        .to_str()
        .ok_or_else(|| invalid("notice path is not UTF-8"))?
        .replace('\\', "/");
    if value.is_empty() || value.contains(':') || value.contains('\0') {
        return Err(invalid("invalid notice path"));
    }
    Ok(value)
}

fn license_name(name: &str) -> bool {
    let value = name.to_ascii_lowercase();
    [
        "license",
        "licence",
        "licenses",
        "licences",
        "copying",
        "copyright",
        "notice",
        "authors",
    ]
    .iter()
    .any(|prefix| {
        value == *prefix
            || value
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with(['.', '-', '_']))
    })
}

fn selected_notice(path: &str, explicit: Option<&str>) -> bool {
    if explicit == Some(path) {
        return true;
    }
    let mut parts = path.split('/');
    parts.next().is_some_and(license_name)
}

fn retain(
    result: &mut CargoNoticeInputs,
    bytes: Vec<u8>,
    kind: &str,
    path: String,
    url: Option<String>,
) -> Result<NoticeText, DistError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_TEXT || std::str::from_utf8(&bytes).is_err() {
        return Err(invalid("license material must be nonempty bounded UTF-8"));
    }
    let digest = hash(&bytes);
    let fact = NoticeText {
        sha256: digest.clone(),
        bytes: bytes.len() as u64,
        kind: kind.to_owned(),
        source_path: path,
        source_url: url,
    };
    match result.texts.entry(digest) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            result.retained_bytes = result
                .retained_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| invalid("notice text size overflow"))?;
            if result.retained_bytes > 64 * 1024 * 1024 {
                return Err(invalid(
                    "retained notice material exceeds its total byte bound",
                ));
            }
            entry.insert(bytes);
        }
        std::collections::btree_map::Entry::Occupied(entry) if entry.get() != &bytes => {
            return Err(invalid("notice digest collision"));
        }
        std::collections::btree_map::Entry::Occupied(_) => {}
    }
    Ok(fact)
}

// Cargo generates this lock layout. Unsupported non-registry sources are rejected,
// not resolved by this reader; Cargo owns dependency resolution.
fn lock_checksums(lock: &str) -> Result<BTreeMap<(String, String, String), String>, DistError> {
    if !lock.lines().any(|line| line.trim() == "version = 4") {
        return Err(invalid("unsupported Cargo lock format"));
    }
    let mut result = BTreeMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut fields = BTreeMap::new();
        for line in block.lines() {
            if let Some((key, value)) = line.trim().split_once(" = ")
                && ["name", "version", "source", "checksum"].contains(&key)
            {
                let decoded: String = serde_json::from_str(value)
                    .map_err(|_| invalid("non-canonical Cargo lock identity field"))?;
                if fields.insert(key, decoded).is_some() {
                    return Err(invalid("duplicate Cargo lock identity field"));
                }
            }
        }
        let name = fields
            .remove("name")
            .ok_or_else(|| invalid("Cargo lock package has no name"))?;
        let version = fields
            .remove("version")
            .ok_or_else(|| invalid("Cargo lock package has no version"))?;
        if let Some(source) = fields.remove("source") {
            if source != REGISTRY {
                return Err(invalid("unqualified Cargo dependency source"));
            }
            let checksum = fields
                .remove("checksum")
                .ok_or_else(|| invalid("registry package has no locked checksum"))?;
            if checksum.len() != 64
                || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit())
                || result.insert((name, version, source), checksum).is_some()
            {
                return Err(invalid("invalid or duplicate Cargo checksum"));
            }
        } else if fields.contains_key("checksum") {
            return Err(invalid(
                "workspace package unexpectedly has a registry checksum",
            ));
        }
    }
    Ok(result)
}

fn supplement(
    result: &mut CargoNoticeInputs,
    directory: &Path,
    package: &Value,
    checksum: &str,
    vcs: Option<&Value>,
) -> Result<Vec<NoticeText>, DistError> {
    let source: Value =
        serde_json::from_slice(&read_file(&directory.join("supplements.json"), MAX_TEXT)?)?;
    if source["schema_version"] != "tidas.license-supplement-sources.v1" {
        return Err(invalid("unsupported license supplement source"));
    }
    let matched: Vec<_> = values(&source, "packages")?
        .iter()
        .filter(|entry| entry["name"] == package["name"] && entry["version"] == package["version"])
        .collect();
    if matched.len() != 1 {
        return Err(invalid("missing or ambiguous upstream license supplement"));
    }
    let entry = matched[0];
    if text(entry, "registry_checksum")? != checksum
        || entry["declared_license"] != package["license"]
        || vcs.is_none_or(|vcs| {
            vcs["git"]["sha1"] != entry["source_commit"]
                || vcs["path_in_vcs"] != entry["path_in_vcs"]
        })
    {
        return Err(invalid(
            "license supplement differs from the locked crate source",
        ));
    }
    let kind = match text(entry, "kind")? {
        "upstream-license-text" => "license-text",
        "upstream-licensing-notice" => "licensing-notice",
        _ => return Err(invalid("unknown license supplement kind")),
    };
    let mut facts = Vec::new();
    for evidence in values(entry, "evidence")? {
        let digest = text(evidence, "sha256")?;
        let selected = text(evidence, "path")?;
        if selected != format!("texts/{digest}.txt") {
            return Err(invalid("unsafe license supplement path"));
        }
        let bytes = read_file(&directory.join(selected), MAX_TEXT)?;
        if hash(&bytes) != digest || Some(bytes.len() as u64) != evidence["bytes"].as_u64() {
            return Err(invalid("license supplement bytes changed"));
        }
        facts.push(retain(
            result,
            bytes,
            kind,
            text(evidence, "source_path")?.to_owned(),
            Some(text(evidence, "source_url")?.to_owned()),
        )?);
    }
    if facts.is_empty() {
        return Err(invalid("license supplement has no material"));
    }
    Ok(facts)
}

type CargoValues<'a> = BTreeMap<&'a str, &'a Value>;
type CargoChecksums = BTreeMap<(String, String, String), String>;

fn cargo_values<'a>(metadata: &'a Value, key: &str) -> Result<CargoValues<'a>, DistError> {
    let source = values(metadata, key)?;
    let result: CargoValues<'a> = source
        .iter()
        .map(|value| Ok((text(value, "id")?, value)))
        .collect::<Result<_, DistError>>()?;
    if result.len() != source.len() || result.len() > 4096 {
        return Err(invalid("duplicate or oversized Cargo metadata"));
    }
    Ok(result)
}

fn cargo_scopes<'a>(
    packages: &CargoValues<'a>,
    nodes: &CargoValues<'a>,
    members: &BTreeSet<&str>,
) -> Result<BTreeMap<&'a str, BTreeSet<&'static str>>, DistError> {
    let roots: Vec<_> = packages
        .iter()
        .filter(|(id, package)| members.contains(**id) && package["name"] == "tidas")
        .map(|(id, _)| *id)
        .collect();
    if roots.len() != 1 {
        return Err(invalid(
            "notice graph requires the unique tidas product root",
        ));
    }
    let mut selected: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut queue = vec![(roots[0], "rust-normal")];
    while let Some((id, scope)) = queue.pop() {
        if !selected.entry(id).or_default().insert(scope) {
            continue;
        }
        let node = nodes
            .get(id)
            .ok_or_else(|| invalid("Cargo dependency node is missing"))?;
        for dependency in values(node, "deps")? {
            for kind in values(dependency, "dep_kinds")? {
                let next = match kind["kind"].as_str() {
                    None if kind["kind"].is_null() => scope,
                    Some("build") => "rust-build",
                    Some("dev") => continue,
                    _ => return Err(invalid("unknown Cargo dependency scope")),
                };
                queue.push((text(dependency, "pkg")?, next));
            }
        }
    }
    Ok(selected)
}

fn package_identity(package: &Value) -> Result<(&str, &str), DistError> {
    let name = text(package, "name")?;
    let version = text(package, "version")?;
    if name.is_empty()
        || version.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(&byte))
    {
        return Err(invalid("invalid Cargo package identity"));
    }
    Ok((name, version))
}

fn explicit_license(package: &Value, directory: &Path) -> Result<Option<String>, DistError> {
    package["license_file"]
        .as_str()
        .map(Path::new)
        .map(|file| {
            if file.is_absolute() {
                file.strip_prefix(directory)
                    .map_err(|_| invalid("license file escaped its package"))
            } else {
                Ok(file)
            }
        })
        .transpose()?
        .map(relative)
        .transpose()
}

fn registry_archive(
    directory: &Path,
    name: &str,
    version: &str,
) -> Result<std::path::PathBuf, DistError> {
    let index = directory
        .parent()
        .ok_or_else(|| invalid("invalid Cargo source cache path"))?;
    let src = index
        .parent()
        .ok_or_else(|| invalid("invalid Cargo source cache path"))?;
    if src.file_name().is_none_or(|name| name != "src") {
        return Err(invalid(
            "registry sources require the verified Cargo cache layout",
        ));
    }
    Ok(src
        .parent()
        .ok_or_else(|| invalid("invalid Cargo registry path"))?
        .join("cache")
        .join(
            index
                .file_name()
                .ok_or_else(|| invalid("invalid registry index path"))?,
        )
        .join(format!("{name}-{version}.crate")))
}

fn registry_materials(
    result: &mut CargoNoticeInputs,
    package: &Value,
    directory: &Path,
    checksum: &str,
    explicit: Option<&str>,
    supplements: Option<&Path>,
) -> Result<Vec<NoticeText>, DistError> {
    let (name, version) = package_identity(package)?;
    let bytes = read_file(&registry_archive(directory, name, version)?, MAX_ARCHIVE)?;
    if hash(&bytes) != checksum {
        return Err(invalid("crate archive differs from its locked checksum"));
    }
    let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()).take(128 * 1024 * 1024));
    let mut vcs = None;
    let mut facts = Vec::new();
    let mut paths = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = relative(&entry.path()?)?;
        if !paths.insert(path.clone()) || paths.len() > 50_000 {
            return Err(invalid("duplicate or oversized crate file inventory"));
        }
        let prefix = format!("{name}-{version}/");
        let Some(selected) = path.strip_prefix(&prefix) else {
            return Err(invalid("crate archive has a foreign package root"));
        };
        if entry.header().entry_type().is_dir() {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(invalid("crate archive contains a non-regular member"));
        }
        if selected_notice(selected, explicit) || selected == ".cargo_vcs_info.json" {
            if entry.size() > MAX_TEXT {
                return Err(invalid("crate notice exceeds its bound"));
            }
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            if selected == ".cargo_vcs_info.json" {
                vcs = Some(serde_json::from_slice(&content)?);
            } else {
                facts.push(retain(
                    result,
                    content,
                    "upstream-license-or-notice",
                    selected.to_owned(),
                    Some(format!(
                        "https://crates.io/api/v1/crates/{name}/{version}/download"
                    )),
                )?);
            }
        }
    }
    if facts.is_empty() {
        facts = supplement(
            result,
            supplements.ok_or_else(|| {
                invalid("locked crate omitted license material and has no reviewed supplement")
            })?,
            package,
            checksum,
            vcs.as_ref(),
        )?;
    }
    Ok(facts)
}

fn package_materials(
    result: &mut CargoNoticeInputs,
    package: &Value,
    workspace: &Path,
    member: bool,
    checksums: &CargoChecksums,
    supplements: Option<&Path>,
) -> Result<(Option<String>, Vec<NoticeText>), DistError> {
    let (name, version) = package_identity(package)?;
    let directory = Path::new(text(package, "manifest_path")?)
        .parent()
        .ok_or_else(|| invalid("package manifest has no directory"))?;
    let explicit = explicit_license(package, directory)?;
    if package["source"].is_null() {
        if !member || !directory.canonicalize()?.starts_with(workspace) {
            return Err(invalid("unqualified local Cargo package"));
        }
        let license = explicit
            .as_ref()
            .map_or_else(|| workspace.join("LICENSE"), |file| directory.join(file));
        let fact = retain(
            result,
            read_file(&license, MAX_TEXT)?,
            "project-license",
            relative(
                license
                    .strip_prefix(workspace)
                    .map_err(|_| invalid("workspace license escaped source"))?,
            )?,
            None,
        )?;
        Ok((None, vec![fact]))
    } else {
        let checksum = checksums
            .get(&(
                name.to_owned(),
                version.to_owned(),
                text(package, "source")?.to_owned(),
            ))
            .ok_or_else(|| invalid("Cargo package is not bound by the source lock"))?;
        let facts = registry_materials(
            result,
            package,
            directory,
            checksum,
            explicit.as_deref(),
            supplements,
        )?;
        Ok((Some(checksum.clone()), facts))
    }
}

fn string_values<'a>(values: impl Iterator<Item = &'a Value>) -> Result<Vec<String>, DistError> {
    Ok(values
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| invalid("invalid Cargo feature or target kind"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect())
}

/// Collects the locked source graph's normal/build notice inputs. No output is
/// treated as a statement that every input is linked into a binary.
pub fn collect_cargo_notice_inputs(
    workspace: &Path,
    metadata: &Value,
    lock: &str,
    supplements: Option<&Path>,
) -> Result<CargoNoticeInputs, DistError> {
    if metadata["version"] != 1 {
        return Err(invalid("unsupported Cargo metadata format"));
    }
    let workspace = workspace.canonicalize()?;
    let checksums = lock_checksums(lock)?;
    let packages = cargo_values(metadata, "packages")?;
    let nodes = cargo_values(&metadata["resolve"], "nodes")?;
    if nodes.len() != packages.len() {
        return Err(invalid("incomplete Cargo metadata graph"));
    }
    let members = values(metadata, "workspace_members")?
        .iter()
        .map(|id| {
            id.as_str()
                .ok_or_else(|| invalid("invalid workspace package id"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let selected = cargo_scopes(&packages, &nodes, &members)?;
    let mut result = CargoNoticeInputs {
        packages: Vec::new(),
        texts: BTreeMap::new(),
        retained_bytes: 0,
    };
    for (id, scopes) in selected {
        let package = packages
            .get(id)
            .ok_or_else(|| invalid("selected Cargo package is missing"))?;
        let (name, version) = package_identity(package)?;
        let (registry_checksum, mut facts) = package_materials(
            &mut result,
            package,
            &workspace,
            members.contains(id),
            &checksums,
            supplements,
        )?;
        let features = string_values(values(nodes[id], "features")?.iter())?;
        let target_kinds = string_values(
            values(package, "targets")?
                .iter()
                .flat_map(|target| target["kind"].as_array().into_iter().flatten()),
        )?;
        facts.sort_by(|left, right| {
            (&left.source_path, &left.sha256).cmp(&(&right.source_path, &right.sha256))
        });
        result.packages.push(CargoNoticePackage {
            name: name.to_owned(),
            version: version.to_owned(),
            registry_checksum,
            declared_license: text(package, "license")?.to_owned(),
            scopes: scopes.into_iter().map(str::to_owned).collect(),
            target_kinds,
            features,
            texts: facts,
        });
    }
    result
        .packages
        .sort_by(|left, right| (&left.name, &left.version).cmp(&(&right.name, &right.version)));
    Ok(result)
}
