/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void)
{
    const char *version = luminate_version();
    if (version == NULL || strcmp(version, "0.1.0") != 0)
    {
        fprintf(stderr, "unexpected library version: %s\n", version ? version : "(null)");
        return 1;
    }

    LuminateStatus status = luminate_client_connect_path("/unused", NULL);
    if (status != LUMINATE_STATUS_NULL_POINTER)
    {
        fprintf(stderr, "expected LUMINATE_STATUS_NULL_POINTER, got %d\n", status);
        return 1;
    }

    size_t needed = luminate_copy_last_error_message(NULL, 0);
    if (needed <= 1)
    {
        fprintf(stderr, "staged library did not expose an error diagnostic\n");
        return 1;
    }
    char *message = calloc(needed, 1);
    if (message == NULL)
    {
        perror("calloc");
        return 1;
    }
    if (luminate_copy_last_error_message(message, needed) != needed ||
        strstr(message, "client output pointer is null") == NULL)
    {
        fprintf(stderr, "unexpected staged-library diagnostic: %s\n", message);
        free(message);
        return 1;
    }
    free(message);
    return 0;
}
