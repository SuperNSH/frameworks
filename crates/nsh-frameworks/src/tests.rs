use super::*;

/// A temporary directory of the test's own.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nsh-frameworks-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A thin 64-bit Mach-O header of `file_type`.
fn mach_o(file_type: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(0xfeed_facf_u32.to_le_bytes());
    bytes.extend(0x0100_000c_u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(file_type.to_le_bytes());
    bytes.extend([0u8; 16]);
    bytes
}

/// A universal file of one slice at offset 4096.
fn universal(slice: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(0xcafe_babe_u32.to_be_bytes());
    bytes.extend(1u32.to_be_bytes());
    bytes.extend(0x0100_000c_u32.to_be_bytes());
    bytes.extend(0u32.to_be_bytes());
    bytes.extend(4096u32.to_be_bytes());
    bytes.extend((slice.len() as u32).to_be_bytes());
    bytes.extend(14u32.to_be_bytes());
    bytes.resize(4096, 0);
    bytes.extend(slice);
    bytes
}

fn archive() -> Vec<u8> {
    b"!<arch>\n".to_vec()
}

/// `<dir>/<name>.xcframework` with a library for each of `libraries`:
/// (identifier, file name, platform, variant, architectures, binary).
#[allow(clippy::type_complexity)]
fn xcframework(
    dir: &Path,
    name: &str,
    libraries: &[(&str, &str, &str, Option<&str>, &[&str], Vec<u8>)],
) -> PathBuf {
    let root = dir.join(format!("{name}.xcframework"));
    let mut available = Vec::new();
    for (identifier, file, platform, variant, archs, binary) in libraries {
        let slice = root.join(identifier);
        fs::create_dir_all(&slice).unwrap();
        if let Some(stem) = file.strip_suffix(".framework") {
            fs::create_dir_all(slice.join(file)).unwrap();
            fs::write(slice.join(file).join(stem), binary).unwrap();
        } else {
            fs::write(slice.join(file), binary).unwrap();
        }
        let mut library = plist::Dictionary::new();
        library.insert("LibraryIdentifier".into(), (*identifier).into());
        library.insert("LibraryPath".into(), (*file).into());
        library.insert("SupportedPlatform".into(), (*platform).into());
        library.insert(
            "SupportedArchitectures".into(),
            plist::Value::Array(archs.iter().map(|a| (*a).into()).collect()),
        );
        if let Some(variant) = variant {
            library.insert("SupportedPlatformVariant".into(), (*variant).into());
        }
        available.push(plist::Value::Dictionary(library));
    }
    let mut info = plist::Dictionary::new();
    info.insert("AvailableLibraries".into(), plist::Value::Array(available));
    plist::Value::Dictionary(info)
        .to_file_xml(root.join("Info.plist"))
        .unwrap();
    root
}

fn listed(path: PathBuf) -> Framework {
    Framework {
        path,
        whole_archive: false,
        listed: "scanner: [package.metadata.nsh-frameworks] `ios`[0]".into(),
    }
}

fn target(triple: &str) -> Target {
    Target::of_triple(triple).unwrap()
}

#[test]
fn rust_triples_are_xcframework_targets() {
    let t = |triple: &str| Target::of_triple(triple).map(|t| (t.platform, t.variant, t.arch));
    assert_eq!(t("aarch64-apple-ios"), Some((Platform::Ios, None, "arm64")));
    assert_eq!(
        t("aarch64-apple-ios-sim"),
        Some((Platform::Ios, Some("simulator"), "arm64"))
    );
    assert_eq!(
        t("x86_64-apple-ios"),
        Some((Platform::Ios, Some("simulator"), "x86_64"))
    );
    assert_eq!(
        t("aarch64-apple-ios-macabi"),
        Some((Platform::Ios, Some("maccatalyst"), "arm64"))
    );
    assert_eq!(
        t("x86_64-apple-darwin"),
        Some((Platform::Macos, None, "x86_64"))
    );
    assert_eq!(t("aarch64-linux-android"), None);
    assert_eq!(t("x86_64-pc-windows-msvc"), None);
    assert_eq!(t("aarch64-apple-tvos"), None);
    assert!(target("aarch64-apple-darwin").is_mac());
    assert!(target("aarch64-apple-ios-macabi").is_mac());
    assert!(!target("aarch64-apple-ios-sim").is_mac());
}

#[test]
fn executables_are_static_or_dynamic() {
    let dir = temp("kinds");
    let check = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        is_dynamic(&path)
    };
    assert!(check("dylib", &mach_o(MH_DYLIB)).unwrap());
    assert!(!check("object", &mach_o(1)).unwrap());
    assert!(!check("archive", &archive()).unwrap());
    assert!(check("fat-dylib", &universal(&mach_o(MH_DYLIB))).unwrap());
    assert!(!check("fat-archive", &universal(&archive())).unwrap());
    assert!(check("text", b"not code").is_err());
    assert!(is_dynamic(&dir.join("missing")).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn each_target_takes_its_library() {
    let dir = temp("slices");
    let sdk = listed(xcframework(
        &dir,
        "Sdk",
        &[
            ("ios-arm64", "libSdk.a", "ios", None, &["arm64"], archive()),
            (
                "ios-arm64_x86_64-simulator",
                "libSdk.a",
                "ios",
                Some("simulator"),
                &["arm64", "x86_64"],
                archive(),
            ),
            (
                "macos-arm64_x86_64",
                "Sdk.framework",
                "macos",
                None,
                &["arm64", "x86_64"],
                universal(&mach_o(MH_DYLIB)),
            ),
        ],
    ));
    assert_eq!(
        sdk.library(target("aarch64-apple-ios")).unwrap(),
        Library::Archive(sdk.path.join("ios-arm64/libSdk.a"))
    );
    for simulator in ["aarch64-apple-ios-sim", "x86_64-apple-ios"] {
        assert_eq!(
            sdk.library(target(simulator)).unwrap(),
            Library::Archive(sdk.path.join("ios-arm64_x86_64-simulator/libSdk.a"))
        );
    }
    let mac = sdk.path.join("macos-arm64_x86_64/Sdk.framework");
    for triple in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
        assert_eq!(
            sdk.library(target(triple)).unwrap(),
            Library::Framework {
                binary: mac.join("Sdk"),
                bundle: mac.clone(),
                dynamic: true
            }
        );
    }
    let error = sdk
        .library(target("aarch64-apple-ios-macabi"))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("no library for Mac Catalyst (arm64) (it has ios-arm64, ios-arm64_x86_64-simulator, macos-arm64_x86_64)"),
        "{error}"
    );
    for path in [
        dir.join("Missing.xcframework"),
        dir.join("Sdk.xcframework/ios-arm64/libSdk.a"),
    ] {
        let error = listed(path)
            .library(target("aarch64-apple-ios"))
            .unwrap_err();
        assert!(error.to_string().starts_with("scanner: "), "{error}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn build_scripts_link_each_kind_of_library() {
    let dir = temp("directives");
    let ios = target("aarch64-apple-ios");
    let ads = xcframework(
        &dir,
        "Ads",
        &[("ios-arm64", "libAds.a", "ios", None, &["arm64"], archive())],
    );
    let slice = ads.join("ios-arm64");
    let lines = listed(ads.clone()).directives(ios).unwrap();
    assert_eq!(
        lines,
        [
            format!(
                "cargo:rerun-if-changed={}",
                ads.join("Info.plist").display()
            ),
            format!(
                "cargo:rerun-if-changed={}",
                slice.join("libAds.a").display()
            ),
            format!("cargo:rustc-link-search=native={}", slice.display()),
            "cargo:rustc-link-lib=static=Ads".to_owned(),
        ]
    );
    let whole = Framework {
        whole_archive: true,
        ..listed(ads.clone())
    };
    assert_eq!(
        whole.directives(ios).unwrap()[3],
        "cargo:rustc-link-lib=static:+whole-archive=Ads"
    );

    // An archive without the `lib` prefix is named as it is.
    let odd = xcframework(
        &dir,
        "Odd",
        &[("ios-arm64", "Odd.a", "ios", None, &["arm64"], archive())],
    );
    assert_eq!(
        listed(odd).directives(ios).unwrap()[3],
        "cargo:rustc-link-lib=static:+verbatim=Odd.a"
    );

    // Frameworks, static or dynamic, are linked as frameworks; a static
    // one's whole archive by its executable.
    let ui = xcframework(
        &dir,
        "Ui",
        &[(
            "ios-arm64",
            "Ui.framework",
            "ios",
            None,
            &["arm64"],
            mach_o(MH_DYLIB),
        )],
    );
    let lines = listed(ui.clone()).directives(ios).unwrap();
    assert_eq!(
        lines[2..],
        [
            format!(
                "cargo:rustc-link-search=framework={}",
                ui.join("ios-arm64").display()
            ),
            "cargo:rustc-link-lib=framework=Ui".to_owned(),
        ]
    );
    let forced_dynamic = Framework {
        whole_archive: true,
        ..listed(ui)
    };
    let error = forced_dynamic.directives(ios).unwrap_err().to_string();
    assert!(error.contains("is a dynamic framework"), "{error}");
    let analytics = xcframework(
        &dir,
        "Analytics",
        &[(
            "ios-arm64",
            "Analytics.framework",
            "ios",
            None,
            &["arm64"],
            archive(),
        )],
    );
    let lines = Framework {
        whole_archive: true,
        ..listed(analytics.clone())
    }
    .directives(ios)
    .unwrap();
    assert_eq!(
        lines[2..],
        [
            format!(
                "cargo:rustc-link-search=native={}",
                analytics
                    .join("ios-arm64")
                    .join("Analytics.framework")
                    .display()
            ),
            "cargo:rustc-link-lib=static:+whole-archive,+verbatim=Analytics".to_owned(),
        ]
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_library_two_crates_list_is_linked_once() {
    let dir = temp("once");
    let ios = target("aarch64-apple-ios");
    let ads = xcframework(
        &dir,
        "Ads",
        &[("ios-arm64", "libAds.a", "ios", None, &["arm64"], archive())],
    );
    let lines = directives(&[listed(ads.clone()), listed(ads.clone())], ios).unwrap();
    assert_eq!(lines.len(), 4);
    // Two libraries of one name cannot both be linked.
    let other = xcframework(
        &dir.join("other"),
        "Ads",
        &[("ios-arm64", "libAds.a", "ios", None, &["arm64"], archive())],
    );
    let error = directives(&[listed(ads), listed(other)], ios)
        .unwrap_err()
        .to_string();
    assert!(error.contains("are both libAds.a"), "{error}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn crates_declare_frameworks_in_their_cargo_toml() {
    let dir = temp("declared");
    let manifest = dir.join("Cargo.toml");
    fs::write(
        &manifest,
        r#"
[package]
name = "scanner"

[package.metadata.nsh-frameworks]
ios = ["apple/Scanner.xcframework"]
macos = ["apple/Scanner.xcframework", { path = "apple/Ads.xcframework", wholeArchive = true }]
"#,
    )
    .unwrap();
    let macos = declared(&manifest, Platform::Macos, "scanner").unwrap();
    assert_eq!(
        macos,
        [
            Framework {
                path: dir.join("apple/Scanner.xcframework"),
                whole_archive: false,
                listed: "scanner: [package.metadata.nsh-frameworks] `macos`[0]".into(),
            },
            Framework {
                path: dir.join("apple/Ads.xcframework"),
                whole_archive: true,
                listed: "scanner: [package.metadata.nsh-frameworks] `macos`[1]".into(),
            },
        ]
    );
    assert_eq!(
        declared(&manifest, Platform::Ios, "scanner").unwrap().len(),
        1
    );

    for (table, wrong) in [
        ("ios = \"a.xcframework\"", "`ios` is a list"),
        ("ios = [3]", "`ios`[0] is a path or { path, wholeArchive }"),
        ("ios = [{ wholeArchive = true }]", "`ios`[0] has no `path`"),
        (
            "ios = [{ path = \"a\", embed = true }]",
            "`embed` is not something",
        ),
        ("tvos = []", "`tvos` is not something"),
    ] {
        fs::write(
            &manifest,
            format!("[package]\nname = \"x\"\n[package.metadata.nsh-frameworks]\n{table}\n"),
        )
        .unwrap();
        let error = format!("{:#}", declared(&manifest, Platform::Ios, "x").unwrap_err());
        assert!(error.contains(wrong), "{table}: {error}");
    }
    // A crate without the table declares none.
    fs::write(&manifest, "[package]\nname = \"x\"\n").unwrap();
    assert!(declared(&manifest, Platform::Ios, "x").unwrap().is_empty());
    fs::remove_dir_all(&dir).unwrap();
}
