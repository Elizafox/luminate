// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#include "luminate.h"

#include <stdbool.h>

static void client_connected(void *context, const LuminateAsyncOperation *operation,
                             LuminateStatus status, LuminateClient *client)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)client;
}

static void no_context_free(void *context) { (void)context; }

static void status_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status)
{
    (void)context;
    (void)operation;
    (void)status;
}

static void server_info_complete(void *context, const LuminateAsyncOperation *operation,
                                 LuminateStatus status, LuminateServerInfo *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void topology_complete(void *context, const LuminateAsyncOperation *operation,
                              LuminateStatus status, LuminateTopologySnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void withdrawn_complete(void *context, const LuminateAsyncOperation *operation,
                               LuminateStatus status, LuminateWithdrawnDeviceList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void attestation_created_complete(void *context, const LuminateAsyncOperation *operation,
                                         LuminateStatus status,
                                         LuminateCreatedAttestation *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void attestation_list_complete(void *context, const LuminateAsyncOperation *operation,
                                      LuminateStatus status, LuminateAttestationList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void policy_complete(void *context, const LuminateAsyncOperation *operation,
                            LuminateStatus status, LuminatePolicyDocument *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void created_token_complete(void *context, const LuminateAsyncOperation *operation,
                                   LuminateStatus status, LuminateCreatedToken *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void token_list_complete(void *context, const LuminateAsyncOperation *operation,
                                LuminateStatus status, LuminateTokenList *payload)
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
    (void)status;
    (void)payload;
}

static void collection_list_complete(void *context, const LuminateAsyncOperation *operation,
                                     LuminateStatus status, LuminateCollectionList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void collection_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                         LuminateStatus status,
                                         LuminateCollectionSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void collection_outcome_complete(void *context, const LuminateAsyncOperation *operation,
                                        LuminateStatus status, LuminateCollectionOutcome *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void scene_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                    LuminateStatus status, LuminateSceneSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void scene_list_complete(void *context, const LuminateAsyncOperation *operation,
                                LuminateStatus status, LuminateSceneList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void u32_complete(void *context, const LuminateAsyncOperation *operation,
                         LuminateStatus status, uint32_t value)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)value;
}

static void frame_ack_complete(void *context, const LuminateAsyncOperation *operation,
                               LuminateStatus status, LuminateFrameAck value)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)value;
}

static void management_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                         LuminateStatus status,
                                         LuminateManagementSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void management_changeset_complete(void *context, const LuminateAsyncOperation *operation,
                                          LuminateStatus status,
                                          LuminateManagementChangeSet *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void plugin_setup_session_complete(void *context, const LuminateAsyncOperation *operation,
                                          LuminateStatus status,
                                          LuminatePluginSetupSession *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void plugin_setup_workflows_complete(void *context,
                                            const LuminateAsyncOperation *operation,
                                            LuminateStatus status,
                                            LuminatePluginSetupWorkflowList *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void transition_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                         LuminateStatus status,
                                         LuminateTransitionSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void event_subscription_complete(void *context, const LuminateAsyncOperation *operation,
                                        LuminateStatus status,
                                        LuminateEventSubscription *subscription)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)subscription;
}

static void subscription_baseline_complete(void *context, const LuminateAsyncOperation *operation,
                                           LuminateStatus status,
                                           LuminateEventSubscription *subscription,
                                           LuminateTopologySnapshot *snapshot)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)subscription;
    (void)snapshot;
}

static void event_complete(void *context, const LuminateAsyncOperation *operation,
                           LuminateStatus status, LuminateEvent *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void shm_stream_complete(void *context, const LuminateAsyncOperation *operation,
                                LuminateStatus status, LuminateShmFrameStream *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void collection_state_snapshot_complete(void *context,
                                               const LuminateAsyncOperation *operation,
                                               LuminateStatus status,
                                               LuminateCollectionStateSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void device_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                     LuminateStatus status, LuminateDeviceSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

static void state_snapshot_complete(void *context, const LuminateAsyncOperation *operation,
                                    LuminateStatus status, LuminateStateSnapshot *payload)
{
    (void)context;
    (void)operation;
    (void)status;
    (void)payload;
}

int main(void)
{
    const char *const groups[1] = {NULL};
    LuminateAsyncOperation *operation = NULL;
    LuminateClient *client = NULL;
    LuminateTarget target = {0};
    LuminateSelectorInput selector = {0};
    LuminateAppearanceSlotInput slot = {0};
    LuminateCollectionMemberInput member = {0};
    LuminateSceneBindingInput scene_binding = {0};
    LuminateTransitionOptions transition_options = {0};

    LuminateTransitionTargetStateInput *transition_states = NULL;

    LuminateStatus status =
        luminate_client_connect_async(NULL, no_context_free, client_connected, &operation);
    (void)status;

    status = luminate_client_connect_path_async("missing.sock", NULL, no_context_free,
                                                client_connected, &operation);
    (void)status;

    status = luminate_client_builder_connect_async(NULL, NULL, no_context_free, client_connected,
                                                   &operation);
    (void)status;

    status = luminate_client_server_info_async(client, NULL, no_context_free,
                                               server_info_complete, &operation);
    (void)status;

    status = luminate_client_purge_withdrawn_device_async(
        client, "device-id", NULL, no_context_free, status_complete, &operation);
    (void)status;

    status = luminate_client_list_devices_async(client, NULL, no_context_free, topology_complete,
                                                &operation);
    (void)status;

    status = luminate_client_list_withdrawn_devices_async(client, NULL, no_context_free,
                                                          withdrawn_complete, &operation);
    (void)status;

    status = luminate_client_refresh_state_async(client, "device-id", NULL, no_context_free,
                                                 status_complete, &operation);
    (void)status;

    status = luminate_client_create_attestation_async(client, "name", "authority", "subject",
                                                      false, 0, NULL, no_context_free,
                                                      attestation_created_complete, &operation);
    (void)status;

    status = luminate_client_create_principal_attestation_async(
        client, "name", "authority", "subject", groups, 1, false, 0, NULL, no_context_free,
        attestation_created_complete, &operation);
    (void)status;

    status = luminate_client_list_attestations_async(client, NULL, no_context_free,
                                                     attestation_list_complete, &operation);
    (void)status;

    status = luminate_client_revoke_attestation_async(client, "name", NULL, no_context_free,
                                                      status_complete, &operation);
    (void)status;

    status = luminate_client_get_access_policy_async(client, NULL, no_context_free,
                                                     policy_complete, &operation);
    (void)status;

    status = luminate_client_replace_access_policy_async(client, 0, NULL, NULL, no_context_free,
                                                         policy_complete, &operation);
    (void)status;

    status =
        luminate_client_create_token_async(client, "id", "authority", "subject", false, 0, NULL,
                                           no_context_free, created_token_complete, &operation);
    (void)status;

    status = luminate_client_list_tokens_async(client, NULL, no_context_free, token_list_complete,
                                               &operation);
    (void)status;

    status = luminate_client_revoke_token_async(client, "id", NULL, no_context_free,
                                                status_complete, &operation);
    (void)status;

    status = luminate_client_rotate_token_async(client, "id", false, 0, NULL, no_context_free,
                                                created_token_complete, &operation);
    (void)status;

    status = luminate_client_create_collection_async(client, "name", "description", "kind", NULL,
                                                     0, NULL, no_context_free, string_complete,
                                                     &operation);
    (void)status;

    status = luminate_client_destroy_collection_async(client, "id", NULL, no_context_free,
                                                      status_complete, &operation);
    (void)status;

    status = luminate_client_add_collection_member_async(
        client, "id", &member, NULL, no_context_free, status_complete, &operation);
    (void)status;

    status = luminate_client_remove_collection_member_async(
        client, "id", &member, NULL, no_context_free, status_complete, &operation);
    (void)status;

    status = luminate_client_list_collections_async(client, NULL, no_context_free,
                                                    collection_list_complete, &operation);
    (void)status;

    status = luminate_client_get_collection_async(client, "id", NULL, no_context_free,
                                                  collection_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_set_appearance_slots_async(
        client, &target, &slot, 0, NULL, no_context_free, status_complete, &operation);
    (void)status;

    status =
        luminate_client_create_scene_async(client, "name", "description", &scene_binding, 1, NULL,
                                           no_context_free, scene_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_capture_scene_async(client, "name", "description", "dynamic-id",
                                                 &target, 0, NULL, no_context_free,
                                                 scene_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_replace_scene_async(client, "id", 0, "name", "description",
                                                 &scene_binding, 1, NULL, no_context_free,
                                                 scene_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_replace_scene_from_builder_async(
        client, NULL, NULL, no_context_free, scene_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_recapture_scene_async(client, "id", 0, "dynamic-id", &target, 0,
                                                   NULL, no_context_free, scene_snapshot_complete,
                                                   &operation);
    (void)status;

    status = luminate_client_delete_scene_async(client, "id", 0, NULL, no_context_free,
                                                status_complete, &operation);
    (void)status;

    status = luminate_client_list_scenes_async(client, NULL, no_context_free, scene_list_complete,
                                               &operation);
    (void)status;

    status = luminate_client_get_scene_async(client, "id", NULL, no_context_free,
                                             scene_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_apply_scene_async(client, "id", NULL, no_context_free,
                                               status_complete, &operation);
    (void)status;

    status = luminate_client_set_effect_async(client, &target, NULL, NULL, no_context_free,
                                              status_complete, &operation);
    (void)status;

    status = luminate_client_set_emission_async(client, &target, 0, NULL, no_context_free,
                                                status_complete, &operation);
    (void)status;

    status = luminate_client_set_brightness_async(client, &target, 0, NULL, no_context_free,
                                                  status_complete, &operation);
    (void)status;

    status = luminate_client_set_brightness_selector_async(
        client, &selector, 0, false, 0, NULL, no_context_free, collection_outcome_complete,
        &operation);
    (void)status;

    status = luminate_client_set_effect_selector_async(client, &selector, NULL, false, 0, NULL,
                                                       no_context_free,
                                                       collection_outcome_complete, &operation);
    (void)status;

    status = luminate_client_set_emission_selector_async(
        client, &selector, 0, NULL, no_context_free, collection_outcome_complete, &operation);
    (void)status;

    status = luminate_client_begin_frame_stream_async(client, &target, NULL, no_context_free,
                                                      u32_complete, &operation);
    (void)status;

    status =
        luminate_client_upload_frame_full_async(client, &target, 0, 0, NULL, 0, false, NULL,
                                                no_context_free, frame_ack_complete, &operation);
    (void)status;

    status = luminate_client_upload_frame_partial_async(client, &target, 0, 0, NULL, NULL, 0,
                                                        false, NULL, no_context_free,
                                                        frame_ack_complete, &operation);
    (void)status;

    status = luminate_client_end_frame_stream_async(client, &target, 0, NULL, no_context_free,
                                                    status_complete, &operation);
    (void)status;

    status = luminate_client_begin_shm_frame_stream_async(client, &target, NULL, no_context_free,
                                                          shm_stream_complete, &operation);
    (void)status;

    status = luminate_client_end_shm_frame_stream_async(NULL, NULL, no_context_free,
                                                        status_complete, &operation);
    (void)status;

    status = luminate_client_get_management_async(client, NULL, no_context_free,
                                                  management_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_patch_management_async(client, NULL, NULL, no_context_free,
                                                    management_changeset_complete, &operation);
    (void)status;

    status = luminate_client_plugin_setup_start_async(client, "plugin", "workflow", NULL,
                                                      no_context_free,
                                                      plugin_setup_session_complete, &operation);
    (void)status;

    status = luminate_client_plugin_setup_choose_async(client, "session-id", 0, "choice", NULL,
                                                       no_context_free,
                                                       plugin_setup_session_complete, &operation);
    (void)status;

    status =
        luminate_client_plugin_setup_confirm_async(client, "session-id", 0, NULL, no_context_free,
                                                   plugin_setup_session_complete, &operation);
    (void)status;

    status = luminate_client_plugin_setup_get_async(client, "session-id", NULL, no_context_free,
                                                    plugin_setup_session_complete, &operation);
    (void)status;

    status = luminate_client_plugin_setup_cancel_async(
        client, "session-id", NULL, no_context_free, plugin_setup_session_complete, &operation);
    (void)status;

    status = luminate_client_plugin_setup_workflows_async(
        client, "plugin", NULL, no_context_free, plugin_setup_workflows_complete, &operation);
    (void)status;

    status = luminate_client_transition_scene_to_scene_async(
        client, "source", "destination", transition_options, NULL, no_context_free,
        transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_current_to_scene_async(
        client, "destination", transition_options, NULL, no_context_free,
        transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_scene_to_states_async(
        client, "source", transition_states, 0, transition_options, NULL, no_context_free,
        transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_current_to_states_async(
        client, transition_states, 0, transition_options, NULL, no_context_free,
        transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_subscribe_async(client, NULL, no_context_free,
                                             event_subscription_complete, &operation);
    (void)status;

    status = luminate_client_subscribe_path_async(client, "events.sock", NULL, no_context_free,
                                                  event_subscription_complete, &operation);
    (void)status;

    status = luminate_client_subscribe_with_baseline_async(
        client, NULL, no_context_free, subscription_baseline_complete, &operation);
    (void)status;

    status = luminate_client_subscribe_with_baseline_path_async(
        client, "events.sock", NULL, no_context_free, subscription_baseline_complete, &operation);
    (void)status;

    status = luminate_event_subscription_next_async(NULL, NULL, no_context_free, event_complete,
                                                    &operation);
    (void)status;

    status = luminate_client_clear_target_async(client, &target, NULL, no_context_free,
                                                status_complete, &operation);
    (void)status;

    status = luminate_client_set_off_async(client, &target, NULL, no_context_free,
                                           status_complete, &operation);
    (void)status;

    status = luminate_client_restore_appearance_async(client, &target, NULL, no_context_free,
                                                      status_complete, &operation);
    (void)status;

    status = luminate_client_save_current_async(client, &target, NULL, no_context_free,
                                                status_complete, &operation);
    (void)status;

    status = luminate_client_clear_target_selector_async(client, &selector, NULL, no_context_free,
                                                         collection_outcome_complete, &operation);
    (void)status;

    status = luminate_client_save_current_selector_async(client, &selector, NULL, no_context_free,
                                                         collection_outcome_complete, &operation);
    (void)status;

    status = luminate_client_restore_appearance_selector_async(
        client, &selector, NULL, no_context_free, collection_outcome_complete, &operation);
    (void)status;

    status = luminate_client_get_device_async(client, "device-id", NULL, no_context_free,
                                              device_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_get_state_async(client, "device-id", NULL, no_context_free,
                                             state_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_get_collection_state_async(
        client, "id", NULL, no_context_free, collection_state_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_get_async(client, "id", NULL, no_context_free,
                                                  transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_abort_async(client, "id", NULL, no_context_free,
                                                    transition_snapshot_complete, &operation);
    (void)status;

    status = luminate_client_transition_wait_async(client, "id", NULL, no_context_free,
                                                   transition_snapshot_complete, &operation);
    (void)status;

    status =
        luminate_client_ping_async(client, NULL, no_context_free, status_complete, &operation);
    (void)status;

    status =
        luminate_client_rescan_async(client, NULL, no_context_free, status_complete, &operation);
    (void)status;

    return 0;
}
