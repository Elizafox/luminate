// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <assert.h>
#include <pthread.h>
#include <stdbool.h>

typedef struct CompletionContext
{
    pthread_mutex_t mutex;
    pthread_cond_t changed;
    bool callback_finished;
    bool context_destroyed;
    LuminateStatus status;
} CompletionContext;

typedef struct PayloadContext
{
    pthread_mutex_t mutex;
    pthread_cond_t changed;
    bool context_destroyed;
    LuminateStatus status;
    LuminateTopologySnapshot *snapshot;
} PayloadContext;

static void topology_complete(void *raw_context, const LuminateAsyncOperation *operation,
                              LuminateStatus status, LuminateTopologySnapshot *snapshot)
{
    PayloadContext *context = raw_context;

    assert(operation != NULL);
    assert(status == LUMINATE_STATUS_OK);
    assert(snapshot != NULL);

    assert(pthread_mutex_lock(&context->mutex) == 0);
    context->status = status;
    context->snapshot = snapshot;
    assert(pthread_cond_broadcast(&context->changed) == 0);
    assert(pthread_mutex_unlock(&context->mutex) == 0);
}

static void payload_context_free(void *raw_context)
{
    PayloadContext *context = raw_context;

    assert(pthread_mutex_lock(&context->mutex) == 0);
    assert(context->snapshot != NULL);
    context->context_destroyed = true;
    assert(pthread_cond_broadcast(&context->changed) == 0);
    assert(pthread_mutex_unlock(&context->mutex) == 0);
}

static void next_complete(void *raw_context, const LuminateAsyncOperation *operation,
                          LuminateStatus status, LuminateEvent *event)
{
    CompletionContext *context = raw_context;
    LuminateStatus published = LUMINATE_STATUS_OK;

    assert(operation != NULL);
    assert(event == NULL);
    assert(luminate_async_operation_status(operation, &published));
    assert(published == status);

    assert(pthread_mutex_lock(&context->mutex) == 0);
    context->status = status;
    context->callback_finished = true;
    assert(pthread_cond_broadcast(&context->changed) == 0);
    assert(pthread_mutex_unlock(&context->mutex) == 0);
}

static void context_free(void *raw_context)
{
    CompletionContext *context = raw_context;

    assert(pthread_mutex_lock(&context->mutex) == 0);
    assert(context->callback_finished);
    context->context_destroyed = true;
    assert(pthread_cond_broadcast(&context->changed) == 0);
    assert(pthread_mutex_unlock(&context->mutex) == 0);
}

int main(int argc, char **argv)
{
    assert(argc == 2);

    LuminateClient *client = NULL;
    assert(luminate_client_connect_path(argv[1], &client) == LUMINATE_STATUS_OK);

    LuminateEventSubscription *subscription = NULL;
    assert(luminate_client_subscribe(client, &subscription) == LUMINATE_STATUS_OK);

    CompletionContext context = {0};
    assert(pthread_mutex_init(&context.mutex, NULL) == 0);
    assert(pthread_cond_init(&context.changed, NULL) == 0);

    PayloadContext payload_context = {0};
    assert(pthread_mutex_init(&payload_context.mutex, NULL) == 0);
    assert(pthread_cond_init(&payload_context.changed, NULL) == 0);

    LuminateAsyncOperation *payload_operation = NULL;
    assert(luminate_client_list_devices_async(client, &payload_context, payload_context_free,
                                              topology_complete,
                                              &payload_operation) == LUMINATE_STATUS_OK);
    assert(payload_operation != NULL);

    LuminateAsyncOperation *operation = NULL;
    assert(luminate_event_subscription_next_async(subscription, &context, context_free,
                                                  next_complete,
                                                  &operation) == LUMINATE_STATUS_OK);
    assert(operation != NULL);

    /* The subscription and operation retain the originating client domain. */
    luminate_client_free(client);
    client = NULL;

    assert(luminate_async_operation_cancel(operation) == LUMINATE_ASYNC_CANCEL_ACCEPTED);

    assert(pthread_mutex_lock(&context.mutex) == 0);
    while (!context.context_destroyed)
    {
        assert(pthread_cond_wait(&context.changed, &context.mutex) == 0);
    }
    assert(context.status == LUMINATE_STATUS_CANCELLED);
    assert(pthread_mutex_unlock(&context.mutex) == 0);

    LuminateStatus published = LUMINATE_STATUS_OK;
    assert(luminate_async_operation_status(operation, &published));
    assert(published == LUMINATE_STATUS_CANCELLED);

    assert(pthread_mutex_lock(&payload_context.mutex) == 0);
    while (!payload_context.context_destroyed)
    {
        assert(pthread_cond_wait(&payload_context.changed, &payload_context.mutex) == 0);
    }
    assert(payload_context.status == LUMINATE_STATUS_OK);
    assert(payload_context.snapshot != NULL);
    LuminateTopologySnapshot *snapshot = payload_context.snapshot;
    payload_context.snapshot = NULL;
    assert(pthread_mutex_unlock(&payload_context.mutex) == 0);

    luminate_topology_snapshot_free(snapshot);

    luminate_async_operation_release(payload_operation);
    luminate_async_operation_release(operation);
    luminate_event_subscription_free(subscription);
    assert(pthread_cond_destroy(&payload_context.changed) == 0);
    assert(pthread_mutex_destroy(&payload_context.mutex) == 0);
    assert(pthread_cond_destroy(&context.changed) == 0);
    assert(pthread_mutex_destroy(&context.mutex) == 0);
    return 0;
}
