#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
coverage_root="$repository_root/target/llvm-cov-target"
report_dir="$coverage_root/libluminate-report"
raw_report="$report_dir/all-sources.json"
summary_report="$report_dir/summary.json"
merged_profile="$report_dir/libluminate.profdata"

command -v cargo >/dev/null
command -v jq >/dev/null
command -v rustc >/dev/null

cd "$repository_root"

cargo llvm-cov clean --workspace
mkdir -p "$report_dir"
cargo llvm-cov \
    --package libluminate \
    --all-features \
    --no-report

rust_test_object=$(find "$coverage_root/debug/deps" -maxdepth 1 -type f \
    -name 'luminate-*' -perm -0100 | sort | sed -n '1p')
shared_library_object="$coverage_root/debug/libluminate.so"
test_source_count=$(find "$repository_root/crates/libluminate/src" -type f \
    -name '*_tests.rs' | wc -l | tr -d ' ')

if [ -z "$rust_test_object" ] || [ ! -x "$rust_test_object" ]; then
    echo "instrumented libluminate Rust test object was not produced" >&2
    exit 1
fi
if [ ! -f "$shared_library_object" ]; then
    echo "instrumented libluminate shared library was not produced" >&2
    exit 1
fi

rust_host=$(rustc -vV | sed -n 's/^host: //p')
llvm_tools=$(rustc --print sysroot)/lib/rustlib/$rust_host/bin
llvm_cov="$llvm_tools/llvm-cov"
llvm_profdata="$llvm_tools/llvm-profdata"

if [ ! -x "$llvm_cov" ] || [ ! -x "$llvm_profdata" ]; then
    echo "the active Rust toolchain does not provide llvm-cov and llvm-profdata" >&2
    exit 1
fi

"$llvm_profdata" merge -sparse "$coverage_root"/*.profraw -o "$merged_profile"
"$llvm_cov" export \
    "$rust_test_object" \
    -object "$shared_library_object" \
    -instr-profile "$merged_profile" \
    -format=text \
    -summary-only \
    -ignore-filename-regex='/rustc/|/\.cargo/|/\.rustup/|/target/|/tests/|_tests\.rs$' \
    >"$raw_report"

jq '
    def line_totals(files):
        reduce files[] as $file (
            {count: 0, covered: 0};
            .count += $file.summary.lines.count
            | .covered += $file.summary.lines.covered
        )
        | . + {
            percent: if .count == 0 then 0 else (.covered * 100 / .count) end
        };

    .data[0].files as $files
    | {
        production: line_totals([
            $files[]
            | select(.filename | contains("/crates/libluminate/src/"))
            | select(.filename | endswith("_tests.rs") | not)
        ]),
        test_sources: {
            excluded_from_production_coverage: true,
            file_count: $test_source_count
        },
        coverage_objects: {
            rust_test: $rust_test,
            shared_library: $shared_library
        }
    }
' --arg rust_test "$rust_test_object" \
    --arg shared_library "$shared_library_object" \
    --argjson test_source_count "$test_source_count" \
    "$raw_report" >"$summary_report"

jq -r '
    "libluminate production lines: \(.production.covered)/\(.production.count) (\(.production.percent | tostring)%)",
    "libluminate test sources: excluded from production coverage (\(.test_sources.file_count) files)",
    "Rust test coverage object: \(.coverage_objects.rust_test)",
    "shared-library coverage object: \(.coverage_objects.shared_library)"
' "$summary_report"

echo "coverage reports: $report_dir"
