// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <cassert>
#include <condition_variable>
#include <mutex>
#include <string>

struct CompletionContext
{
    std::mutex mutex;
    std::condition_variable changed;
    bool callback_finished = false;
    bool context_destroyed = false;
    LuminateStatus status = LUMINATE_STATUS_OK;
    LuminateAsyncOperation *retained_operation = nullptr;
};

static void connect_complete(void *raw_context, const LuminateAsyncOperation *operation,
                             LuminateStatus status, LuminateClient *client)
{
    auto &context = *static_cast<CompletionContext *>(raw_context);
    assert(operation != nullptr);
    assert(status != LUMINATE_STATUS_OK);
    assert(client == nullptr);

    auto *retained = luminate_async_operation_retain(operation);
    assert(retained != nullptr);

    std::lock_guard<std::mutex> lock(context.mutex);
    context.status = status;
    context.retained_operation = retained;
    context.callback_finished = true;
    context.changed.notify_all();
}

static void context_free(void *raw_context)
{
    auto &context = *static_cast<CompletionContext *>(raw_context);
    std::lock_guard<std::mutex> lock(context.mutex);
    assert(context.callback_finished);
    context.context_destroyed = true;
    context.changed.notify_all();
}

int main(int argc, char **argv)
{
    assert(argc == 2);

    CompletionContext context;
    LuminateAsyncOperation *submission_operation = nullptr;
    assert(luminate_client_connect_path_async(argv[1], &context, context_free, connect_complete,
                                              &submission_operation) == LUMINATE_STATUS_OK);
    assert(submission_operation != nullptr);

    {
        std::unique_lock<std::mutex> lock(context.mutex);
        context.changed.wait(lock, [&context] { return context.context_destroyed; });
    }

    assert(context.status != LUMINATE_STATUS_OK);
    assert(context.retained_operation != nullptr);

    LuminateStatus published = LUMINATE_STATUS_OK;
    assert(luminate_async_operation_status(context.retained_operation, &published));
    assert(published == context.status);

    const LuminateStringView diagnostic =
        luminate_async_operation_error_message(context.retained_operation);
    assert(diagnostic.data != nullptr);
    assert(diagnostic.len != 0);
    const std::string copied_diagnostic(diagnostic.data, diagnostic.len);
    assert(!copied_diagnostic.empty());

    luminate_async_operation_release(submission_operation);
    luminate_async_operation_release(context.retained_operation);
    return 0;
}
