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
        return 2;
    }

    struct LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client),
                   LUMINATE_STATUS_DAEMON_UNAVAILABLE, "missing daemon socket");
    if (client != NULL)
    {
        fprintf(stderr, "failed connect populated client handle\n");
        return 1;
    }

    require_status(luminate_client_connect_path(NULL, &client), LUMINATE_STATUS_NULL_POINTER,
                   "null daemon path");
    const char invalid_utf8[] = {(char)0xff, '\0'};
    require_status(luminate_client_connect_path(invalid_utf8, &client),
                   LUMINATE_STATUS_INVALID_UTF8, "invalid UTF-8 daemon path");

    return 0;
}
