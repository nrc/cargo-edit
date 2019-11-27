//! `cargo tidy`

use cargo_edit::Manifest;
use std::path::PathBuf;
use structopt::StructOpt;

#[macro_use]
extern crate error_chain;

mod errors {
    error_chain! {
        links {
            CargoEditLib(::cargo_edit::Error, ::cargo_edit::ErrorKind);
        }
        foreign_links {
            Io(::std::io::Error);
        }
    }
}
use crate::errors::*;

#[derive(Debug, StructOpt)]
#[structopt(bin_name = "cargo")]
enum Command {
    /// Reformat a Cargo.toml manifest file.
    #[structopt(name = "tidy")]
    Tidy(Args),
}

#[derive(Debug, StructOpt)]
struct Args {
    /// Path to the manifest to remove a dependency from.
    #[structopt(long = "manifest-path", value_name = "path")]
    manifest_path: Option<PathBuf>,

    /// Reformat all packages in the workspace.
    #[structopt(long = "all")]
    all: bool,

    /// Do not print any output in case of success.
    #[structopt(long = "quiet", short = "q")]
    quiet: bool,
}

// TODO print msg and handle errors

fn handle_tidy(args: &Args) -> Result<()> {
    let manifest_path = &args.manifest_path;
    let mut manifest = Manifest::open(manifest_path)?;
    manifest.sort_table(&["dependencies".to_owned()])?;

    let mut file = Manifest::find_file(manifest_path)?;
    manifest.write_to_file(&mut file)?;

    Ok(())
}

fn main() {
    let args: Command = Command::from_args();
    let Command::Tidy(args) = args;

    if let Err(e) = handle_tidy(&args) {
        eprintln!("error {:?}", e);
    }
}
