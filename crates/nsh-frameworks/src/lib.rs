//! Prebuilt native SDKs for Native Shell (NSH) apps, linked from an nshm
//! crate's build script and carried in app packages: Apple frameworks
//! (`.xcframework`, `.framework`) on iOS and macOS, libraries on Linux and
//! Windows ([`desktop`]).
//!
//! A crate lists them once, in its `Cargo.toml`, relative to the crate:
//!
//! ```toml
//! [package.metadata.nsh-frameworks]
//! ios = ["apple/Scanner.xcframework"]
//! macos = ["apple/Scanner.xcframework", { path = "apple/Ads.xcframework", wholeArchive = true }]
//! linux = ["vendor/linux/{arch}/libscanner.so"]
//! windows = ["vendor/windows/{arch}/scanner.lib", "vendor/windows/{arch}/scanner.dll"]
//! ```
//!
//! and links them from its `build.rs` ([`link_declared`]):
//!
//! ```no_run
//! fn main() {
//!     nsh_frameworks::link_declared().unwrap();
//! }
//! ```
//!
//! Cargo passes a build script's `rustc-link-search` and `rustc-link-lib`
//! on to the final link of every executable that depends on the crate, the
//! app's included. `ios` is for iOS devices, the simulator and Mac Catalyst,
//! `macos` for macOS; other targets link nothing, so the crate still builds
//! everywhere. Each build takes the XCFramework library of its target
//! ([`Framework::library`]).
//!
//! A static library becomes part of the executable. A dynamic framework,
//! `.so` or `.dll` is linked against, and the app needs a copy of it at run
//! time: the `nsh-frameworks embed` command copies it into the package NSH
//! made ([`embed`](https://github.com/SuperNSH/frameworks#use)).

// The example is a build script's `main`, written out as one.
#![allow(clippy::needless_doctest_main)]

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};

pub mod desktop;

/// The table of a crate's `Cargo.toml` this crate reads.
pub const METADATA_KEY: &str = "nsh-frameworks";

/// An Apple target, as XCFrameworks name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// `ios` (devices, the simulator, Mac Catalyst) or `macos`.
    pub platform: Platform,
    /// `None` for devices and Macs, `simulator`, `maccatalyst`.
    pub variant: Option<&'static str>,
    /// `arm64` or `x86_64`.
    pub arch: &'static str,
}

/// The platforms a crate lists frameworks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Ios,
    Macos,
    Linux,
    Windows,
}

impl Platform {
    /// Every one, in the order errors name them.
    pub const ALL: [Platform; 4] = [
        Platform::Ios,
        Platform::Macos,
        Platform::Linux,
        Platform::Windows,
    ];

    /// Its key in `[package.metadata.nsh-frameworks]` (and, for Apple's, in
    /// an XCFramework's `Info.plist`).
    pub fn key(self) -> &'static str {
        match self {
            Platform::Ios => "ios",
            Platform::Macos => "macos",
            Platform::Linux => "linux",
            Platform::Windows => "windows",
        }
    }

    /// The platform of a Rust `triple`, if it is one of these.
    pub fn of_triple(triple: &str) -> Option<Platform> {
        if let Some(target) = Target::of_triple(triple) {
            return Some(target.platform);
        }
        desktop::DesktopTarget::of_triple(triple).map(|t| match t.os {
            desktop::Os::Linux => Platform::Linux,
            desktop::Os::Windows => Platform::Windows,
        })
    }
}

impl Target {
    /// The target of a Rust `triple`; `None` for one that is not iOS or
    /// macOS.
    pub fn of_triple(triple: &str) -> Option<Target> {
        let arch = match triple.split('-').next()? {
            "aarch64" | "arm64" | "arm64e" => "arm64",
            "x86_64" => "x86_64",
            _ => return None,
        };
        if triple.ends_with("-apple-darwin") {
            return Some(Target {
                platform: Platform::Macos,
                variant: None,
                arch,
            });
        }
        let (_, rest) = triple.split_once("-apple-ios")?;
        let variant = match rest {
            // The Intel triple of iOS is the simulator's.
            "" if arch == "x86_64" => Some("simulator"),
            "" => None,
            "-sim" => Some("simulator"),
            "-macabi" => Some("maccatalyst"),
            _ => return None,
        };
        Some(Target {
            platform: Platform::Ios,
            variant,
            arch,
        })
    }

    /// What errors call it.
    pub fn name(self) -> String {
        match (self.platform, self.variant) {
            (Platform::Macos, _) => format!("macOS ({})", self.arch),
            (_, None) => format!("iOS devices ({})", self.arch),
            (_, Some("simulator")) => format!("the iOS simulator ({})", self.arch),
            (_, Some(_)) => format!("Mac Catalyst ({})", self.arch),
        }
    }

    /// Whether a bundle of this target keeps its frameworks in
    /// `Contents/Frameworks` (a Mac app) rather than `Frameworks`.
    pub fn is_mac(self) -> bool {
        self.platform == Platform::Macos || self.variant == Some("maccatalyst")
    }
}

/// One framework a crate lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framework {
    /// The `.xcframework` or `.framework`.
    pub path: PathBuf,
    /// Links every member of a static library, not only those the code
    /// refers to: Objective-C categories and classes an SDK looks up at run
    /// time (what `-ObjC` and `-force_load` do in Xcode).
    pub whole_archive: bool,
    /// Who lists it, for errors.
    pub listed: String,
}

/// What a framework links for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Library {
    /// A static library (`lib<name>.a`).
    Archive(PathBuf),
    /// A framework bundle and its executable, static or dynamic.
    Framework {
        bundle: PathBuf,
        binary: PathBuf,
        dynamic: bool,
    },
}

impl Library {
    /// The file the linker reads.
    pub fn input(&self) -> &Path {
        match self {
            Library::Archive(path) => path,
            Library::Framework { binary, .. } => binary,
        }
    }

    /// A dynamic framework's bundle, which the app carries.
    pub fn dynamic_bundle(&self) -> Option<&Path> {
        match self {
            Library::Framework {
                bundle,
                dynamic: true,
                ..
            } => Some(bundle),
            _ => None,
        }
    }
}

impl Framework {
    /// What it links for `target`: an XCFramework's library for that
    /// target, or the framework as it is.
    pub fn library(&self, target: Target) -> anyhow::Result<Library> {
        if !self.path.is_dir() {
            bail!(
                "{}: {} is not a directory: list an `.xcframework` or a `.framework`",
                self.listed,
                self.path.display()
            );
        }
        let path = match self.path.extension().and_then(|e| e.to_str()) {
            Some("xcframework") => self.xcframework_library(target)?,
            Some("framework") => self.path.clone(),
            _ => bail!(
                "{}: {} is neither an XCFramework nor a framework",
                self.listed,
                self.path.display()
            ),
        };
        match path.extension().and_then(|e| e.to_str()) {
            Some("a") => Ok(Library::Archive(path)),
            Some("framework") => {
                let name = path.file_stem().context("a framework has a name")?;
                let binary = path.join(name);
                let dynamic = is_dynamic(&binary)
                    .with_context(|| format!("{}: {}", self.listed, self.path.display()))?;
                Ok(Library::Framework {
                    bundle: path,
                    binary,
                    dynamic,
                })
            }
            _ => bail!(
                "{}: {} is not a static library (`.a`) or a framework: apps carry dynamic \
                 libraries as frameworks",
                self.listed,
                path.display()
            ),
        }
    }

    /// The library of an XCFramework for `target`, by its `Info.plist`'s
    /// `AvailableLibraries`.
    fn xcframework_library(&self, target: Target) -> anyhow::Result<PathBuf> {
        let info_path = self.path.join("Info.plist");
        let info = plist::Value::from_file(&info_path)
            .with_context(|| format!("{}: cannot read {}", self.listed, info_path.display()))?;
        let libraries = info
            .as_dictionary()
            .and_then(|d| d.get("AvailableLibraries"))
            .and_then(plist::Value::as_array)
            .with_context(|| {
                format!(
                    "{}: {} lists no AvailableLibraries",
                    self.listed,
                    info_path.display()
                )
            })?;
        let text = |library: &plist::Dictionary, key: &str| {
            library
                .get(key)
                .and_then(plist::Value::as_string)
                .map(str::to_owned)
        };
        let mut available = Vec::new();
        for library in libraries.iter().filter_map(plist::Value::as_dictionary) {
            let (Some(identifier), Some(path)) = (
                text(library, "LibraryIdentifier"),
                text(library, "LibraryPath"),
            ) else {
                continue;
            };
            let archs: Vec<&str> = library
                .get("SupportedArchitectures")
                .and_then(plist::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(plist::Value::as_string)
                .collect();
            if text(library, "SupportedPlatform").as_deref() == Some(target.platform.key())
                && text(library, "SupportedPlatformVariant").as_deref() == target.variant
                && archs.contains(&target.arch)
            {
                return Ok(self.path.join(identifier).join(path));
            }
            available.push(identifier);
        }
        bail!(
            "{}: {} has no library for {} (it has {})",
            self.listed,
            self.path.display(),
            target.name(),
            if available.is_empty() {
                "none".to_owned()
            } else {
                available.join(", ")
            }
        )
    }

    /// The build script directives that link it for `target`.
    pub fn directives(&self, target: Target) -> anyhow::Result<Vec<String>> {
        let library = self.library(target)?;
        let dir = |path: &Path| -> anyhow::Result<String> {
            Ok(path
                .parent()
                .context("a library is in a directory")?
                .display()
                .to_string())
        };
        let mut lines = Vec::new();
        if let Some(plist) = self
            .path
            .extension()
            .is_some_and(|e| e == "xcframework")
            .then(|| self.path.join("Info.plist"))
        {
            lines.push(format!("cargo:rerun-if-changed={}", plist.display()));
        }
        lines.push(format!(
            "cargo:rerun-if-changed={}",
            library.input().display()
        ));
        // rustc copies a static library into the crate's rlib (`+bundle`,
        // its default), reading it itself: the linker never searches for
        // it by name, which `+verbatim` names it by.
        let kind = |verbatim: bool| {
            let modifiers: Vec<&str> = [
                self.whole_archive.then_some("+whole-archive"),
                verbatim.then_some("+verbatim"),
            ]
            .into_iter()
            .flatten()
            .collect();
            if modifiers.is_empty() {
                "static".to_owned()
            } else {
                format!("static:{}", modifiers.join(","))
            }
        };
        match &library {
            Library::Archive(archive) => {
                let file = archive
                    .file_name()
                    .context("a library has a name")?
                    .to_string_lossy();
                lines.push(format!("cargo:rustc-link-search=native={}", dir(archive)?));
                match file
                    .strip_prefix("lib")
                    .and_then(|name| name.strip_suffix(".a"))
                {
                    Some(name) => {
                        lines.push(format!("cargo:rustc-link-lib={}={name}", kind(false)))
                    }
                    None => lines.push(format!("cargo:rustc-link-lib={}={file}", kind(true))),
                }
            }
            Library::Framework {
                bundle,
                binary,
                dynamic,
            } => {
                let name = bundle
                    .file_stem()
                    .context("a framework has a name")?
                    .to_string_lossy();
                if *dynamic && self.whole_archive {
                    bail!(
                        "{}: wholeArchive links all of a static library, and {} is a dynamic framework",
                        self.listed,
                        bundle.display()
                    );
                }
                if self.whole_archive {
                    // A static framework's executable is an archive.
                    lines.push(format!("cargo:rustc-link-search=native={}", dir(binary)?));
                    lines.push(format!("cargo:rustc-link-lib={}={name}", kind(true)));
                } else {
                    lines.push(format!(
                        "cargo:rustc-link-search=framework={}",
                        dir(bundle)?
                    ));
                    lines.push(format!("cargo:rustc-link-lib=framework={name}"));
                }
            }
        }
        Ok(lines)
    }
}

/// Mach-O's file type of a dynamic library (`MH_DYLIB`).
const MH_DYLIB: u32 = 6;

/// Whether the framework executable `binary` is a dynamic library (a
/// Mach-O `MH_DYLIB`, or a universal file of them) rather than a static
/// archive (`!<arch>`, or a universal file of them).
pub fn is_dynamic(binary: &Path) -> anyhow::Result<bool> {
    let mut file = fs::File::open(binary)
        .with_context(|| format!("{} has no executable", binary.display()))?;
    let mut bytes = Vec::new();
    file.by_ref().take(4096).read_to_end(&mut bytes)?;
    let kind_at = |bytes: &[u8]| -> Option<bool> {
        if bytes.starts_with(b"!<arch>\n") {
            return Some(false);
        }
        let magic = u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?);
        if matches!(magic, 0xfeed_face | 0xfeed_facf) {
            let file_type = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?);
            return Some(file_type == MH_DYLIB);
        }
        None
    };
    let be = |at: usize| -> Option<u64> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?).into())
    };
    let found = match be(0) {
        // A universal file: its first slice tells, the slices being one
        // library built for several processors.
        Some(0xcafe_babe) => be(16).and_then(|offset| slice_kind(binary, offset, kind_at)),
        Some(0xcafe_babf) => be(24)
            .zip(be(28))
            .and_then(|(high, low)| slice_kind(binary, (high << 32) | low, kind_at)),
        _ => kind_at(&bytes),
    };
    found.with_context(|| {
        format!(
            "{} is neither a static library nor a dynamic one",
            binary.display()
        )
    })
}

/// [`is_dynamic`] of the slice at `offset` of a universal file.
fn slice_kind(binary: &Path, offset: u64, kind_at: impl Fn(&[u8]) -> Option<bool>) -> Option<bool> {
    use std::io::Seek as _;
    let mut file = fs::File::open(binary).ok()?;
    file.seek(std::io::SeekFrom::Start(offset)).ok()?;
    let mut head = [0u8; 16];
    let read = file.read(&mut head).ok()?;
    kind_at(&head[..read])
}

/// The frameworks the crate whose manifest is `manifest` lists for
/// `platform` in `[package.metadata.nsh-frameworks]`, relative to its
/// directory. `crate_name` names the crate in errors.
pub fn declared(
    manifest: &Path,
    platform: Platform,
    crate_name: &str,
) -> anyhow::Result<Vec<Framework>> {
    let text = fs::read_to_string(manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    let toml: toml::Table = text
        .parse()
        .with_context(|| format!("{} is not TOML", manifest.display()))?;
    let Some(table) = toml
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get(METADATA_KEY))
    else {
        return Ok(Vec::new());
    };
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let value = serde_value(table);
    frameworks_of(&value, dir, platform, crate_name)
}

/// [`declared`] of a crate's `[package.metadata.nsh-frameworks]` as a JSON
/// value (`cargo metadata`'s `metadata.nsh-frameworks`), its paths
/// relative to `dir`.
pub fn frameworks_of(
    table: &Value,
    dir: &Path,
    platform: Platform,
    crate_name: &str,
) -> anyhow::Result<Vec<Framework>> {
    let at = format!("{crate_name}: [package.metadata.{METADATA_KEY}]");
    let Value::Table(table) = table else {
        bail!("{at} is a table");
    };
    for (key, _) in table {
        if !Platform::ALL.iter().any(|p| p.key() == key) {
            bail!(
                "{at}: `{key}` is not something nsh-frameworks reads (`ios`, `macos`, `linux`, \
                 `windows`)"
            );
        }
    }
    let Some(list) = table
        .iter()
        .find(|(k, _)| k.as_str() == platform.key())
        .map(|(_, v)| v)
    else {
        return Ok(Vec::new());
    };
    let Value::List(entries) = list else {
        bail!("{at} `{}` is a list", platform.key());
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let listed = format!("{at} `{}`[{index}]", platform.key());
            let (path, whole_archive) = match entry {
                Value::Text(path) => (path.clone(), false),
                Value::Table(fields) => {
                    let mut path = None;
                    let mut whole_archive = false;
                    for (key, value) in fields {
                        match (key.as_str(), value) {
                            ("path", Value::Text(p)) => path = Some(p.clone()),
                            ("wholeArchive", Value::Flag(on)) => whole_archive = *on,
                            _ => bail!(
                                "{listed}: `{key}` is not something nsh-frameworks reads \
                                 (`path`, a string; `wholeArchive`, a boolean)"
                            ),
                        }
                    }
                    (
                        path.with_context(|| format!("{listed} has no `path`"))?,
                        whole_archive,
                    )
                }
                _ => bail!("{listed} is a path or {{ path, wholeArchive }}"),
            };
            Ok(Framework {
                path: dir.join(path),
                whole_archive,
                listed,
            })
        })
        .collect()
}

/// The part of TOML and JSON values a crate's table is read from.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Flag(bool),
    List(Vec<Value>),
    Table(Vec<(String, Value)>),
    Other,
}

/// A TOML value as a [`Value`].
fn serde_value(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::Text(s.clone()),
        toml::Value::Boolean(b) => Value::Flag(*b),
        toml::Value::Array(items) => Value::List(items.iter().map(serde_value).collect()),
        toml::Value::Table(fields) => Value::Table(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), serde_value(v)))
                .collect(),
        ),
        _ => Value::Other,
    }
}

/// The directives that link `frameworks` for `target`, each library once.
pub fn directives(frameworks: &[Framework], target: Target) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    let mut names: Vec<(String, &Framework)> = Vec::new();
    for framework in frameworks {
        let library = framework.library(target)?;
        let name = match &library {
            Library::Archive(path) => path.file_name(),
            Library::Framework { bundle, .. } => bundle.file_name(),
        }
        .context("a library has a name")?
        .to_string_lossy()
        .into_owned();
        if let Some((_, earlier)) = names.iter().find(|(n, _)| *n == name) {
            if earlier.path == framework.path {
                continue;
            }
            bail!(
                "{} ({}) and {} ({}) are both {name}",
                earlier.path.display(),
                earlier.listed,
                framework.path.display(),
                framework.listed
            );
        }
        names.push((name, framework));
        for line in framework.directives(target)? {
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    Ok(lines)
}

/// Links the frameworks the crate being built lists in its `Cargo.toml`
/// `[package.metadata.nsh-frameworks]` for the target Cargo builds: call it
/// from the crate's `build.rs`. Other targets link nothing.
pub fn link_declared() -> anyhow::Result<()> {
    let dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR")
            .context("not run by Cargo: no CARGO_MANIFEST_DIR")?,
    );
    let manifest = dir.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let triple = std::env::var("TARGET").context("not run by Cargo: no TARGET")?;
    let Some(platform) = Platform::of_triple(&triple) else {
        return Ok(());
    };
    let name = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "the crate".into());
    let frameworks = declared(&manifest, platform, &name)?;
    for line in directives_for(&frameworks, &triple)? {
        println!("{line}");
    }
    Ok(())
}

/// The directives that link `frameworks` (listed for the platform of
/// `triple`) for `triple`.
pub fn directives_for(frameworks: &[Framework], triple: &str) -> anyhow::Result<Vec<String>> {
    if let Some(target) = Target::of_triple(triple) {
        return directives(frameworks, target);
    }
    match desktop::DesktopTarget::of_triple(triple) {
        Some(target) => desktop::directives(frameworks, target),
        None => Ok(Vec::new()),
    }
}

/// Links the framework or library at `path` (relative to the crate) for
/// the target Cargo builds, if it is one this crate knows: [`link_declared`]
/// without the `Cargo.toml` list, for a build script that chooses itself.
/// `nsh-frameworks embed` finds only what a crate declares.
pub fn link(path: impl AsRef<Path>, whole_archive: bool) -> anyhow::Result<()> {
    let triple = std::env::var("TARGET").context("not run by Cargo: no TARGET")?;
    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let framework = Framework {
        path: dir.join(path.as_ref()),
        whole_archive,
        listed: format!("build.rs `{}`", path.as_ref().display()),
    };
    for line in directives_for(std::slice::from_ref(&framework), &triple)? {
        println!("{line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
