#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

set -eu

fixture_dir=${0%/*}
input=${fixture_dir}/provider-input
output=${fixture_dir}/provider-output

exec 3<&0 4>&1
rm -f "${input}" "${output}"
mkfifo "${input}" "${output}"
cat <&3 >"${input}" &
cat "${output}" >&4 &

LUMINATE_PROVIDER_TEST_CASE=$(cat "${fixture_dir}/case") \
LUMINATE_PROVIDER_TEST_DIR=${fixture_dir} \
LUMINATE_PROVIDER_TEST_INPUT=${input} \
LUMINATE_PROVIDER_TEST_OUTPUT=${output} \
    "${fixture_dir}/test-binary" \
    --ignored \
    --exact authentication_provider::tests::provider_child_probe \
    --test-threads=1 \
    >"${fixture_dir}/harness.log" 2>&1

wait
