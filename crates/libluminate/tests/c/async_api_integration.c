// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <assert.h>
#include <stdbool.h>
#include <string.h>
#include <threads.h>

typedef struct Completion
{
    mtx_t mutex;
    cnd_t changed;
    bool done;
    LuminateStatus status;
    void *payload;
} Completion;

static bool view_equal(LuminateStringView view, const char *expected)
{
    const size_t expected_length = strlen(expected);
    return view.len == expected_length && memcmp(view.data, expected, expected_length) == 0;
}

static void completion_initialize(Completion *completion)
{
    assert(mtx_init(&completion->mutex, mtx_plain) == thrd_success);
    assert(cnd_init(&completion->changed) == thrd_success);
    completion->done = false;
    completion->status = LUMINATE_STATUS_INTERNAL;
    completion->payload = NULL;
}

static void completion_publish(Completion *completion, LuminateStatus status, void *payload)
{
    assert(mtx_lock(&completion->mutex) == thrd_success);
    completion->status = status;
    completion->payload = payload;
    completion->done = true;
    assert(cnd_signal(&completion->changed) == thrd_success);
    assert(mtx_unlock(&completion->mutex) == thrd_success);
}

static void completion_wait(Completion *completion)
{
    assert(mtx_lock(&completion->mutex) == thrd_success);
    while (!completion->done)
    {
        assert(cnd_wait(&completion->changed, &completion->mutex) == thrd_success);
    }
    assert(mtx_unlock(&completion->mutex) == thrd_success);
}

static void completion_reset(Completion *completion)
{
    assert(mtx_lock(&completion->mutex) == thrd_success);
    completion->done = false;
    completion->status = LUMINATE_STATUS_INTERNAL;
    completion->payload = NULL;
    assert(mtx_unlock(&completion->mutex) == thrd_success);
}

static void completion_destroy(Completion *completion)
{
    cnd_destroy(&completion->changed);
    mtx_destroy(&completion->mutex);
}

static void connected(void *context, const LuminateAsyncOperation *operation,
                      LuminateStatus status, LuminateClient *client)
{
    assert(operation != NULL);
    completion_publish(context, status, client);
}

static void server_info_complete(void *context, const LuminateAsyncOperation *operation,
                                 LuminateStatus status, LuminateServerInfo *info)
{
    assert(operation != NULL);
    completion_publish(context, status, info);
}

static void status_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status)
{
    assert(operation != NULL);
    completion_publish(context, status, NULL);
}

static void topology_complete(void *context, const LuminateAsyncOperation *operation,
                              LuminateStatus status, LuminateTopologySnapshot *topology)
{
    assert(operation != NULL);
    completion_publish(context, status, topology);
}

static void device_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status, LuminateDeviceSnapshot *device)
{
    assert(operation != NULL);
    completion_publish(context, status, device);
}

static void await_operation(Completion *completion, LuminateAsyncOperation *operation,
                            LuminateStatus expected)
{
    completion_wait(completion);
    assert(completion->status == expected);

    LuminateStatus terminal_status = LUMINATE_STATUS_INTERNAL;
    assert(luminate_async_operation_status(operation, &terminal_status));
    assert(terminal_status == expected);
    luminate_async_operation_release(operation);
}

int main(int argc, char **argv)
{
    assert(argc == 2);

    Completion completion;
    completion_initialize(&completion);

    LuminateAsyncOperation *operation = NULL;
    assert(luminate_client_connect_path_async(argv[1], &completion, NULL, connected,
                                              &operation) == LUMINATE_STATUS_OK);
    assert(operation != NULL);
    await_operation(&completion, operation, LUMINATE_STATUS_OK);
    LuminateClient *client = completion.payload;
    assert(client != NULL);

    completion_reset(&completion);
    operation = NULL;
    assert(luminate_client_server_info_async(client, &completion, NULL, server_info_complete,
                                             &operation) == LUMINATE_STATUS_OK);
    await_operation(&completion, operation, LUMINATE_STATUS_OK);
    LuminateServerInfo *info = completion.payload;
    assert(info != NULL);
    assert(view_equal(luminate_server_info_daemon_name(info), "luminated-mock"));
    assert(view_equal(luminate_server_info_daemon_version(info), "mock-daemon-0.1"));
    assert(luminate_server_info_protocol_abi_version(info) != 0);
    luminate_server_info_free(info);

    completion_reset(&completion);
    operation = NULL;
    assert(luminate_client_ping_async(client, &completion, NULL, status_complete, &operation) ==
           LUMINATE_STATUS_OK);
    await_operation(&completion, operation, LUMINATE_STATUS_OK);
    assert(completion.payload == NULL);

    completion_reset(&completion);
    operation = NULL;
    assert(luminate_client_list_devices_async(client, &completion, NULL, topology_complete,
                                              &operation) == LUMINATE_STATUS_OK);
    await_operation(&completion, operation, LUMINATE_STATUS_OK);
    LuminateTopologySnapshot *topology = completion.payload;
    assert(topology != NULL);
    assert(luminate_topology_snapshot_device_count(topology) == 0);
    assert(luminate_topology_snapshot_device_at(topology, 0) == NULL);
    luminate_topology_snapshot_free(topology);

    completion_reset(&completion);
    operation = NULL;
    assert(luminate_client_get_device_async(client, "missing", &completion, NULL, device_complete,
                                            &operation) == LUMINATE_STATUS_OK);
    await_operation(&completion, operation, LUMINATE_STATUS_NOT_FOUND);
    assert(completion.payload == NULL);

    luminate_client_free(client);
    completion_destroy(&completion);
    return 0;
}
