/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdint.h>
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

int main(void)
{
    struct LuminateSceneSnapshot *scene = NULL;
    struct LuminateSceneList *scenes = NULL;
    struct LuminateTransitionSnapshot *transition = NULL;
    const struct LuminateSceneBindingInput binding = {0};
    const struct LuminateTransitionOptions timing = {
        .duration_ms = 100,
        .step_interval_ms = 0,
        .function = LUMINATE_TRANSITION_FUNCTION_LINEAR,
        .colour_interpolation = LUMINATE_TRANSITION_COLOUR_ENCODED,
        .hue_direction = LUMINATE_HUE_DIRECTION_SHORTEST,
    };

    require_status(luminate_client_list_scenes(NULL, &scenes), LUMINATE_STATUS_NULL_POINTER,
                   "list scenes null session");
    require_status(luminate_client_get_scene(NULL, "scene", &scene), LUMINATE_STATUS_NULL_POINTER,
                   "get scene null session");
    require_status(luminate_client_create_scene(NULL, "scene", NULL, &binding, 0, &scene),
                   LUMINATE_STATUS_NULL_POINTER, "create scene null session");
    require_status(luminate_client_capture_scene(NULL, "scene", NULL, NULL, NULL, 0, &scene),
                   LUMINATE_STATUS_NULL_POINTER, "capture scene null session");
    require_status(
        luminate_client_replace_scene(NULL, "scene", 1, "scene", NULL, &binding, 0, &scene),
        LUMINATE_STATUS_NULL_POINTER, "replace scene null session");
    struct LuminateSceneBuilder *builder = NULL;
    require_status(luminate_scene_builder_from_scene(NULL, &builder),
                   LUMINATE_STATUS_NULL_POINTER, "seed scene builder null scene");
    require_status(luminate_scene_builder_set_name(NULL, "scene"), LUMINATE_STATUS_NULL_POINTER,
                   "scene builder name null builder");
    require_status(luminate_scene_builder_set_description(NULL, NULL),
                   LUMINATE_STATUS_NULL_POINTER, "scene builder description null builder");
    require_status(luminate_scene_builder_add_binding(NULL, &binding),
                   LUMINATE_STATUS_NULL_POINTER, "scene builder add null builder");
    require_status(luminate_scene_builder_replace_binding(NULL, 0, &binding),
                   LUMINATE_STATUS_NULL_POINTER, "scene builder replace null builder");
    require_status(luminate_scene_builder_remove_binding(NULL, 0), LUMINATE_STATUS_NULL_POINTER,
                   "scene builder remove null builder");
    require_status(luminate_client_replace_scene_from_builder(NULL, builder, &scene),
                   LUMINATE_STATUS_NULL_POINTER, "replace from builder null session");
    if (luminate_scene_builder_binding_count(NULL) != 0 ||
        luminate_scene_builder_binding_at(NULL, 0) != NULL)
    {
        return 1;
    }
    require_status(luminate_client_recapture_scene(NULL, "scene", 1, NULL, NULL, 0, &scene),
                   LUMINATE_STATUS_NULL_POINTER, "recapture scene null session");
    require_status(luminate_client_delete_scene(NULL, "scene", 1), LUMINATE_STATUS_NULL_POINTER,
                   "delete scene null session");
    require_status(luminate_client_apply_scene(NULL, "scene"), LUMINATE_STATUS_NULL_POINTER,
                   "apply scene null session");
    require_status(luminate_client_transition_scene_to_scene(NULL, "source", "destination",
                                                             timing, &transition),
                   LUMINATE_STATUS_NULL_POINTER, "scene transition null session");
    require_status(
        luminate_client_transition_current_to_scene(NULL, "destination", timing, &transition),
        LUMINATE_STATUS_NULL_POINTER, "current transition null session");

    luminate_scene_list_free(NULL);
    luminate_scene_snapshot_free(NULL);
    luminate_scene_builder_free(NULL);
    luminate_transition_snapshot_free(NULL);
    return 0;
}
