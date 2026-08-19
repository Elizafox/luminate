/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

LUMINATE_REQUIRE_C_ABI(28);
static_assert(sizeof(LuminateStatus) == sizeof(uint32_t), "status width");

int main()
{
    const auto target = luminate_target_group("device", "group");
    return luminate_c_abi_version() == LUMINATE_C_ABI_VERSION && target.group_id != nullptr ? 0
                                                                                            : 1;
}
