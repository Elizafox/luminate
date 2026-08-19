// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <assert.h>
#include <stdbool.h>
#include <string.h>

static bool view_equal(LuminateStringView view, const char *expected)
{
    const size_t expected_length = strlen(expected);
    return view.len == expected_length && memcmp(view.data, expected, expected_length) == 0;
}

int main(int argc, char **argv)
{
    assert(argc == 2);

    LuminateClient *client = NULL;
    assert(luminate_client_connect_path(argv[1], &client) == LUMINATE_STATUS_OK);
    assert(client != NULL);

    LuminateServerInfo *info = NULL;
    assert(luminate_client_server_info(client, &info) == LUMINATE_STATUS_OK);
    assert(info != NULL);
    assert(view_equal(luminate_server_info_daemon_name(info), "luminated-mock"));
    assert(view_equal(luminate_server_info_daemon_version(info), "mock-daemon-0.1"));
    assert(luminate_server_info_protocol_abi_version(info) != 0);
    luminate_server_info_free(info);

    assert(luminate_client_ping(client) == LUMINATE_STATUS_OK);

    LuminateTopologySnapshot *topology = NULL;
    assert(luminate_client_list_devices(client, &topology) == LUMINATE_STATUS_OK);
    assert(topology != NULL);
    assert(luminate_topology_snapshot_device_count(topology) == 0);
    assert(luminate_topology_snapshot_device_at(topology, 0) == NULL);
    luminate_topology_snapshot_free(topology);

    LuminateDeviceSnapshot *device = NULL;
    assert(luminate_client_get_device(client, "missing", &device) == LUMINATE_STATUS_NOT_FOUND);
    assert(device == NULL);

    luminate_client_free(client);
    return 0;
}
