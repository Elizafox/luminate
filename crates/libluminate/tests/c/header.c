/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

LUMINATE_REQUIRE_C_ABI(28);

_Static_assert(sizeof(LuminateStatus) == sizeof(uint32_t), "status width");
_Static_assert(sizeof(LuminateEffectKind) == sizeof(uint32_t), "effect kind width");

int main(void)
{
    LuminateStringView absent = {NULL, 0};
    LuminateStringView empty = {"", 0};
    LuminateTarget target = luminate_target_element("device", "surface", "element");
    LuminateRgb colour = luminate_rgb(1, 2, 3);
    uint64_t expiry = 0;

    if (luminate_c_abi_version() != LUMINATE_C_ABI_VERSION ||
        !luminate_status_is_ok(LUMINATE_STATUS_OK) ||
        luminate_status_is_error(LUMINATE_STATUS_OK) || luminate_string_view_is_present(absent) ||
        !luminate_string_view_is_empty(empty) || target.element_id == NULL || colour.g != 2)
    {
        return 1;
    }

    if (luminate_attestation_list_count(NULL) != 0 ||
        luminate_created_attestation_secret(NULL, NULL, 0) != 0 ||
        luminate_created_attestation_expires_at_unix_ms(NULL, &expiry) ||
        luminate_attestation_expires_at_unix_ms(NULL, &expiry) ||
        luminate_token_list_at(NULL, 0) != NULL ||
        luminate_attestation_list_at(NULL, 0) != NULL ||
        luminate_plugin_setup_session_choice_at(NULL, 0) != NULL ||
        luminate_string_view_is_present(luminate_plugin_setup_session_message(NULL)) ||
        luminate_string_view_is_present(luminate_plugin_setup_workflow_plugin(NULL)) ||
        luminate_string_view_is_present(luminate_plugin_setup_choice_description(NULL)) ||
        luminate_string_view_is_present(luminate_withdrawn_device_list_at(NULL, 0)) ||
        luminate_appearance_slot_count(NULL) != 0 ||
        luminate_appearance_slot_at(NULL, 0) != NULL ||
        luminate_string_view_is_present(luminate_created_attestation_group_at(NULL, 0)))
    {
        return 1;
    }
    luminate_attestation_list_free(NULL);
    luminate_created_attestation_free(NULL);
    return 0;
}
