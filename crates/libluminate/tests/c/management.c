/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void require(int condition, const char *message)
{
    if (!condition)
    {
        fprintf(stderr, "management C check failed: %s (%s)\n", message,
                luminate_last_error_message() ? luminate_last_error_message() : "no detail");
        exit(1);
    }
}

static void require_ok(LuminateStatus status, const char *message)
{
    require(status == LUMINATE_STATUS_OK, message);
}

static int view_equal(LuminateStringView value, const char *expected)
{
    LuminateStringView other = {expected, strlen(expected)};
    return luminate_string_view_equal(value, other);
}

int main(int argc, char **argv)
{
    require(argc == 2, "socket argument");

    LuminateClient *client = NULL;
    require_ok(luminate_client_connect_path(argv[1], &client), "connect");

    LuminateManagementSnapshot *snapshot = NULL;
    require_ok(luminate_client_get_management(client, &snapshot), "get management");
    require(luminate_management_snapshot_revision(snapshot) == 7, "snapshot revision");
    require(luminate_management_snapshot_plugin_count(snapshot) == 1, "plugin count");
    require(luminate_management_snapshot_locked_daemon_setting_count(snapshot) == 1,
            "locked setting count");
    require(view_equal(luminate_management_snapshot_locked_daemon_setting_at(snapshot, 0),
                       "reconciliation-policy"),
            "locked setting");
    const LuminateDaemonPreferences *preferences =
        luminate_management_snapshot_desired_daemon(snapshot);
    require(luminate_daemon_preferences_has_prefer_shm(preferences) &&
                luminate_daemon_preferences_prefer_shm(preferences),
            "optional daemon preference");
    require(luminate_daemon_preferences_has_default_unsupported_policy(preferences) &&
                luminate_daemon_preferences_default_unsupported_policy(preferences) ==
                    LUMINATE_UNSUPPORTED_POLICY_REJECT,
            "default unsupported policy");
    require(luminate_daemon_preferences_has_reconciliation_policy(preferences) &&
                luminate_daemon_preferences_reconciliation_policy(preferences) ==
                    LUMINATE_RECONCILIATION_POLICY_RESTORE,
            "default reconciliation policy");
    require(luminate_daemon_preferences_device_reconciliation_count(preferences) == 1,
            "device reconciliation count");
    const LuminateDeviceReconciliationPreference *device_reconciliation =
        luminate_daemon_preferences_device_reconciliation_at(preferences, 0);
    require(view_equal(luminate_device_reconciliation_preference_device_id(device_reconciliation),
                       "fixture-device") &&
                luminate_device_reconciliation_preference_policy(device_reconciliation) ==
                    LUMINATE_RECONCILIATION_POLICY_ADOPT,
            "device reconciliation preference");
    require(luminate_daemon_preferences_has_cct_emulation(preferences) &&
                luminate_daemon_preferences_cct_emulation(preferences) ==
                    LUMINATE_CCT_EMULATION_DISABLED,
            "CCT emulation preference");
    require(luminate_daemon_preferences_has_prefer_client_shm(preferences) &&
                !luminate_daemon_preferences_prefer_client_shm(preferences),
            "client shared-memory preference");
    const LuminateDaemonPreferences *effective =
        luminate_management_snapshot_effective_daemon(snapshot);
    require(luminate_daemon_preferences_has_prefer_shm(effective),
            "effective daemon preferences");
    const LuminateManagedPlugin *plugin = luminate_management_snapshot_plugin_at(snapshot, 0);
    require(view_equal(luminate_managed_plugin_name(plugin), "fixture"), "plugin name");
    require(view_equal(luminate_managed_plugin_version(plugin), "1.2.3"), "plugin version");
    require(!luminate_managed_plugin_required(plugin), "plugin is optional");
    require(luminate_managed_plugin_has_desired_enabled(plugin) &&
                luminate_managed_plugin_desired_enabled(plugin),
            "desired plugin activation");
    require(luminate_managed_plugin_effective_enabled(plugin), "effective plugin activation");
    require(!luminate_managed_plugin_activation_locked(plugin), "plugin activation unlocked");
    require(!luminate_managed_plugin_has_desired_reconciliation(plugin) &&
                !luminate_managed_plugin_has_effective_reconciliation(plugin),
            "absent plugin reconciliation");
    require(luminate_managed_plugin_runtime_kind(plugin) == LUMINATE_PLUGIN_RUNTIME_LOADED,
            "plugin runtime");
    require(!luminate_string_view_is_present(luminate_managed_plugin_runtime_diagnostic(plugin)),
            "loaded plugin has no diagnostic");
    require(luminate_managed_plugin_schema_count(plugin) == 1, "schema count");
    const LuminatePluginSettingSchema *schema = luminate_managed_plugin_schema_at(plugin, 0);
    require(view_equal(luminate_plugin_setting_schema_key(schema), "mode"), "schema key");
    require(view_equal(luminate_plugin_setting_schema_label(schema), "Mode"), "schema label");
    require(view_equal(luminate_plugin_setting_schema_description(schema), "Fixture mode"),
            "schema description");
    require(luminate_plugin_setting_schema_kind(schema) == LUMINATE_PLUGIN_SETTING_STRING,
            "schema kind");
    require(luminate_plugin_setting_schema_required(schema) &&
                !luminate_plugin_setting_schema_sensitive(schema) &&
                luminate_plugin_setting_schema_restart_required(schema),
            "schema flags");
    require(!luminate_plugin_setting_schema_has_minimum(schema) &&
                !luminate_plugin_setting_schema_has_maximum(schema),
            "schema numeric bounds absent");
    const LuminateReportedSettingValue *default_value =
        luminate_plugin_setting_schema_default(schema);
    require(luminate_reported_setting_value_kind(default_value) ==
                    LUMINATE_REPORTED_SETTING_VISIBLE &&
                view_equal(luminate_setting_value_view_string(
                               luminate_reported_setting_value_visible(default_value)),
                           "calm"),
            "schema default");
    const LuminateReportedSettingValue *constraints =
        luminate_plugin_setting_schema_constraints(schema);
    require(luminate_reported_setting_value_kind(constraints) ==
                LUMINATE_REPORTED_SETTING_VISIBLE,
            "visible constraints");
    const LuminateSettingValueView *constraint_value =
        luminate_reported_setting_value_visible(constraints);
    require(luminate_setting_value_view_kind(constraint_value) ==
                    LUMINATE_MANAGEMENT_SETTING_ARRAY &&
                luminate_setting_value_view_count(constraint_value) == 2,
            "recursive array view");
    require(view_equal(luminate_setting_value_view_string(
                           luminate_setting_value_view_array_at(constraint_value, 1)),
                       "party"),
            "recursive value");
    LuminateSettingValue *constraint_copy = NULL;
    require_ok(luminate_setting_value_view_clone(constraint_value, &constraint_copy),
               "clone visible recursive value");
    luminate_setting_value_free(constraint_copy);
    require(luminate_reported_setting_value_kind(luminate_managed_plugin_desired_setting_value_at(
                plugin, 0)) == LUMINATE_REPORTED_SETTING_REDACTED,
            "redacted desired setting");
    require(luminate_managed_plugin_desired_setting_count(plugin) == 1 &&
                view_equal(luminate_managed_plugin_desired_setting_key_at(plugin, 0), "token"),
            "desired setting key");
    require(
        luminate_managed_plugin_effective_setting_count(plugin) == 1 &&
            view_equal(luminate_managed_plugin_effective_setting_key_at(plugin, 0), "mode") &&
            view_equal(luminate_setting_value_view_string(luminate_reported_setting_value_visible(
                           luminate_managed_plugin_effective_setting_value_at(plugin, 0))),
                       "calm"),
        "effective setting");
    require(luminate_managed_plugin_locked_setting_count(plugin) == 1 &&
                view_equal(luminate_managed_plugin_locked_setting_at(plugin, 0), "mode"),
            "locked plugin setting");
    luminate_management_snapshot_free(snapshot);

    LuminateSettingValue *secret = NULL;
    LuminateSettingValue *boolean = NULL;
    LuminateSettingValue *integer = NULL;
    LuminateSettingValue *number = NULL;
    LuminateSettingValue *array = NULL;
    LuminateSettingValue *table = NULL;
    require_ok(luminate_setting_value_new_string("hidden", &secret), "string value");
    require_ok(luminate_setting_value_new_boolean(true, &boolean), "Boolean value");
    require_ok(luminate_setting_value_new_integer(-2, &integer), "integer value");
    require_ok(luminate_setting_value_new_number(1.5, &number), "number value");
    require_ok(luminate_setting_value_new_array(&array), "array value");
    require_ok(luminate_setting_value_array_push(array, secret), "array push");
    require_ok(luminate_setting_value_new_table(&table), "table value");
    require_ok(luminate_setting_value_table_insert(table, "tokens", array), "table insert");

    LuminateManagementPatchBuilder *patch = NULL;
    require_ok(luminate_management_patch_builder_new(7, &patch), "patch builder");
    require_ok(luminate_management_patch_builder_set_expected_revision(patch, 7),
               "update expected revision");
    require_ok(luminate_management_patch_builder_set_plugin_enabled(patch, "fixture", true, true),
               "plugin enabled mutation");
    require_ok(luminate_management_patch_builder_set_plugin_reconciliation(
                   patch, "fixture", true, LUMINATE_RECONCILIATION_POLICY_ADOPT),
               "plugin reconciliation mutation");
    require_ok(luminate_management_patch_builder_set_plugin_setting(patch, "fixture",
                                                                    "credentials", table),
               "plugin setting mutation");
    require_ok(luminate_management_patch_builder_clear_plugin_setting(patch, "fixture", "old"),
               "clear plugin setting mutation");
    LuminateDeviceReconciliationPreferenceInput device_preference = {
        .device_id = "lamp",
        .policy = LUMINATE_RECONCILIATION_POLICY_LEAVE,
    };
    LuminateDaemonPreferencesInput daemon_preferences = {
        .has_default_unsupported_policy = true,
        .default_unsupported_policy = LUMINATE_UNSUPPORTED_POLICY_REJECT,
        .has_reconciliation_policy = true,
        .reconciliation_policy = LUMINATE_RECONCILIATION_POLICY_RESTORE,
        .device_reconciliation = &device_preference,
        .device_reconciliation_count = 1,
        .has_cct_emulation = true,
        .cct_emulation = LUMINATE_CCT_EMULATION_DISABLED,
        .has_prefer_shm = true,
        .prefer_shm = false,
        .has_prefer_client_shm = true,
        .prefer_client_shm = true,
    };
    require_ok(
        luminate_management_patch_builder_set_daemon_preferences(patch, &daemon_preferences),
        "daemon preferences mutation");

    LuminateManagementChangeSet *changes = NULL;
    require_ok(luminate_client_patch_management(client, patch, &changes), "patch management");
    require(luminate_management_change_set_revision(changes) == 8, "change revision");
    require(luminate_management_change_set_count(changes) == 1, "change count");
    const LuminateManagementChange *change = luminate_management_change_set_at(changes, 0);
    require(luminate_management_change_kind(change) ==
                LUMINATE_MANAGEMENT_CHANGE_PLUGIN_ACTIVATION,
            "change kind");
    require(view_equal(luminate_management_change_plugin(change), "fixture"), "change plugin");
    luminate_management_change_set_free(changes);

    LuminateTarget target = luminate_target_device("lamp");
    LuminateColourChannelInput channels[] = {
        {LUMINATE_COLOUR_CHANNEL_TEMPERATURE, 4200},
    };
    LuminateColourInput colour = {
        .encoding = LUMINATE_COLOUR_ENCODING_CCT,
        .channels = channels,
        .channel_count = 1,
    };
    LuminateEffect *static_effect = NULL;
    require_ok(luminate_effect_create_static(&colour, &static_effect), "generic static effect");
    require_ok(luminate_client_set_effect(client, &target, static_effect), "generic colour");
    require_ok(luminate_client_refresh_state(client, "lamp"), "refresh state");
    require_ok(luminate_client_save_current(client, &target), "save current");
    require_ok(luminate_client_restore_appearance(client, &target), "restore appearance");
    require_ok(luminate_client_set_emission(client, &target, LUMINATE_EMISSION_EMITTING),
               "set emission");
    require_ok(luminate_client_rescan(client), "rescan");

    LuminateSelectorInput selector = {
        .kind = 0,
        .target = target,
    };
    LuminateCollectionOutcome *outcome = NULL;
    require_ok(
        luminate_client_set_effect_selector(client, &selector, static_effect, false, 0, &outcome),
        "selector colour");
    require(luminate_collection_outcome_applied_count(outcome) == 0 &&
                luminate_collection_outcome_denied_count(outcome) == 0 &&
                luminate_collection_outcome_applied_at(outcome, 0) == NULL &&
                luminate_collection_outcome_denied_at(outcome, 0) == NULL,
            "atomic target selector outcome");
    luminate_collection_outcome_free(outcome);
    outcome = NULL;
    require_ok(luminate_client_set_brightness_selector(client, &selector, 12, false, 0, &outcome),
               "selector brightness");
    luminate_collection_outcome_free(outcome);
    LuminateEffect *effect = NULL;
    require_ok(luminate_effect_create_off(&effect), "selector effect value");
    outcome = NULL;
    require_ok(luminate_client_set_effect_selector(client, &selector, effect, false, 0, &outcome),
               "selector effect");
    luminate_collection_outcome_free(outcome);
    luminate_effect_free(effect);
    outcome = NULL;
    require_ok(luminate_client_set_emission_selector(client, &selector, LUMINATE_EMISSION_DARK,
                                                     &outcome),
               "selector emission");
    luminate_collection_outcome_free(outcome);
    outcome = NULL;
    require_ok(luminate_client_clear_target_selector(client, &selector, &outcome),
               "selector clear");
    luminate_collection_outcome_free(outcome);
    outcome = NULL;
    require_ok(luminate_client_save_current_selector(client, &selector, &outcome),
               "selector save current");
    luminate_collection_outcome_free(outcome);
    outcome = NULL;
    require_ok(luminate_client_restore_appearance_selector(client, &selector, &outcome),
               "selector restore appearance");
    luminate_collection_outcome_free(outcome);

    luminate_management_patch_builder_free(patch);
    luminate_setting_value_free(table);
    luminate_setting_value_free(array);
    luminate_setting_value_free(number);
    luminate_setting_value_free(integer);
    luminate_setting_value_free(boolean);
    luminate_setting_value_free(secret);
    luminate_effect_free(static_effect);
    luminate_client_free(client);
    return 0;
}
