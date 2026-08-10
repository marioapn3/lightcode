#!/usr/bin/env bash
#
# LightCode release script.
#
# Usage:
#   ./release.sh 0.2.0
#
# Does:
#   1. bump version in ligthcode-apps/Cargo.toml (Cargo.lock follows via cargo check)
#   2. commit "release vX.Y.Z"
#   3. tag vX.Y.Z and push both to origin
#
# After that the GitHub Actions "release" workflow builds & uploads binaries,
# and the homebrew tap formula auto-updates within ~4h.
set -euo pipefail

VERSION="${1:?usage: ./release.sh 0.2.0}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must be X.Y.Z, got: $VERSION" >&2
  exit 1
fi

CARGOTOML="ligthcode-apps/Cargo.toml"
if [[ ! -f "$CARGOTOML" ]]; then
  echo "error: run from the repo root (where ligthcode-apps/ lives)" >&2
  exit 1
fi

echo "==> bumping version to $VERSION in $CARGOTOML"
sed -i '' "s/^version = \".*\"$/version = \"$VERSION\"/" "$CARGOTOML"

echo "==> syncing Cargo.lock"
cargo check --manifest-path ligthcode-apps/Cargo.toml --locked 2>/dev/null || \
  cargo check --manifest-path ligthcode-apps/Cargo.toml

echo "==> committing"
git add "$CARGOTOML" ligthcode-apps/Cargo.lock
git commit -m "release v$VERSION"

echo "==> tagging + pushing"
git tag "v$VERSION"
git push origin main
git push origin "v$VERSION"

echo ""
echo "✓ released v$VERSION"
echo "  CI: https://github.com/marioapn3/lightcode/actions"
echo "  Release binary muncul otomatis; formula brew update ≤4 jam kemudian."
echo "  User: brew upgrade lightcode"
