/* SPDX-License-Identifier: GPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include <stdio.h>
#include <stdlib.h>
#include <threads.h>

#include "luminate.h"

struct Completion
{
    mtx_t mutex;
    cnd_t changed;
    int context_destroyed;
    LuminateStatus status;
};

static void ping_complete(void *raw_context, const LuminateAsyncOperation *operation,
                          LuminateStatus status)
{
    struct Completion *completion = raw_context;
    LuminateStatus published = LUMINATE_STATUS_OK;

    if (!luminate_async_operation_status(operation, &published) || published != status)
    {
        abort();
    }

    mtx_lock(&completion->mutex);
    completion->status = status;
    mtx_unlock(&completion->mutex);
}

static void completion_free(void *raw_context)
{
    struct Completion *completion = raw_context;

    mtx_lock(&completion->mutex);
    completion->context_destroyed = 1;
    cnd_signal(&completion->changed);
    mtx_unlock(&completion->mutex);
}

int main(int argc, char **argv)
{
    LuminateClient *client = NULL;
    LuminateAsyncOperation *operation = NULL;
    struct Completion completion = {0};

    if (mtx_init(&completion.mutex, mtx_plain) != thrd_success ||
        cnd_init(&completion.changed) != thrd_success)
    {
        return EXIT_FAILURE;
    }

    LuminateStatus status = argc == 2 ? luminate_client_connect_path(argv[1], &client)
                                      : luminate_client_connect(&client);
    if (status != LUMINATE_STATUS_OK)
    {
        fprintf(stderr, "connect failed: %s\n", luminate_last_error_message());
        return EXIT_FAILURE;
    }

    status = luminate_client_ping_async(client, &completion, completion_free, ping_complete,
                                        &operation);
    if (status != LUMINATE_STATUS_OK)
    {
        fprintf(stderr, "ping submission failed: %s\n", luminate_last_error_message());
        luminate_client_free(client);
        return EXIT_FAILURE;
    }

    /* The accepted operation keeps its client execution domain alive. */
    luminate_client_free(client);

    mtx_lock(&completion.mutex);
    while (!completion.context_destroyed)
    {
        cnd_wait(&completion.changed, &completion.mutex);
    }
    status = completion.status;
    mtx_unlock(&completion.mutex);

    if (status != LUMINATE_STATUS_OK)
    {
        LuminateStringView message = luminate_async_operation_error_message(operation);
        fprintf(stderr, "ping failed: %.*s\n", (int)message.len, message.data);
    }

    luminate_async_operation_release(operation);
    cnd_destroy(&completion.changed);
    mtx_destroy(&completion.mutex);
    return status == LUMINATE_STATUS_OK ? EXIT_SUCCESS : EXIT_FAILURE;
}
