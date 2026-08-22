// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#define _POSIX_C_SOURCE 200809L

#include "luminate.h"

#include <assert.h>
#include <stdbool.h>
#include <time.h>

static void no_context_free(void *context) { (void)context; }

static void wait_for_completion(LuminateAsyncOperation *operation);

static void status_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status)
{
    (void)context;
    (void)operation;
    (void)status;
}

static void async_client_connected(void *raw_context, const LuminateAsyncOperation *operation,
                                   LuminateStatus status, LuminateClient *connected_client)
{
    (void)raw_context;
    (void)operation;

    if (status == LUMINATE_STATUS_OK && connected_client != NULL)
    {
        luminate_client_free(connected_client);
    }
}

static void server_info_complete(void *context, const LuminateAsyncOperation *operation,
                                 LuminateStatus status, LuminateServerInfo *snapshot)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && snapshot != NULL)
    {
        luminate_server_info_free(snapshot);
    }
}

static void topology_complete(void *context, const LuminateAsyncOperation *operation,
                              LuminateStatus status, LuminateTopologySnapshot *snapshot)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && snapshot != NULL)
    {
        luminate_topology_snapshot_free(snapshot);
    }
}

static void withdrawn_devices_complete(void *context, const LuminateAsyncOperation *operation,
                                       LuminateStatus status,
                                       LuminateWithdrawnDeviceList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void string_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status, char *payload)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && payload != NULL)
    {
        luminate_string_free(payload);
    }
}

static void collection_list_complete(void *context, const LuminateAsyncOperation *operation,
                                     LuminateStatus status, LuminateCollectionList *payload)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && payload != NULL)
    {
        luminate_collection_list_free(payload);
    }
}

static void collection_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                         LuminateStatus status,
                                         LuminateCollectionSnapshot *payload)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && payload != NULL)
    {
        luminate_collection_snapshot_free(payload);
    }
}

static void subscription_complete(void *context, const LuminateAsyncOperation *operation,
                                  LuminateStatus status, LuminateEventSubscription *subscription)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && subscription != NULL)
    {
        luminate_event_subscription_free(subscription);
    }
}

static void subscription_baseline_complete(void *context, const LuminateAsyncOperation *operation,
                                           LuminateStatus status,
                                           LuminateEventSubscription *subscription,
                                           LuminateTopologySnapshot *snapshot)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && subscription != NULL)
    {
        luminate_event_subscription_free(subscription);
    }
    if (status == LUMINATE_STATUS_OK && snapshot != NULL)
    {
        luminate_topology_snapshot_free(snapshot);
    }
}

static void plugin_setup_workflows_complete(void *context,
                                            const LuminateAsyncOperation *operation,
                                            LuminateStatus status,
                                            LuminatePluginSetupWorkflowList *payload)
{
    (void)context;
    (void)operation;
    if (status == LUMINATE_STATUS_OK && payload != NULL)
    {
        luminate_plugin_setup_workflow_list_free(payload);
    }
}

static void wait_for_completion(LuminateAsyncOperation *operation)
{
    const LuminateAsyncOperation *const_handle = operation;
    const struct timespec delay = {0, 1000000};
    LuminateStatus status = LUMINATE_STATUS_OK;
    bool complete = false;
    for (unsigned int attempt = 0; attempt < 4000; ++attempt)
    {
        complete = luminate_async_operation_status(const_handle, &status);
        if (complete)
        {
            break;
        }
        nanosleep(&delay, NULL);
    }
    assert(complete);
    (void)status;
    luminate_async_operation_release(operation);
}

int main(int argc, char **argv)
{
    assert(argc == 2);
    assert(argv[1] != NULL);

    LuminateAsyncOperation *operation = NULL;
    LuminateClient *client = NULL;
    LuminateStatus status = LUMINATE_STATUS_OK;

    status = luminate_client_list_devices_async(NULL, NULL, no_context_free, topology_complete,
                                                &operation);
    assert(status == LUMINATE_STATUS_NULL_POINTER);
    operation = NULL;

    status = luminate_client_connect_path(argv[1], &client);
    assert(status == LUMINATE_STATUS_OK);
    assert(client != NULL);

    status = luminate_client_connect_path_async(argv[1], NULL, no_context_free,
                                                async_client_connected, &operation);
    assert(status == LUMINATE_STATUS_OK);
    assert(operation != NULL);
    const struct timespec spin_delay = {0, 1000000};
    for (unsigned int attempt = 0; attempt < 20; ++attempt)
    {
        (void)attempt;
        assert(nanosleep(&spin_delay, NULL) == 0);
    }
    const LuminateAsyncCancelResult cancel_result = luminate_async_operation_cancel(operation);
    assert(cancel_result == LUMINATE_ASYNC_CANCEL_ACCEPTED ||
           cancel_result == LUMINATE_ASYNC_CANCEL_ALREADY_COMPLETED ||
           cancel_result == LUMINATE_ASYNC_CANCEL_ALREADY_CANCELLED);
    luminate_async_operation_release(operation);
    operation = NULL;

    status = luminate_client_server_info_async(client, NULL, no_context_free,
                                               server_info_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_list_withdrawn_devices_async(client, NULL, no_context_free,
                                                          withdrawn_devices_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_create_collection_async(client, "name", "description", "kind", NULL,
                                                     0, NULL, NULL, string_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_get_collection_async(client, "id", NULL, NULL,
                                                  collection_snapshot_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_list_collections_async(client, NULL, NULL, collection_list_complete,
                                                    &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_list_devices_async(client, NULL, no_context_free, topology_complete,
                                                &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status =
        luminate_client_ping_async(client, NULL, no_context_free, status_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_subscribe_async(client, NULL, no_context_free, subscription_complete,
                                             &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    char *event_socket = NULL;
    status = luminate_client_event_socket_path(client, &event_socket);
    assert(status == LUMINATE_STATUS_OK);
    assert(event_socket != NULL);
    status = luminate_client_subscribe_path_async(client, event_socket, NULL, no_context_free,
                                                  subscription_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_subscribe_with_baseline_async(
        client, NULL, no_context_free, subscription_baseline_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    status = luminate_client_subscribe_with_baseline_path_async(
        client, event_socket, NULL, no_context_free, subscription_baseline_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;
    luminate_string_free(event_socket);
    event_socket = NULL;

    status = luminate_client_plugin_setup_workflows_async(
        client, "plugin", NULL, no_context_free, plugin_setup_workflows_complete, &operation);
    assert(status == LUMINATE_STATUS_OK);
    wait_for_completion(operation);
    operation = NULL;

    luminate_client_free(client);
    return 0;
}
