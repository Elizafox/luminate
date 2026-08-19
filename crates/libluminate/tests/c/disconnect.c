/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>

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

    LuminateTopologySnapshot *topology = NULL;
    require_status(luminate_client_list_devices(client, &topology), LUMINATE_STATUS_UNAVAILABLE,
                   "request interrupted by daemon disconnect");
    if (topology != NULL)
    {
        fprintf(stderr, "failed request populated topology output\n");
        return 1;
    }

    require_status(luminate_client_list_devices(client, &topology), LUMINATE_STATUS_UNAVAILABLE,
                   "request after daemon disconnect");
    if (topology != NULL)
    {
        fprintf(stderr, "disconnected request populated topology output\n");
        return 1;
    }

    luminate_client_free(client);
    return 0;
}
