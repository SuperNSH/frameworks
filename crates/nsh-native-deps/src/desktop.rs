//! Prebuilt libraries for Linux and Windows apps: what a crate lists under
//! `linux` and `windows`, linked from its build script, and the shared
//! libraries the app carries next to its executable.
//!
//! ```toml
//! [package.metadata.nsh-native-deps]
//! linux = ["vendor/linux/{arch}/libscanner.so"]
//! windows = ["vendor/windows/{arch}/scanner.lib", "vendor/windows/{arch}/scanner.dll"]
//! ```
//!
//! `{arch}` is the Rust target's processor (`x86_64`, `aarch64`).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

use crate::Framework;

/// A Linux or Windows target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopTarget {
    pub os: Os,
    /// The Rust target's processor: `x86_64`, `aarch64`.
    pub arch: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
}

impl DesktopTarget {
    /// The target of a Rust `triple`; `None` for one that is not Linux or
    /// Windows on `x86_64` or `aarch64`.
    pub fn of_triple(triple: &str) -> Option<DesktopTarget> {
        let arch = match triple.split('-').next()? {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            _ => return None,
        };
        // Android's triples say `linux` too, and are not Linux apps.
        let os = if triple.contains("-linux-") && !triple.contains("android") {
            Os::Linux
        } else if triple.contains("-windows-") {
            Os::Windows
        } else {
            return None;
        };
        Some(DesktopTarget { os, arch })
    }

    /// The Rust triple NSH builds for it.
    pub fn triple(self) -> String {
        match self.os {
            Os::Linux => format!("{}-unknown-linux-gnu", self.arch),
            Os::Windows => format!("{}-pc-windows-msvc", self.arch),
        }
    }
}

/// What one listed library is for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopLibrary {
    /// A static library (`.a`; on Windows a `.lib` with no `.dll` beside it
    /// in the list): part of the executable.
    Static(PathBuf),
    /// A Linux shared library (`.so`, `.so.<version>`): linked against, and
    /// carried next to the executable.
    Shared(PathBuf),
    /// A Windows import library (`.lib` with its `.dll` in the list):
    /// linked against.
    Import(PathBuf),
    /// A Windows `.dll`: carried next to the executable.
    Runtime(PathBuf),
}

impl DesktopLibrary {
    pub fn path(&self) -> &Path {
        match self {
            DesktopLibrary::Static(p)
            | DesktopLibrary::Shared(p)
            | DesktopLibrary::Import(p)
            | DesktopLibrary::Runtime(p) => p,
        }
    }

    /// Whether the app carries it next to its executable.
    pub fn carried(&self) -> bool {
        matches!(self, DesktopLibrary::Shared(_) | DesktopLibrary::Runtime(_))
    }
}

/// `framework`'s path for `target`, `{arch}` replaced.
pub fn resolve(framework: &Framework, target: DesktopTarget) -> PathBuf {
    PathBuf::from(
        framework
            .path
            .to_string_lossy()
            .replace("{arch}", target.arch),
    )
}

/// What `frameworks` (one crate's list or the app's) are for `target`, in
/// their order.
pub fn libraries(
    frameworks: &[Framework],
    target: DesktopTarget,
) -> anyhow::Result<Vec<(DesktopLibrary, &Framework)>> {
    let paths: Vec<PathBuf> = frameworks.iter().map(|f| resolve(f, target)).collect();
    let stem = |p: &Path| {
        p.file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    let mut found = Vec::new();
    for (framework, path) in frameworks.iter().zip(&paths) {
        if !path.is_file() {
            bail!("{}: {} is not a file", framework.listed, path.display());
        }
        let name = path
            .file_name()
            .context("a library has a name")?
            .to_string_lossy()
            .into_owned();
        let extension = |e: &str| name.to_lowercase().ends_with(e);
        let library = match target.os {
            Os::Linux if extension(".a") => DesktopLibrary::Static(path.clone()),
            Os::Linux if name.ends_with(".so") || name.contains(".so.") => {
                DesktopLibrary::Shared(path.clone())
            }
            Os::Windows if extension(".dll") => DesktopLibrary::Runtime(path.clone()),
            Os::Windows if extension(".lib") => {
                let with_dll = paths.iter().any(|other| {
                    other
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                        && stem(other) == stem(path)
                });
                if with_dll {
                    DesktopLibrary::Import(path.clone())
                } else {
                    DesktopLibrary::Static(path.clone())
                }
            }
            Os::Linux => bail!(
                "{}: {} is not a static (`.a`) or shared (`.so`) library",
                framework.listed,
                path.display()
            ),
            Os::Windows => bail!(
                "{}: {} is not a library (`.lib`) or a DLL (`.dll`)",
                framework.listed,
                path.display()
            ),
        };
        if framework.whole_archive && !matches!(library, DesktopLibrary::Static(_)) {
            bail!(
                "{}: wholeArchive links all of a static library, and {} is not one",
                framework.listed,
                path.display()
            );
        }
        found.push((library, framework));
    }
    Ok(found)
}

/// The build script directives that link `frameworks` for `target`.
pub fn directives(frameworks: &[Framework], target: DesktopTarget) -> anyhow::Result<Vec<String>> {
    let mut lines = Vec::new();
    for (library, framework) in libraries(frameworks, target)? {
        let path = library.path();
        lines.push(format!("cargo:rerun-if-changed={}", path.display()));
        if let DesktopLibrary::Runtime(_) = library {
            continue;
        }
        let dir = path.parent().context("a library is in a directory")?;
        let file = path
            .file_name()
            .context("a library has a name")?
            .to_string_lossy()
            .into_owned();
        // The name `-l` takes, when the file is named as the linker
        // looks for it.
        let plain = match (&library, target.os) {
            (DesktopLibrary::Shared(_), _) => {
                file.strip_prefix("lib").and_then(|n| n.strip_suffix(".so"))
            }
            (_, Os::Linux) => file.strip_prefix("lib").and_then(|n| n.strip_suffix(".a")),
            (_, Os::Windows) => file
                .len()
                .checked_sub(4)
                .filter(|&at| file[at..].eq_ignore_ascii_case(".lib"))
                .map(|at| &file[..at]),
        };
        let kind = match library {
            DesktopLibrary::Static(_) if framework.whole_archive => "static:+whole-archive",
            DesktopLibrary::Static(_) => "static",
            _ => "dylib",
        };
        lines.push(format!("cargo:rustc-link-search=native={}", dir.display()));
        lines.push(match plain {
            Some(name) => format!("cargo:rustc-link-lib={kind}={name}"),
            None if kind.contains(':') => format!("cargo:rustc-link-lib={kind},+verbatim={file}"),
            None => format!("cargo:rustc-link-lib={kind}:+verbatim={file}"),
        });
    }
    let mut unique = Vec::new();
    for line in lines {
        if !unique.contains(&line) {
            unique.push(line);
        }
    }
    Ok(unique)
}

/// The libraries an app of `target` carries next to its executable.
pub fn carried(frameworks: &[Framework], target: DesktopTarget) -> anyhow::Result<Vec<PathBuf>> {
    let mut found: Vec<PathBuf> = Vec::new();
    for (library, framework) in libraries(frameworks, target)? {
        if !library.carried() {
            continue;
        }
        let path = library.path().to_owned();
        if let Some(other) = found
            .iter()
            .find(|p| p.file_name() == path.file_name() && **p != path)
        {
            bail!(
                "{}: {} and {} have one name, and the app carries one",
                framework.listed,
                other.display(),
                path.display()
            );
        }
        if !found.contains(&path) {
            found.push(path);
        }
    }
    Ok(found)
}

/// The processor of the Linux or Windows executable `bytes` begin (ELF or
/// PE), as a [`DesktopTarget`].
pub fn executable_target(bytes: &[u8]) -> Option<DesktopTarget> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    if bytes.starts_with(b"\x7fELF") {
        let arch = match u16_at(18)? {
            62 => "x86_64",
            183 => "aarch64",
            _ => return None,
        };
        return Some(DesktopTarget {
            os: Os::Linux,
            arch,
        });
    }
    if bytes.starts_with(b"MZ") {
        let pe = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let arch = match u16_at(pe + 4)? {
            0x8664 => "x86_64",
            0xaa64 => "aarch64",
            _ => return None,
        };
        return Some(DesktopTarget {
            os: Os::Windows,
            arch,
        });
    }
    None
}
