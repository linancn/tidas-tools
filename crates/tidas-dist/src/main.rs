use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tidas_dist::{PackageRequest, package, render_package_metadata, verify};

#[derive(Debug, Parser)]
#[command(
    name = "tidas-dist",
    about = "Internal deterministic distribution builder for the native tidas CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print the workspace release version.
    Version,
    /// Collect locked normal/build Cargo notice inputs for owner review.
    CargoNotices {
        #[arg(long)]
        target: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Export installed native and Rust library notice inputs for owner review.
    NativeNotices {
        #[arg(long)]
        vcpkg_installed: PathBuf,
        #[arg(long)]
        rust_sysroot: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Collect a complete executable-bound native notice bundle from build inputs.
    Notices {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        vcpkg_root: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Build one deterministic platform archive and checksum.
    Package {
        #[arg(long)]
        binary: PathBuf,
        #[arg(long)]
        license: PathBuf,
        #[arg(long)]
        notices_dir: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Verify checksum, archive contract, and optionally run packaged smoke probes.
    Verify {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        checksum: PathBuf,
        #[arg(long)]
        target: String,
        #[arg(long)]
        smoke: bool,
    },
    /// Generate Homebrew and Winget metadata from the four exact archive checksums.
    Metadata {
        #[arg(long)]
        release_base_url: String,
        #[arg(long)]
        artifacts_dir: PathBuf,
        #[arg(long)]
        output_dir: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), tidas_dist::DistError> {
    let version = env!("CARGO_PKG_VERSION");
    match Cli::parse().command {
        Commands::Version => println!("{version}"),
        Commands::CargoNotices { target, output_dir } => {
            let packages =
                tidas_dist::notices::write_current_cargo_notice_inputs(&target, &output_dir)?;
            println!(
                "{}",
                serde_json::json!({"status":"collected","scope":"cargo-source-notice-inputs","packages":packages})
            );
        }
        Commands::NativeNotices {
            vcpkg_installed,
            rust_sysroot,
            target,
            output_dir,
        } => {
            let packages = tidas_dist::notices::write_native_notice_inputs(
                &vcpkg_installed,
                &rust_sysroot,
                &target,
                &output_dir,
            )?;
            println!(
                "{}",
                serde_json::json!({"status":"collected","scope":"native-source-notice-inputs","packages":packages})
            );
        }
        Commands::Notices {
            binary,
            target,
            vcpkg_root,
            output_dir,
        } => {
            let manifest =
                tidas_dist::notice_bundle::collect(&tidas_dist::notice_bundle::CollectRequest {
                    binary: &binary,
                    target: &target,
                    version,
                    vcpkg_root: &vcpkg_root,
                    output_dir: &output_dir,
                })?;
            println!(
                "{}",
                serde_json::json!({"status":"collected","schema_version":manifest.schema_version,"target":manifest.target,"version":manifest.version,"files":manifest.files.len()})
            );
        }
        Commands::Package {
            binary,
            license,
            notices_dir,
            target,
            output_dir,
        } => {
            let artifact = package(&PackageRequest {
                binary: &binary,
                license: &license,
                notices_dir: &notices_dir,
                target: &target,
                version,
                output_dir: &output_dir,
            })?;
            println!("{}", serde_json::to_string(&artifact)?);
        }
        Commands::Verify {
            archive,
            checksum,
            target,
            smoke,
        } => {
            let manifest = verify(&archive, &checksum, &target, version, smoke)?;
            println!("{}", serde_json::to_string(&manifest)?);
        }
        Commands::Metadata {
            release_base_url,
            artifacts_dir,
            output_dir,
        } => {
            for path in
                render_package_metadata(version, &release_base_url, &artifacts_dir, &output_dir)?
            {
                println!("{}", path.display());
            }
        }
    }
    Ok(())
}
