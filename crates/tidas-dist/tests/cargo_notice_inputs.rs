use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::{Compression, GzBuilder};
use serde_json::json;
use sha2::{Digest, Sha256};
use tidas_dist::notices::collect_cargo_notice_inputs;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").unwrap();
            output
        })
}

fn registry_package(root: &Path, name: &str, license: &[u8]) -> (serde_json::Value, String) {
    let version = "1.0.0";
    let source = root.join(format!("registry/src/index/{name}-{version}"));
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("Cargo.toml"), "").unwrap();
    let mut archive = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(license.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, format!("{name}-{version}/LICENSE"), license)
        .unwrap();
    let tar = archive.into_inner().unwrap();
    let mut gzip = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    gzip.write_all(&tar).unwrap();
    let bytes = gzip.finish().unwrap();
    let cache = root.join("registry/cache/index");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join(format!("{name}-{version}.crate")), &bytes).unwrap();
    let id = format!("registry+https://github.com/rust-lang/crates.io-index#{name}@{version}");
    (
        json!({"id":id,"name":name,"version":version,"source":"registry+https://github.com/rust-lang/crates.io-index","license":"MIT","license_file":null,"manifest_path":source.join("Cargo.toml"),"repository":"https://example.invalid/source","targets":[{"kind":["lib"]}]}),
        digest(&bytes),
    )
}

#[test]
fn locked_cargo_notice_inputs_preserve_normal_and_build_scope_without_development_packages() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::write(root.join("LICENSE"), "Owner license\n").unwrap();
    fs::write(root.join("Cargo.toml"), "").unwrap();
    let (normal, normal_hash) = registry_package(root, "normal", b"Normal license\n");
    let (build, build_hash) = registry_package(root, "build", b"Build license\n");
    let (dev, _) = registry_package(root, "dev", b"Unrelated development license\n");
    let root_id = "path+file:///fixture#tidas@0.2.2";
    let metadata = json!({"version":1,"workspace_members":[root_id],"packages":[
        {"id":root_id,"name":"tidas","version":"0.2.2","source":null,"license":"MIT","license_file":null,"manifest_path":root.join("Cargo.toml"),"targets":[{"kind":["bin"]}]}, normal, build, dev
    ],"resolve":{"nodes":[
        {"id":root_id,"features":[],"deps":[
            {"pkg":normal["id"],"dep_kinds":[{"kind":null}]},
            {"pkg":build["id"],"dep_kinds":[{"kind":"build"}]},
            {"pkg":dev["id"],"dep_kinds":[{"kind":"dev"}]}
        ]},
        {"id":normal["id"],"features":["std"],"deps":[]},
        {"id":build["id"],"features":[],"deps":[]},
        {"id":dev["id"],"features":[],"deps":[]}
    ]}});
    let lock = format!(
        "version = 4\n\n[[package]]\nname = \"normal\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{normal_hash}\"\n\n[[package]]\nname = \"build\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{build_hash}\"\n"
    );
    let result = collect_cargo_notice_inputs(root, &metadata, &lock, None).unwrap();
    assert_eq!(result.packages.len(), 3);
    assert_eq!(result.texts.len(), 3);
    assert!(result.packages.iter().all(|package| package.name != "dev"));
    assert_eq!(
        result
            .packages
            .iter()
            .find(|package| package.name == "normal")
            .unwrap()
            .scopes,
        ["rust-normal"]
    );
    assert_eq!(
        result
            .packages
            .iter()
            .find(|package| package.name == "build")
            .unwrap()
            .scopes,
        ["rust-build"]
    );
    let cache = root.join("registry/cache/index/normal-1.0.0.crate");
    fs::write(cache, "changed").unwrap();
    assert!(collect_cargo_notice_inputs(root, &metadata, &lock, None).is_err());
}
