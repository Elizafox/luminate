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

int main(int argc, char **argv)
{
    if (argc != 3)
    {
        fprintf(stderr, "usage: %s PRIMARY_SOCKET EVENT_SOCKET\n", argv[0]);
        return 2;
    }

    struct LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client), LUMINATE_STATUS_OK, "connect");

    struct LuminateEventSubscription *subscription = NULL;
    LuminateTopologySnapshot *baseline = NULL;
    require_status(
        luminate_client_subscribe_with_baseline_path(client, argv[2], &subscription, &baseline),
        LUMINATE_STATUS_OK, "explicit event baseline");
    if (baseline == NULL || luminate_topology_snapshot_device_count(baseline) != 0)
    {
        fprintf(stderr, "unexpected typed baseline\n");
        return 1;
    }
    luminate_topology_snapshot_free(baseline);

    LuminateEvent *event = NULL;
    require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                   "explicit event next");
    LuminateStringView changed = luminate_event_topology_device_at(event, 0);
    if (changed.len != strlen("changed-device") ||
        memcmp(changed.data, "changed-device", changed.len) != 0)
    {
        fprintf(stderr, "unexpected typed event\n");
        return 1;
    }
    luminate_event_free(event);
    for (size_t i = 0; i < 3; ++i)
    {
        event = NULL;
        require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                       "remaining explicit event");
        luminate_event_free(event);
    }
    luminate_event_subscription_free(subscription);
    luminate_client_free(client);
    return 0;
}
