/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>

static void require(bool condition, const char *context)
{
    if (!condition)
    {
        fprintf(stderr, "%s: condition failed\n", context);
        exit(1);
    }
}

static void require_status(LuminateStatus actual, LuminateStatus expected, const char *context)
{
    if (actual != expected)
    {
        fprintf(stderr, "%s: expected %u, got %u: %s\n", context, expected, actual,
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

    LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client), LUMINATE_STATUS_OK, "connect");
    LuminateTarget target = luminate_target_surface("fixture-device", "panel");

    uint32_t generation = 0;
    require_status(luminate_client_begin_frame_stream(client, &target, &generation),
                   LUMINATE_STATUS_OK, "begin frame stream");
    require(generation == 1, "frame stream generation");

    LuminateRgb full[] = {luminate_rgb(1, 2, 3), luminate_rgb(4, 5, 6)};
    LuminateFrameAck ack = {0};
    require_status(
        luminate_client_upload_frame_full(client, &target, generation, 10, full, 2, 0, &ack),
        LUMINATE_STATUS_OK, "upload full frame");
    require(ack.sequence == 10 && ack.dropped == 0, "full frame acknowledgement");

    const uint32_t indices[] = {1};
    const LuminateRgb partial[] = {luminate_rgb(7, 8, 9)};
    require_status(luminate_client_upload_frame_partial(client, &target, generation, 11, indices,
                                                        partial, 1, 1, &ack),
                   LUMINATE_STATUS_OK, "upload partial frame");
    require(ack.sequence == 11 && ack.dropped == 0, "partial frame acknowledgement");

    require_status(luminate_client_upload_frame_partial(client, &target, generation, 12, NULL,
                                                        partial, 1, 0, &ack),
                   LUMINATE_STATUS_NULL_POINTER, "reject missing partial indices");
    require_status(luminate_client_end_frame_stream(client, &target, generation),
                   LUMINATE_STATUS_OK, "end frame stream");

    LuminateShmFrameStream *shm = NULL;
    require_status(luminate_client_begin_shm_frame_stream(client, &target, &shm),
                   LUMINATE_STATUS_UNSUPPORTED, "shared-memory fallback");
    require(shm == NULL, "unsupported shared-memory stream has no handle");

    luminate_client_free(client);
    return 0;
}
