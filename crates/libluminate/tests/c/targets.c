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
    require_status(luminate_client_connect_path(argv[1], &client), LUMINATE_STATUS_OK, "connect");

    LuminateTarget device = luminate_target_device("device");
    LuminateTarget surface = luminate_target_surface("device", "surface");
    LuminateTarget element = luminate_target_element("device", "surface", "element");
    LuminateTarget group = luminate_target_group("device", "group");

    LuminateColourChannelInput channels[] = {
        {LUMINATE_COLOUR_CHANNEL_RED, 1},
        {LUMINATE_COLOUR_CHANNEL_GREEN, 2},
        {LUMINATE_COLOUR_CHANNEL_BLUE, 3},
    };
    LuminateColourInput colour = {
        .encoding = LUMINATE_COLOUR_ENCODING_ADDITIVE,
        .channels = channels,
        .channel_count = 3,
    };
    LuminateEffect *effect = NULL;
    require_status(luminate_effect_create_static(&colour, &effect), LUMINATE_STATUS_OK,
                   "create static effect");
    require_status(luminate_client_set_effect(client, &device, effect), LUMINATE_STATUS_OK,
                   "device target");
    require_status(luminate_client_set_brightness(client, &surface, 50), LUMINATE_STATUS_OK,
                   "surface target");
    require_status(luminate_client_set_off(client, &element), LUMINATE_STATUS_OK,
                   "element target");
    require_status(luminate_client_clear_target(client, &group), LUMINATE_STATUS_OK,
                   "group target");

    LuminateTarget invalid_element = {"device", NULL, "element", NULL};
    require_status(luminate_client_set_brightness(client, &invalid_element, 1),
                   LUMINATE_STATUS_INVALID_ARGUMENT, "element without surface");
    LuminateTarget invalid_group = {"device", "surface", NULL, "group"};
    require_status(luminate_client_set_off(client, &invalid_group),
                   LUMINATE_STATUS_INVALID_ARGUMENT, "group mixed with surface");
    require_status(luminate_client_clear_target(client, NULL), LUMINATE_STATUS_NULL_POINTER,
                   "null device id");

    luminate_effect_free(effect);
    luminate_client_free(client);
    return 0;
}
