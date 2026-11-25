//! `nsh-native-deps android`: adds Android libraries from Maven repositories
//! to an NSH app or nshm crate. See [`nsh_native_deps::maven`].

use std::path::PathBuf;

use anyhow::bail;
use clap::Subcommand;
use nsh_native_deps::maven::{self, Coordinate, Repositories, Target};

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
    /// The app (its `nsh.json`) or crate (its `Cargo.toml`, with `--crate`)
    /// at this directory or the nearest one above it.
    #[arg(long, global = true, default_value = ".")]
    dir: PathBuf,
    /// Vendor into an nshm crate instead of an app.
    #[arg(long = "crate", global = true)]
    crate_: bool,
    /// The repositories to search, in order, `https://` or `file://`
    /// (default: Google's Maven repository, then Maven Central). Kept in
    /// `android/maven/maven.json` for later runs.
    #[arg(long, global = true, value_name = "URL")]
    repository: Vec<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Add libraries, as `group:artifact:version`; one added before takes
    /// the version given.
    Add {
        #[arg(required = true, value_name = "GROUP:ARTIFACT:VERSION")]
        libraries: Vec<String>,
    },
    /// Remove libraries, as `group:artifact`, with what only they needed.
    Remove {
        #[arg(required = true, value_name = "GROUP:ARTIFACT")]
        libraries: Vec<String>,
    },
    /// Resolve and download the libraries added before again, as
    /// `android/maven/maven.json` records them (a crate's: as `Cargo.toml`
    /// `[package.metadata.nsh-native-deps] android` lists them).
    Sync,
    /// List the libraries added and the files they resolved to.
    List,
}

pub fn run(args: &Args) -> anyhow::Result<()> {
    let target = Target::find(&args.dir, args.crate_)?;
    let mut record = target.record()?;
    let mut wanted = target.wanted(&record)?;
    match &args.command {
        Command::List => {
            for library in &wanted {
                println!("{library}");
            }
            for file in &record.resolved {
                println!("  {}  {}", file.path, file.library);
            }
            return Ok(());
        }
        Command::Sync => {}
        Command::Add { libraries } => {
            for text in libraries {
                let c: Coordinate = text.parse()?;
                wanted.retain(|w| w.module != c.module);
                wanted.push(c);
            }
        }
        Command::Remove { libraries } => {
            for name in libraries {
                let before = wanted.len();
                wanted.retain(|c| c.module.to_string() != *name);
                if wanted.len() == before {
                    bail!("`{name}` is not one of the libraries added (group:artifact)");
                }
            }
        }
    }
    if !args.repository.is_empty() {
        record.repositories = args.repository.clone();
    }
    let mut source = Repositories::new(&maven::repositories_of(&record))?;
    let summary = maven::vendor(&target, &mut source, &wanted, record.repositories)?;
    for warning in &summary.warnings {
        eprintln!("warning: {warning}");
    }
    let listed = match target {
        Target::App(_) => "nsh.json `android.libraries`",
        Target::Crate(_) => "Cargo.toml `[package.metadata.nsh.android] libraries`",
    };
    println!(
        "{} libraries ({} modules, {} files) are in {}/ and {listed}",
        wanted.len(),
        summary.modules.len(),
        summary.files,
        maven::VENDOR_DIR
    );
    Ok(())
}
