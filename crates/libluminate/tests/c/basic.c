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

static void require_string(const char *actual, const char *expected, const char *context)
{
    if (actual == NULL || strcmp(actual, expected) != 0)
    {
        fprintf(stderr, "%s: expected %s, got %s\n", context, expected,
                actual ? actual : "(null)");
        exit(1);
    }
}

static void require_view(LuminateStringView actual, const char *expected, const char *context)
{
    size_t expected_len = strlen(expected);
    if (actual.data == NULL || actual.len != expected_len ||
        memcmp(actual.data, expected, expected_len) != 0)
    {
        fprintf(stderr, "%s: unexpected string view\n", context);
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

    require_string(luminate_version(), "0.1.0", "lib version");

    LuminateStatus status = luminate_client_connect_path(argv[1], NULL);
    require_status(status, LUMINATE_STATUS_NULL_POINTER, "null client out pointer");

    size_t needed = luminate_copy_last_error_message(NULL, 0);
    if (needed <= 1)
    {
        fprintf(stderr, "expected last error to need more than a terminator\n");
        return 1;
    }
    char *error_buf = calloc(needed, 1);
    if (error_buf == NULL)
    {
        perror("calloc");
        return 1;
    }
    size_t copied = luminate_copy_last_error_message(error_buf, needed);
    if (copied != needed || strstr(error_buf, "client output pointer is null") == NULL)
    {
        fprintf(stderr, "unexpected copied error: %s\n", error_buf);
        return 1;
    }
    free(error_buf);

    struct LuminateClient *client = NULL;
    status = luminate_client_connect_path(argv[1], &client);
    require_status(status, LUMINATE_STATUS_OK, "connect");
    if (client == NULL)
    {
        fprintf(stderr, "connect returned LUMINATE_STATUS_OK with null client\n");
        return 1;
    }

    for (unsigned int cycle = 0; cycle < 3; ++cycle)
    {
        char *daemon_version = NULL;
        status = luminate_client_daemon_version(client, &daemon_version);
        require_status(status, LUMINATE_STATUS_OK, "daemon version");
        require_string(daemon_version, "mock-daemon-0.1", "daemon version");
        luminate_string_free(daemon_version);

        char *socket_path = NULL;
        status = luminate_client_socket_path(client, &socket_path);
        require_status(status, LUMINATE_STATUS_OK, "socket path");
        require_string(socket_path, argv[1], "socket path");
        luminate_string_free(socket_path);

        char *event_socket_path = NULL;
        status = luminate_client_event_socket_path(client, &event_socket_path);
        require_status(status, LUMINATE_STATUS_OK, "event socket path");
        if (event_socket_path == NULL || event_socket_path[0] == '\0' ||
            strcmp(event_socket_path, argv[1]) == 0)
        {
            fprintf(stderr, "unexpected event socket path\n");
            return 1;
        }
        luminate_string_free(event_socket_path);

        struct LuminateServerInfo *info = NULL;
        status = luminate_client_server_info(client, &info);
        require_status(status, LUMINATE_STATUS_OK, "server info");
        require_view(luminate_server_info_daemon_name(info), "luminated-mock", "daemon name");
        require_view(luminate_server_info_daemon_version(info), "mock-daemon-0.1",
                     "info daemon version");
        if (luminate_server_info_protocol_abi_version(info) == 0)
        {
            fprintf(stderr, "unexpected server protocol ABI version\n");
            return 1;
        }
        luminate_server_info_free(info);
    }

    LuminateTopologySnapshot *topology = NULL;
    status = luminate_client_list_devices(client, &topology);
    require_status(status, LUMINATE_STATUS_OK, "list devices");
    if (topology == NULL || luminate_topology_snapshot_device_count(topology) != 0 ||
        luminate_topology_snapshot_device_at(topology, 0) != NULL)
    {
        fprintf(stderr, "unexpected typed topology baseline\n");
        return 1;
    }
    luminate_topology_snapshot_free(topology);

    LuminateWithdrawnDeviceList *withdrawn = NULL;
    status = luminate_client_list_withdrawn_devices(client, &withdrawn);
    require_status(status, LUMINATE_STATUS_OK, "list withdrawn devices");
    LuminateStringView withdrawn_id = luminate_withdrawn_device_list_at(withdrawn, 0);
    if (withdrawn == NULL || luminate_withdrawn_device_list_count(withdrawn) != 1 ||
        withdrawn_id.len != strlen("retired-device") ||
        memcmp(withdrawn_id.data, "retired-device", withdrawn_id.len) != 0)
    {
        fprintf(stderr, "unexpected withdrawn device list\n");
        return 1;
    }
    luminate_withdrawn_device_list_free(withdrawn);

    LuminateDeviceSnapshot *device = NULL;
    status = luminate_client_get_device(client, "missing", &device);
    require_status(status, LUMINATE_STATUS_NOT_FOUND, "missing device");

    LuminateStateSnapshot *state = NULL;
    status = luminate_client_get_state(client, "missing", &state);
    require_status(status, LUMINATE_STATUS_NOT_FOUND, "missing device state");

    status = luminate_client_purge_withdrawn_device(client, "retired-device");
    require_status(status, LUMINATE_STATUS_OK, "purge withdrawn device");

    LuminateTarget target = luminate_target_element("demo-keyboard", "zones", "g1");
    LuminateColourChannelInput static_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_RED, 12},
        {LUMINATE_COLOUR_CHANNEL_GREEN, 34},
        {LUMINATE_COLOUR_CHANNEL_BLUE, 56},
    };
    LuminateColourInput static_colour = {
        .encoding = LUMINATE_COLOUR_ENCODING_ADDITIVE,
        .channels = static_channels,
        .channel_count = 3,
    };
    LuminateEffect *effect = NULL;
    require_status(luminate_effect_create_static(&static_colour, &effect), LUMINATE_STATUS_OK,
                   "create static effect");
    status = luminate_client_set_effect(client, &target, effect);
    require_status(status, LUMINATE_STATUS_OK, "set static effect");

    status = luminate_client_set_brightness(client, &target, 80);
    require_status(status, LUMINATE_STATUS_OK, "set brightness");

    status = luminate_client_set_effect(client, &target, effect);
    require_status(status, LUMINATE_STATUS_OK, "set effect");

    status = luminate_client_set_off(client, &target);
    require_status(status, LUMINATE_STATUS_OK, "set off");

    status = luminate_client_clear_target(client, &target);
    require_status(status, LUMINATE_STATUS_OK, "clear target");

    target = (LuminateTarget){"demo-keyboard", "zones", NULL, "all"};
    status = luminate_client_set_effect(client, &target, effect);
    require_status(status, LUMINATE_STATUS_INVALID_ARGUMENT, "invalid mixed target");

    luminate_effect_free(effect);
    luminate_client_free(client);
    return 0;
}
