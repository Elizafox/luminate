// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <assert.h>
#include <stdbool.h>
#include <threads.h>

typedef struct CompletionContext
{
    mtx_t mutex;
    cnd_t changed;
    bool callback_finished;
    bool context_destroyed;
    LuminateStatus status;
} CompletionContext;

typedef struct PayloadContext
{
    mtx_t mutex;
    cnd_t changed;
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

    assert(mtx_lock(&context->mutex) == thrd_success);
    context->status = status;
    context->snapshot = snapshot;
    assert(cnd_broadcast(&context->changed) == thrd_success);
    assert(mtx_unlock(&context->mutex) == thrd_success);
}

static void payload_context_free(void *raw_context)
{
    PayloadContext *context = raw_context;

    assert(mtx_lock(&context->mutex) == thrd_success);
    assert(context->snapshot != NULL);
    context->context_destroyed = true;
    assert(cnd_broadcast(&context->changed) == thrd_success);
    assert(mtx_unlock(&context->mutex) == thrd_success);
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

    assert(mtx_lock(&context->mutex) == thrd_success);
    context->status = status;
    context->callback_finished = true;
    assert(cnd_broadcast(&context->changed) == thrd_success);
    assert(mtx_unlock(&context->mutex) == thrd_success);
}

static void context_free(void *raw_context)
{
    CompletionContext *context = raw_context;

    assert(mtx_lock(&context->mutex) == thrd_success);
    assert(context->callback_finished);
    context->context_destroyed = true;
    assert(cnd_broadcast(&context->changed) == thrd_success);
    assert(mtx_unlock(&context->mutex) == thrd_success);
}

int main(int argc, char **argv)
{
    assert(argc == 2);

    LuminateClient *client = NULL;
    assert(luminate_client_connect_path(argv[1], &client) == LUMINATE_STATUS_OK);

    LuminateEventSubscription *subscription = NULL;
    assert(luminate_client_subscribe(client, &subscription) == LUMINATE_STATUS_OK);

    CompletionContext context = {0};
    assert(mtx_init(&context.mutex, mtx_plain) == thrd_success);
    assert(cnd_init(&context.changed) == thrd_success);

    PayloadContext payload_context = {0};
    assert(mtx_init(&payload_context.mutex, mtx_plain) == thrd_success);
    assert(cnd_init(&payload_context.changed) == thrd_success);

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

    assert(mtx_lock(&context.mutex) == thrd_success);
    while (!context.context_destroyed)
    {
        assert(cnd_wait(&context.changed, &context.mutex) == thrd_success);
    }
    assert(context.status == LUMINATE_STATUS_CANCELLED);
    assert(mtx_unlock(&context.mutex) == thrd_success);

    LuminateStatus published = LUMINATE_STATUS_OK;
    assert(luminate_async_operation_status(operation, &published));
    assert(published == LUMINATE_STATUS_CANCELLED);

    assert(mtx_lock(&payload_context.mutex) == thrd_success);
    while (!payload_context.context_destroyed)
    {
        assert(cnd_wait(&payload_context.changed, &payload_context.mutex) == thrd_success);
    }
    assert(payload_context.status == LUMINATE_STATUS_OK);
    assert(payload_context.snapshot != NULL);
    LuminateTopologySnapshot *snapshot = payload_context.snapshot;
    payload_context.snapshot = NULL;
    assert(mtx_unlock(&payload_context.mutex) == thrd_success);

    luminate_topology_snapshot_free(snapshot);

    luminate_async_operation_release(payload_operation);
    luminate_async_operation_release(operation);
    luminate_event_subscription_free(subscription);
    cnd_destroy(&payload_context.changed);
    mtx_destroy(&payload_context.mutex);
    cnd_destroy(&context.changed);
    mtx_destroy(&context.mutex);
    return 0;
}
