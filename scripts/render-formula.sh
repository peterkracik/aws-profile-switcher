#!/bin/sh
# Renders the Homebrew formula from packaging/homebrew/aws-profile-switcher.rb.tmpl.
#
# Usage: scripts/render-formula.sh <version> <dist-dir>
#   <version>   e.g. 1.0.0 (no leading "v")
#   <dist-dir>  directory with aws-profile-switcher-<target>.tar.gz.sha256 files
# Writes the formula to stdout.
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <version> <dist-dir>" >&2
  exit 2
fi
version=$1
dist=$2

case $version in
  v*) echo "error: version must not start with 'v': $version" >&2; exit 2 ;;
  *[!0-9A-Za-z.+-]*|'') echo "error: invalid version: $version" >&2; exit 2 ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
template=$root/packaging/homebrew/aws-profile-switcher.rb.tmpl

sha() {
  file=$dist/aws-profile-switcher-$1.tar.gz.sha256
  if [ ! -f "$file" ]; then
    echo "error: missing $file" >&2
    exit 1
  fi
  value=$(awk '{print $1; exit}' "$file")
  case $value in
    *[!0-9a-f]*|'') echo "error: bad sha256 in $file" >&2; exit 1 ;;
  esac
  if [ "${#value}" -ne 64 ]; then
    echo "error: bad sha256 in $file" >&2
    exit 1
  fi
  printf '%s' "$value"
}

arm=$(sha aarch64-apple-darwin)
intel=$(sha x86_64-apple-darwin)
linux=$(sha x86_64-unknown-linux-musl)

sed \
  -e "s/{{VERSION}}/$version/g" \
  -e "s/{{SHA256_AARCH64_APPLE_DARWIN}}/$arm/g" \
  -e "s/{{SHA256_X86_64_APPLE_DARWIN}}/$intel/g" \
  -e "s/{{SHA256_X86_64_UNKNOWN_LINUX_MUSL}}/$linux/g" \
  "$template"
