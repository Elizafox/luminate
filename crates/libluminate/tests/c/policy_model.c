/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

static bool view_equal(LuminateStringView actual, const char *expected)
{
    LuminateStringView expected_view = {expected, strlen(expected)};
    return luminate_string_view_equal(actual, expected_view);
}

static void add_rule(struct LuminatePolicyDocumentBuilder *builder, const char *role,
                     const char *id, uint32_t effect, const char *const *device_ids,
                     size_t device_id_count, const char *reason, bool has_cache_hint)
{
    const uint32_t operations[] = {LUMINATE_POLICY_OP_OBSERVE};
    const char *provider_instances[] = {"provider-a"};
    const char *collections[] = {"room"};
    LuminateRuleInput rule = {
        .id = id,
        .effect = effect,
        .operations = operations,
        .operation_count = 1,
        .resources =
            {
                .device_ids = device_ids,
                .device_id_count = device_id_count,
                .provider_instances = provider_instances,
                .provider_instance_count = 1,
                .has_host_attached = true,
                .host_attached = true,
                .collections = collections,
                .collection_count = 1,
            },
        .reason = reason,
        .has_cache_hint_ms = has_cache_hint,
        .cache_hint_ms = 1234,
    };
    require_status(luminate_policy_document_builder_role_add_rule(builder, role, &rule),
                   LUMINATE_STATUS_OK, "add policy rule");
}

static struct LuminateAuthorizationEvaluation *
evaluate(const struct LuminatePolicyDocument *document,
         const struct LuminateRemotePrincipal *principal, const char *device_id)
{
    const char *collections[] = {"room", "other"};
    LuminateResourceInput resource = {
        .device_id = device_id,
        .provider_instance = "provider-a",
        .host_attached = true,
        .collections = collections,
        .collection_count = 2,
    };
    struct LuminateAuthorizationEvaluation *evaluation = NULL;
    require_status(luminate_policy_document_evaluate(document, principal,
                                                     LUMINATE_POLICY_OP_OBSERVE, &resource, 1,
                                                     &evaluation),
                   LUMINATE_STATUS_OK, "evaluate policy");
    require(evaluation != NULL, "evaluation result");
    return evaluation;
}

int main(void)
{
    struct LuminatePolicyDocumentBuilder *builder = NULL;
    const LuminateRuleInput empty_rule = {0};
    require_status(luminate_policy_document_builder_from_document(NULL, &builder),
                   LUMINATE_STATUS_NULL_POINTER, "seed null policy document");
    require_status(luminate_policy_document_builder_remove_role(NULL, "role"),
                   LUMINATE_STATUS_NULL_POINTER, "remove role null builder");
    require_status(luminate_policy_document_builder_role_remove_parent(NULL, "role", "parent"),
                   LUMINATE_STATUS_NULL_POINTER, "remove parent null builder");
    require_status(
        luminate_policy_document_builder_role_replace_rule(NULL, "role", 0, &empty_rule),
        LUMINATE_STATUS_NULL_POINTER, "replace rule null builder");
    require_status(luminate_policy_document_builder_role_remove_rule(NULL, "role", 0),
                   LUMINATE_STATUS_NULL_POINTER, "remove rule null builder");
    require_status(luminate_policy_document_builder_replace_binding(NULL, 0, "authority", NULL, 0,
                                                                    NULL, 0, NULL, 0),
                   LUMINATE_STATUS_NULL_POINTER, "replace binding null builder");
    require_status(luminate_policy_document_builder_remove_binding(NULL, 0),
                   LUMINATE_STATUS_NULL_POINTER, "remove binding null builder");
    require_status(luminate_policy_document_builder_new(41, &builder), LUMINATE_STATUS_OK,
                   "create policy builder");

    require_status(luminate_policy_document_builder_role_add_parent(builder, "operator", "base"),
                   LUMINATE_STATUS_OK, "add role parent");
    const char *allowed_devices[] = {"lamp", "blocked"};
    add_rule(builder, "base", "allow-room", LUMINATE_RULE_EFFECT_ALLOW, allowed_devices, 2,
             "room access", true);
    const char *blocked_devices[] = {"blocked"};
    add_rule(builder, "operator", "deny-blocked", LUMINATE_RULE_EFFECT_DENY, blocked_devices, 1,
             "device blocked", false);

    const char *subjects[] = {"alice"};
    const char *groups[] = {"operators", "operators"};
    const char *roles[] = {"operator"};
    require_status(luminate_policy_document_builder_add_binding(builder, "example", subjects, 1,
                                                                groups, 2, roles, 1),
                   LUMINATE_STATUS_OK, "add policy binding");

    struct LuminatePolicyDocument *document = NULL;
    require_status(luminate_policy_document_build(builder, &document), LUMINATE_STATUS_OK,
                   "build policy document");
    require(document != NULL, "built document");

    require(luminate_policy_document_revision(document) == 41, "document revision");
    require(luminate_policy_document_role_count(document) == 2, "role count");
    require(view_equal(luminate_policy_document_role_name_at(document, 0), "base"),
            "canonical first role");
    require(view_equal(luminate_policy_document_role_name_at(document, 1), "operator"),
            "canonical second role");
    require(!luminate_string_view_is_present(luminate_policy_document_role_name_at(document, 9)),
            "out-of-range role");

    const LuminatePolicyRole *operator_role = luminate_policy_document_role_at(document, 1);
    require(operator_role != NULL, "operator role view");
    require(luminate_policy_role_parent_count(operator_role) == 1, "parent count");
    require(view_equal(luminate_policy_role_parent_at(operator_role, 0), "base"), "parent name");
    require(!luminate_string_view_is_present(luminate_policy_role_parent_at(operator_role, 9)),
            "out-of-range parent");

    const LuminatePolicyRole *base_role = luminate_policy_document_role_at(document, 0);
    require(base_role != NULL, "base role view");
    require(luminate_policy_role_rule_count(base_role) == 1, "base rule count");
    const LuminatePolicyRule *rule = luminate_policy_role_rule_at(base_role, 0);
    require(rule != NULL, "rule view");
    require(view_equal(luminate_policy_rule_id(rule), "allow-room"), "rule id");
    require(luminate_policy_rule_effect(rule) == LUMINATE_RULE_EFFECT_ALLOW, "rule effect");
    require(luminate_policy_rule_operation_count(rule) == 1, "rule operation count");
    require(luminate_policy_rule_operation_at(rule, 0) == LUMINATE_POLICY_OP_OBSERVE,
            "rule operation");
    require(luminate_policy_rule_operation_at(rule, 9) == LUMINATE_DISCRIMINANT_INVALID,
            "out-of-range operation");
    require(view_equal(luminate_policy_rule_reason(rule), "room access"), "rule reason");
    uint64_t cache_hint_ms = 0;
    require(luminate_policy_rule_cache_hint_ms(rule, &cache_hint_ms) && cache_hint_ms == 1234,
            "rule cache hint");
    require(luminate_policy_rule_device_id_count(rule) == 2, "device constraint count");
    require(view_equal(luminate_policy_rule_device_id_at(rule, 0), "blocked"),
            "canonical device constraint");
    require(luminate_policy_rule_provider_instance_count(rule) == 1, "provider constraint count");
    require(view_equal(luminate_policy_rule_provider_instance_at(rule, 0), "provider-a"),
            "provider constraint");
    require(luminate_policy_rule_collection_count(rule) == 1, "collection constraint count");
    require(view_equal(luminate_policy_rule_collection_at(rule, 0), "room"),
            "collection constraint");
    bool host_attached = false;
    require(luminate_policy_rule_host_attached(rule, &host_attached) && host_attached,
            "host constraint");

    require(luminate_policy_document_binding_count(document) == 1, "binding count");
    const LuminatePolicyBinding *binding = luminate_policy_document_binding_at(document, 0);
    require(binding != NULL, "binding view");
    require(view_equal(luminate_policy_binding_authority(binding), "example"),
            "binding authority");
    require(luminate_policy_binding_subject_count(binding) == 1, "binding subject count");
    require(view_equal(luminate_policy_binding_subject_at(binding, 0), "alice"),
            "binding subject");
    require(luminate_policy_binding_group_count(binding) == 1,
            "deduplicated binding group count");
    require(view_equal(luminate_policy_binding_group_at(binding, 0), "operators"),
            "binding group");
    require(luminate_policy_binding_role_count(binding) == 1, "binding role count");
    require(view_equal(luminate_policy_binding_role_at(binding, 0), "operator"), "binding role");

    const char *principal_groups[] = {"zeta", "operators", "alpha"};
    struct LuminateRemotePrincipal *principal = NULL;
    require_status(
        luminate_remote_principal_new("example", "bob", principal_groups, 3, &principal),
        LUMINATE_STATUS_OK, "create group-bound principal");
    require(view_equal(luminate_remote_principal_authority(principal), "example"),
            "principal authority");
    require(view_equal(luminate_remote_principal_subject(principal), "bob"), "principal subject");
    require(luminate_remote_principal_group_count(principal) == 3, "principal group count");
    require(view_equal(luminate_remote_principal_group_at(principal, 0), "alpha"),
            "canonical principal group");
    require(!luminate_string_view_is_present(luminate_remote_principal_group_at(principal, 9)),
            "out-of-range principal group");

    struct LuminateAuthorizationEvaluation *evaluation = evaluate(document, principal, "lamp");
    require(luminate_authorization_evaluation_is_allowed(evaluation), "inherited allow");
    require(view_equal(luminate_authorization_evaluation_reason(evaluation), "room access"),
            "allow reason");
    require(view_equal(luminate_authorization_evaluation_audit_rule(evaluation), "allow-room"),
            "allow audit rule");
    require(luminate_authorization_evaluation_has_cache_hint_ms(evaluation),
            "allow cache hint presence");
    require(luminate_authorization_evaluation_cache_hint_ms(evaluation) == 1234,
            "allow cache hint");
    require(luminate_authorization_evaluation_revision(evaluation) == 41,
            "allow policy revision");
    luminate_authorization_evaluation_free(evaluation);

    evaluation = evaluate(document, principal, "blocked");
    require(!luminate_authorization_evaluation_is_allowed(evaluation), "deny overrides allow");
    require(view_equal(luminate_authorization_evaluation_reason(evaluation), "device blocked"),
            "deny reason");
    require(view_equal(luminate_authorization_evaluation_audit_rule(evaluation), "deny-blocked"),
            "deny audit rule");
    require(!luminate_authorization_evaluation_has_cache_hint_ms(evaluation),
            "deny cache hint absence");
    luminate_authorization_evaluation_free(evaluation);

    evaluation = evaluate(document, principal, "unknown");
    require(!luminate_authorization_evaluation_is_allowed(evaluation), "default deny");
    require(!luminate_string_view_is_present(
                luminate_authorization_evaluation_audit_rule(evaluation)),
            "default deny has no audit rule");
    luminate_authorization_evaluation_free(evaluation);

    struct LuminateAuthorizationEvaluation *invalid = NULL;
    require_status(luminate_policy_document_evaluate(
                       document, principal, LUMINATE_DISCRIMINANT_INVALID, NULL, 0, &invalid),
                   LUMINATE_STATUS_INVALID_ARGUMENT, "reject invalid operation");
    require(invalid == NULL, "invalid operation has no result");

    require_status(luminate_policy_document_builder_set_revision(builder, 42), LUMINATE_STATUS_OK,
                   "update builder revision");
    struct LuminatePolicyDocument *updated = NULL;
    require_status(luminate_policy_document_build(builder, &updated), LUMINATE_STATUS_OK,
                   "rebuild policy document");
    require(luminate_policy_document_revision(document) == 41, "built document is immutable");
    require(luminate_policy_document_revision(updated) == 42, "rebuilt document revision");

    luminate_policy_document_free(updated);
    luminate_remote_principal_free(principal);
    luminate_policy_document_free(document);
    luminate_policy_document_builder_free(builder);
    luminate_authorization_evaluation_free(NULL);
    luminate_remote_principal_free(NULL);
    luminate_policy_document_free(NULL);
    luminate_policy_document_builder_free(NULL);
    return 0;
}
