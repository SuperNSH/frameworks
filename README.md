# nsh-native-deps

The native dependencies of Native Shell (NSH) apps, from one list: Apple frameworks (`.xcframework`, `.framework`) on iOS and macOS, libraries (`.so`, `.a`, `.dll`, `.lib`) on Linux and Windows, and Maven libraries (`.aar`, `.jar`) on Android.

```toml
[package.metadata.nsh-native-deps]
ios = ["apple/Scanner.xcframework"]
macos = ["apple/Scanner.xcframework"]
linux = ["vendor/linux/{arch}/libscanner.so"]
windows = ["vendor/windows/{arch}/scanner.lib", "vendor/windows/{arch}/scanner.dll"]
android = ["com.google.mlkit:text-recognition:16.0.1"]
```

- **iOS, macOS, Linux, Windows**: NSH links what a crate's build script asks for, so an nshm that wraps an SDK links it from its own `build.rs` ([NSH's guide](https://github.com/longbridge/nsh/blob/main/website/docs/nshm.md)). `nsh-native-deps` does that from the list, picking each target's library. After `nsh package` it copies the SDK's dynamic libraries into the package (and signs them, on Apple's platforms), which a build script cannot do.
- **Android**: NSH packages the library files it is given and resolves no Maven dependencies. `nsh-native-deps android` resolves a library and everything it depends on as Gradle does, downloads the files, and lists them where NSH reads them.

NSH itself is unchanged, and `nsh package` and `nsh run` stay offline.

This crate replaces `nsh-frameworks` and `nsh-maven`: see [Moving](#moving).

## Status

Usable. Frameworks and libraries were checked, before the rename, on an Apple silicon Mac with NSH `main`, through an nshm that links a static Objective-C library (with a category, so `wholeArchive`) and a dynamic, versioned framework from XCFrameworks:

- `nsh package --target macos`, then `nsh-native-deps embed`: the app runs and calls both libraries, the category is linked, the framework sits in `Contents/Frameworks` with its links and without the download's quarantine, and `codesign --verify --deep --strict` passes. Without `embed` the app stops at launch (`Library not loaded`).
- `nsh package --target ios-simulator`, then `nsh-native-deps embed`: the app runs the same in the iOS simulator.
- `nsh run --target macos`, with `nsh-native-deps env`.
- `embed` keeps the app's entitlements, and running it twice changes nothing more.
- Static frameworks, archives not named `lib<name>.a`, and `wholeArchive` (a member nothing calls still runs its constructor) were linked in a plain Cargo workspace.

- Linux and Windows (CI, `e2e/run.sh`): a shared and a static C library linked into a Cargo app through `link_declared`, placed in a package directory laid out as `nsh package` writes; the app fails to start before `embed` and runs after it, the `.tar.gz` / `.zip` holds the library, and it runs where Cargo built it with `env`. Not yet with an `nsh package` of a real NSH app on Linux or Windows.

Android: the resolver was checked offline against Gradle 9.3.1 with the Android Gradle plugin 9.1.0, through a `file://` mirror of a local Gradle cache:

- Whole graphs: `androidx.core:core-ktx:1.19.0` (26 modules), `kotlinx-coroutines-android:1.10.2` (6) and `okhttp:4.12.1` (8) resolve to exactly Gradle's module set.
- Module by module: in a Firebase Firestore, Crashlytics, Analytics and Play services auth graph, 139 of 142 modules' dependencies and constraints match Gradle's tree. The 3 that differ are edges Gradle hides under exclusions, and one module Gradle could not fetch offline.
- A vendored closure (`lifecycle-process`, `coroutines-android`, `okhttp`: 19 files) was packaged with NSH and ran on an Android emulator.

Against the live Google Maven repository and Maven Central, `androidx.core:core-ktx:1.15.0`, `okhttp:4.12.0` and `com.google.mlkit:text-recognition:16.0.1` resolved and downloaded together (70 modules, 65 files, every checksum checked). That set was not packaged or compared with Gradle's.

Not yet checked: device packages signed with an identity, `.ipa` files, `nsh run --target ios-simulator` (`SIMCTL_CHILD_…`), Intel Macs, Mac Catalyst.

## Install

```sh
cargo install nsh-native-deps
```

The library, for a build script:

```sh
cargo add --build nsh-native-deps --no-default-features
```

## Use

In the nshm (or the app's own crate), list the frameworks, relative to the crate:

```toml
[package.metadata.nsh-native-deps]
ios = ["apple/Scanner.xcframework"]
macos = ["apple/Scanner.xcframework", { path = "apple/Ads.xcframework", wholeArchive = true }]

[build-dependencies]
nsh-native-deps = { version = "0.2", default-features = false }
```

and link them from `build.rs`:

```rust
fn main() {
    nsh_native_deps::link_declared().unwrap();
}
```

Call the SDK under `#[cfg(target_os = "ios")]` or `#[cfg(target_os = "macos")]`: other targets link nothing, so the crate still builds everywhere.

- `ios` is for iOS devices, the simulator and Mac Catalyst; `macos` for macOS. Each build takes the XCFramework's library for its target and architecture, and stops with an error naming the libraries there are when there is none.
- A static library (`lib<name>.a`, or a static framework) becomes part of the app's executable. `wholeArchive` links all of it, not only what the code calls: the Objective-C categories and classes an SDK looks up at run time, where Xcode's instructions say `-ObjC` or `-force_load`.
- A dynamic framework is linked against, and the app needs a copy of it at run time. After `nsh package`, in the app's directory:

  ```sh
  nsh-native-deps embed .nsh/out/Scanner.app      # macOS, iOS simulator
  nsh-native-deps embed .nsh/out/Scanner.ipa      # iOS devices
  ```

  It reads the frameworks the app's crates and nshm declare (through `cargo metadata` on `.nsh/rust/Cargo.toml`, NSH's workspace of them), copies the dynamic ones into `Frameworks/` (`Contents/Frameworks/` on macOS) without extended attributes, adds the executable's rpath to them, and signs them and the app again with the identity the app is signed with (`--sign` for another), keeping its entitlements. A universal macOS app needs a framework of both architectures (`macos-arm64_x86_64`).
- `nsh` also runs programs that link the app's crates where Cargo built them: the development host of `nsh run`, the headless runner of `nsh test`, and the program that describes the app's crates, which `nsh package` runs on this Mac whatever the target. They find a dynamic framework through `DYLD_FALLBACK_FRAMEWORK_PATH`, which dyld reads only for what it finds nowhere else:

  ```sh
  eval "$(nsh-native-deps env)"
  nsh package --target macos
  ```

  It reads the list from `.nsh/rust/Cargo.toml`, which `nsh` writes on its first run, or from the app's own `Cargo.toml`. `nsh-native-deps env --target aarch64-apple-ios-sim` prints the simulator's variable (`SIMCTL_CHILD_…`), which the simulator passes on to the app, for `nsh run --target ios-simulator`; run it besides the first, which the describing program on the Mac needs.
- `nsh-native-deps list` shows what each framework links for this machine (`--target` for another).

### Linux and Windows

```toml
[package.metadata.nsh-native-deps]
linux = ["vendor/linux/{arch}/libscanner.so"]
windows = ["vendor/windows/{arch}/scanner.lib", "vendor/windows/{arch}/scanner.dll"]
```

`{arch}` is the Rust target's processor (`x86_64`, `aarch64`); the same `build.rs` links them.

- Linux: a `.a` is part of the executable (`wholeArchive` as above); a `.so` (`.so.<version>` too) is linked against and carried.
- Windows: a `.lib` is linked; with its `.dll` in the list it is that DLL's import library, else a static library. A `.dll` is carried.
- After `nsh package`: `nsh-native-deps embed .nsh/out/<Name>` copies the carried libraries next to the executable (a Linux one under its soname), gives a Linux executable the rpath `$ORIGIN` (with `patchelf`, which must be installed), and makes the `.tar.gz` or `.zip` again. A Windows installer is compiled again from `<Name>.iss` when Inno Setup is installed; else run `ISCC.exe` on it.
- `nsh run` / `nsh test`: `eval "$(nsh-native-deps env)"` sets `LD_LIBRARY_PATH` on Linux; on Windows, in PowerShell, `nsh-native-deps env | Invoke-Expression` puts the DLLs' directories on `PATH`.

`nsh_native_deps::link(path, whole_archive)` links one framework from a build script that chooses itself; `embed` finds only those the `Cargo.toml` table lists.

### Android

In an app, at or below the directory that holds `nsh.json`:

```sh
nsh-native-deps android add com.google.mlkit:text-recognition:16.0.1   # add a library, or change its version
nsh-native-deps android remove com.google.mlkit:text-recognition       # remove one, with what only it needed
nsh-native-deps android sync                                           # resolve and download again
nsh-native-deps android list                                           # what was added, and the files
```

In an nshm crate, add `--crate`: the files go to the crate's `android/maven/` and its `Cargo.toml` `[package.metadata.nsh.android] libraries`. NSH then adds them to every app that links the crate. The rest of `Cargo.toml` stays as written.

`--repository <URL>` (repeatable, `https://` or `file://`) searches other repositories, in order, instead of Google's Maven repository and Maven Central. Later runs keep them. `--dir <PATH>` picks the app or crate.

A crate's libraries are also listed in `[package.metadata.nsh-native-deps] android`: `sync` takes the list as it is there, so it can be edited by hand. An app's are kept in `android/maven/maven.json`.

Every run resolves the whole set again from the libraries named, and replaces exactly the files and entries the previous run added. Entries of your own stay first. `android/maven/maven.json` records the libraries, the repositories and each file's SHA-256: commit it with the files, or commit it alone and run `nsh-native-deps android sync` to download them.

## How Android libraries resolve

- **Gradle module metadata** (`.module`), where the POM says it was published with it:
  - the variant an Android app runs: `java-runtime`, Android before plain JVM, release before debug, and an attribute the variant leaves out counts as a match;
  - `available-at`, so Kotlin Multiplatform roots lead to their `-jvm` or `-android` module;
  - dependencies, and a dependency with no version takes its constraint's;
  - constraints, and platform dependencies whose BOM constrains the graph;
  - each file's SHA-256.
- **Otherwise the POM**:
  - parents and properties;
  - `dependencyManagement` with imported BOMs, which also become constraints when the module is asked for as a platform;
  - `compile` and `runtime` dependencies that are not optional;
  - exclusions: a module stays when any path to it does not exclude it;
  - exact (`[24.0.0]`) and listed (`maven-metadata.xml`) version ranges.
- **One version per module**, the highest the graph asks for; a constraint raises a module only if the graph has it. The graph is walked again until the versions settle, so what an evicted version asked for does not count. An evicted version whose metadata a repository no longer has does not matter. An error names the library that pulled the failing one in.
- **Downloads** are checked against the module metadata's SHA-256, or the repository's `.sha256`/`.sha1`. A file with none to check against is reported. Files go through a staging directory, so a failed run changes nothing.

## Limits

- System frameworks need none of this: link them as usual (`objc2` crates, or `cargo:rustc-link-lib=framework=StoreKit`).
- No Swift runtime search paths: a static library with Swift code needs the crate's build script to add them.
- A static library's privacy manifest is not merged into the app's: copy what it declares into `nsh.json` `ios.privacyManifest`.
- Run `embed` again after each `nsh package`, which writes a new package. An app that updates itself (`updater`) publishes the archive `nsh package` copied for the update feed: copy the one `embed` made over it.
- List a static library in one crate only: each crate that lists it carries a copy.

Android:

- No Gradle plugins: put the values `google-services.json` makes in the app's `android/res/values`.
- No dependency substitution rules or capability conflicts, which a build script would settle (guava against `listenablefuture`).
- No POM relocations, and no `rejects` or `strictly` beyond taking the version.
- `kotlin-stdlib-jdk7` and `-jdk8` that older Kotlin libraries ask for are kept, where the Kotlin Gradle plugin may drop them. Since Kotlin 1.8 they hold no classes an app runs.

## Moving

From `nsh-frameworks`:

- depend on `nsh-native-deps`, and call `nsh_native_deps::link_declared()` in `build.rs`;
- rename `[package.metadata.nsh-frameworks]` to `[package.metadata.nsh-native-deps]`;
- run `nsh-native-deps` in place of `nsh-frameworks`. The commands are the same.

From `nsh-maven`:

- run `nsh-native-deps android add`, `remove`, `sync` and `list` in place of `nsh-maven add` and so on;
- the library is `nsh_native_deps::maven`. `android/maven/` and `maven.json` stay as they are.

## Layout

- `crates/nsh-native-deps/src/lib.rs`: targets, XCFramework libraries, the build script directives, the `Cargo.toml` table.
- `crates/nsh-native-deps/src/desktop.rs`: Linux and Windows libraries.
- `crates/nsh-native-deps/src/maven.rs`: the Android resolver, the vendoring and their tests.
- `crates/nsh-native-deps/src/embed.rs`, `android.rs`, `main.rs`: the `nsh-native-deps` command.
- `crates/nsh-native-deps/src/tests.rs`: the library's tests.

## Checks

CI (`.github/workflows/ci.yml`) runs `cargo fmt --all -- --check`, then Clippy (`-D warnings`) and the tests on Linux, macOS and Windows, and `e2e/run.sh` on Linux and Windows. The unit tests build XCFrameworks in temporary directories; none runs Xcode's tools. The Android tests resolve against repositories in memory; none reaches the network.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/native-deps
