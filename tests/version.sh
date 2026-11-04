#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/ramcleaner"
cp "$repo/scripts/version.sh" "$fixture/scripts/"
cat >"$fixture/Cargo.toml" <<'EOF'
[package]
name = "ztop"
version = "0.9.0+20260905.2041"
EOF
cat >"$fixture/Cargo.lock" <<'EOF'
[[package]]
name = "ztop"
version = "0.9.0+20260905.2041"
EOF
printf '**Status:** v0.9.0+20260905.2041 (fixture)\n' >"$fixture/README.md"
printf '# Changelog\n\n## [0.9.0+20260905.2041]\n\n- Current\n\n## [0.8.18]\n\n- Earlier\n' >"$fixture/CHANGELOG.md"
printf 'RC_VERSION="0.9.0+20260905.2041"\n' >"$fixture/ramcleaner/ztop-ramcleaner"
cd "$fixture"
bash scripts/version.sh --check
for invalid in 0.1.1 0.9.0 01.10.0 1.2 1.0.0+build; do
    if bash scripts/version.sh "$invalid" 'must fail' >/dev/null 2>&1; then
        printf 'unexpected accepted version: %s\n' "$invalid" >&2
        exit 1
    fi
done
bash scripts/version.sh 0.10.0 'Version automation fixture'
bash scripts/version.sh 1.0.0 'Major release fixture'
# Drift in a separately distributed component must fail the release check.
sed -i 's/^RC_VERSION=.*/RC_VERSION="0.0.0"/' ramcleaner/ztop-ramcleaner
before=$(sha256sum Cargo.toml Cargo.lock README.md CHANGELOG.md ramcleaner/ztop-ramcleaner)
if bash scripts/version.sh --check >/dev/null 2>&1; then
    printf 'version drift was not detected\n' >&2
    exit 1
fi
if bash scripts/version.sh 1.1.0 'reject drift before writing' >/dev/null 2>&1; then
    printf 'bump accepted inconsistent sources\n' >&2
    exit 1
fi
[ "$before" = "$(sha256sum Cargo.toml Cargo.lock README.md CHANGELOG.md ramcleaner/ztop-ramcleaner)" ]
printf 'Version automation tests passed\n'
