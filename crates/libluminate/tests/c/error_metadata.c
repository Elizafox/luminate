/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require_status(LuminateStatus actual, LuminateStatus expected, const char *context)
{
    if (actual != expected)
    {
        fprintf(stderr, "%s: expected %d, got %d: %s\n", context, expected, actual,
                luminate_last_error_message() ? luminate_last_error_message() : "(none)");
        exit(1);
    }
}

int main(int argc, char **argv)
{
    if (argc != 2)
    {
        fprintf(stderr, "usage: %s SOCKET\n", argv[0]);
        return 2;
    }

    struct LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client), LUMINATE_STATUS_OK, "connect");
    LuminateTarget target = luminate_target_device("keyboard");

    require_status(luminate_client_set_off(client, &target), LUMINATE_STATUS_RATE_LIMITED,
                   "rate limited mutation");
    uint64_t retry_after_ms = 0;
    if (!luminate_last_error_retry_after_ms(&retry_after_ms) || retry_after_ms != 375)
    {
        fprintf(stderr, "missing retry guidance\n");
        return 1;
    }
    if (luminate_last_error_retry_after_ms(NULL))
    {
        fprintf(stderr, "null retry output unexpectedly succeeded\n");
        return 1;
    }

    size_t needed = luminate_copy_last_error_message(NULL, 0);
    char truncated[8] = {0};
    if (needed <= sizeof(truncated) ||
        luminate_copy_last_error_message(truncated, sizeof(truncated)) != needed ||
        truncated[sizeof(truncated) - 1] != '\0')
    {
        fprintf(stderr, "error truncation contract was not preserved\n");
        return 1;
    }

    require_status(luminate_client_set_off(client, &target), LUMINATE_STATUS_PARTIAL_MUTATION,
                   "partial mutation");
    if (luminate_last_error_applied_target_count() != 1)
    {
        fprintf(stderr, "unexpected applied target count\n");
        return 1;
    }
    LuminateTarget applied = {0};
    if (!luminate_last_error_applied_target(0, &applied) || applied.device_id == NULL ||
        applied.surface_id == NULL || applied.element_id == NULL || applied.group_id != NULL ||
        strcmp(applied.device_id, "keyboard") != 0 || strcmp(applied.surface_id, "keys") != 0 ||
        strcmp(applied.element_id, "escape") != 0)
    {
        fprintf(stderr, "unexpected applied target metadata\n");
        return 1;
    }
    if (luminate_last_error_applied_target(1, &applied) ||
        luminate_last_error_applied_target(0, NULL))
    {
        fprintf(stderr, "invalid applied target lookup unexpectedly succeeded\n");
        return 1;
    }

    require_status(luminate_client_set_off(client, &target), LUMINATE_STATUS_PERMISSION_DENIED,
                   "denied mutation");
    LuminateStringView denied_reason = luminate_last_error_permission_denied_reason();
    if (!luminate_string_view_is_present(denied_reason) || denied_reason.len != 18 ||
        memcmp(denied_reason.data, "safe policy reason", denied_reason.len) != 0)
    {
        fprintf(stderr, "missing safe permission-denial reason\n");
        return 1;
    }
    if (luminate_string_view_is_present(luminate_last_error_incompatible_daemon_version()) ||
        luminate_string_view_is_present(luminate_last_error_incompatibility_reason()))
    {
        fprintf(stderr, "denied mutation exposed incompatibility metadata\n");
        return 1;
    }

    luminate_client_free(client);
    return 0;
}
