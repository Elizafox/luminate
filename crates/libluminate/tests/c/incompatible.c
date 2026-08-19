/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

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

static void require_error_contains(const char *needle, const char *context)
{
    const char *error = luminate_last_error_message();
    if (error == NULL || strstr(error, needle) == NULL)
    {
        fprintf(stderr, "%s: unexpected last error: %s\n", context, error ? error : "(none)");
        exit(1);
    }
}

int main(int argc, char **argv)
{
    if (argc != 4)
    {
        fprintf(stderr, "usage: %s INCOMPATIBLE_PRIMARY PRIMARY_SOCKET EVENT_SOCKET\n", argv[0]);
        return 2;
    }

    struct LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client),
                   LUMINATE_STATUS_INCOMPATIBLE_DAEMON, "incompatible primary socket");
    if (client != NULL)
    {
        fprintf(stderr, "incompatible daemon populated client handle\n");
        return 1;
    }
    require_error_contains("daemon is incompatible", "primary diagnostic");
    LuminateStringView daemon_version = luminate_last_error_incompatible_daemon_version();
    LuminateStringView reason = luminate_last_error_incompatibility_reason();
    uint32_t protocol_version = 0;
    uint32_t event_version = 77;
    if (!luminate_string_view_is_present(daemon_version) || daemon_version.len != 24 ||
        memcmp(daemon_version.data, "mock-incompatible-daemon", daemon_version.len) != 0 ||
        !luminate_string_view_is_present(reason) || reason.len != 30 ||
        memcmp(reason.data, "mock primary protocol mismatch", reason.len) != 0 ||
        !luminate_last_error_supported_protocol_abi_version(&protocol_version) ||
        protocol_version != 31 ||
        luminate_last_error_supported_event_protocol_version(&event_version) ||
        event_version != 77)
    {
        fprintf(stderr, "incomplete primary incompatibility metadata\n");
        return 1;
    }

    require_status(luminate_client_connect_path(argv[2], &client), LUMINATE_STATUS_OK,
                   "compatible primary socket");
    struct LuminateEventSubscription *subscription = NULL;
    require_status(luminate_client_subscribe_path(client, argv[3], &subscription),
                   LUMINATE_STATUS_INCOMPATIBLE_EVENT_SOCKET, "incompatible event socket");
    if (subscription != NULL)
    {
        fprintf(stderr, "incompatible event socket populated subscription handle\n");
        return 1;
    }
    require_error_contains("event socket is incompatible", "event diagnostic");
    daemon_version = luminate_last_error_incompatible_daemon_version();
    reason = luminate_last_error_incompatibility_reason();
    protocol_version = 88;
    event_version = 0;
    if (!luminate_string_view_is_present(daemon_version) || daemon_version.len != 24 ||
        memcmp(daemon_version.data, "mock-incompatible-daemon", daemon_version.len) != 0 ||
        !luminate_string_view_is_present(reason) || reason.len != 28 ||
        memcmp(reason.data, "mock event protocol mismatch", reason.len) != 0 ||
        luminate_last_error_supported_protocol_abi_version(&protocol_version) ||
        protocol_version != 88 ||
        !luminate_last_error_supported_event_protocol_version(&event_version) ||
        event_version != 9)
    {
        fprintf(stderr, "incomplete event incompatibility metadata\n");
        return 1;
    }
    luminate_client_free(client);
    return 0;
}
