<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# D-Bus and libluminate parity matrix

This inventory is the checked implementation ledger for the optional D-Bus
companion. Crate tests read libluminate's client sources, fail when a new public
operation is not named here, and reject unresolved “Planned” rows. Existing
D-Bus signatures remain governed by the introspection snapshot.

## Connection, inspection, and events

| libluminate operation | D-Bus representation | Status |
| --- | --- | --- |
| `builder::new`, `builder::path`, `builder::authentication`, `builder::scope` | D-Bus connection setup and bus policy replace local builder configuration | Intentional transport exception |
| `builder::connect` | D-Bus connection and caller attestation | Intentional transport exception |
| `connection::builder` | No remote operation; constructs the local client builder | Intentional transport exception |
| `connection::policy_administration`, `connection::authentication_administration`, `connection::transitions` | Namespace accessors map to `Manager1`/`Manager2` methods | Intentional local API exception |
| `connection::connect`, `connection::connect_path` | Bus activation replaces local socket construction | Intentional transport exception |
| `connection::subscribe`, `connection::subscribe_path` | `Manager2` dirty signals | Intentional transport exception |
| `connection::subscribe_with_baseline`, `connection::subscribe_with_baseline_path` | ObjectManager baseline followed by `Manager2` dirty signals | Intentional transport exception |
| `connection::server_info` | `Manager2.ServerInformation` | Implemented |
| `connection::daemon_version` | `Manager2.ServerInformation` | Implemented |
| `connection::session` | Redacted `Manager2.SessionInformation` | Implemented |
| `connection::socket_path`, `connection::event_socket_path` | No remote operation; the D-Bus service owns its daemon sockets | Intentional transport exception |
| `connection::ping` | `Manager2.Ping` | Implemented |
| `connection::list_devices` | `Manager2.ListDevices` and ObjectManager | Implemented |
| `connection::list_withdrawn_devices` | `Manager2.ListWithdrawnDevices` | Implemented |
| `connection::get_device` | `Manager2.GetDevice` | Implemented |
| `connection::get_state` | `Manager2.GetDeviceState` and target-filtered `Target3.State` | Implemented |
| `connection::get_collection_state` | `Manager2.GetCollectionState` | Implemented |
| `connection::refresh_state` | `Target2.RefreshState` | Implemented |
| `connection::purge_withdrawn_device` | `Manager2.PurgeWithdrawnDevice` | Implemented |
| `connection::rescan` | `Manager2.Rescan` | Implemented |
| `events::next_event` | `Manager2` signals; topology is updated before its dirty signal | Implemented |

Local socket paths, event tickets, and event-socket selection are deliberately
not remote operations. The bus sender and its attested process identity define
the daemon session instead.

## Collections and scenes

| libluminate operation | D-Bus representation | Status |
| --- | --- | --- |
| `collections::create_collection` | `Manager2.CreateCollection` | Implemented |
| `collections::destroy_collection` | `Manager2.DestroyCollection` | Implemented |
| `collections::add_collection_member` | `Manager2.AddCollectionMember` | Implemented |
| `collections::remove_collection_member` | `Manager2.RemoveCollectionMember` | Implemented |
| `collections::list_collections` | `Manager2.ListCollections` | Implemented |
| `collections::get_collection` | `Manager2.GetCollection` | Implemented |
| `scenes::create_scene` | `Manager1.CreateScene` | Implemented |
| `scenes::capture_scene` | `Manager1.CaptureScene` | Implemented |
| `scenes::replace_scene` | `Manager1.ReplaceScene` | Implemented |
| `scenes::recapture_scene` | `Manager1.RecaptureScene` | Implemented |
| `scenes::delete_scene` | `Manager1.DeleteScene` | Implemented |
| `scenes::list_scenes` | `Manager1.ListScenes` | Implemented |
| `scenes::get_scene` | `Manager1.GetScene` | Implemented |
| `scenes::apply_scene` | `Manager1.ApplyScene` | Implemented |

Collection mutation follows libluminate's current non-revisioned contract. No
synthetic expected revision is accepted or claimed to be enforced.

## Direct and selector control

| libluminate operation | D-Bus representation | Status |
| --- | --- | --- |
| `control::set_appearance_slots` | `Target2.SetAppearanceSlots` | Implemented direct form |
| `control::set_effect` | `Target2.SetEffect` | Implemented direct form |
| `control::set_effect_selector` | `Manager2.SetEffectSelector` | Implemented |
| `control::set_colour` | `Target3.SetColour` | Implemented |
| `control::set_colour_selector` | `Manager2.SetColourSelector` | Implemented |
| `control::set_rgb` | `Target3.SetRgb` | Implemented |
| `control::set_rgb_selector` | `Manager2.SetRgbSelector` | Implemented |
| `control::set_cct` | `Target3.SetCct` | Implemented |
| `control::set_cct_selector` | `Manager2.SetCctSelector` | Implemented |
| `control::set_brightness` | `Target2.SetBrightness` | Implemented direct form |
| `control::set_brightness_selector` | `Manager2.SetBrightnessSelector` | Implemented |
| `control::clear_target` | `Target2.ClearDesiredState` | Implemented direct form |
| `control::clear_target_selector` | `Manager2.ClearSelector` | Implemented |
| `control::save_current` | `Target3.SaveCurrent` | Implemented |
| `control::save_current_selector` | `Manager2.SaveCurrentSelector` | Implemented |
| `control::set_off` | `Target2.Off` | Implemented direct form |
| `control::restore_appearance` | `Target3.RestoreAppearance` | Implemented |
| `control::restore_appearance_selector` | `Manager2.RestoreAppearanceSelector` | Implemented |
| `control::set_emission` | `Target3.SetEmission` | Implemented |
| `control::set_emission_selector` | `Manager2.SetEmissionSelector` | Implemented |

## Transitions and frames

| libluminate operation | D-Bus representation | Status |
| --- | --- | --- |
| `transitions::scene_to_scene` | `Manager2.CreateSceneToSceneTransition` | Implemented |
| `transitions::current_to_scene` | `Manager2.CreateCurrentToSceneTransition` | Implemented |
| `transitions::scene_to_states` | `Manager2.CreateSceneToStatesTransition` | Implemented |
| `transitions::current_to_states` | `Manager2.CreateCurrentToStatesTransition` | Implemented |
| `transitions::get` | `Manager2.GetTransition` | Implemented |
| `transitions::abort` | `Manager2.AbortTransition` | Implemented |
| `transitions::wait` | `Manager2.WaitTransition` | Implemented |
| `frames::begin_frame_stream` | `Manager2.BeginFrameStream` | Implemented |
| `frames::upload_frame` | `Manager2.UploadFrame` | Implemented |
| `frames::end_frame_stream` | `Manager2.EndFrameStream` | Implemented |
| `frames::begin_shm_frame_stream` | Capability metadata and `ShmStreamEnded`; negotiation is local-only | Intentional transport exception |
| `frames::end_shm_frame_stream` | `ShmStreamEnded`; stream ownership remains local-only | Intentional transport exception |

Shared-memory handles are tied to the local libluminate connection and are not
safe or useful to relay through D-Bus. Ordinary frame upload is the remote
equivalent.

## Setup, management, and administration

| libluminate operation | D-Bus representation | Status |
| --- | --- | --- |
| `setup::plugin_setup_workflows` | `Manager2.PluginSetupWorkflows` | Implemented |
| `setup::start_plugin_setup` | `Manager2.StartPluginSetup` | Implemented |
| `setup::respond_plugin_setup` | `Manager2.RespondPluginSetup` | Implemented |
| `setup::plugin_setup_session` | `Manager2.PluginSetupSession` | Implemented |
| `setup::cancel_plugin_setup` | `Manager2.CancelPluginSetup` | Implemented |
| `management::get_management` | `Manager1.GetManagement` | Implemented |
| `management::patch_management` | `Manager1.PatchManagement` | Implemented |
| `administration::get` | `Manager1.GetAccessPolicy` | Implemented policy operation |
| `administration::replace` | `Manager1.ReplaceAccessPolicy` | Implemented policy operation |
| `administration::create_attestation` | `Manager2.CreateAttestation` | Implemented |
| `administration::create_principal_attestation` | `Manager2.CreatePrincipalAttestation` | Implemented |
| `administration::list_attestations` | `Manager2.ListAttestations` | Implemented |
| `administration::revoke_attestation` | `Manager2.RevokeAttestation` | Implemented |
| `administration::create_token` | `Manager1.CreateToken` | Implemented |
| `administration::list_tokens` | `Manager1.ListTokens` | Implemented |
| `administration::rotate_token` | `Manager1.RotateToken` | Implemented |
| `administration::revoke_token` | `Manager1.RevokeToken` | Implemented |

## Consumer model families

| Model family | Native representation | Status |
| --- | --- | --- |
| Topology and relationships | `Device2`, `Surface2`, `Group2`, `Element2`, and `Manager2` snapshots; ordered `PhysicalTags` on device, surface, and element scopes | Implemented |
| Target capabilities | `Target3.Capabilities` PascalCase dictionaries | Implemented |
| Device observations and adoption | `Manager2.GetDeviceState` and target-filtered `Target3.State` dictionaries | Implemented |
| Collections and aggregate state | `Manager2` collection dictionaries | Implemented |
| Scenes | Existing typed records and effect dictionaries | Implemented |
| Management and policy | Existing strict dictionaries | Implemented |
| Transitions | `Manager2` dictionaries and records | Implemented |
| Plugin setup | `Manager2` workflow and session dictionaries | Implemented |
| Attestations and session metadata | Sanitized `Manager2` records | Implemented |
| Frames | Ordinary upload dictionaries and acknowledgements | Implemented |

Secrets never appear in snapshots, properties, signals, or this matrix.
