# nsh-frameworks

> **Moved.** This crate is now [`nsh-native-deps`](https://crates.io/crates/nsh-native-deps), which also covers Android. It gets no more releases here. To switch: depend on `nsh-native-deps`, call `nsh_native_deps::link_declared()`, rename the table to `[package.metadata.nsh-native-deps]`, and run `nsh-native-deps` in place of `nsh-frameworks`. Source: https://github.com/SuperNSH/native-deps

Prebuilt native SDKs for Native Shell (NSH) apps, linked from an nshm crate and carried in the app's packages: Apple frameworks (`.xcframework`, `.framework`) on iOS and macOS, libraries (`.so`, `.a`, `.dll`, `.lib`) on Linux and Windows.

NSH links what a crate's build script asks for, so an nshm that wraps an SDK links it from its own `build.rs` ([NSH's guide](https://github.com/longbridge/nsh/blob/main/website/docs/nshm.md)). `nsh-frameworks` does that from one list in the crate's `Cargo.toml`, picking each target's library. After `nsh package` it copies the SDK's dynamic libraries into the package (and signs them, on Apple's platforms), which a build script cannot do. NSH itself is unchanged.

## Status

Usable. Checked on an Apple silicon Mac with NSH `main`, through an nshm that links a static Objective-C library (with a category, so `wholeArchive`) and a dynamic, versioned framework from XCFrameworks:

- `nsh package --target macos`, then `nsh-frameworks embed`: the app runs and calls both libraries, the category is linked, the framework sits in `Contents/Frameworks` with its links and without the download's quarantine, and `codesign --verify --deep --strict` passes. Without `embed` the app stops at launch (`Library not loaded`).
- `nsh package --target ios-simulator`, then `nsh-frameworks embed`: the app runs the same in the iOS simulator.
- `nsh run --target macos`, with `nsh-frameworks env`.
- `embed` keeps the app's entitlements, and running it twice changes nothing more.
- Static frameworks, archives not named `lib<name>.a`, and `wholeArchive` (a member nothing calls still runs its constructor) were linked in a plain Cargo workspace.

- Linux and Windows (CI, `e2e/run.sh`): a shared and a static C library linked into a Cargo app through `link_declared`, placed in a package directory laid out as `nsh package` writes; the app fails to start before `embed` and runs after it, the `.tar.gz` / `.zip` holds the library, and it runs where Cargo built it with `env`. Not yet with an `nsh package` of a real NSH app on Linux or Windows.

Not yet checked: device packages signed with an identity, `.ipa` files, `nsh run --target ios-simulator` (`SIMCTL_CHILD_…`), Intel Macs, Mac Catalyst.

## Install

```sh
cargo install nsh-frameworks
```

The library, for a build script:

```sh
cargo add --build nsh-frameworks --no-default-features
```

## Use

In the nshm (or the app's own crate), list the frameworks, relative to the crate:

```toml
[package.metadata.nsh-frameworks]
ios = ["apple/Scanner.xcframework"]
macos = ["apple/Scanner.xcframework", { path = "apple/Ads.xcframework", wholeArchive = true }]

[build-dependencies]
nsh-frameworks = { version = "0.1", default-features = false }
```

and link them from `build.rs`:

```rust
fn main() {
    nsh_frameworks::link_declared().unwrap();
}
```

Call the SDK under `#[cfg(target_os = "ios")]` or `#[cfg(target_os = "macos")]`: other targets link nothing, so the crate still builds everywhere.

- `ios` is for iOS devices, the simulator and Mac Catalyst; `macos` for macOS. Each build takes the XCFramework's library for its target and architecture, and stops with an error naming the libraries there are when there is none.
- A static library (`lib<name>.a`, or a static framework) becomes part of the app's executable. `wholeArchive` links all of it, not only what the code calls: the Objective-C categories and classes an SDK looks up at run time, where Xcode's instructions say `-ObjC` or `-force_load`.
- A dynamic framework is linked against, and the app needs a copy of it at run time. After `nsh package`, in the app's directory:

  ```sh
  nsh-frameworks embed .nsh/out/Scanner.app      # macOS, iOS simulator
  nsh-frameworks embed .nsh/out/Scanner.ipa      # iOS devices
  ```

  It reads the frameworks the app's crates and nshm declare (through `cargo metadata` on `.nsh/rust/Cargo.toml`, NSH's workspace of them), copies the dynamic ones into `Frameworks/` (`Contents/Frameworks/` on macOS) without extended attributes, adds the executable's rpath to them, and signs them and the app again with the identity the app is signed with (`--sign` for another), keeping its entitlements. A universal macOS app needs a framework of both architectures (`macos-arm64_x86_64`).
- `nsh` also runs programs that link the app's crates where Cargo built them: the development host of `nsh run`, the headless runner of `nsh test`, and the program that describes the app's crates, which `nsh package` runs on this Mac whatever the target. They find a dynamic framework through `DYLD_FALLBACK_FRAMEWORK_PATH`, which dyld reads only for what it finds nowhere else:

  ```sh
  eval "$(nsh-frameworks env)"
  nsh package --target macos
  ```

  It reads the list from `.nsh/rust/Cargo.toml`, which `nsh` writes on its first run, or from the app's own `Cargo.toml`. `nsh-frameworks env --target aarch64-apple-ios-sim` prints the simulator's variable (`SIMCTL_CHILD_…`), which the simulator passes on to the app, for `nsh run --target ios-simulator`; run it besides the first, which the describing program on the Mac needs.
- `nsh-frameworks list` shows what each framework links for this machine (`--target` for another).

### Linux and Windows

```toml
[package.metadata.nsh-frameworks]
linux = ["vendor/linux/{arch}/libscanner.so"]
windows = ["vendor/windows/{arch}/scanner.lib", "vendor/windows/{arch}/scanner.dll"]
```

`{arch}` is the Rust target's processor (`x86_64`, `aarch64`); the same `build.rs` links them.

- Linux: a `.a` is part of the executable (`wholeArchive` as above); a `.so` (`.so.<version>` too) is linked against and carried.
- Windows: a `.lib` is linked; with its `.dll` in the list it is that DLL's import library, else a static library. A `.dll` is carried.
- After `nsh package`: `nsh-frameworks embed .nsh/out/<Name>` copies the carried libraries next to the executable (a Linux one under its soname), gives a Linux executable the rpath `$ORIGIN` (with `patchelf`, which must be installed), and makes the `.tar.gz` or `.zip` again. A Windows installer is compiled again from `<Name>.iss` when Inno Setup is installed; else run `ISCC.exe` on it.
- `nsh run` / `nsh test`: `eval "$(nsh-frameworks env)"` sets `LD_LIBRARY_PATH` on Linux; on Windows, in PowerShell, `nsh-frameworks env | Invoke-Expression` puts the DLLs' directories on `PATH`.

`nsh_frameworks::link(path, whole_archive)` links one framework from a build script that chooses itself; `embed` finds only those the `Cargo.toml` table lists.

## Limits

- System frameworks need none of this: link them as usual (`objc2` crates, or `cargo:rustc-link-lib=framework=StoreKit`).
- No Swift runtime search paths: a static library with Swift code needs the crate's build script to add them.
- A static library's privacy manifest is not merged into the app's: copy what it declares into `nsh.json` `ios.privacyManifest`.
- Run `embed` again after each `nsh package`, which writes a new package. An app that updates itself (`updater`) publishes the archive `nsh package` copied for the update feed: copy the one `embed` made over it.
- List a static library in one crate only: each crate that lists it carries a copy.

## Layout

- `crates/nsh-frameworks/src/lib.rs`: targets, XCFramework libraries, the build script directives, the `Cargo.toml` table.
- `crates/nsh-frameworks/src/desktop.rs`: Linux and Windows libraries.
- `crates/nsh-frameworks/src/embed.rs`, `main.rs`: the `nsh-frameworks` command.
- `crates/nsh-frameworks/src/tests.rs`: the library's tests.

## Checks

CI (`.github/workflows/ci.yml`) runs `cargo fmt --all -- --check`, then Clippy (`-D warnings`) and the tests on Linux, macOS and Windows, and `e2e/run.sh` on Linux and Windows. The unit tests build XCFrameworks in temporary directories; none runs Xcode's tools.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/frameworks
