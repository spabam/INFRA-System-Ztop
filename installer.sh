#!/usr/bin/env bash

# Copyright (c) 2026 Andrea Bodei <info@andreabodei.com>
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# ZTOP installer — bootstraps Rust + build tools as needed, builds the release
# binary, and installs it as $PREFIX/bin/ztop.
#
# Usage:
#   ./installer.sh                   # bootstrap + build + install /usr/bin/ztop
#   PREFIX=/usr/local ./installer.sh # install /usr/local/bin/ztop instead
#   ./installer.sh --uninstall       # remove the installed binary + ramcleaner
#   ./installer.sh --check           # audit without changes (fails if GPU checks fail)
#   ./installer.sh --help            # show this help
#   CARGO_TARGET_DIR=/dir ./installer.sh   # build there instead of ~/.cache/ztop-target
#
# Optional ramcleaner (separate installation, preview mode every 10 min):
#   ./installer.sh --install-ramcleaner        # script + config + cron.d + logrotate
#   ./installer.sh --uninstall-ramcleaner      # remove all of the above
#   ./installer.sh --install-ramcleaner-tuning # optional vm.* sysctl tuning
#   sudo ztop-ramcleaner --apply              # explicitly run one active pass
#
# A normal install installs only the monitor. The separate ramcleaner install
# creates a preview-only configuration and never runs an active verification pass.
# Existing operator configuration is preserved. Swap recycling requires its own
# opt-in setting; see ramcleaner/README.md before enabling actions.
#
# Run this installer as your normal user. It may ask for sudo for system-wide
# package installs and the final copy step, but it refuses to build as root.
# Every mutating run restores ownership of the entire checkout (.git/ included)
# to the installing user when it starts and again when it exits; sudo is used
# for that only when something in the tree is owned by another user.
# The release build lands in $XDG_CACHE_HOME/ztop-target (~/.cache/ztop-target)
# unless CARGO_TARGET_DIR is set; see cargo_target_dir below for why the
# checkout's own target/ is not used. The installed ztop binary can still be run
# with sudo/root when NVML needs elevated privileges for per-process GPU usage.
# The NVIDIA driver is NOT auto-installed (kernel modules and secure boot make
# that unsafe to do from a shell script); we only detect it and warn if missing.

set -euo pipefail

PREFIX="${PREFIX:-/usr}"
BINDIR="$PREFIX/bin"
NAME="ztop"
TARGET="$BINDIR/$NAME"

# ramcleaner component
SBINDIR="$PREFIX/sbin"
RC_NAME="ztop-ramcleaner"
RC_SRC_DIR="ramcleaner"
RC_TARGET="$SBINDIR/$RC_NAME"
RC_CONF_DIR="/etc/ztop"
RC_CONF="$RC_CONF_DIR/ramcleaner.conf"
RC_CRON="/etc/cron.d/ztop-ramcleaner"
RC_LOGROTATE="/etc/logrotate.d/ztop-ramcleaner"
RC_SYSCTL="/etc/sysctl.d/99-ztop-ramcleaner.conf"
RC_STATE_DIR="/var/lib/ztop-ramcleaner"
RC_LOG="/var/log/ztop-ramcleaner.log"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SELF="$HERE/$(basename "${BASH_SOURCE[0]}")"
SCRIPT_ARGS="$*"          # for error messages; $* inside a function is not this
cd "$HERE"

SUDO=""
REQUIRED_RUST_VERSION="1.88.0"
REQUIRED_RUST_TOOLCHAIN="1.88.0"
NVIDIA_EXPECTED=0
NVIDIA_DEVICE_COUNT=0

# --- helpers --------------------------------------------------------------

log()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!! \033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mxx \033[0m %s\n' "$*" >&2; exit 1; }

print_install_banner() {
    printf '%s\n' \
        '' \
        '========================================' \
        ' ZTOP' \
        ' a tool by Andrea Bodei' \
        ' www.andreabodei.com' \
        '========================================' \
        ''
}

# Lock this checkout without creating or deleting files. All mutating commands
# acquire the lock before package installation, builds, or system file changes.
acquire_install_lock() {
    command -v flock >/dev/null 2>&1 || die "flock is required; install util-linux first"
    exec 8<"$HERE" || die "cannot open checkout for locking: $HERE"
    if ! flock -n 8; then
        log "another installer is using this checkout; waiting"
        flock 8 || die "could not lock $HERE"
    fi
}

# The checkout must stay owned by the person who installs it (spabam on the
# maintainer's machines). Root-owned leftovers from a build or git command run
# in a root shell make the next user build fail, cannot be maintained by a
# user-level file-sync client, and get written back into .git/index with the
# wrong owner. Every mutating run therefore restores ownership of the ENTIRE
# checkout, .git/ included, right after taking the lock and again when the
# installer exits, whether it succeeded or failed. The owner is ZTOP_OWNER
# (user or user:group) when set; otherwise the invoking user (SUDO_USER under
# sudo); otherwise, in a plain root shell, the owner of the checkout directory.
checkout_owner() {
    local user="${ZTOP_OWNER:-}" group
    case "$user" in *:*) printf '%s\n' "$user"; return 0 ;; esac
    if [ -z "$user" ]; then
        user="${SUDO_USER:-}"
        if [ -z "$user" ] || [ "$user" = root ]; then user="$(id -un)"; fi
        if [ "$user" = root ]; then user="$(stat -c %U "$HERE")"; fi
    fi
    group="$(id -gn "$user" 2>/dev/null)" \
        || die "cannot resolve the checkout owner '$user'; set ZTOP_OWNER=user:group"
    printf '%s:%s\n' "$user" "$group"
}

# Entries of the checkout not owned by $1 (user:group); prints nothing when clean.
foreign_owned() {
    find "$HERE" \( ! -user "${1%%:*}" -o ! -group "${1#*:}" \) -print 2>/dev/null || true
}

# $1 = start (fatal when ownership cannot be restored: the build would fail on
# the foreign files anyway) or end (warns only: the requested work is already
# done and the next run repairs the tree).
restore_checkout_ownership() {
    local stage="$1" owner foreign elevate=""
    owner="$(checkout_owner)"
    foreign="$(foreign_owned "$owner")"
    [ -n "$foreign" ] || return 0
    log "restoring ownership of $HERE to $owner ($stage of run)"
    if [ "$(id -u)" -ne 0 ]; then
        elevate="sudo"
        if ! command -v sudo >/dev/null 2>&1; then
            ownership_failure "$stage" "$owner" "sudo is not available"
            return
        fi
        if [ "$stage" = start ]; then
            SUDO="sudo"
            sudo_authenticate
        elif ! sudo -n true 2>/dev/null; then
            ownership_failure "$stage" "$owner" "sudo credentials are no longer cached"
            return
        fi
    fi
    $elevate chown -Rf "$owner" "$HERE" || true
    foreign="$(foreign_owned "$owner")"
    [ -n "$foreign" ] || return 0
    ownership_failure "$stage" "$owner" \
        "still foreign-owned: $(printf '%s\n' "$foreign" | head -n 3 | tr '\n' ' ' || true)"
}

ownership_failure() {
    local stage="$1" owner="$2" reason="$3" fix
    fix="sudo chown -Rf $owner \"$HERE\""
    if [ "$stage" = start ]; then
        die "cannot restore ownership of $HERE to $owner ($reason); fix with: $fix"
    fi
    warn "ownership of $HERE was not fully restored to $owner ($reason); fix with: $fix"
    return 1
}

# EXIT trap for mutating runs: repeat the restore, keep the run's exit status.
finish_checkout_ownership() {
    local status=$?
    restore_checkout_ownership end || true
    exit "$status"
}

# --help prints the banner comment block. Its line numbers shift whenever the
# installer banner above it grows, so find it by marker rather than
# hardcoding a range (the old '2,21p' printed the prologue, not the usage).
# $SELF, not $0: $0 is "bash" when this file is sourced rather than executed.
usage() {
    local range
    range="$(awk '/^# ZTOP installer/ && !s { s = NR }
                  /^set -euo pipefail/ { print s "," (NR-2); exit }' "$SELF" 2>/dev/null)"
    if [ -n "$range" ]; then
        sed -n "${range}p" "$SELF"
    else
        grep '^#' "$SELF" | sed -n '20,60p'
    fi
}

run_with_progress() {
    local label="$1"
    shift

    local interval="${PROGRESS_INTERVAL:-10}"
    local started now elapsed pid status

    log "$label"
    "$@" &
    pid=$!
    started=$(date +%s)

    while kill -0 "$pid" 2>/dev/null; do
        sleep "$interval"
        if kill -0 "$pid" 2>/dev/null; then
            now=$(date +%s)
            elapsed=$((now - started))
            log "$label ... still running (${elapsed}s elapsed)"
        fi
    done

    if wait "$pid"; then
        status=0
    else
        status=$?
    fi
    now=$(date +%s)
    elapsed=$((now - started))

    if [ "$status" -eq 0 ]; then
        log "$label ... done (${elapsed}s)"
    else
        warn "$label ... failed after ${elapsed}s"
    fi
    return "$status"
}

source_cargo_env() {
    if [ -n "${HOME:-}" ] && [ -f "$HOME/.cargo/env" ]; then
        # shellcheck source=/dev/null
        . "$HOME/.cargo/env"
    fi
}

rustup_toolchain_installed() {
    local toolchain_dir
    toolchain_dir="$(rustup_toolchain_dir)"
    [ -n "$toolchain_dir" ] \
        && [ -x "$toolchain_dir/bin/rustc" ] \
        && [ -x "$toolchain_dir/bin/cargo" ]
}

rustup_toolchain_dir() {
    local rustup_home="${RUSTUP_HOME:-${HOME:-}/.rustup}"
    [ -d "$rustup_home/toolchains" ] || return 0

    find "$rustup_home/toolchains" -maxdepth 1 -type d \
        \( -name "$REQUIRED_RUST_TOOLCHAIN" -o -name "$REQUIRED_RUST_TOOLCHAIN-*" \) \
        -print -quit 2>/dev/null
}

rustc_status() {
    local toolchain_dir
    toolchain_dir="$(rustup_toolchain_dir)"
    if [ -n "$toolchain_dir" ] && [ -x "$toolchain_dir/bin/rustc" ]; then
        "$toolchain_dir/bin/rustc" --version 2>/dev/null || true
        return 0
    fi

    local output
    if command -v rustc >/dev/null 2>&1; then
        local rustc_path rustc_real
        rustc_path="$(command -v rustc)"
        rustc_real="$(readlink -f "$rustc_path" 2>/dev/null || printf '%s' "$rustc_path")"
        if [ "$(basename "$rustc_real")" != "rustup" ]; then
            rustc --version 2>/dev/null || true
            return 0
        fi
    fi
    output=""
    printf '%s' "$output"
}

current_rust_version() {
    local output
    output="$(rustc_status)"
    awk '{print $2}' <<< "$output"
}

version_ge() {
    local have="$1"
    local need="$2"
    local have_major have_minor have_patch need_major need_minor need_patch

    IFS=. read -r have_major have_minor have_patch <<< "$have"
    IFS=. read -r need_major need_minor need_patch <<< "$need"
    have_patch="${have_patch%%[^0-9]*}"
    need_patch="${need_patch%%[^0-9]*}"

    have_major="${have_major:-0}"
    have_minor="${have_minor:-0}"
    have_patch="${have_patch:-0}"
    need_major="${need_major:-0}"
    need_minor="${need_minor:-0}"
    need_patch="${need_patch:-0}"

    [[ "$have_major" =~ ^[0-9]+$ ]] || return 1
    [[ "$have_minor" =~ ^[0-9]+$ ]] || return 1
    [[ "$have_patch" =~ ^[0-9]+$ ]] || return 1
    [[ "$need_major" =~ ^[0-9]+$ ]] || return 1
    [[ "$need_minor" =~ ^[0-9]+$ ]] || return 1
    [[ "$need_patch" =~ ^[0-9]+$ ]] || return 1

    if (( have_major != need_major )); then
        (( have_major > need_major ))
        return
    fi
    if (( have_minor != need_minor )); then
        (( have_minor > need_minor ))
        return
    fi
    (( have_patch >= need_patch ))
}

refuse_root_build() {
    if [ "$(id -u)" -ne 0 ]; then
        return 0
    fi

    die "refusing to build as root: build scripts would run with full privileges and any Rust bootstrap would land in root's home; run ./installer.sh as your normal user (sudo is requested only when system packages or $BINDIR need it), then use 'sudo ztop' when hardware requires root for GPU metrics"
}

# Cargo's default target directory sits inside the checkout. Build outside it
# instead: a checkout kept in a synced folder loses its build directory
# mid-build (Dropbox renames a selectively excluded target/ to "target
# (Selective Sync Conflict)" within seconds, after which every cargo write
# fails with "No such file or directory"), and a per-user cache keeps gigabytes
# of artifacts out of the source tree. An explicit CARGO_TARGET_DIR still wins.
# XDG_CACHE_HOME must be absolute per the XDG spec; a relative value is ignored.
cargo_target_dir() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
    elif [[ "${XDG_CACHE_HOME:-}" == /* ]]; then
        printf '%s\n' "$XDG_CACHE_HOME/ztop-target"
    elif [ -n "${HOME:-}" ]; then
        printf '%s\n' "$HOME/.cache/ztop-target"
    else
        printf '%s\n' "$HERE/target"
    fi
}

# A previous build may have run as another user (e.g. root), leaving
# artifacts the current user cannot open — cargo then fails with
# "failed to open: .../target/release/.cargo-lock: Permission denied".
# Catch that before building so the error names the actual fix.
ensure_writable_workspace() {
    local bad="" target_dir repair_path="$HERE/Cargo.lock"
    target_dir="$(cargo_target_dir)"
    if [ -e "$HERE/Cargo.lock" ] && [ ! -w "$HERE/Cargo.lock" ]; then
        bad="$HERE/Cargo.lock"
    elif [ -d "$target_dir" ]; then
        repair_path="$target_dir"
        bad="$(find "$target_dir" ! -writable -print -quit 2>/dev/null)"
    fi
    [ -z "$bad" ] && return 0
    local owner
    owner="$(stat -c %U "$bad" 2>/dev/null || echo unknown)"
    die "cannot write $bad (owned by $owner) — a previous build likely ran as a different user; fix with: sudo chown -R $(id -un):$(id -gn) \"$repair_path\" && bash installer.sh"
}

# Restored/copied caches can retain Cargo fingerprints but lose execute bits.
# Cargo then reuses a build script it cannot run. Repair only known executable
# artifacts, preserving data-file modes and avoiding a full cache rebuild.
ensure_executable_artifact() {
    local artifact="$1"
    [ -e "$artifact" ] || [ -L "$artifact" ] || return 0
    if [ ! -f "$artifact" ] || [ -L "$artifact" ]; then
        die "expected a regular build artifact: $artifact"
    fi
    [ -x "$artifact" ] && return 0

    log "restoring execute permission: $artifact"
    chmod u+x -- "$artifact" \
        || die "cannot restore execute permission on $artifact; check its ownership and permissions"
    [ -x "$artifact" ] \
        || die "cannot execute $artifact after restoring permissions; check directory permissions and filesystem mount options (noexec); use CARGO_TARGET_DIR on an executable filesystem"
}

ensure_executable_build_scripts() {
    local build_dir artifact
    build_dir="$(cargo_target_dir)/release/build"
    [ -d "$build_dir" ] || return 0
    while IFS= read -r -d '' artifact; do
        ensure_executable_artifact "$artifact"
    done < <(find "$build_dir" -mindepth 2 -maxdepth 2 \
        -type f -name build-script-build -print0)
}

need_sudo() {
    if [ -n "$SUDO" ] || [ "$(id -u)" -eq 0 ]; then
        return 0
    fi
    command -v sudo >/dev/null 2>&1 \
        || die "need root or sudo for system packages and writing to $BINDIR"
    SUDO="sudo"
    sudo_authenticate
}

# Authenticate before long stages, with an explanation. Sudo may request fresh
# authentication if its configured credential timeout expires. Without this the prompt appears unannounced in
# the middle of a stage -- or, with no TTY, sudo fails with a bare
# "a terminal is required" that does not say what to do about it.
sudo_authenticate() {
    [ "$(id -u)" -eq 0 ] && return 0
    sudo -n true 2>/dev/null && return 0      # credential already cached

    if [ -n "${SUDO_ASKPASS:-}" ]; then
        sudo -A -v 2>/dev/null && return 0
    fi

    if [ -t 0 ] || [ -t 1 ]; then
        log "root privileges are required to install into $PREFIX and /etc"
        log "please enter your password for sudo"
        sudo -v || die "sudo authentication failed; nothing was installed"
        return 0
    fi

    die "root privileges are required but no terminal is available to prompt for a password.
     Pre-authenticate: sudo -v && ./installer.sh ${SCRIPT_ARGS}
     or set an askpass helper: SUDO_ASKPASS=/path/to/askpass ./installer.sh ${SCRIPT_ARGS}"
}

# Long stages (rust bootstrap, cargo build) must not be interrupted by a
# password prompt, so authenticate before any of them start.
sudo_preauth() {
    [ "$(id -u)" -eq 0 ] && return 0
    monitor_install_needs_sudo || return 0
    command -v sudo >/dev/null 2>&1 || return 0
    SUDO="sudo"
    sudo_authenticate
}

monitor_install_needs_sudo() {
    if [ -e "$TARGET" ] && [ ! -w "$TARGET" ]; then
        return 0
    fi
    local ancestor="$BINDIR"
    while [ ! -e "$ancestor" ]; do
        ancestor=$(dirname -- "$ancestor")
    done
    [ ! -d "$ancestor" ] || [ ! -w "$ancestor" ] || [ ! -x "$ancestor" ]
}

detect_pkg_manager() {
    if command -v apt-get >/dev/null 2>&1; then echo apt
    elif command -v dnf      >/dev/null 2>&1; then echo dnf
    elif command -v yum      >/dev/null 2>&1; then echo yum
    elif command -v pacman   >/dev/null 2>&1; then echo pacman
    elif command -v zypper   >/dev/null 2>&1; then echo zypper
    elif command -v apk      >/dev/null 2>&1; then echo apk
    else echo unknown
    fi
}

pkg_install() {
    # pkg_install <one or more package names for the detected PM>
    local pm
    pm=$(detect_pkg_manager)
    need_sudo
    case "$pm" in
        apt)    $SUDO apt-get update -y >/dev/null
                $SUDO apt-get install -y "$@" ;;
        dnf)    $SUDO dnf install -y "$@" ;;
        yum)    $SUDO yum install -y "$@" ;;
        pacman) $SUDO pacman -S --needed --noconfirm "$@" ;;
        zypper) $SUDO zypper install -y "$@" ;;
        apk)    $SUDO apk add --no-cache "$@" ;;
        *)      die "no supported package manager found; install [$*] manually" ;;
    esac
}

# --- bootstrap stages -----------------------------------------------------

ensure_rust() {
    # Always source rustup's env first. On many distros /usr/bin/cargo exists
    # but is older than this crate's locked dependency graph; rustup's shim
    # must take precedence when it is already installed.
    source_cargo_env

    local rust_version=""
    if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
        rust_version="$(current_rust_version)"
        if version_ge "$rust_version" "$REQUIRED_RUST_VERSION"; then
            log "Rust toolchain present: $(rustc_status)"
            return 0
        fi
        warn "Rust toolchain too old: rustc ${rust_version:-unknown}; need >= $REQUIRED_RUST_VERSION"
    else
        log "Rust toolchain not found"
    fi

    if command -v rustup >/dev/null 2>&1; then
        run_with_progress \
            "installing Rust $REQUIRED_RUST_TOOLCHAIN via rustup (minimal profile)" \
            rustup toolchain install "$REQUIRED_RUST_TOOLCHAIN" --profile minimal
    else
        if ! command -v curl >/dev/null 2>&1; then
            log "curl not found; installing it first"
            pkg_install curl
        fi
        log "rustup not found — downloading rustup installer"
        local rustup_init
        rustup_init="$(mktemp)"
        if ! curl --proto '=https' --tlsv1.2 -fL --progress-bar https://sh.rustup.rs -o "$rustup_init"; then
            rm -f "$rustup_init"
            return 1
        fi
        if ! run_with_progress \
            "installing Rust $REQUIRED_RUST_TOOLCHAIN via rustup (minimal profile)" \
            sh "$rustup_init" -y --default-toolchain "$REQUIRED_RUST_TOOLCHAIN" --profile minimal; then
            rm -f "$rustup_init"
            return 1
        fi
        rm -f "$rustup_init"
    fi

    source_cargo_env
    command -v cargo >/dev/null 2>&1 || \
        die "rustup ran but cargo is still not on PATH — open a new shell and re-run"
    command -v rustc >/dev/null 2>&1 || \
        die "rustup ran but rustc is still not on PATH — open a new shell and re-run"

    rust_version="$(current_rust_version)"
    version_ge "$rust_version" "$REQUIRED_RUST_VERSION" || \
        die "active rustc is ${rust_version:-unknown}; need >= $REQUIRED_RUST_VERSION"

    log "Rust toolchain ready: $(rustc_status)"
    warn "to use 'cargo' in your shell after this script exits, run: . \"\$HOME/.cargo/env\" (or open a new shell)"
}

ensure_build_tools() {
    local missing=()
    command -v cc   >/dev/null 2>&1 || missing+=(cc)
    command -v make >/dev/null 2>&1 || missing+=(make)
    if [ ${#missing[@]} -eq 0 ]; then
        log "build tools present (cc, make)"
        return 0
    fi
    log "missing build tools: ${missing[*]} — installing"
    local pm
    pm=$(detect_pkg_manager)
    case "$pm" in
        apt)    pkg_install build-essential pkg-config ;;
        dnf)    pkg_install gcc make pkg-config ;;
        yum)    pkg_install gcc make pkgconfig ;;
        pacman) pkg_install base-devel ;;
        zypper) need_sudo; $SUDO zypper install -y -t pattern devel_basis ;;
        apk)    pkg_install build-base ;;
        *)      die "no supported package manager; install a C compiler and make manually" ;;
    esac
}

nvidia_runtime_present() {
    if [ -e /dev/nvidiactl ] || [ -r /proc/driver/nvidia/version ]; then
        return 0
    fi
    local d
    for d in /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /usr/lib64 /usr/lib /lib /lib64; do
        [ ! -e "$d/libnvidia-ml.so.1" ] || return 0
    done
    if command -v ldconfig >/dev/null 2>&1; then
        ldconfig -p 2>/dev/null | grep 'libnvidia-ml\.so\.1' >/dev/null
        return "$?"
    fi
    return 1
}

nvidia_failure_advice() {
    local diagnostic="$1"
    if [[ "${diagnostic,,}" == *"mismatch"* ]]; then
        warn "NVIDIA driver/library version mismatch: the NVML userspace library must match the loaded NVIDIA kernel module"
        if [ -r /proc/driver/nvidia/version ]; then
            warn "loaded kernel driver: $(head -n1 /proc/driver/nvidia/version)"
        fi
        warn "in a container, match its NVIDIA userspace libraries to the host driver; rebooting the container cannot reload the host module"
        warn "on a host after a driver upgrade, reboot into the matching driver or restore matching userspace packages"
    else
        warn "check NVIDIA driver/library installation and GPU device permissions; ztop cannot supply a working kernel driver"
    fi
}

# This check never installs NVIDIA packages: ztop loads the runtime soname
# libnvidia-ml.so.1 directly, so an unversioned development symlink is unnecessary.
# Library/device files establish only that GPU support is expected, not that it
# works. The monitor's own headless sample is the final verification below.
check_nvidia() {
    NVIDIA_EXPECTED=0
    NVIDIA_DEVICE_COUNT=0
    if command -v nvidia-smi >/dev/null 2>&1; then
        NVIDIA_EXPECTED=1
        local names name
        if ! names=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>&1); then
            warn "NVIDIA query failed: ${names:-nvidia-smi returned no diagnostic}"
            nvidia_failure_advice "$names"
            return 1
        fi
        while IFS= read -r name; do
            [ -n "${name//[[:space:]]/}" ] || continue
            log "NVIDIA detected: $name"
            NVIDIA_DEVICE_COUNT=$((NVIDIA_DEVICE_COUNT + 1))
        done <<< "$names"
        if [ "$NVIDIA_DEVICE_COUNT" -eq 0 ]; then
            warn "NVIDIA query succeeded but returned no GPU names"
            return 1
        fi
    elif nvidia_runtime_present; then
        NVIDIA_EXPECTED=1
        log "NVIDIA runtime/device files found; GPU access still needs verification with ztop"
    else
        warn "no NVIDIA runtime detected; ztop can monitor CPU/RAM without GPU metrics"
    fi
}

verify_ztop_gpu() {
    local binary="$1" snapshot counts found fresh diagnostic
    # Ignore operator config so show_gpu=false cannot conceal a broken install.
    # Device telemetry needs no per-process accounting privileges.
    if ! snapshot=$("$binary" --no-config --gpu --no-pid-gpu --no-temp --json --top 1 2>&1); then
        warn "$binary could not produce a verification snapshot: $snapshot"
        return 1
    fi
    # Keep startup warnings visible even if a scoped NVML fallback succeeds.
    # Without any NVIDIA runtime on the host, "GPU init failed" only restates
    # the preflight result, so report it as information rather than as a
    # second warning.
    diagnostic=$(sed -n '/^ztop:/p' <<< "$snapshot")
    if [ -n "$diagnostic" ]; then
        if [ "$NVIDIA_EXPECTED" -eq 1 ]; then warn "$diagnostic"; else log "$diagnostic"; fi
    fi
    # --json is pretty-printed with one field per line. Inspect only its GPU
    # array, and require current samples for every discovered device. This
    # deliberately avoids adding Python or jq as an installer dependency.
    if ! counts=$(awk '
        /^[[:space:]]*"gpus": \[/ { gpu = 1; seen = 1; next }
        gpu && /^[[:space:]]*\]/ { gpu = 0 }
        gpu && /^[[:space:]]*"index":/ { found++ }
        gpu && /^[[:space:]]*"status": "fresh"/ { fresh++ }
        END { if (!seen) exit 1; print found+0, fresh+0 }
    ' <<< "$snapshot"); then
        warn "$binary verification output did not contain a GPU snapshot"
        return 1
    fi
    read -r found fresh <<< "$counts"
    if [ "$found" -gt 0 ] && [ "$fresh" -eq "$found" ] \
        && { [ "$NVIDIA_DEVICE_COUNT" -eq 0 ] || [ "$found" -eq "$NVIDIA_DEVICE_COUNT" ]; }; then
        log "ztop GPU verification passed: $fresh device(s) with fresh telemetry"
        return 0
    fi
    if [ "$NVIDIA_EXPECTED" -eq 0 ] && [ "$found" -eq 0 ]; then
        log "ztop snapshot verified; GPU monitoring unavailable on this host"
        return 0
    fi
    warn "ztop GPU verification failed: $fresh fresh of $found sampled device(s); nvidia-smi reported $NVIDIA_DEVICE_COUNT"
    # Preserve collector diagnostics without printing the whole process table.
    nvidia_failure_advice "$diagnostic"
    return 1
}

# --- ramcleaner component -------------------------------------------------

# The companion is installed only on an explicit --install-ramcleaner request.
ramcleaner_sources_present() {
    local f
    for f in "$RC_NAME" ramcleaner.conf cron.d.ztop-ramcleaner logrotate.ztop-ramcleaner; do
        [ -f "$HERE/$RC_SRC_DIR/$f" ] || return 1
    done
}

ensure_cron() {
    [ -d /etc/cron.d ] || { warn "no /etc/cron.d; install a cron daemon first"; return 1; }
    if command -v systemctl >/dev/null 2>&1; then
        if systemctl is-active --quiet cron 2>/dev/null || systemctl is-active --quiet crond 2>/dev/null; then
            return 0
        fi
    fi
    if command -v pgrep >/dev/null 2>&1 && { pgrep -x cron >/dev/null || pgrep -x crond >/dev/null; }; then
        return 0
    fi
    warn "no running cron daemon detected; start cron/crond before installing ramcleaner"
    return 1
}

install_ramcleaner() {
    ramcleaner_sources_present || die "incomplete ramcleaner sources in $HERE/$RC_SRC_DIR"
    local executable
    for executable in bash flock awk date nproc mktemp mv install; do
        command -v "$executable" >/dev/null 2>&1 || die "ramcleaner prerequisite missing: $executable"
    done
    ensure_cron || die "ramcleaner requires a running cron daemon"
    [[ "$RC_TARGET" =~ ^/[a-zA-Z0-9_./-]+$ ]] || die "ramcleaner PREFIX must be an absolute path without spaces or shell metacharacters"
    log "installing optional ramcleaner (new configuration: preview only)"
    need_sudo
    $SUDO install -D -m 0755 "$HERE/$RC_SRC_DIR/$RC_NAME" "$RC_TARGET"
    $SUDO install -d -m 0755 "$RC_CONF_DIR" "$RC_STATE_DIR"
    if [ -e "$RC_CONF" ]; then
        log "  config: $RC_CONF exists; operator settings preserved (may already be active)"
    else
        $SUDO install -m 0644 "$HERE/$RC_SRC_DIR/ramcleaner.conf" "$RC_CONF"
    fi
    if [ ! -e "$RC_LOG" ]; then
        $SUDO install -D -m 0640 /dev/null "$RC_LOG"
    fi
    if [ -d /etc/logrotate.d ]; then
        $SUDO install -m 0644 "$HERE/$RC_SRC_DIR/logrotate.ztop-ramcleaner" "$RC_LOGROTATE"
    fi

    # Report capabilities even when current pressure triggers no tier.
    $SUDO "$RC_TARGET" --status || die "ramcleaner capability inspection failed"
    # Verify in preview before enabling the scheduler, including on reinstalls.
    $SUDO "$RC_TARGET" --dry-run --verbose || die "ramcleaner preview failed; cron was not updated; see $RC_LOG"
    local tmp
    tmp=$(mktemp) || die "mktemp failed"
    sed "s|__RC_BIN__|$RC_TARGET|g" "$HERE/$RC_SRC_DIR/cron.d.ztop-ramcleaner" >"$tmp"
    if ! $SUDO install -m 0644 -o root -g root "$tmp" "$RC_CRON"; then
        rm -f "$tmp"
        die "could not install ramcleaner cron entry"
    fi
    rm -f "$tmp"
    log "installed: $($SUDO "$RC_TARGET" --version)"
    log "  preview verified; cron scheduled every 10 min using $RC_CONF"
    log "  one active pass: sudo $RC_TARGET --apply"
    log "  enable scheduled actions: sudo sed -i 's/^DRY_RUN=1/DRY_RUN=0/' $RC_CONF"
    log "  swap recycling stays disabled unless ENABLE_T3_SWAP_RECYCLE=1 is set separately"
    log "  status: sudo $RC_TARGET --status"
    log "  suspend: sudo touch /run/ztop-ramcleaner.disable"
}

uninstall_ramcleaner() {
    need_sudo
    local removed=0 f
    # Remove cron first so no pass can start while the script is disappearing.
    for f in "$RC_CRON" "$RC_LOGROTATE" "$RC_TARGET"; do
        if [ -e "$f" ]; then
            $SUDO rm -f "$f" && log "removed $f" && removed=1
        fi
    done
    if [ "$removed" -eq 0 ]; then
        warn "ramcleaner does not appear to be installed"
    fi
    # Config, state and log are operator data -- report, never delete silently.
    for f in "$RC_CONF" "$RC_STATE_DIR" "$RC_LOG" "$RC_SYSCTL"; do
        if [ -e "$f" ]; then log "kept $f (remove by hand if you want it gone)"; fi
    done
}

install_ramcleaner_tuning() {
    local src="$HERE/$RC_SRC_DIR/99-ztop-ramcleaner.sysctl.conf"
    [ -f "$src" ] || die "missing $src"
    command -v sysctl >/dev/null 2>&1 || die "sysctl is required for kernel tuning"
    grep -qs '^/dev/zram' /proc/swaps || die "this tuning profile requires active zram; review $src and configure zram first"
    need_sudo
    $SUDO install -m 0644 "$src" "$RC_SYSCTL"
    log "installed $RC_SYSCTL"
    if $SUDO sysctl -p "$RC_SYSCTL"; then
        log "ramcleaner sysctls reloaded"
    else
        die "could not apply ramcleaner sysctls; review $RC_SYSCTL before reboot"
    fi
}

# First line of `ztop --version`; the second line is the copyright notice.
monitor_version() {
    local output
    output="$("$1" --version 2>/dev/null)" || return 1
    printf '%s\n' "${output%%$'\n'*}"
}

# --- subcommands ----------------------------------------------------------

uninstall() {
    need_sudo
    if [ -e "$TARGET" ]; then
        log "removing $TARGET"
        $SUDO rm -f "$TARGET"
    else
        warn "$TARGET does not exist; nothing to uninstall"
    fi
    uninstall_ramcleaner
    log "done"
}

do_check() {
    log "environment audit (no changes made)"
    source_cargo_env
    if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
        local rust_version
        rust_version="$(current_rust_version)"
        if version_ge "$rust_version" "$REQUIRED_RUST_VERSION"; then
            log "  cargo:        $(rustc_status)"
        else
            warn "  cargo:        rustc ${rust_version:-unknown} too old; need >= $REQUIRED_RUST_VERSION (would install via rustup)"
        fi
    else
        warn "  cargo:        missing (would install via rustup)"
    fi
    local missing=()
    command -v cc   >/dev/null 2>&1 || missing+=(cc)
    command -v make >/dev/null 2>&1 || missing+=(make)
    if [ ${#missing[@]} -eq 0 ]; then
        log "  build tools:  cc, make present"
    else
        warn "  build tools:  missing ${missing[*]} (would install via $(detect_pkg_manager))"
    fi
    local gpu_status=0
    check_nvidia || gpu_status=1
    if [ -e "$TARGET" ]; then
        log "  target:       $TARGET exists ($(monitor_version "$TARGET" || echo unknown))"
        if verify_ztop_gpu "$TARGET"; then gpu_status=0; else gpu_status=1; fi
    else
        log "  target:       $TARGET does not exist yet"
    fi

    if [ -e "$RC_TARGET" ]; then
        log "  ramcleaner:   $RC_TARGET installed"
    else
        log "  ramcleaner:   not installed (optional: --install-ramcleaner)"
    fi
    if [ -e "$RC_CRON" ]; then
        log "  ramcleaner cron: $RC_CRON present"
        ensure_cron || warn "  ramcleaner schedule is installed but not running"
    else
        log "  ramcleaner cron: absent (optional companion)"
    fi
    if [ -r "$RC_CONF" ]; then
        if grep -qs '^DRY_RUN=1' "$RC_CONF"; then
            warn "  ramcleaner mode: preview only (DRY_RUN=1 in $RC_CONF)"
        elif grep -qs '^DRY_RUN=0' "$RC_CONF"; then
            log "  ramcleaner mode: scheduled actions enabled"
        else
            warn "  ramcleaner mode: inspect $RC_CONF (no simple DRY_RUN setting found)"
        fi
    fi
    if [ -r /proc/pressure/memory ] && grep -qE '^some .*avg60=[0-9]' /proc/pressure/memory; then
        log "  PSI:          enabled (pressure-based triggers available)"
    else
        warn "  PSI:          unavailable; check kernel PSI support and boot settings"
    fi
    if grep -qs '^/dev/zram' /proc/swaps; then
        log "  zram swap:    active"
    else
        warn "  zram swap:    none; see $RC_SRC_DIR/zram-generator.conf.example (recommended)"
    fi
    return "$gpu_status"
}

install_ztop() {
    refuse_root_build
    log "ZTOP installer — bootstrapping any missing dependencies"
    sudo_preauth
    ensure_rust
    ensure_build_tools
    check_nvidia || warn "NVIDIA preflight failed; the installed monitor will be checked directly after building"
    CARGO_TARGET_DIR="$(cargo_target_dir)"
    export CARGO_TARGET_DIR
    log "build directory: $CARGO_TARGET_DIR"
    ensure_writable_workspace
    ensure_executable_build_scripts

    # The explicit || die matters: without it a failed build could fall
    # through to the sudo install step and ship a stale release/ztop left
    # over from an earlier build.
    run_with_progress "building release binary" cargo build --locked --release --quiet \
        || die "release build failed in $CARGO_TARGET_DIR — fix the cargo error above and re-run; nothing was installed"

    BIN="$CARGO_TARGET_DIR/release/$NAME"
    ensure_executable_artifact "$BIN"
    [ -x "$BIN" ] || die "build did not produce $BIN"

    if monitor_install_needs_sudo; then
        need_sudo
    fi
    if [ -e "$TARGET" ]; then
        log "$TARGET already exists; will replace"
    fi

    log "installing $TARGET (mode 0755)"
    $SUDO install -D -m 0755 "$BIN" "$TARGET"

    local version
    if version=$(monitor_version "$TARGET"); then
        log "installed: $version"
    else
        die "$TARGET installed but --version failed; verify the binary and runtime dependencies"
    fi

    log "optional memory management is separate: ./installer.sh --install-ramcleaner"
    if ! verify_ztop_gpu "$TARGET"; then
        warn "the monitor is installed, but GPU monitoring needs repair; run ./installer.sh --check after fixing the NVIDIA stack"
        return 1
    fi
}

# --- entrypoint -----------------------------------------------------------

main() {
    [ "$#" -le 1 ] || die "expected one command (try --help)"
    # Inspection and argument validation happen before acquiring any mutation lock.
    case "${1:-install}" in
        --check|check) do_check; return ;;
        -h|--help|help) usage; return ;;
        install|--uninstall|uninstall|--install-ramcleaner|--uninstall-ramcleaner|--install-ramcleaner-tuning) ;;
        *) die "unknown argument: $1 (try --help)" ;;
    esac
    case "${1:-install}" in
        install|--install-ramcleaner|--install-ramcleaner-tuning) print_install_banner ;;
    esac
    acquire_install_lock
    restore_checkout_ownership start
    trap finish_checkout_ownership EXIT
    case "${1:-install}" in
        install) install_ztop ;;
        --uninstall|uninstall) uninstall ;;
        --install-ramcleaner) install_ramcleaner ;;
        --uninstall-ramcleaner) uninstall_ramcleaner ;;
        --install-ramcleaner-tuning) install_ramcleaner_tuning ;;
    esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
