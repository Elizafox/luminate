#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
coverage_root="$repository_root/target/llvm-cov-target"
report_dir="$coverage_root/workspace-report"
raw_report="$report_dir/all-sources.json"
summary_report="$report_dir/summary.json"
files_report="$report_dir/files.json"

# shellcheck source=common.sh
. "$script_dir/common.sh"

command -v cargo >/dev/null
command -v jq >/dev/null
command -v rustc >/dev/null

cd "$repository_root"

metadata=$(cargo metadata --format-version 1 --no-deps)

cargo llvm-cov clean --workspace
cargo llvm-cov --workspace --all-features --no-report -- --test-threads=1

# The workspace test target does not build standalone integration-test crates
# for every package. Include the plugin ABI smoke test explicitly so generated
# export callbacks contribute to the workspace report.
cargo llvm-cov -p luminate-plugin-api --test export_smoke --all-features --no-report \
    -- --test-threads=1

# These suites build and exercise real workspace binaries and plugin shared
# objects. Keep the child-only config probe out of the generic `--ignored` run,
# and serialize suites which share daemon socket and build artefacts.
cargo llvm-cov -p luminate-plugin-wled --all-features --no-report -- \
    --ignored --test-threads=1
cargo llvm-cov -p luminated --test plugin_conformance --all-features --no-report \
    -- --ignored --test-threads=1
cargo llvm-cov -p luminate-cli --test cli_integration --all-features --no-report \
    -- --ignored --test-threads=1
cargo llvm-cov -p luminate-dbus --test dbus_integration --all-features --no-report \
    -- --ignored --test-threads=1

# Failure-path integration tests deliberately terminate wedged subprocesses.
# Retain malformed profiles for diagnosis without allowing one truncated child
# profile to invalidate every normally exited process in the run.
coverage_initialize_llvm
coverage_quarantine_invalid_profiles
coverage_merge_profiles

coverage_objects=()
while IFS= read -r target; do
    coverage_add_objects "$coverage_root/debug/deps" -perm -0100 \
        -name "${target//-/_}-*"
done < <(jq -r '
    [.workspace_members[]] as $workspace
    | [.packages[]
        | select(.id as $id | $workspace | index($id))
        | .targets[].name]
    | unique[]
' <<<"$metadata")
coverage_add_objects "$coverage_root/debug" -perm -0100 \
        \( -name 'luminate' \
        -o -name 'luminate-dbus' \
        -o -name 'luminated' \
        -o -name 'libluminate.so*' \)
coverage_add_objects "$repository_root/target/debug" -name 'libluminate_plugin_*.so'

coverage_export "$raw_report"

jq '
    [.data[0].files[]
        | select(.filename as $filename
            | $source_roots
            | any(. as $root | $filename | startswith($root)))
        | select(.filename | test("/(tests|[^/]+_tests)\\.rs$|/tests/") | not)
        | {
            file: (.filename | sub($repository_root + "/"; "")),
            covered: .summary.lines.covered,
            count: .summary.lines.count
        }
        | . + {
            uncovered: (.count - .covered),
            percent: (if .count == 0 then 0 else (.covered * 100 / .count) end)
        }]
    | sort_by(.percent, -.count, .file)
' --arg repository_root "$repository_root" \
    --argjson source_roots "$(jq '
        [.workspace_members[]] as $workspace
        | [.packages[]
            | select(.id as $id | $workspace | index($id))
            | .manifest_path
            | sub("/Cargo.toml$"; "/src/")]
        | unique
    ' <<<"$metadata")" \
    "$raw_report" >"$files_report"

coverage_summarize_files "$files_report" "$summary_report"
coverage_enforce_floor "$summary_report" 80 workspace
