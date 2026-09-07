use std::fs;
use tidas_dist::notices::{
    collect_rust_library_notice_inputs, collect_vcpkg_notice_inputs, write_native_notice_inputs,
};

fn native_fixture(root: &std::path::Path) {
    fs::create_dir_all(root.join("vcpkg")).unwrap();
    fs::create_dir_all(root.join("arm64-osx/share/libxml2")).unwrap();
    fs::write(root.join("vcpkg/status"), "Package: libxml2\nVersion: 2.15.0\nArchitecture: arm64-osx\nStatus: install ok installed\n").unwrap();
    fs::write(
        root.join("arm64-osx/share/libxml2/copyright"),
        "Native original text\n",
    )
    .unwrap();
    fs::write(root.join("LICENSE-MIT"), "Toolchain MIT text\n").unwrap();
    fs::write(root.join("LICENSE-APACHE"), "Toolchain Apache text\n").unwrap();
    fs::write(
        root.join("COPYRIGHT-library.html"),
        "Original library notice report\n",
    )
    .unwrap();
}

#[test]
fn native_export_is_repeatable_and_preserves_existing_output_on_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    native_fixture(root);
    let first = root.join("first");
    let second = root.join("second");
    for output in [&first, &second] {
        assert_eq!(
            write_native_notice_inputs(root, root, "aarch64-apple-darwin", output).unwrap(),
            1
        );
    }
    let first_bytes = fs::read(first.join("native-notice-inputs.json")).unwrap();
    assert_eq!(
        first_bytes,
        fs::read(second.join("native-notice-inputs.json")).unwrap()
    );
    let record: serde_json::Value = serde_json::from_slice(&first_bytes).unwrap();
    let status_hash = record["vcpkg"]["installed_status"]["sha256"]
        .as_str()
        .unwrap();
    assert_eq!(
        fs::read(first.join(format!("texts/{status_hash}.txt"))).unwrap(),
        fs::read(root.join("vcpkg/status")).unwrap()
    );
    assert!(
        record["scope"]
            .as_str()
            .unwrap()
            .contains("not an executable-bound")
    );
    assert!(write_native_notice_inputs(root, root, "aarch64-apple-darwin", &first).is_err());
    fs::remove_file(root.join("COPYRIGHT-library.html")).unwrap();
    assert!(
        write_native_notice_inputs(root, root, "aarch64-apple-darwin", &root.join("failed"))
            .is_err()
    );
    assert!(!root.join("failed").exists());
    assert_eq!(
        first_bytes,
        fs::read(first.join("native-notice-inputs.json")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn native_material_cannot_escape_through_a_directory_symlink() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    native_fixture(root);
    let share = root.join("arm64-osx/share");
    fs::rename(&share, root.join("outside")).unwrap();
    std::os::unix::fs::symlink(root.join("outside"), &share).unwrap();
    assert!(collect_vcpkg_notice_inputs(root, "arm64-osx").is_err());
}

#[test]
fn iconv_adapter_requires_the_exact_installed_file_inventory() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    native_fixture(root);
    let mut status = fs::read_to_string(root.join("vcpkg/status")).unwrap();
    status.push_str("\nPackage: libiconv\nVersion: 1.19\nArchitecture: arm64-osx\nStatus: install ok installed\n");
    fs::write(root.join("vcpkg/status"), status).unwrap();
    fs::create_dir_all(root.join("vcpkg/info")).unwrap();
    fs::create_dir_all(root.join("arm64-osx/share/iconv")).unwrap();
    fs::create_dir_all(root.join("arm64-osx/share/libiconv")).unwrap();
    for name in ["vcpkg.spdx.json", "vcpkg_abi_info.txt"] {
        fs::write(
            root.join("arm64-osx/share/libiconv").join(name),
            "metadata fixture\n",
        )
        .unwrap();
    }
    fs::write(
        root.join("arm64-osx/share/iconv/vcpkg-cmake-wrapper.cmake"),
        "system iconv wrapper fixture\n",
    )
    .unwrap();
    let inventory = "arm64-osx/\narm64-osx/share/\narm64-osx/share/iconv/\narm64-osx/share/iconv/vcpkg-cmake-wrapper.cmake\narm64-osx/share/libiconv/\narm64-osx/share/libiconv/vcpkg.spdx.json\narm64-osx/share/libiconv/vcpkg_abi_info.txt\n";
    let list = root.join("vcpkg/info/libiconv_1.19_arm64-osx.list");
    fs::write(&list, inventory).unwrap();
    let result = collect_vcpkg_notice_inputs(root, "arm64-osx").unwrap();
    assert_eq!(result.packages[0].scope, "vcpkg-installed-build-adapter");
    assert!(
        result.packages[0]
            .texts
            .iter()
            .all(|text| text.kind.starts_with("build-"))
    );
    fs::write(&list, format!("{inventory}arm64-osx/lib/libiconv.a\n")).unwrap();
    assert!(collect_vcpkg_notice_inputs(root, "arm64-osx").is_err());
    fs::remove_file(list).unwrap();
    assert!(collect_vcpkg_notice_inputs(root, "arm64-osx").is_err());
}

#[test]
fn native_target_inputs_retain_every_installed_port_and_reject_missing_notices() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::create_dir_all(root.join("vcpkg")).unwrap();
    fs::write(root.join("vcpkg/status"), "Package: libxml2\nVersion: 2.15.0\nArchitecture: arm64-osx\nStatus: install ok installed\n\nPackage: libxml2\nFeature: zlib\nArchitecture: arm64-osx\nStatus: install ok installed\n\nPackage: host-tool\nVersion: 1\nArchitecture: x64-linux\nStatus: install ok installed\n").unwrap();
    let share = root.join("arm64-osx/share/libxml2");
    fs::create_dir_all(&share).unwrap();
    fs::write(share.join("copyright"), "Original native notices\n").unwrap();
    let result = collect_vcpkg_notice_inputs(root, "arm64-osx").unwrap();
    assert_eq!(result.packages.len(), 1);
    assert_eq!(result.packages[0].name, "libxml2");
    assert_eq!(result.packages[0].features, ["zlib"]);
    assert_eq!(result.packages[0].scope, "vcpkg-target-build-input");
    assert_eq!(result.texts.len(), 1);
    fs::remove_file(share.join("copyright")).unwrap();
    assert!(collect_vcpkg_notice_inputs(root, "arm64-osx").is_err());
    assert!(collect_vcpkg_notice_inputs(root, "../escape").is_err());
}

#[test]
fn rust_library_collection_requires_the_actual_library_notice_report_and_terms() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    fs::write(root.join("LICENSE-MIT"), "Toolchain MIT text\n").unwrap();
    fs::write(root.join("LICENSE-APACHE"), "Toolchain Apache text\n").unwrap();
    assert!(collect_rust_library_notice_inputs(root).is_err());
    fs::create_dir_all(root.join("share/doc/rustc")).unwrap();
    fs::write(
        root.join("share/doc/rustc/COPYRIGHT-library.html"),
        "Original library notice report\n",
    )
    .unwrap();
    let result = collect_rust_library_notice_inputs(root).unwrap();
    assert_eq!(result.texts.len(), 3);
    assert_eq!(result.texts[2].kind, "rust-library-notice-report");
}
