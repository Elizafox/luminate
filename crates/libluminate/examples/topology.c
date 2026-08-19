/* SPDX-License-Identifier: GPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include <stdio.h>
#include <stdlib.h>

#include "luminate.h"

static int report_error(const char *operation, LuminateStatus status)
{
    const char *message = luminate_last_error_message();
    fprintf(stderr, "%s failed (%d): %s\n", operation, (int)status,
            message != NULL ? message : "no detail");
    return EXIT_FAILURE;
}

int main(int argc, char **argv)
{
    struct LuminateClient *client = NULL;
    struct LuminateServerInfo *server = NULL;
    LuminateTopologySnapshot *topology = NULL;

    LuminateStatus status = argc == 2 ? luminate_client_connect_path(argv[1], &client)
                                      : luminate_client_connect(&client);
    if (status != LUMINATE_STATUS_OK)
    {
        return report_error("connect", status);
    }

    status = luminate_client_server_info(client, &server);
    if (status != LUMINATE_STATUS_OK)
    {
        luminate_client_free(client);
        return report_error("server info", status);
    }
    LuminateStringView daemon_name = luminate_server_info_daemon_name(server);
    LuminateStringView daemon_version = luminate_server_info_daemon_version(server);
    fwrite(daemon_name.data, 1, daemon_name.len, stdout);
    putchar(' ');
    fwrite(daemon_version.data, 1, daemon_version.len, stdout);
    printf(" (protocol ABI %u)\n", luminate_server_info_protocol_abi_version(server));
    luminate_server_info_free(server);

    status = luminate_client_list_devices(client, &topology);
    if (status != LUMINATE_STATUS_OK)
    {
        luminate_client_free(client);
        return report_error("topology", status);
    }
    for (uintptr_t i = 0; i < luminate_topology_snapshot_device_count(topology); ++i)
    {
        const LuminateDevice *device = luminate_topology_snapshot_device_at(topology, i);
        LuminateStringView id = luminate_device_id(device);
        LuminateStringView name = luminate_device_name(device);
        printf("%.*s: %.*s\n", (int)id.len, id.data, (int)name.len, name.data);
    }
    luminate_topology_snapshot_free(topology);
    luminate_client_free(client);
    return EXIT_SUCCESS;
}
