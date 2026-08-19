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
        const char *error = luminate_last_error_message();
        fprintf(stderr, "%s: expected status %d, got %d; last error: %s\n", context, expected,
                actual, error ? error : "(none)");
        exit(1);
    }
}

static bool view_equal(LuminateStringView view, const char *expected)
{
    size_t len = strlen(expected);
    return view.data != NULL && view.len == len && memcmp(view.data, expected, len) == 0;
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

    struct LuminateEventSubscription *subscription = NULL;
    LuminateTopologySnapshot *baseline = NULL;
    require_status(luminate_client_subscribe_with_baseline(client, &subscription, &baseline),
                   LUMINATE_STATUS_OK, "subscribe with baseline");
    if (subscription == NULL || baseline == NULL ||
        luminate_topology_snapshot_device_count(baseline) != 0)
    {
        fprintf(stderr, "invalid typed subscription baseline\n");
        return 1;
    }
    luminate_topology_snapshot_free(baseline);

    struct LuminateEventSubscription *second_subscription = NULL;
    require_status(luminate_client_subscribe(client, &second_subscription),
                   LUMINATE_STATUS_DAEMON_UNAVAILABLE, "closed mock daemon session");
    if (second_subscription != NULL)
    {
        fprintf(stderr, "second subscription populated a handle\n");
        return 1;
    }

    require_status(luminate_event_subscription_next(subscription, NULL),
                   LUMINATE_STATUS_NULL_POINTER, "null event json output");

    LuminateEvent *event = NULL;
    require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                   "next event");
    LuminateStringView changed = luminate_event_topology_device_at(event, 0);
    if (event == NULL || luminate_event_kind(event) != LUMINATE_EVENT_TOPOLOGY_CHANGED ||
        changed.len != strlen("changed-device") ||
        memcmp(changed.data, "changed-device", changed.len) != 0)
    {
        fprintf(stderr, "unexpected typed event\n");
        return 1;
    }
    luminate_event_free(event);

    event = NULL;
    require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                   "next state event");
    changed = luminate_event_state_device_at(event, 0);
    if (event == NULL || luminate_event_kind(event) != LUMINATE_EVENT_STATE_CHANGED ||
        luminate_event_state_device_count(event) != 1 ||
        luminate_event_topology_device_count(event) != 0 ||
        changed.len != strlen("changed-device") ||
        memcmp(changed.data, "changed-device", changed.len) != 0)
    {
        fprintf(stderr, "unexpected typed state event\n");
        return 1;
    }
    luminate_event_free(event);

    event = NULL;
    require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                   "next configuration event");
    const LuminateManagementChangeSetView *changes = luminate_event_configuration_changes(event);
    const LuminateManagementChange *change = luminate_management_change_set_view_at(changes, 0);
    if (event == NULL || luminate_event_kind(event) != LUMINATE_EVENT_CONFIGURATION_CHANGED ||
        changes == NULL || luminate_management_change_set_view_revision(changes) != 8 ||
        luminate_management_change_set_view_count(changes) != 1 || change == NULL ||
        luminate_management_change_kind(change) != LUMINATE_MANAGEMENT_CHANGE_PLUGIN_ACTIVATION ||
        !view_equal(luminate_management_change_plugin(change), "fixture"))
    {
        fprintf(stderr, "unexpected typed configuration event\n");
        return 1;
    }
    luminate_event_free(event);

    event = NULL;
    require_status(luminate_event_subscription_next(subscription, &event), LUMINATE_STATUS_OK,
                   "next shared-memory event");
    const LuminateTargetView *target = luminate_event_shm_stream_target(event);
    uint32_t stream_generation = 0;
    if (event == NULL || luminate_event_kind(event) != LUMINATE_EVENT_SHM_STREAM_ENDED ||
        target == NULL || !luminate_event_shm_stream_generation(event, &stream_generation) ||
        stream_generation != 9 || luminate_target_view_kind(target) != LUMINATE_TARGET_SURFACE ||
        !luminate_string_view_equal(luminate_target_view_device_id(target),
                                    (LuminateStringView){"changed-device", 14}) ||
        !luminate_string_view_equal(luminate_target_view_surface_id(target),
                                    (LuminateStringView){"panel", 5}))
    {
        fprintf(stderr, "unexpected typed shared-memory event\n");
        return 1;
    }
    luminate_event_free(event);
    luminate_event_subscription_free(subscription);
    luminate_event_subscription_free(NULL);

    require_status(luminate_effect_create_hardware(NULL, NULL), LUMINATE_STATUS_NULL_POINTER,
                   "invalid effect builder pointers");

    luminate_client_free(client);
    luminate_client_free(NULL);
    luminate_string_free(NULL);
    luminate_server_info_free(NULL);
    luminate_effect_free(NULL);
    luminate_event_free(NULL);
    luminate_topology_snapshot_free(NULL);
    return 0;
}
