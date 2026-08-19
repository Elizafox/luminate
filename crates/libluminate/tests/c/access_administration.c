/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford */

#include "luminate.h"

#include <stdio.h>
#include <stdlib.h>

static void require(int condition, const char *message)
{
    if (!condition)
    {
        fprintf(stderr, "access-administration C check failed: %s\n", message);
        exit(1);
    }
}

int main(void)
{
    LuminateCreatedToken *token = NULL;
    LuminateCreatedAttestation *attestation = NULL;
    LuminateTokenList *tokens = NULL;
    LuminateAttestationList *attestations = NULL;

    require(luminate_client_create_token(NULL, "desk", "local", "alice", false, 0, &token) ==
                LUMINATE_STATUS_NULL_POINTER,
            "create token rejects a null client");
    require(luminate_client_create_attestation(NULL, "browser", "local", "alice", false, 0,
                                               &attestation) == LUMINATE_STATUS_NULL_POINTER,
            "create attestation rejects a null client");
    require(luminate_client_list_tokens(NULL, &tokens) == LUMINATE_STATUS_NULL_POINTER,
            "list tokens rejects a null client");
    require(luminate_client_list_attestations(NULL, &attestations) ==
                LUMINATE_STATUS_NULL_POINTER,
            "list attestations rejects a null client");
    require(luminate_created_token_secret(NULL, NULL, 0) == 0, "null token has no secret");
    require(luminate_created_attestation_secret(NULL, NULL, 0) == 0,
            "null attestation has no secret");
    require(luminate_token_list_count(NULL) == 0, "null token list is empty");
    require(luminate_attestation_list_count(NULL) == 0, "null attestation list is empty");

    luminate_created_token_free(NULL);
    luminate_created_attestation_free(NULL);
    luminate_token_list_free(NULL);
    luminate_attestation_list_free(NULL);
    return 0;
}
