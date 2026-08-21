/* SPDX-License-Identifier: GPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>

#include "luminate.h"

struct Completion
{
    pthread_mutex_t mutex;
    pthread_cond_t changed;
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

    pthread_mutex_lock(&completion->mutex);
    completion->status = status;
    pthread_mutex_unlock(&completion->mutex);
}

static void completion_free(void *raw_context)
{
    struct Completion *completion = raw_context;

    pthread_mutex_lock(&completion->mutex);
    completion->context_destroyed = 1;
    pthread_cond_signal(&completion->changed);
    pthread_mutex_unlock(&completion->mutex);
}

int main(int argc, char **argv)
{
    LuminateClient *client = NULL;
    LuminateAsyncOperation *operation = NULL;
    struct Completion completion = {0};

    if (pthread_mutex_init(&completion.mutex, NULL) != 0 ||
        pthread_cond_init(&completion.changed, NULL) != 0)
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

    pthread_mutex_lock(&completion.mutex);
    while (!completion.context_destroyed)
    {
        pthread_cond_wait(&completion.changed, &completion.mutex);
    }
    status = completion.status;
    pthread_mutex_unlock(&completion.mutex);

    if (status != LUMINATE_STATUS_OK)
    {
        LuminateStringView message = luminate_async_operation_error_message(operation);
        fprintf(stderr, "ping failed: %.*s\n", (int)message.len, message.data);
    }

    luminate_async_operation_release(operation);
    pthread_cond_destroy(&completion.changed);
    pthread_mutex_destroy(&completion.mutex);
    return status == LUMINATE_STATUS_OK ? EXIT_SUCCESS : EXIT_FAILURE;
}
