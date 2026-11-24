# nsh-maven

Android libraries from Maven repositories for Native Shell (NSH) apps and nshm crates.

NSH packages Android libraries (`.aar`, `.jar`, `.dex`) that an app lists in `nsh.json` `android.libraries`, or that a crate lists in `[package.metadata.nsh.android] libraries`. It reads only local files and resolves no Maven dependencies, so an SDK such as Firebase or ML Kit means finding and downloading dozens of artifacts by hand. `nsh-maven` does that once:

```sh
nsh-maven add com.google.firebase:firebase-messaging:26.0.1
```

It resolves the library and everything it depends on as Gradle resolves an Android app's runtime classpath, downloads the files into `android/maven/`, checks them against the repository's checksums, and lists them where NSH reads them. `nsh package` and `nsh run` stay offline and unchanged.

## Status

Usable. The resolver was checked offline against Gradle 9.3.1 with the Android Gradle plugin 9.1.0, through a `file://` mirror of a local Gradle cache:

- Whole graphs: `androidx.core:core-ktx:1.19.0` (26 modules), `kotlinx-coroutines-android:1.10.2` (6) and `okhttp:4.12.1` (8) resolve to exactly Gradle's module set.
- Module by module: in a Firebase Firestore, Crashlytics, Analytics and Play services auth graph, 139 of 142 modules' dependencies and constraints match Gradle's tree. The 3 that differ are edges Gradle hides under exclusions, and one module Gradle could not fetch offline.
- A vendored closure (`lifecycle-process`, `coroutines-android`, `okhttp`: 19 files) was packaged with NSH and ran on an Android emulator.

Not yet run against the live Google Maven repository and Maven Central end to end.

## Install

```sh
cargo install nsh-maven
```

## Use

In an app, at or below the directory that holds `nsh.json`:

```sh
nsh-maven add com.google.mlkit:text-recognition:16.0.1   # add a library, or change its version
nsh-maven remove com.google.mlkit:text-recognition       # remove one, with what only it needed
nsh-maven sync                                           # resolve and download again
nsh-maven list                                           # what was added, and the files
```

In an nshm crate, add `--crate`: the files go to the crate's `android/maven/` and its `Cargo.toml` `[package.metadata.nsh.android] libraries`. NSH then adds them to every app that links the crate. The rest of `Cargo.toml` stays as written.

`--repository <URL>` (repeatable, `https://` or `file://`) searches other repositories, in order, instead of Google's Maven repository and Maven Central. Later runs keep them. `--dir <PATH>` picks the app or crate.

Every run resolves the whole set again from the libraries named, and replaces exactly the files and entries the previous run added. Entries of your own stay first. `android/maven/maven.json` records the libraries, the repositories and each file's SHA-256: commit it with the files, or commit it alone and run `nsh-maven sync` to download them.

## How it resolves

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

- No Gradle plugins: put the values `google-services.json` makes in the app's `android/res/values`.
- No dependency substitution rules or capability conflicts, which a build script would settle (guava against `listenablefuture`).
- No POM relocations, and no `rejects` or `strictly` beyond taking the version.
- `kotlin-stdlib-jdk7` and `-jdk8` that older Kotlin libraries ask for are kept, where the Kotlin Gradle plugin may drop them. Since Kotlin 1.8 they hold no classes an app runs.

## Layout

- `crates/nsh-maven/src/lib.rs`: the resolver, the vendoring and the tests.
- `crates/nsh-maven/src/main.rs`: the `nsh-maven` command.
- `.github/workflows/ci.yml`: format, Clippy and tests on Linux, macOS and Windows.

## Checks

CI (`.github/workflows/ci.yml`) runs `cargo fmt --all -- --check`, then Clippy (`-D warnings`) and the tests on Linux, macOS and Windows. The tests resolve against repositories in memory and write into temporary directories; none reaches the network.

## Licence

MIT. See [LICENSE](LICENSE).

Repository: https://github.com/SuperNSH/maven
