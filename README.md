# nsh-frameworks

Prebuilt Apple frameworks (`.xcframework`, `.framework`) for Native Shell (NSH) apps, linked from an nshm crate and embedded into the app's packages.

NSH links what a crate's build script asks for, so an nshm that wraps an SDK links it from its own `build.rs`. `nsh-frameworks` does that from one list in the crate's `Cargo.toml`, picking the XCFramework library of each target. After `nsh package` it copies the SDK's dynamic frameworks into the package and signs them, which a build script cannot do. NSH itself is unchanged.

## Status

Usable. Checked on an Apple silicon Mac with NSH `main`, through an nshm that links a static Objective-C library (with a category, so `wholeArchive`) and a dynamic, versioned framework from XCFrameworks:

- `nsh package --target macos`, then `nsh-frameworks embed`: the app runs and calls both libraries, the category is linked, the framework sits in `Contents/Frameworks` with its links and without the download's quarantine, and `codesign --verify --deep --strict` passes. Without `embed` the app stops at launch (`Library not loaded`).
- `nsh package --target ios-simulator`, then `nsh-frameworks embed`: the app runs the same in the iOS simulator.
- `nsh run --target macos`, with `nsh-frameworks env`.
- `embed` keeps the app's entitlements, and running it twice changes nothing more.
- Static frameworks, archives not named `lib<name>.a`, and `wholeArchive` (a member nothing calls still runs its constructor) were linked in a plain Cargo workspace.

Not yet checked: device packages signed with an identity, `.ipa` files, `nsh run --target ios-simulator` (`SIMCTL_CHILD_…`), Intel Macs, Mac Catalyst.

## Install

```sh
cargo install --git https://github.com/SuperNSH/frameworks nsh-frameworks
```

## Use

In the nshm (or the app's own crate), list the frameworks, relative to the crate:

```toml
[package.metadata.nsh-frameworks]
ios = ["apple/Scanner.xcframework"]
macos = ["apple/Scanner.xcframework", { path = "apple/Ads.xcframework", wholeArchive = true }]

[build-dependencies]
nsh-frameworks = { git = "https://github.com/SuperNSH/frameworks", default-features = false }
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
- `nsh-frameworks list` shows what each framework links for this Mac (`--target` for another).

`nsh_frameworks::link(path, whole_archive)` links one framework from a build script that chooses itself; `embed` finds only those the `Cargo.toml` table lists.

## Limits

- System frameworks need none of this: link them as usual (`objc2` crates, or `cargo:rustc-link-lib=framework=StoreKit`).
- No Swift runtime search paths: a static library with Swift code needs the crate's build script to add them.
- A static library's privacy manifest is not merged into the app's: copy what it declares into `nsh.json` `ios.privacyManifest`.
- Run `embed` again after each `nsh package`, which writes a new package.
- List a static library in one crate only: each crate that lists it carries a copy.

## Layout

- `crates/nsh-frameworks/src/lib.rs`: targets, XCFramework libraries, the build script directives, the `Cargo.toml` table.
- `crates/nsh-frameworks/src/embed.rs`, `main.rs`: the `nsh-frameworks` command.
- `crates/nsh-frameworks/src/tests.rs`: the library's tests.

## Checks

CI (`.github/workflows/ci.yml`) runs `cargo fmt --all -- --check`, then Clippy (`-D warnings`) and the tests on Linux, macOS and Windows. The tests build XCFrameworks in temporary directories; none runs Xcode's tools.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/frameworks
