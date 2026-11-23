//! `nsh-frameworks embed`: the dynamic frameworks the app's crates declare,
//! copied into a package NSH made and signed with it.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, bail};
use nsh_frameworks::{Framework, Platform, Target, Value};

/// Runs `program` with `args`, failing with its error output.
pub fn run(program: &str, args: &[&std::ffi::OsStr]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// `cargo metadata`'s `json` as the frameworks its resolved packages
/// declare for `platform`, in the order Cargo lists them, each once.
pub fn frameworks_in_metadata(
    json: &serde_json::Value,
    platform: Platform,
) -> anyhow::Result<Vec<Framework>> {
    let resolved: Vec<&str> = json["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n["id"].as_str())
        .collect();
    let mut frameworks: Vec<Framework> = Vec::new();
    for package in json["packages"].as_array().into_iter().flatten() {
        let Some(table) = package["metadata"].get(nsh_frameworks::METADATA_KEY) else {
            continue;
        };
        if !package["id"]
            .as_str()
            .is_some_and(|id| resolved.contains(&id))
        {
            continue;
        }
        let name = package["name"].as_str().unwrap_or("?");
        let dir = package["manifest_path"]
            .as_str()
            .and_then(|m| Path::new(m).parent())
            .with_context(|| format!("{name}: cargo metadata names no Cargo.toml"))?;
        for framework in nsh_frameworks::frameworks_of(&value(table), dir, platform, name)? {
            if !frameworks.iter().any(|f| f.path == framework.path) {
                frameworks.push(framework);
            }
        }
    }
    Ok(frameworks)
}

/// A JSON value as a [`Value`].
fn value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Bool(b) => Value::Flag(*b),
        serde_json::Value::Array(items) => Value::List(items.iter().map(value).collect()),
        serde_json::Value::Object(fields) => {
            Value::Table(fields.iter().map(|(k, v)| (k.clone(), value(v))).collect())
        }
        _ => Value::Other,
    }
}

/// The frameworks the crates of the Cargo workspace or package `manifest`
/// declare for the targets `triples`, as Cargo resolves it for them.
pub fn declared(manifest: &Path, triples: &[String]) -> anyhow::Result<Vec<Framework>> {
    let platform = triples
        .iter()
        .find_map(|t| Target::of_triple(t))
        .map_or(Platform::Macos, |t| t.platform);
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "metadata".as_ref(),
        "--format-version".as_ref(),
        "1".as_ref(),
        "--manifest-path".as_ref(),
        manifest.as_os_str(),
    ];
    for triple in triples {
        args.extend::<[&std::ffi::OsStr; 2]>(["--filter-platform".as_ref(), triple.as_ref()]);
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(&cargo)
        .args(&args)
        .output()
        .context("run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed for {}: {}",
            manifest.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("cargo metadata printed no JSON")?;
    frameworks_in_metadata(&json, platform)
}

/// The Cargo manifest an app's frameworks are read from: `--manifest-path`,
/// else NSH's generated workspace of the app's crates and nshm
/// (`.nsh/rust/Cargo.toml`), else the crate in `dir`.
pub fn default_manifest(dir: &Path) -> anyhow::Result<PathBuf> {
    for candidate in [dir.join(".nsh/rust/Cargo.toml"), dir.join("Cargo.toml")] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "no .nsh/rust/Cargo.toml or Cargo.toml in {}: run in the app's directory after `nsh package` \
         (or `nsh run`), or pass --manifest-path",
        dir.display()
    )
}

/// A package's app bundle: where its executable and frameworks are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub app: PathBuf,
    pub executable: PathBuf,
    /// Its Rust targets, one per architecture of the executable.
    pub targets: Vec<(String, Target)>,
}

impl Bundle {
    /// The bundle at `app`, read from its `Info.plist` and executable.
    pub fn read(app: &Path) -> anyhow::Result<Bundle> {
        let contents = app.join("Contents");
        let mac = contents.join("Info.plist").is_file();
        let info_path = if mac {
            contents.join("Info.plist")
        } else {
            app.join("Info.plist")
        };
        let info = plist::Value::from_file(&info_path)
            .with_context(|| format!("{} is not an app bundle", app.display()))?;
        let name = info
            .as_dictionary()
            .and_then(|d| d.get("CFBundleExecutable"))
            .and_then(plist::Value::as_string)
            .with_context(|| format!("{} names no CFBundleExecutable", info_path.display()))?;
        let executable = if mac {
            contents.join("MacOS").join(name)
        } else {
            app.join(name)
        };
        let loads = run("otool", &["-l".as_ref(), executable.as_os_str()])?;
        let platform = build_platform(&loads)
            .with_context(|| format!("{} names no platform", executable.display()))?;
        let archs = run("lipo", &["-archs".as_ref(), executable.as_os_str()])?;
        let mut targets = Vec::new();
        for arch in archs.split_whitespace() {
            let triple = triple_of(platform, arch).with_context(|| {
                format!(
                    "{}: no Rust target for platform {platform} on {arch}",
                    executable.display()
                )
            })?;
            let target = Target::of_triple(&triple).context("an Apple target")?;
            targets.push((triple, target));
        }
        if targets.is_empty() {
            bail!("{} holds no architecture", executable.display());
        }
        Ok(Bundle {
            app: app.to_owned(),
            executable,
            targets,
        })
    }

    /// Where it keeps its frameworks, and the rpath its executable finds
    /// them by.
    pub fn frameworks_dir(&self) -> (PathBuf, &'static str) {
        if self.targets[0].1.is_mac() {
            (
                self.app.join("Contents/Frameworks"),
                "@executable_path/../Frameworks",
            )
        } else {
            (self.app.join("Frameworks"), "@executable_path/Frameworks")
        }
    }
}

/// The platform number of the first `LC_BUILD_VERSION` in `otool -l`'s
/// output (`LC_VERSION_MIN_*` for older executables).
pub fn build_platform(otool: &str) -> Option<u32> {
    let mut lines = otool.lines().map(str::trim);
    while let Some(line) = lines.next() {
        match line {
            "cmd LC_BUILD_VERSION" => {
                for line in lines.by_ref().take(4) {
                    if let Some(n) = line.strip_prefix("platform ") {
                        return n.trim().parse().ok();
                    }
                }
            }
            "cmd LC_VERSION_MIN_MACOSX" => return Some(1),
            "cmd LC_VERSION_MIN_IPHONEOS" => return Some(2),
            _ => {}
        }
    }
    None
}

/// The Rust triple of Mach-O `platform` (`PLATFORM_*`) on `arch`.
pub fn triple_of(platform: u32, arch: &str) -> Option<String> {
    let arch = match arch {
        "arm64" | "arm64e" => "aarch64",
        "x86_64" => "x86_64",
        _ => return None,
    };
    let os = match (platform, arch) {
        (1, _) => "apple-darwin",
        (2, _) => "apple-ios",
        (6, _) => "apple-ios-macabi",
        (7, "x86_64") => "apple-ios",
        (7, _) => "apple-ios-sim",
        _ => return None,
    };
    Some(format!("{arch}-{os}"))
}

/// The identity to sign with: `asked` (`--sign`), else the one the app is
/// signed with now, read from `codesign -dvv`'s output `signature` (ad hoc,
/// `-`, for an app without one).
pub fn identity(asked: Option<&str>, signature: &str) -> String {
    if let Some(asked) = asked {
        return asked.to_owned();
    }
    signature
        .lines()
        .find_map(|line| line.strip_prefix("Authority="))
        .map_or_else(|| "-".to_owned(), str::to_owned)
}

/// The dynamic framework bundles of `frameworks` the bundle carries: one per
/// framework, the same library for each of its architectures.
pub fn dynamic_bundles(frameworks: &[Framework], bundle: &Bundle) -> anyhow::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for framework in frameworks {
        let mut chosen: Option<PathBuf> = None;
        for (_, target) in &bundle.targets {
            let library = framework.library(*target)?;
            let this = library.dynamic_bundle().map(Path::to_owned);
            match (&chosen, this) {
                (_, None) => {}
                (None, Some(this)) => chosen = Some(this),
                (Some(earlier), Some(this)) if *earlier == this => {}
                (Some(_), Some(_)) => bail!(
                    "{}: {} has another dynamic framework for each architecture, and a universal \
                     app carries one copy: build one framework of both (an XCFramework library \
                     `macos-arm64_x86_64`), or package one architecture",
                    framework.listed,
                    framework.path.display()
                ),
            }
        }
        if let Some(chosen) = chosen {
            let name = chosen.file_name().context("a framework has a name")?;
            if found
                .iter()
                .any(|b: &PathBuf| b.file_name() == Some(name) && *b != chosen)
            {
                bail!(
                    "two frameworks are called {}: an app carries one",
                    name.to_string_lossy()
                );
            }
            if !found.contains(&chosen) {
                found.push(chosen);
            }
        }
    }
    Ok(found)
}

/// What [`embed`] did.
#[derive(Debug, Default)]
pub struct Embedded {
    pub frameworks: Vec<PathBuf>,
    pub rpath_added: bool,
    pub identity: String,
}

/// Copies the dynamic frameworks of `frameworks` into the app bundle `app`,
/// gives its executable the rpath to them, and signs them and the app with
/// `asked` (`--sign`), or the identity the app is signed with, keeping the
/// app's entitlements and identifier.
pub fn embed(
    app: &Path,
    frameworks: &[Framework],
    asked: Option<&str>,
) -> anyhow::Result<Embedded> {
    let bundle = Bundle::read(app)?;
    let bundles = dynamic_bundles(frameworks, &bundle)?;
    let mut done = Embedded::default();
    if bundles.is_empty() {
        return Ok(done);
    }
    let (dir, rpath) = bundle.frameworks_dir();
    std::fs::create_dir_all(&dir)?;
    for source in &bundles {
        let dest = dir.join(source.file_name().context("a framework has a name")?);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        // `ditto` keeps a framework's symbolic links (macOS ones are
        // versioned) and leaves out extended attributes (a downloaded SDK's
        // quarantine), which `codesign` refuses.
        run(
            "ditto",
            &[
                "--norsrc".as_ref(),
                "--noextattr".as_ref(),
                "--noqtn".as_ref(),
                source.as_os_str(),
                dest.as_os_str(),
            ],
        )?;
        // `ditto --noqtn` keeps the quarantine on some macOS versions; the
        // SDK's download is not the app's.
        let _ = run(
            "xattr",
            &[
                "-dr".as_ref(),
                "com.apple.quarantine".as_ref(),
                dest.as_os_str(),
            ],
        );
        done.frameworks.push(dest);
    }
    let loads = run("otool", &["-l".as_ref(), bundle.executable.as_os_str()])?;
    if !has_rpath(&loads, rpath) {
        run(
            "install_name_tool",
            &[
                "-add_rpath".as_ref(),
                rpath.as_ref(),
                bundle.executable.as_os_str(),
            ],
        )?;
        done.rpath_added = true;
    }
    let signature = run("codesign", &["-dvv".as_ref(), bundle.app.as_os_str()]).unwrap_or_default();
    done.identity = identity(asked, &signature);
    let ad_hoc = done.identity == "-";
    // The app's signature seals its frameworks': they are signed first.
    for framework in &done.frameworks {
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "--force".as_ref(),
            "--sign".as_ref(),
            done.identity.as_ref(),
        ];
        if ad_hoc {
            args.push("--timestamp=none".as_ref());
        }
        args.push(framework.as_os_str());
        run("codesign", &args)?;
    }
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "--force".as_ref(),
        "--sign".as_ref(),
        done.identity.as_ref(),
        "--preserve-metadata=identifier,entitlements,flags,runtime".as_ref(),
    ];
    if ad_hoc {
        args.push("--timestamp=none".as_ref());
    }
    args.push(bundle.app.as_os_str());
    run("codesign", &args)?;
    Ok(done)
}

/// Whether `otool -l`'s output has the `LC_RPATH` `rpath`.
pub fn has_rpath(otool: &str, rpath: &str) -> bool {
    let mut lines = otool.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line == "cmd LC_RPATH" {
            for line in lines.by_ref().take(3) {
                if let Some(path) = line.strip_prefix("path ") {
                    let path = path.split(" (offset").next().unwrap_or(path).trim();
                    if path == rpath {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// [`embed`] for an `.ipa`: its `Payload/*.app`, zipped again in place.
pub fn embed_ipa(
    ipa: &Path,
    frameworks: &[Framework],
    asked: Option<&str>,
) -> anyhow::Result<Embedded> {
    let work = ipa.with_extension("nsh-frameworks");
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    run(
        "ditto",
        &[
            "-x".as_ref(),
            "-k".as_ref(),
            ipa.as_os_str(),
            work.as_os_str(),
        ],
    )?;
    let payload = work.join("Payload");
    let app = std::fs::read_dir(&payload)
        .with_context(|| format!("{} has no Payload", ipa.display()))?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .with_context(|| format!("{} has no app in Payload", ipa.display()))?;
    let done = embed(&app, frameworks, asked)?;
    let zipped = ipa.with_extension("ipa.new");
    run(
        "ditto",
        &[
            "-c".as_ref(),
            "-k".as_ref(),
            "--norsrc".as_ref(),
            "--keepParent".as_ref(),
            payload.as_os_str(),
            zipped.as_os_str(),
        ],
    )?;
    std::fs::rename(&zipped, ipa)?;
    std::fs::remove_dir_all(&work)?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mach_o_platforms_name_rust_targets() {
        assert_eq!(
            triple_of(1, "arm64").as_deref(),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(
            triple_of(1, "x86_64").as_deref(),
            Some("x86_64-apple-darwin")
        );
        assert_eq!(triple_of(2, "arm64").as_deref(), Some("aarch64-apple-ios"));
        assert_eq!(
            triple_of(7, "arm64").as_deref(),
            Some("aarch64-apple-ios-sim")
        );
        assert_eq!(triple_of(7, "x86_64").as_deref(), Some("x86_64-apple-ios"));
        assert_eq!(
            triple_of(6, "arm64").as_deref(),
            Some("aarch64-apple-ios-macabi")
        );
        assert_eq!(triple_of(3, "arm64"), None);
        assert_eq!(triple_of(1, "i386"), None);
    }

    #[test]
    fn otool_output_gives_the_platform_and_rpaths() {
        let otool = "Load command 9\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 7\n    minos 15.0\n\
                     Load command 20\n          cmd LC_RPATH\n      cmdsize 48\n         path @executable_path/Frameworks (offset 12)\n";
        assert_eq!(build_platform(otool), Some(7));
        assert!(has_rpath(otool, "@executable_path/Frameworks"));
        assert!(!has_rpath(otool, "@executable_path/../Frameworks"));
        assert_eq!(build_platform("      cmd LC_VERSION_MIN_MACOSX\n"), Some(1));
        assert_eq!(build_platform("nothing"), None);
    }

    #[test]
    fn the_app_is_signed_again_as_it_was() {
        let adhoc = "Executable=/x/App\nIdentifier=com.example.app\nSignature=adhoc\n";
        assert_eq!(identity(None, adhoc), "-");
        let device = "Authority=Apple Development: Someone (ABCDE12345)\nAuthority=Apple Worldwide Developer Relations Certification Authority\n";
        assert_eq!(
            identity(None, device),
            "Apple Development: Someone (ABCDE12345)"
        );
        assert_eq!(
            identity(Some("Developer ID Application: X"), device),
            "Developer ID Application: X"
        );
        assert_eq!(identity(None, ""), "-");
    }

    #[test]
    fn cargo_metadata_gives_the_resolved_crates_frameworks() {
        let json = serde_json::json!({
            "packages": [
                {
                    "id": "scanner 0.1.0", "name": "scanner",
                    "manifest_path": "/work/scanner/Cargo.toml",
                    "metadata": { "nsh-frameworks": {
                        "macos": ["apple/Scanner.xcframework", { "path": "apple/Ads.xcframework", "wholeArchive": true }],
                        "ios": ["apple/ScannerIos.xcframework"],
                    } },
                },
                {
                    "id": "unused 0.1.0", "name": "unused",
                    "manifest_path": "/work/unused/Cargo.toml",
                    "metadata": { "nsh-frameworks": { "macos": ["Unused.xcframework"] } },
                },
                {
                    "id": "other 0.1.0", "name": "other",
                    "manifest_path": "/work/other/Cargo.toml",
                    "metadata": { "nsh-frameworks": { "macos": ["../scanner/apple/Scanner.xcframework"] } },
                },
            ],
            "resolve": { "nodes": [{ "id": "scanner 0.1.0" }, { "id": "other 0.1.0" }] },
        });
        let found = frameworks_in_metadata(&json, Platform::Macos).unwrap();
        let paths: Vec<(PathBuf, bool)> = found
            .iter()
            .map(|f| (f.path.clone(), f.whole_archive))
            .collect();
        assert_eq!(
            paths,
            [
                (
                    PathBuf::from("/work/scanner/apple/Scanner.xcframework"),
                    false
                ),
                (PathBuf::from("/work/scanner/apple/Ads.xcframework"), true),
                (
                    PathBuf::from("/work/other/../scanner/apple/Scanner.xcframework"),
                    false
                ),
            ]
        );
        assert_eq!(
            found[0].listed,
            "scanner: [package.metadata.nsh-frameworks] `macos`[0]"
        );
        let ios = frameworks_in_metadata(&json, Platform::Ios).unwrap();
        assert_eq!(ios.len(), 1);
        let bad = serde_json::json!({
            "packages": [{ "id": "x", "name": "x", "manifest_path": "/x/Cargo.toml",
                           "metadata": { "nsh-frameworks": { "watchos": [] } } }],
            "resolve": { "nodes": [{ "id": "x" }] },
        });
        let error = frameworks_in_metadata(&bad, Platform::Macos)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("`watchos` is not something nsh-frameworks reads"),
            "{error}"
        );
    }

    #[test]
    fn the_app_is_found_in_the_generated_workspace_first() {
        let dir =
            std::env::temp_dir().join(format!("nsh-frameworks-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".nsh/rust")).unwrap();
        assert!(default_manifest(&dir).is_err());
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert_eq!(default_manifest(&dir).unwrap(), dir.join("Cargo.toml"));
        std::fs::write(dir.join(".nsh/rust/Cargo.toml"), "").unwrap();
        assert_eq!(
            default_manifest(&dir).unwrap(),
            dir.join(".nsh/rust/Cargo.toml")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
