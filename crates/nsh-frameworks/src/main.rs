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
    about = "Prebuilt native SDKs for Native Shell (NSH) apps: Apple frameworks, Linux and Windows libraries"
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
    /// Copy the dynamic frameworks or shared libraries the app's crates
    /// declare into a package: an `.app` or `.ipa` (the rpath added, signed
    /// again with its entitlements kept), or the directory `nsh package`
    /// wrote for Linux or Windows (its `.tar.gz`, `.zip` and installer made
    /// again).
    Embed {
        /// The package `nsh package` made: `.nsh/out/<Name>.app`,
        /// `.nsh/out/<Name>.ipa`, `.nsh/out/<Name>`.
        package: PathBuf,
        /// The identity to sign with. Default: the one the package is
        /// signed with (ad hoc for simulator and macOS packages).
        #[arg(long)]
        sign: Option<String>,
    },
    /// Print the shell line that lets the programs `nsh` builds and runs
    /// on this machine (`nsh run`, and the describing program of `nsh
    /// package` and `nsh test`) find the dynamic libraries where they are:
    /// `eval "$(nsh-frameworks env)"`; on Windows, PowerShell's
    /// `nsh-frameworks env | Invoke-Expression`.
    Env {
        /// The Rust target `nsh run` builds for. Default: this machine.
        #[arg(long)]
        target: Option<String>,
    },
    /// List what each framework links for a target.
    List {
        /// The Rust target. Default: this machine.
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

/// The Rust target of this machine, as `nsh` builds for it.
fn host_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", _) => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", _) => "x86_64-unknown-linux-gnu",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => "x86_64-pc-windows-msvc",
    }
}

/// Embeds into the Linux or Windows package `package`.
fn embed_desktop(package: &Path, manifest: &Path) -> anyhow::Result<()> {
    let dir = embed::desktop_dir(package);
    let (executable, target) = embed::desktop_executable(&dir)?;
    let frameworks = embed::declared(manifest, &[target.triple()])?;
    let carried = nsh_frameworks::desktop::carried(&frameworks, target)?;
    let done = embed::embed_desktop(&dir, &executable, target, &carried)?;
    if done.libraries.is_empty() {
        println!(
            "{}: the app's crates declare no shared libraries for it; nothing to embed",
            dir.display()
        );
        return Ok(());
    }
    for library in &done.libraries {
        println!("embedded {}", library.display());
    }
    if done.rpath_added {
        println!("set the executable's rpath to $ORIGIN");
    }
    for archive in &done.archives {
        println!("made {} again", archive.display());
    }
    for note in &done.notes {
        println!("note: {note}");
    }
    Ok(())
}

/// The shell line of `env` for a Linux or Windows `target`.
fn desktop_env(
    frameworks: &[nsh_frameworks::Framework],
    target: nsh_frameworks::desktop::DesktopTarget,
) -> anyhow::Result<String> {
    use nsh_frameworks::desktop::Os;
    let mut dirs: Vec<String> = Vec::new();
    for library in nsh_frameworks::desktop::carried(frameworks, target)? {
        let dir = library
            .parent()
            .context("a library is in a directory")?
            .display()
            .to_string();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    Ok(match target.os {
        Os::Linux => format!(
            "export LD_LIBRARY_PATH={}\"${{LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}}\"",
            shell_quote(&dirs.join(":"))
        ),
        // PowerShell, Windows' shell.
        Os::Windows => format!(
            "$env:PATH = '{};' + $env:PATH",
            dirs.join(";").replace('\'', "''")
        ),
    })
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
            let apple = package
                .extension()
                .is_some_and(|e| e == "app" || e == "ipa");
            if !apple {
                return embed_desktop(package, &manifest);
            }
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
            let frameworks = embed::declared(&manifest(&cli.manifest_path)?, &[triple.into()])?;
            if let Some(desktop) = nsh_frameworks::desktop::DesktopTarget::of_triple(triple) {
                println!("{}", desktop_env(&frameworks, desktop)?);
                return Ok(());
            }
            let target = Target::of_triple(triple)
                .with_context(|| format!("{triple} is not a target nsh-frameworks knows"))?;
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
            let frameworks = embed::declared(&manifest(&cli.manifest_path)?, &[triple.into()])?;
            if let Some(desktop) = nsh_frameworks::desktop::DesktopTarget::of_triple(triple) {
                use nsh_frameworks::desktop::DesktopLibrary;
                if frameworks.is_empty() {
                    println!("the app's crates declare no libraries for {triple}");
                }
                for (library, framework) in
                    nsh_frameworks::desktop::libraries(&frameworks, desktop)?
                {
                    let kind = match library {
                        DesktopLibrary::Static(_) => "static library",
                        DesktopLibrary::Shared(_) => "shared library, embedded",
                        DesktopLibrary::Import(_) => "import library",
                        DesktopLibrary::Runtime(_) => "DLL, embedded",
                    };
                    println!(
                        "{}\n  ({kind})\n  listed by {}",
                        library.path().display(),
                        framework.listed
                    );
                }
                return Ok(());
            }
            let target = Target::of_triple(triple)
                .with_context(|| format!("{triple} is not a target nsh-frameworks knows"))?;
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
