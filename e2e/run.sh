#!/usr/bin/env bash
# Builds a shared and a static C library, links them into a Cargo app
# through `nsh_native_deps::link_declared`, puts the app in a package
# directory laid out as `nsh package` writes Linux and Windows ones, and
# checks that it runs only after `nsh-native-deps embed`, and from where
# Cargo built it with `nsh-native-deps env`. Linux and Windows.
set -euo pipefail
cd "$(dirname "$0")"
root=$PWD
tool="$root/../target/debug/nsh-native-deps"
arch=$(uname -m)
[ "$arch" = arm64 ] && arch=aarch64
case "$(uname -s)" in
  Linux) os=linux ;;
  MINGW* | MSYS* | CYGWIN*) os=windows; tool="$tool.exe"; arch=${PROCESSOR_ARCHITECTURE:-AMD64}
    [ "$arch" = ARM64 ] && arch=aarch64 || arch=x86_64 ;;
  *) echo "e2e: Linux and Windows only"; exit 1 ;;
esac
vendor="sdk/vendor/$arch"
rm -rf "$vendor" out && mkdir -p "$vendor" out
if [ $os = linux ]; then
  # The soname differs from the listed file: the package carries it by its
  # soname, which the executable asks for.
  cc -shared -fPIC -Wl,-soname,libscanner.so.1 -o "$vendor/libscanner.so" c/scanner.c
  cc -c -fPIC -o "$vendor/codec.o" c/codec.c && ar rcs "$vendor/libcodec.a" "$vendor/codec.o"
  exe=app; name=App; packaged="out/$name/$name"
else
  clang -shared -o "$vendor/scanner.dll" c/scanner.c
  clang -c -o "$vendor/codec.obj" c/codec.c && llvm-lib "/out:$vendor/codec.lib" "$vendor/codec.obj"
  exe=app.exe; name=App; packaged="out/$name/$name.exe"
fi
cargo build --quiet -p app
mkdir -p "out/$name" && cp "target/debug/$exe" "$packaged"
if [ $os = linux ]; then tar -czf "out/$name.tar.gz" -C out "$name"; else (cd out && tar -a -c -f "$name.zip" "$name"); fi

# Packaged without the shared library, the app does not start.
if (cd / && "$root/$packaged") >/dev/null 2>&1; then
  echo "e2e: the package ran before embed"; exit 1
fi
"$tool" --manifest-path Cargo.toml list
"$tool" --manifest-path Cargo.toml embed "out/$name"
out=$(cd / && "$root/$packaged")
echo "packaged: $out"
[ "$out" = "scanner=42 codec=7" ]
if [ $os = linux ]; then
  tar -tzf "out/$name.tar.gz" | grep -qx "$name/libscanner.so.1"
  [ "$(patchelf --print-rpath "$packaged")" = '$ORIGIN' ]
else
  # Windows' own tar reads zips; Git Bash's does not.
  "$(cygpath "$SYSTEMROOT")/System32/tar.exe" -tf "out/$name.zip" | tr -d '\r' | grep -qx "$name/scanner.dll"
fi

# Where Cargo built it, `env` lets it find the library.
if [ $os = linux ]; then
  eval "$("$tool" --manifest-path Cargo.toml env)"
  out=$("$root/target/debug/$exe")
else
  out=$(powershell -NoProfile -Command "$("$tool" --manifest-path Cargo.toml env); & '$(cygpath -w "$root/target/debug/$exe")'" | tr -d '\r')
fi
echo "from target: $out"
[ "$out" = "scanner=42 codec=7" ]
echo "e2e: ok"
