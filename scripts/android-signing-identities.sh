#!/usr/bin/env bash
set -euo pipefail

apk="${1:-}"
android_sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"

if [[ -z "$apk" || ! -f "$apk" ]]; then
  printf 'Usage: %s /absolute/path/to/signed.apk\n' "$0" >&2
  exit 1
fi
if [[ -z "$android_sdk" || ! -d "$android_sdk/build-tools" ]]; then
  printf 'Android SDK build-tools not found. Set ANDROID_HOME.\n' >&2
  exit 1
fi

build_tools="$(find "$android_sdk/build-tools" -mindepth 1 -maxdepth 1 -type d -print | sort -V | tail -n 1)"
apksigner="$build_tools/apksigner"
if [[ ! -x "$apksigner" ]]; then
  printf 'apksigner not found under %s\n' "$build_tools" >&2
  exit 1
fi

signing="$($apksigner verify --print-certs "$apk")"
sha1_line="$(printf '%s\n' "$signing" | sed -n 's/^Signer #1 certificate SHA-1 digest: //p' | head -n 1)"
if [[ -z "$sha1_line" ]]; then
  printf 'Could not read the APK signing certificate.\n' >&2
  exit 1
fi

google_sha1="$(printf '%s' "$sha1_line" | sed 's/../&:/g; s/:$//' | tr '[:lower:]' '[:upper:]')"
sha1_escaped="$(printf '%s' "$sha1_line" | sed 's/../\\x&/g')"
microsoft_hash="$(printf '%b' "$sha1_escaped" | base64 | tr -d '\n')"
microsoft_path="$(printf '%s' "$microsoft_hash" | sed 's/+/%2B/g; s#/#%2F#g; s/=/%3D/g')"

printf 'Package name: com.flectar.mail\n'
printf 'Google Android OAuth SHA-1: %s\n' "$google_sha1"
printf 'Microsoft signature hash: %s\n' "$microsoft_hash"
printf 'Microsoft redirect URI: msauth://com.flectar.mail/%s\n' "$microsoft_path"
