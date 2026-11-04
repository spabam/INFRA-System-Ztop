#!/usr/bin/env bash
# Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
die() { printf 'version: %s\n' "$*" >&2; exit 1; }
current=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
valid_core() { [[ "$1" =~ ^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})$ ]]; }
newer() {
    local a b c x y z
    IFS=. read -r a b c <<<"${1%%+*}"
    IFS=. read -r x y z <<<"${2%%+*}"
    (( a > x || (a == x && b > y) || (a == x && b == y && c > z) ))
}

check() {
    valid_core "${current%%+*}" || die "invalid SemVer core: $current"
    local lock rc documented latest previous
    lock=$(awk '/^name = "ztop"$/ {in_package=1; next} in_package && /^version = / {gsub(/"/, "", $3); print $3; exit}' Cargo.lock)
    rc=$(sed -n 's/^RC_VERSION="\([^"]*\)"/\1/p' ramcleaner/ztop-ramcleaner)
    documented=$(sed -n 's/^\*\*Status:\*\* v\([^ ]*\).*/\1/p' README.md)
    latest=$(sed -n 's/^## \[\([^]]*\)\].*/\1/p' CHANGELOG.md | head -n 1)
    previous=$(sed -n 's/^## \[\([^]]*\)\].*/\1/p' CHANGELOG.md | sed -n '2p')
    [ "$lock" = "$current" ] || die "Cargo.lock differs from Cargo.toml"
    [ "$rc" = "$current" ] || die "ramcleaner version differs from Cargo.toml"
    [ "$documented" = "$current" ] || die "README Status differs from Cargo.toml"
    [ "$latest" = "$current" ] || die "latest changelog entry differs from Cargo.toml"
    if [ -n "$previous" ]; then
        valid_core "${previous%%+*}" || die "invalid previous release: $previous"
        newer "$current" "$previous" || die "version must increase independently of build metadata"
    fi
    printf 'Versions consistent: %s\n' "$current"
}

if [ "${1:-}" = --check ]; then
    check
    exit 0
fi
[ "$#" -eq 2 ] || die 'usage: scripts/version.sh --check | MAJOR.MINOR.PATCH "change summary"'
valid_core "$1" || die 'expected MAJOR.MINOR.PATCH without leading zeros'
valid_core "${current%%+*}" || die "invalid current release: $current"
newer "$1" "$current" || die "new version must be greater than ${current%%+*}"
[ -n "$2" ] || die 'change summary cannot be empty'
# Reject drift before preparing a bump, so a missed substitution cannot leave
# the manifest advanced while a separately distributed component stays behind.
check >/dev/null

next="$1+$(date -u '+%Y%m%d.%H%M')"
version_tmp=$(mktemp -d)
version_writing=0
version_files=(Cargo.toml Cargo.lock README.md CHANGELOG.md ramcleaner/ztop-ramcleaner)
cleanup() {
    local status=$? file restore_failed=0
    trap - EXIT INT TERM
    if (( status != 0 && version_writing )); then
        for file in "${version_files[@]}"; do
            if ! cat "$version_tmp/backup/$file" >"$file"; then
                restore_failed=1
                printf 'version: restore %s from %s\n' "$file" "$version_tmp/backup/$file" >&2
            fi
        done
    fi
    if (( ! restore_failed )); then rm -rf -- "$version_tmp"; fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$version_tmp/ramcleaner" "$version_tmp/backup/ramcleaner"
for file in "${version_files[@]}"; do
    cp "$file" "$version_tmp/backup/$file"
done
sed "s/^version = \"$current\"/version = \"$next\"/" Cargo.toml >"$version_tmp/Cargo.toml"
awk -v version="$next" '
    /^name = "ztop"$/ {in_package=1}
    in_package && /^version = / {print "version = \"" version "\""; in_package=0; next}
    {print}
' Cargo.lock >"$version_tmp/Cargo.lock"
sed "s/^RC_VERSION=\"$current\"/RC_VERSION=\"$next\"/" ramcleaner/ztop-ramcleaner >"$version_tmp/ramcleaner/ztop-ramcleaner"
sed "s/^\*\*Status:\*\* v$current /\*\*Status:\*\* v$next /" README.md >"$version_tmp/README.md"
awk '/^## \[/ {exit} {print}' CHANGELOG.md >"$version_tmp/CHANGELOG.md"
printf '## [%s] — %s\n\n- %s\n\n' "$next" "$(date -u '+%Y-%m-%d')" "$2" >>"$version_tmp/CHANGELOG.md"
awk '/^## \[/ {entries=1} entries {print}' CHANGELOG.md >>"$version_tmp/CHANGELOG.md"
(cd "$version_tmp"; current=$next; check >/dev/null)
version_writing=1
for file in "${version_files[@]}"; do
    cat "$version_tmp/$file" >"$file"
done
current=$next
check
version_writing=0
