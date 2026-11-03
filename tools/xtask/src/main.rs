mod command;
mod packaging;
mod release;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Windwatcher repository tooling")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    Release {
        #[command(subcommand)]
        command: Release,
    },
    Packaging {
        #[command(subcommand)]
        command: Packaging,
    },
}

#[derive(Args)]
struct Metadata {
    #[arg(value_parser = ["windwatcher", "windwatcher-server"])]
    package: String,
    version: String,
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    date: Option<String>,
    #[arg(long, default_value = "origin/main")]
    main_ref: String,
}

#[derive(Subcommand)]
enum Release {
    ValidateVersion(Metadata),
    Prepare(Metadata),
    State(Metadata),
    Notes(Metadata),
    AllowList(Metadata),
    Dependency(Metadata),
    Registry {
        #[command(subcommand)]
        command: Registry,
    },
    Artifacts {
        #[command(subcommand)]
        command: Artifacts,
    },
    Github {
        #[command(subcommand)]
        command: Github,
    },
    Oci {
        #[command(subcommand)]
        command: Oci,
    },
}

#[derive(Subcommand)]
enum Registry {
    Dependency(Metadata),
    Publish(Metadata),
}

#[derive(Subcommand)]
enum Artifacts {
    Collect {
        version: String,
        source: PathBuf,
        destination: PathBuf,
    },
    Validate {
        version: String,
        directory: PathBuf,
        #[arg(long)]
        create: bool,
    },
}

#[derive(Subcommand)]
enum Github {
    Finalize {
        #[arg(value_parser = ["windwatcher", "windwatcher-server"])]
        package: String,
        version: String,
        revision: String,
        #[arg(long)]
        assets: Option<PathBuf>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Args)]
struct Image {
    version: String,
    revision: String,
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(Subcommand)]
enum Oci {
    Architecture {
        #[command(flatten)]
        image: Image,
        #[arg(long, value_parser = ["amd64", "arm64"])]
        arch: String,
    },
    Manifest(Image),
}

#[derive(Subcommand)]
enum Packaging {
    Reproducibility {
        #[arg(value_parser = ["x86_64", "aarch64"])]
        arch: String,
        binary: PathBuf,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> Result<()> {
    let runner = command::Process;
    match Cli::parse().command {
        Task::Release { command } => match command {
            Release::ValidateVersion(a) => {
                release::version::parse(&a.version)?;
            }
            Release::Prepare(a) => release::metadata::prepare(
                &runner,
                &a.root,
                &a.package,
                &a.version,
                a.date.as_deref(),
            )?,
            Release::State(a) => println!(
                "{}",
                release::state::state(&runner, &a.root, &a.package, &a.version, &a.main_ref)?
            ),
            Release::Notes(a) => println!(
                "{}",
                release::metadata::notes(&a.root, &a.package, &a.version)?
            ),
            Release::AllowList(a) => {
                release::version::parse(&a.version)?;
                release::state::check_diff(&runner, &a.root, &a.package)?;
            }
            Release::Dependency(a) => {
                release::version::parse(&a.version)?;
                println!(
                    "{}",
                    release::metadata::requirement(&std::fs::read_to_string(
                        a.root
                            .join(release::metadata::manifest("windwatcher-server"))
                    )?)?
                );
            }
            Release::Registry { command } => match command {
                Registry::Dependency(a) => {
                    release::registry::run(&runner, &a.root, &a.package, &a.version, false)?
                }
                Registry::Publish(a) => {
                    release::registry::run(&runner, &a.root, &a.package, &a.version, true)?
                }
            },
            Release::Artifacts { command } => match command {
                Artifacts::Collect {
                    version,
                    source,
                    destination,
                } => release::artifacts::collect(&source, &destination, &version)?,
                Artifacts::Validate {
                    version,
                    directory,
                    create,
                } => release::artifacts::validate(&directory, &version, create)?,
            },
            Release::Github {
                command:
                    Github::Finalize {
                        package,
                        version,
                        revision,
                        assets,
                        root,
                    },
            } => release::github::finalize(
                &runner,
                &root,
                &package,
                &version,
                &revision,
                assets.as_deref(),
            )?,
            Release::Oci { command } => match command {
                Oci::Architecture { image, arch } => release::oci::architecture(
                    &runner,
                    &image.root,
                    &arch,
                    &image.version,
                    &image.revision,
                )?,
                Oci::Manifest(image) => {
                    release::oci::manifest(&runner, &image.root, &image.version, &image.revision)?
                }
            },
        },
        Task::Packaging {
            command: Packaging::Reproducibility { arch, binary, root },
        } => packaging::reproducibility(&root, &arch, &binary)?,
    }
    Ok(())
}
