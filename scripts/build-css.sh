#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=v4.3.5
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) asset=tailwindcss-linux-x64; digest=e47695cee3f0d90623a91f3939b08bdee610eb5ad99340adff9d4145b284d6cc ;;
  Linux-aarch64|Linux-arm64) asset=tailwindcss-linux-arm64; digest=ffcd0d21dbd7c605dc12743ff0c0b47debc963c86b399674084c1c8f2c96ecad ;;
  Darwin-arm64) asset=tailwindcss-macos-arm64; digest=f3f4f5d1727e2b693b8f1c9a1492b5dd87008c86ea286bee1589cb458e6235fc ;;
  Darwin-x86_64) asset=tailwindcss-macos-x64; digest=a63702686a3ed048b3135d84a73be49ea9d47f880c9b20e54b3ef60934ad8cdf ;;
  *) printf '%s\n' 'Unsupported platform. Use Linux or macOS.' >&2; exit 1 ;;
esac

verify() {
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$1" | cut -d ' ' -f 1)
  else
    actual=$(shasum -a 256 "$1" | cut -d ' ' -f 1)
  fi
  [ "$actual" = "$digest" ] || { printf '%s\n' 'Tailwind checksum mismatch.' >&2; exit 1; }
}

mkdir -p "$root/.tools"
binary="$root/.tools/$asset-$version"
temporary=
trap '[ -z "$temporary" ] || rm -f "$temporary"' EXIT HUP INT TERM
if [ ! -f "$binary" ]; then
  temporary=$(mktemp "$binary.XXXXXX")
  curl --fail --location --retry 3 --proto '=https' --tlsv1.2 \
    "https://github.com/tailwindlabs/tailwindcss/releases/download/$version/$asset" --output "$temporary"
  verify "$temporary"
  chmod +x "$temporary"
  mv "$temporary" "$binary"
  temporary=
fi
verify "$binary"
cd "$root"
case "${1:-}" in
  '') "$binary" --input static/styles.css --output static/app.css --minify ;;
  --watch) "$binary" --input static/styles.css --output static/app.css --minify --watch ;;
  --check)
    temporary=$(mktemp)
    "$binary" --input static/styles.css --output "$temporary" --minify
    cmp "$temporary" static/app.css || { printf '%s\n' 'Run sh scripts/build-css.sh to update CSS.' >&2; exit 1; }
    ;;
  *) printf '%s\n' 'Usage: sh scripts/build-css.sh [--watch|--check]' >&2; exit 1 ;;
esac
