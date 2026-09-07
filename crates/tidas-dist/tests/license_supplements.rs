use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn retained_upstream_supplements_match_locked_crates_and_exact_text_bytes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = root.join("packaging/third-party-notices");
    let value: Value =
        serde_json::from_slice(&fs::read(directory.join("supplements.json")).unwrap()).unwrap();
    assert_eq!(
        value["schema_version"],
        "tidas.license-supplement-sources.v1"
    );
    let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
    let packages = value["packages"].as_array().unwrap();
    assert!(!packages.is_empty());
    let mut identities = BTreeSet::new();
    let mut retained = BTreeSet::new();
    for package in packages {
        let name = package["name"].as_str().unwrap();
        let version = package["version"].as_str().unwrap();
        assert!(identities.insert((name, version)));
        let checksum = package["registry_checksum"].as_str().unwrap();
        let block = lock
            .split("[[package]]")
            .find(|block| {
                block
                    .lines()
                    .any(|line| line == format!("name = \"{name}\""))
                    && block
                        .lines()
                        .any(|line| line == format!("version = \"{version}\""))
            })
            .unwrap();
        assert!(
            block
                .lines()
                .any(|line| line == format!("checksum = \"{checksum}\""))
        );
        assert!(matches!(
            package["kind"].as_str().unwrap(),
            "upstream-license-text" | "upstream-licensing-notice"
        ));
        let repository = package["repository"].as_str().unwrap();
        assert!(repository.starts_with("https://github.com/"));
        let commit = package["source_commit"].as_str().unwrap();
        assert_eq!(commit.len(), 40);
        assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let evidence = package["evidence"].as_array().unwrap();
        assert!(!evidence.is_empty());
        for text in evidence {
            let digest = text["sha256"].as_str().unwrap();
            let relative = text["path"].as_str().unwrap();
            assert_eq!(relative, format!("texts/{digest}.txt"));
            let bytes = fs::read(directory.join(relative)).unwrap();
            assert_eq!(bytes.len() as u64, text["bytes"].as_u64().unwrap());
            let actual = Sha256::digest(&bytes).iter().fold(
                String::with_capacity(64),
                |mut output, byte| {
                    write!(output, "{byte:02x}").unwrap();
                    output
                },
            );
            assert_eq!(actual, digest);
            assert!(std::str::from_utf8(&bytes).is_ok());
            let source_path = text["source_path"].as_str().unwrap();
            assert!(matches!(source_path, "LICENSE" | "LICENSE.md"));
            assert_eq!(
                text["source_url"],
                format!(
                    "https://raw.githubusercontent.com/{}/{commit}/{source_path}",
                    repository.trim_start_matches("https://github.com/")
                )
            );
            assert_eq!(text["git_blob_sha1"].as_str().unwrap().len(), 40);
            retained.insert(relative.to_owned());
        }
    }
    let actual = fs::read_dir(directory.join("texts"))
        .unwrap()
        .map(|entry| format!("texts/{}", entry.unwrap().file_name().to_string_lossy()))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, retained);
}
