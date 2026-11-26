# shellcheck shell=bash
# Copyright (c) 2026 Andrea Bodei — www.andreabodei.com
# VANTAGE installer progress: one helper for every product installer.
#
# A progress bar is mandatory: every installation shows how far it is. Source
# this file and report each real stage of the work:
#
#   vantage_progress FROM TO LABEL   the work that starts now covers FROM..TO
#                                    percent of this installer's work
#   vantage_step N TOTAL LABEL       step N of TOTAL steps of about the same length
#   vantage_progress_done [LABEL]    this installer has finished
#
# Started by the platform installer ($VANTAGE_PROGRESS_FILE is set), each call
# is a line "percent<TAB>step<TAB>0-100<TAB>label" in that file and the
# platform installer draws the bar. Started on its own, the installer prints a
# progress line on standard error at every call (standard output stays for
# what scripts read from it).
#
# An installer started during a part reports inside that part: the part is
# passed on in VANTAGE_PROGRESS_BASE and VANTAGE_PROGRESS_SPAN.
#
# The canonical copy is System/MUUUUI/deploy/vantage-progress.sh. Each product
# keeps an identical copy in its own repository, so that it installs on its
# own; tests/test_progress_helpers.py checks that every copy matches.
_VP_BASE="${VANTAGE_PROGRESS_BASE:-0}"; _VP_SPAN="${VANTAGE_PROGRESS_SPAN:-100}"
[[ "$_VP_BASE" =~ ^[0-9]+([.][0-9]+)?$ ]] || _VP_BASE=0
[[ "$_VP_SPAN" =~ ^[0-9]+([.][0-9]+)?$ ]] || _VP_SPAN=100
# The first installer started on its own keeps the clock and the last
# percentage shown; the installers it starts continue them.
if [[ -z "${VANTAGE_PROGRESS_FILE:-}" && -z "${VANTAGE_PROGRESS_STARTED:-}" ]]; then
  VANTAGE_PROGRESS_STARTED="$(date +%s)"; export VANTAGE_PROGRESS_STARTED
fi
_vp_line() { # _vp_line PERCENT LABEL: one progress line of an installer started on its own
  local percent="$1" label="$2" filled bar now
  [[ "$percent" =~ ^[0-9]+$ ]] || return 0
  (( percent <= 100 )) || percent=100
  (( percent >= ${VANTAGE_PROGRESS_SHOWN:-0} )) || percent="${VANTAGE_PROGRESS_SHOWN:-0}"
  VANTAGE_PROGRESS_SHOWN="$percent"; export VANTAGE_PROGRESS_SHOWN
  filled=$(( percent * 24 / 100 ))
  bar="$(printf '%*s' "$filled" '' | tr ' ' '#')$(printf '%*s' "$(( 24 - filled ))" '' | tr ' ' '-')"
  now=$(( $(date +%s) - ${VANTAGE_PROGRESS_STARTED:-$(date +%s)} ))
  printf '[progress] [%s] %3d%%  %s  %dm%02ds\n' "$bar" "$percent" "$label" "$(( now / 60 ))" "$(( now % 60 ))" >&2
}
vantage_progress() { # vantage_progress FROM TO LABEL
  local at
  at="$(awk -v f="${1:-0}" -v t="${2:-100}" -v b="$_VP_BASE" -v s="$_VP_SPAN" \
    'BEGIN { printf "%.2f %.4f", b + s * f / 100, s * (t - f) / 100 }' 2>/dev/null)" || return 0
  [[ -n "$at" ]] || return 0
  export VANTAGE_PROGRESS_BASE="${at% *}" VANTAGE_PROGRESS_SPAN="${at#* }"
  if [[ -n "${VANTAGE_PROGRESS_FILE:-}" ]]; then
    [[ -w "$VANTAGE_PROGRESS_FILE" ]] || return 0
    printf 'percent\t%s\t%s\t%s\n' "${VANTAGE_PROGRESS_MODULE:-}" "${at% *}" "${3:-}" >> "$VANTAGE_PROGRESS_FILE" 2>/dev/null || true
  else
    _vp_line "${at%%.*}" "${3:-}"
  fi
  return 0
}
vantage_step() { # vantage_step N TOTAL LABEL
  (( ${2:-0} > 0 )) || return 0
  vantage_progress "$(( ($1 - 1) * 100 / $2 ))" "$(( $1 * 100 / $2 ))" "${3:-} ($1/$2)"
}
vantage_progress_done() { # vantage_progress_done [LABEL]
  local end
  end="$(awk -v b="$_VP_BASE" -v s="$_VP_SPAN" 'BEGIN { printf "%.2f", b + s }' 2>/dev/null)" || return 0
  if [[ -n "${VANTAGE_PROGRESS_FILE:-}" ]]; then
    [[ -w "$VANTAGE_PROGRESS_FILE" ]] || return 0
    printf 'percent\t%s\t%s\t%s\n' "${VANTAGE_PROGRESS_MODULE:-}" "$end" "${1:-done}" >> "$VANTAGE_PROGRESS_FILE" 2>/dev/null || true
  else
    _vp_line "${end%%.*}" "${1:-done}"
  fi
  return 0
}
