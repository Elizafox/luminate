#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
coverage_root="$repository_root/target/llvm-cov-target"
report_dir="$coverage_root/luminated-report"
raw_report="$report_dir/all-sources.json"
summary_report="$report_dir/summary.json"
files_report="$report_dir/files.json"

# shellcheck source=common.sh
. "$script_dir/common.sh"

command -v cargo >/dev/null
command -v jq >/dev/null
command -v rustc >/dev/null

cd "$repository_root"

metadata=$(cargo metadata --format-version 1 --all-features)
package_names=$(jq -r '
    . as $metadata
    | [.workspace_members[]] as $workspace
    | (.resolve.nodes | map({key: .id, value: [.deps[].pkg]}) | from_entries) as $dependencies
    | (.packages[] | select(.name == "luminated") | .id) as $root
    | def closure($ids):
        reduce $ids[] as $id
            ($ids; . + [($dependencies[$id] // [])[] | select(. as $dependency | $workspace | index($dependency))])
        | unique
        | if length == ($ids | length) then . else closure(.) end;
      closure([$root])[] as $id
    | $metadata.packages[]
    | select(.id == $id)
    | .name
' <<<"$metadata")

package_args=()
while IFS= read -r package; do
    package_args+=( --package "$package" )
done <<<"$package_names"

cargo llvm-cov clean --workspace
cargo llvm-cov "${package_args[@]}" --all-features --no-report -- --test-threads=1

# Exercise the daemon's process boundary and real shared-memory negotiation.
# Select the integration targets explicitly so child-only ignored unit tests are
# not run as standalone tests.
cargo llvm-cov -p luminated --test plugin_conformance --all-features --no-report \
    -- --ignored --test-threads=1
cargo llvm-cov -p luminated --test shm_client_stream --all-features --no-report \
    -- --ignored --test-threads=1

# Front-end source is outside the measured package set, but these process-level
# scenarios are the strongest existing black-box drivers for daemon topology,
# management, authorization, transition, and reconnection behaviour.
LUMINATE_TEST_DAEMON="$coverage_root/debug/luminated" \
    cargo llvm-cov -p luminate-cli --test cli_integration --all-features --no-report \
    -- --ignored --test-threads=1
LUMINATE_TEST_DAEMON="$coverage_root/debug/luminated" \
    cargo llvm-cov -p luminate-dbus --test dbus_integration --all-features --no-report \
    -- --ignored --test-threads=1

# Generated plugin exports live in a standalone integration target that Cargo
# does not consistently select through a dependency-only package invocation.
cargo llvm-cov -p luminate-plugin-api --test export_smoke --all-features --no-report \
    -- --test-threads=1

# Failure-path integration tests deliberately terminate wedged subprocesses.
# LLVM can leave a truncated profile for such a process, and one malformed
# input makes llvm-profdata reject every otherwise valid profile in the run.
coverage_initialize_llvm
coverage_quarantine_invalid_profiles
coverage_merge_profiles

coverage_objects=()
coverage_add_objects "$coverage_root/debug/deps" -perm -0100 \
        \( -name 'luminate_core-*' \
        -o -name 'luminate_host_supervisor-*' \
        -o -name 'luminate_platform-*' \
        -o -name 'luminate_plugin_api-*' \
        -o -name 'luminate_protocol-*' \
        -o -name 'luminated-*' \
        -o -name 'export_smoke-*' \
        -o -name 'cli_integration-*' \
        -o -name 'dbus_integration-*' \
        -o -name 'plugin_conformance-*' \
        -o -name 'shm_client_stream-*' \)
coverage_add_objects "$coverage_root/debug" -perm -0100 -name 'luminated'
coverage_add_objects "$repository_root/target/debug" -perm -0100 -name 'luminated'

coverage_export "$raw_report"

package_pattern=$(printf '%s\n' $package_names | jq -Rrsc 'split("\n")[:-1] | join("|")')
jq '
    [.data[0].files[]
        | select(.filename | startswith($repository_root + "/crates/"))
        | select(.filename | test("/crates/(" + $packages + ")/src/"))
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
' --arg repository_root "$repository_root" --arg packages "$package_pattern" \
    "$raw_report" >"$files_report"

coverage_summarize_files "$files_report" "$summary_report"
coverage_enforce_floor "$summary_report" 85 luminated
