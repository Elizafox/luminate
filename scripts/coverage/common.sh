#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

# Shared report plumbing for Rust coverage workflows. Callers set
# repository_root, coverage_root, and report_dir before invoking these helpers.

coverage_initialize_llvm() {
  rust_host=$(rustc -vV | sed -n 's/^host: //p')
  llvm_tools="$(rustc --print sysroot)/lib/rustlib/$rust_host/bin"
  llvm_cov="$llvm_tools/llvm-cov"
  if [ ! -x "$llvm_cov" ]; then
    echo "the active Rust toolchain does not provide llvm-cov" >&2
    exit 1
  fi
}

coverage_quarantine_invalid_profiles() {
  invalid_profiles="$report_dir/invalid-profraw"
  mkdir -p "$invalid_profiles"
  for profile in "$coverage_root"/*.profraw; do
    [ -e "$profile" ] || continue
    if ! "$llvm_tools/llvm-profdata" merge -sparse "$profile" \
      -o "$report_dir/profile-check.profdata" >/dev/null 2>&1; then
      mv "$profile" "$invalid_profiles/"
    fi
  done
  rm -f "$report_dir/profile-check.profdata"
}

coverage_merge_profiles() {
  cargo llvm-cov report --summary-only >/dev/null
  profdata=$(find "$coverage_root" -maxdepth 1 -type f -name '*.profdata' | sort | head -1)
  if [ -z "$profdata" ]; then
    echo "cargo-llvm-cov did not produce a merged profile under $coverage_root" >&2
    exit 1
  fi
}

coverage_add_objects() {
  directory=$1
  shift
  while IFS= read -r object; do
    coverage_objects+=( -object "$object" )
  done < <(find "$directory" -maxdepth 1 -type f "$@" | sort)
}

coverage_export() {
  raw_report=$1
  if [ "${#coverage_objects[@]}" -eq 0 ]; then
    echo "no coverage objects were produced" >&2
    exit 1
  fi
  mkdir -p "$report_dir"
  "$llvm_cov" export \
    "${coverage_objects[@]}" \
    -instr-profile="$profdata" \
    -summary-only \
    -ignore-filename-regex='/rustc/|/\.cargo/|/\.rustup/|/target/|/tests/|/(tests|[^/]+_tests)\.rs$' \
    >"$raw_report"
}

coverage_summarize_files() {
  files_report=$1
  summary_report=$2
  jq '
      {
          covered: (map(.covered) | add),
          count: (map(.count) | add)
      }
      | . + {
          uncovered: (.count - .covered),
          percent: (if .count == 0 then 0 else (.covered * 100 / .count) end)
      }
  ' "$files_report" >"$summary_report"
}

coverage_enforce_floor() {
  summary_report=$1
  floor=$2
  label=$3
  jq -r --arg label "$label" \
    '"\($label) production lines: \(.covered)/\(.count) (\(.percent | tostring)%)"' \
    "$summary_report"
  if ! jq -e ".percent >= $floor" "$summary_report" >/dev/null; then
    echo "$label production line coverage is below the $floor% floor" >&2
    exit 1
  fi
  echo "coverage reports: $report_dir"
}
