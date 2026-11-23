//! `nsh-frameworks`: embeds the dynamic frameworks an app's crates declare
//! into the package NSH made, and lists what each target links.

mod embed;

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use nsh_frameworks::Target;

#[derive(Parser)]
#[command(
    name = "nsh-frameworks",
    version,
    about = "Prebuilt Apple frameworks for Native Shell (NSH) apps"
)]
struct Cli {
    /// The Cargo manifest of the app's crates. Default: `.nsh/rust/Cargo.toml`
    /// (NSH's workspace of the app's crates and nshm), else `Cargo.toml`, in
    /// the current directory.
    #[arg(long, global = true)]
    manifest_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Copy the dynamic frameworks the app's crates declare into a package
    /// (`.app` or `.ipa`), add the rpath to them, and sign them and the app
    /// again, keeping its entitlements.
    Embed {
        /// The package `nsh package` made.
        package: PathBuf,
        /// The identity to sign with. Default: the one the package is
        /// signed with (ad hoc for simulator and macOS packages).
        #[arg(long)]
        sign: Option<String>,
    },
    /// Print the shell line that lets the programs `nsh` builds and runs
    /// on this Mac (`nsh run`, and the describing program of `nsh package`
    /// and `nsh test`) find the dynamic frameworks where they are:
    /// `eval "$(nsh-frameworks env)"`.
    Env {
        /// The Rust target `nsh run` builds for. Default: this Mac.
        #[arg(long)]
        target: Option<String>,
    },
    /// List what each framework links for a target.
    List {
        /// The Rust target. Default: this Mac.
        #[arg(long)]
        target: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn host_triple() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-apple-darwin"
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let manifest = |cli_path: &Option<PathBuf>| match cli_path {
        Some(path) => Ok(path.clone()),
        None => embed::default_manifest(Path::new(".")),
    };
    match &cli.command {
        Command::Embed { package, sign } => {
            let manifest = manifest(&cli.manifest_path)?;
            let ipa = package.extension().is_some_and(|e| e == "ipa");
            let app = if ipa {
                None
            } else {
                Some(embed::Bundle::read(package)?)
            };
            // The targets of an `.ipa` are those of its app, read after
            // unzipping; its crates are iOS's either way.
            let triples: Vec<String> = match &app {
                Some(bundle) => bundle.targets.iter().map(|(t, _)| t.clone()).collect(),
                None => vec!["aarch64-apple-ios".into()],
            };
            let frameworks = embed::declared(&manifest, &triples)?;
            let done = if ipa {
                embed::embed_ipa(package, &frameworks, sign.as_deref())?
            } else {
                embed::embed(package, &frameworks, sign.as_deref())?
            };
            if done.frameworks.is_empty() {
                println!(
                    "{}: the app's crates declare no dynamic frameworks for it; nothing to embed",
                    package.display()
                );
            } else {
                for framework in &done.frameworks {
                    println!("embedded {}", framework.display());
                }
                if done.rpath_added {
                    println!("added the executable's rpath to them");
                }
                println!(
                    "signed {} {}",
                    package.display(),
                    if done.identity == "-" {
                        "ad hoc".to_owned()
                    } else {
                        format!("with {}", done.identity)
                    }
                );
            }
        }
        Command::Env { target } => {
            let triple = target.as_deref().unwrap_or(host_triple());
            let target = Target::of_triple(triple)
                .with_context(|| format!("{triple} is not an iOS or macOS target"))?;
            let frameworks = embed::declared(&manifest(&cli.manifest_path)?, &[triple.into()])?;
            let mut dirs: Vec<String> = Vec::new();
            for framework in &frameworks {
                if let Some(bundle) = framework.library(target)?.dynamic_bundle() {
                    let dir = bundle
                        .parent()
                        .context("a framework is in a directory")?
                        .display()
                        .to_string();
                    if !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                }
            }
            // dyld looks there only for what it finds nowhere else, so a
            // package's own copy still wins; the defaults it replaces stay.
            dirs.extend(
                [
                    "~/Library/Frameworks",
                    "/Library/Frameworks",
                    "/System/Library/Frameworks",
                ]
                .map(|d| d.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)),
            );
            // The simulator passes the variables prefixed so to the app.
            let name = if target.variant == Some("simulator") {
                "SIMCTL_CHILD_DYLD_FALLBACK_FRAMEWORK_PATH"
            } else {
                "DYLD_FALLBACK_FRAMEWORK_PATH"
            };
            println!("export {name}={}", shell_quote(&dirs.join(":")));
        }
        Command::List { target } => {
            let triple = target.as_deref().unwrap_or(host_triple());
            let target = Target::of_triple(triple)
                .with_context(|| format!("{triple} is not an iOS or macOS target"))?;
            let frameworks = embed::declared(&manifest(&cli.manifest_path)?, &[triple.into()])?;
            if frameworks.is_empty() {
                println!(
                    "the app's crates declare no frameworks for {}",
                    target.name()
                );
            }
            for framework in &frameworks {
                let library = framework.library(target)?;
                let kind = match &library {
                    nsh_frameworks::Library::Archive(_) => "static library",
                    nsh_frameworks::Library::Framework { dynamic: true, .. } => {
                        "dynamic framework, embedded"
                    }
                    nsh_frameworks::Library::Framework { .. } => "static framework",
                };
                println!(
                    "{}\n  {} ({kind})\n  listed by {}",
                    framework.path.display(),
                    library.input().display(),
                    framework.listed
                );
            }
        }
    }
    Ok(())
}

/// `text` quoted for a POSIX shell.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}
