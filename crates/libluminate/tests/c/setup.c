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
        fprintf(stderr, "setup C check failed: %s (%s)\n", message,
                luminate_last_error_message() ? luminate_last_error_message() : "no detail");
        exit(1);
    }
}

static int view_equal(LuminateStringView value, const char *expected)
{
    return value.len == strlen(expected) && memcmp(value.data, expected, value.len) == 0;
}

static void view_copy(LuminateStringView value, char *destination, size_t capacity)
{
    require(value.len + 1 <= capacity, "string view copy capacity");
    memcpy(destination, value.data, value.len);
    destination[value.len] = '\0';
}

static void check_workflows(LuminatePluginSetupWorkflowList *workflows)
{
    require(luminate_plugin_setup_workflow_list_count(workflows) == 1, "workflow count");
    const LuminatePluginSetupWorkflow *workflow =
        luminate_plugin_setup_workflow_list_at(workflows, 0);
    require(workflow != NULL, "workflow lookup");
    require(view_equal(luminate_plugin_setup_workflow_plugin(workflow), "example"),
            "plugin name");
    require(view_equal(luminate_plugin_setup_workflow_id(workflow), "pair"), "workflow id");
    require(view_equal(luminate_plugin_setup_workflow_label(workflow), "Pair hardware"),
            "workflow label");
    require(view_equal(luminate_plugin_setup_workflow_description(workflow),
                       "Connect nearby hardware."),
            "workflow description");
    require(luminate_plugin_setup_workflow_kind(workflow) == LUMINATE_PLUGIN_SETUP_PROVISION,
            "workflow kind");
    require(luminate_plugin_setup_workflow_list_at(workflows, 1) == NULL, "out-of-range lookup");
}

int main(int argc, char **argv)
{
    require(argc == 2, "socket argument");

    LuminateClient *client = NULL;
    require(luminate_client_connect_path(argv[1], &client) == LUMINATE_STATUS_OK, "connect");

    LuminatePluginSetupWorkflowList *workflows = NULL;
    require(luminate_client_plugin_setup_workflows(client, "example", &workflows) ==
                LUMINATE_STATUS_OK,
            "blocking workflow list");
    check_workflows(workflows);
    luminate_plugin_setup_workflow_list_free(workflows);

    LuminatePluginSetupSession *session = NULL;
    require(luminate_client_plugin_setup_start(client, "example", "pair", &session) ==
                LUMINATE_STATUS_OK,
            "start setup session");
    require(luminate_plugin_setup_session_state(session) == LUMINATE_PLUGIN_SETUP_PHYSICAL_ACTION,
            "physical action state");
    require(view_equal(luminate_plugin_setup_session_message(session), "Press the button."),
            "physical action message");
    require(luminate_plugin_setup_session_generation(session) == 1, "initial generation");
    char session_id[33];
    view_copy(luminate_plugin_setup_session_id(session), session_id, sizeof(session_id));
    luminate_plugin_setup_session_free(session);

    session = NULL;
    require(luminate_client_plugin_setup_get(client, session_id, &session) == LUMINATE_STATUS_OK,
            "get setup session");
    require(luminate_plugin_setup_session_state(session) == LUMINATE_PLUGIN_SETUP_PHYSICAL_ACTION,
            "get preserves state");
    luminate_plugin_setup_session_free(session);

    session = NULL;
    require(luminate_client_plugin_setup_cancel(client, session_id, &session) ==
                LUMINATE_STATUS_OK,
            "cancel setup session");
    require(luminate_plugin_setup_session_state(session) == LUMINATE_PLUGIN_SETUP_CANCELLED,
            "cancelled state");
    luminate_plugin_setup_session_free(session);

    session = NULL;
    require(luminate_client_plugin_setup_confirm(client, session_id, 1, &session) ==
                LUMINATE_STATUS_OK,
            "confirm setup physical action");
    require(luminate_plugin_setup_session_state(session) == LUMINATE_PLUGIN_SETUP_COMPLETED,
            "completed state");
    require(view_equal(luminate_plugin_setup_session_message(session), "Connected."),
            "completion summary");
    uint64_t completed_revision = 0;
    require(luminate_plugin_setup_session_revision(session, &completed_revision) &&
                completed_revision == 7,
            "completion revision");
    luminate_plugin_setup_session_free(session);

    luminate_client_free(client);
    return 0;
}
