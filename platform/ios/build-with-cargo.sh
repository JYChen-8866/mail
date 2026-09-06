#!/usr/bin/env bash
set -euo pipefail

binary_name="${1:?binary name is required}"
project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
profile=debug
release_args=()
if [[ "${CONFIGURATION:-Debug}" != "Debug" ]]; then
  profile=release
  release_args=(--release)
fi

export PATH="/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH:$HOME/.cargo/bin"
export CARGO_PROFILE_RELEASE_DEBUG="${CARGO_PROFILE_RELEASE_DEBUG:-1}"
export CARGO_TARGET_DIR="${DERIVED_FILE_DIR:?}/cargo"

is_simulator=0
if [[ "${LLVM_TARGET_TRIPLE_SUFFIX:-}" == "-simulator" ]]; then
  is_simulator=1
fi

executables=()
for arch in $ARCHS; do
  case "$arch" in
    arm64)
      if [[ "$is_simulator" -eq 1 ]]; then
        cargo_target=aarch64-apple-ios-sim
      else
        cargo_target=aarch64-apple-ios
      fi
      ;;
    x86_64)
      cargo_target=x86_64-apple-ios
      ;;
    *)
      printf 'Unsupported Xcode architecture: %s\n' "$arch" >&2
      exit 1
      ;;
  esac

  cargo build \
    --manifest-path "$project_dir/Cargo.toml" \
    --locked \
    --target "$cargo_target" \
    --bin "$binary_name" \
    --no-default-features \
    --features remote-content,ios \
    "${release_args[@]}"
  executables+=("$CARGO_TARGET_DIR/$cargo_target/$profile/$binary_name")
done

lipo -create -output "$TARGET_BUILD_DIR/$EXECUTABLE_PATH" "${executables[@]}"

# The UI fonts are embedded in the executable. Ship their OFL notices in the
# application bundle as required for redistributed copies of the fonts.
font_license_root="$TARGET_BUILD_DIR/${UNLOCALIZED_RESOURCES_FOLDER_PATH:?}/Licenses"
google_font_license_dir="$font_license_root/Google Sans Flex"
emoji_font_license_dir="$font_license_root/Noto Emoji"
mkdir -p "$google_font_license_dir" "$emoji_font_license_dir"
cp "$project_dir/resources/fonts/google-sans-flex/OFL.txt" "$google_font_license_dir/OFL.txt"
cp "$project_dir/resources/fonts/google-sans-flex/README.md" "$google_font_license_dir/README.md"
cp "$project_dir/resources/fonts/noto-emoji/OFL.txt" "$emoji_font_license_dir/OFL.txt"
cp "$project_dir/resources/fonts/noto-emoji/README.md" "$emoji_font_license_dir/README.md"

if [[ -n "${DWARF_DSYM_FOLDER_PATH:-}" && -n "${DWARF_DSYM_FILE_NAME:-}" ]]; then
  mkdir -p "$DWARF_DSYM_FOLDER_PATH"
  dsymutil "$TARGET_BUILD_DIR/$EXECUTABLE_PATH" \
    -o "$DWARF_DSYM_FOLDER_PATH/$DWARF_DSYM_FILE_NAME"
fi

if [[ "$is_simulator" -eq 0 && "${CODE_SIGNING_ALLOWED:-YES}" != "NO" && -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]]; then
  codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"
fi
