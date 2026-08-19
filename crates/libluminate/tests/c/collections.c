/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require_status(LuminateStatus actual, LuminateStatus expected, const char *context)
{
    if (actual != expected)
    {
        fprintf(stderr, "%s: expected %d, got %d: %s\n", context, expected, actual,
                luminate_last_error_message() ? luminate_last_error_message() : "(none)");
        exit(1);
    }
}

static void require(bool condition, const char *context)
{
    if (!condition)
    {
        fprintf(stderr, "%s: condition failed\n", context);
        exit(1);
    }
}

static bool view_equal(LuminateStringView actual, const char *expected)
{
    LuminateStringView expected_view = {expected, strlen(expected)};
    return luminate_string_view_equal(actual, expected_view);
}

int main(int argc, char **argv)
{
    if (argc != 2)
    {
        return 2;
    }

    struct LuminateClient *client = NULL;
    require_status(luminate_client_connect_path(argv[1], &client), LUMINATE_STATUS_OK, "connect");

    LuminateCollectionMemberInput target_member = {
        .is_collection = false,
        .target = luminate_target_device("lamp"),
        .collection_id = NULL,
    };

    char *created_id = NULL;
    require_status(luminate_client_create_collection(client, "Living Room", "Downstairs lighting",
                                                     "location", &target_member, 1, &created_id),
                   LUMINATE_STATUS_OK, "create collection");
    require(created_id != NULL, "created id is non-null");
    require(strlen(created_id) > 0, "created id is non-empty");
    luminate_string_free(created_id);

    LuminateCollectionMemberInput nested_member = {
        .is_collection = true,
        .target = {0},
        .collection_id = "nook",
    };
    require_status(luminate_client_add_collection_member(client, "living-room", &nested_member),
                   LUMINATE_STATUS_OK, "add member");
    require_status(
        luminate_client_remove_collection_member(client, "living-room", &nested_member),
        LUMINATE_STATUS_OK, "remove member");

    struct LuminateCollectionList *list = NULL;
    require_status(luminate_client_list_collections(client, &list), LUMINATE_STATUS_OK,
                   "list collections");
    require(luminate_collection_list_count(list) == 1, "collection list count");
    const LuminateCollection *collection = luminate_collection_list_at(list, 0);
    require(collection != NULL, "listed collection");
    require(view_equal(luminate_collection_id(collection), "living-room"), "collection id");
    require(view_equal(luminate_collection_name(collection), "Living Room"), "collection name");
    require(view_equal(luminate_collection_description(collection), "Downstairs lighting"),
            "collection description");
    require(view_equal(luminate_collection_kind(collection), "location"), "collection kind");
    const LuminateOwnerIdentity *owner = luminate_collection_owner(collection);
    uint32_t owner_uid = 0;
    require(owner != NULL && luminate_owner_identity_kind(owner) == LUMINATE_OWNER_KIND_UID &&
                luminate_owner_identity_uid(owner, &owner_uid) && owner_uid == 1000,
            "collection owner");
    require(!luminate_string_view_is_present(luminate_owner_identity_sid(owner)),
            "UID owner has no SID");
    require(luminate_collection_member_count(collection) == 2, "collection member count");
    const LuminateCollectionMember *target = luminate_collection_member_at(collection, 0);
    require(luminate_collection_member_kind(target) == LUMINATE_COLLECTION_MEMBER_TARGET,
            "target member kind");
    const LuminateTargetView *target_view = luminate_collection_member_target(target);
    require(target_view != NULL &&
                luminate_target_view_kind(target_view) == LUMINATE_TARGET_SURFACE &&
                view_equal(luminate_target_view_device_id(target_view), "lamp") &&
                view_equal(luminate_target_view_surface_id(target_view), "shade"),
            "target member");
    const LuminateCollectionMember *nested = luminate_collection_member_at(collection, 1);
    require(luminate_collection_member_kind(nested) == LUMINATE_COLLECTION_MEMBER_COLLECTION &&
                view_equal(luminate_collection_member_collection_id(nested), "nook"),
            "nested collection member");
    require(luminate_collection_list_at(list, 1) == NULL, "out-of-range list access is null");
    luminate_collection_list_free(list);

    struct LuminateCollectionSnapshot *snapshot = NULL;
    require_status(luminate_client_get_collection(client, "living-room", &snapshot),
                   LUMINATE_STATUS_OK, "get collection");
    require(snapshot != NULL, "collection snapshot");
    require(view_equal(luminate_collection_id(luminate_collection_snapshot_collection(snapshot)),
                       "living-room"),
            "snapshot collection");
    luminate_collection_snapshot_free(snapshot);

    require_status(luminate_client_destroy_collection(client, "living-room"), LUMINATE_STATUS_OK,
                   "destroy collection");

    require_status(
        luminate_client_create_collection(client, NULL, NULL, NULL, NULL, 0, &created_id),
        LUMINATE_STATUS_NULL_POINTER, "create collection with null name");
    require_status(luminate_client_add_collection_member(client, "living-room", NULL),
                   LUMINATE_STATUS_NULL_POINTER, "add member with null member");
    require_status(luminate_client_create_collection(client, "Living Room", NULL, NULL, NULL, 1,
                                                     &created_id),
                   LUMINATE_STATUS_NULL_POINTER,
                   "create collection with member_count > 0 but null members");

    luminate_client_free(client);
    return 0;
}
