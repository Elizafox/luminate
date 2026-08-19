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
        fprintf(stderr, "typed model check failed: %s\n", message);
        exit(1);
    }
}

static int view_equal(LuminateStringView value, const char *expected)
{
    LuminateStringView other = {expected, strlen(expected)};
    return luminate_string_view_equal(value, other);
}

static void require_ok(LuminateStatus status, const char *operation)
{
    if (!luminate_status_is_ok(status))
    {
        fprintf(stderr, "%s: %s\n", operation,
                luminate_last_error_message() ? luminate_last_error_message() : "no detail");
        exit(1);
    }
}

static void require_fixed_colour(const LuminateColourInput *input,
                                 const LuminateColourChannelInput *expected,
                                 uintptr_t expected_count, const char *operation)
{
    LuminateEffect *effect = NULL;
    require_ok(luminate_effect_create_static(input, &effect), operation);
    const LuminateColour *colour = luminate_effect_static_colour(effect);
    require(colour != NULL, "static fixed-model colour");
    require(luminate_colour_channel_count(colour) == expected_count,
            "fixed-model colour channel count");

    for (uintptr_t i = 0; i < expected_count; ++i)
    {
        LuminateColourChannel channel = LUMINATE_COLOUR_CHANNEL_RED;
        uint32_t value = 0;
        require(luminate_colour_channel_at(colour, i, &channel, &value),
                "fixed-model component is enumerable");
        require(channel == expected[i].channel, "fixed-model component order");
        require(value == expected[i].value, "fixed-model enumerated value");
        require(luminate_colour_value(colour, expected[i].channel, &value),
                "fixed-model component is available by channel");
        require(value == expected[i].value, "fixed-model component value");
    }

    LuminateColourChannel channel = LUMINATE_COLOUR_CHANNEL_RED;
    uint32_t value = 123;
    require(!luminate_colour_channel_at(colour, expected_count, &channel, &value),
            "fixed-model index is bounded");
    require(channel == LUMINATE_COLOUR_CHANNEL_RED && value == 123,
            "failed enumeration leaves outputs unchanged");

    luminate_effect_free(effect);
}

int main(int argc, char **argv)
{
    require(argc == 2, "socket argument");
    LuminateClient *client = NULL;
    require_ok(luminate_client_connect_path(argv[1], &client), "connect");

    const LuminateColourChannelInput hsv_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_HUE, 120},
        {LUMINATE_COLOUR_CHANNEL_SATURATION, 75},
        {LUMINATE_COLOUR_CHANNEL_VALUE, 60},
    };
    const LuminateColourInput hsv = {
        .encoding = LUMINATE_COLOUR_ENCODING_HSV,
        .channels = hsv_channels,
        .channel_count = 3,
    };
    require_fixed_colour(&hsv, hsv_channels, 3, "create HSV colour");

    const LuminateColourChannelInput hsl_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_HUE, 240},
        {LUMINATE_COLOUR_CHANNEL_SATURATION, 50},
        {LUMINATE_COLOUR_CHANNEL_LIGHTNESS, 40},
    };
    const LuminateColourInput hsl = {
        .encoding = LUMINATE_COLOUR_ENCODING_HSL,
        .channels = hsl_channels,
        .channel_count = 3,
    };
    require_fixed_colour(&hsl, hsl_channels, 3, "create HSL colour");

    const LuminateColourChannelInput cct_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_TEMPERATURE, 4200},
    };
    const LuminateColourInput cct = {
        .encoding = LUMINATE_COLOUR_ENCODING_CCT,
        .channels = cct_channels,
        .channel_count = 1,
    };
    require_fixed_colour(&cct, cct_channels, 1, "create CCT colour");

    const LuminateColourChannelInput monochrome_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_INTENSITY, 80},
    };
    const LuminateColourInput monochrome = {
        .encoding = LUMINATE_COLOUR_ENCODING_MONOCHROME,
        .channels = monochrome_channels,
        .channel_count = 1,
    };
    require_fixed_colour(&monochrome, monochrome_channels, 1, "create monochrome colour");

    LuminateTopologySnapshot *topology = NULL;
    require_ok(luminate_client_list_devices(client, &topology), "list");
    require(luminate_topology_snapshot_device_count(topology) == 1, "device count");
    require(luminate_topology_snapshot_device_at(topology, 9) == NULL, "invalid device index");
    const LuminateDevice *device = luminate_topology_snapshot_device_at(topology, 0);
    require(view_equal(luminate_device_id(device), "fixture-device"), "device id");
    require(view_equal(luminate_device_name(device), "Fixture"), "device name");
    require(view_equal(luminate_device_vendor(device), "Luminate"), "vendor");
    require(!luminate_string_view_is_present(luminate_device_model(device)), "absent model");
    require(view_equal(luminate_device_category(device), "fixture-kind"), "open category");
    require(!luminate_device_host_attached(device), "device is not host attached");
    require(luminate_device_physical_tag_count(device) == 1 &&
                view_equal(luminate_device_physical_tag_at(device, 0), "shape:modular-light-bar"),
            "device physical tag");
    require(!luminate_string_view_is_present(luminate_device_physical_tag_at(device, 1)),
            "device physical tag index is bounded");
    require(luminate_device_physical_tag_count(NULL) == 0 &&
                !luminate_string_view_is_present(luminate_device_physical_tag_at(NULL, 0)),
            "null device has no physical tags");
    require(luminate_device_note_count(device) == 1 &&
                view_equal(luminate_device_note_at(device, 0), "note"),
            "device note");
    require(luminate_device_warning_count(device) == 1 &&
                view_equal(luminate_device_warning_at(device, 0), "warning"),
            "device warning");
    require(luminate_device_surface_count(device) == 1, "device surface count");
    require(luminate_device_group_count(device) == 1, "device group count");

    const LuminateCapabilitySet *capabilities = luminate_device_capabilities(device);
    require(luminate_capability_set_colour_count(capabilities) == 1, "colour capability count");
    const LuminateColourCapability *colour = luminate_capability_set_colour_at(capabilities, 0);
    require(luminate_colour_capability_encoding(colour) == LUMINATE_COLOUR_ENCODING_ADDITIVE,
            "colour encoding");
    require(luminate_colour_capability_channel_count(colour) == 3, "colour channels");
    const LuminateColourChannelCapability *red = luminate_colour_capability_channel_at(colour, 0);
    require(luminate_colour_channel_capability_channel(red) == LUMINATE_COLOUR_CHANNEL_RED,
            "red channel");
    require(luminate_colour_channel_capability_bits(red) == 8, "red channel bits");
    require(luminate_capability_set_cct_emulation(capabilities) == LUMINATE_CCT_EMULATION_AUTO,
            "CCT emulation");
    require(luminate_capability_set_brightness_kind(capabilities) ==
                LUMINATE_CAPABILITY_INDEPENDENT,
            "brightness kind");
    uint8_t capability_brightness_bits = 0;
    uint32_t capability_brightness_maximum = 0;
    uint32_t capability_brightness_scope = LUMINATE_DISCRIMINANT_INVALID;
    require(luminate_capability_set_brightness_bits(capabilities, &capability_brightness_bits) &&
                capability_brightness_bits == 7,
            "brightness bits");
    require(luminate_capability_set_brightness_maximum(capabilities,
                                                       &capability_brightness_maximum) &&
                capability_brightness_maximum == 100,
            "brightness max");
    require(
        luminate_capability_set_brightness_scope(capabilities, &capability_brightness_scope) &&
            capability_brightness_scope == LUMINATE_SCOPE_DEVICE,
        "brightness scope");
    require(luminate_capability_set_emission(capabilities), "emission capability");
    const LuminateFrameUploadCapability *frame =
        luminate_capability_set_frame_upload(capabilities);
    require(luminate_frame_upload_scope(frame) == LUMINATE_SCOPE_SURFACE &&
                luminate_frame_upload_update_mode(frame) == LUMINATE_FRAME_UPDATE_BOTH &&
                luminate_frame_upload_has_max_rate_hz(frame) &&
                luminate_frame_upload_max_rate_hz(frame) == 60 &&
                luminate_frame_upload_atomic(frame) &&
                luminate_frame_upload_buffering(frame) == LUMINATE_BUFFERING_DOUBLE,
            "reserved frame metadata");
    const LuminateShmFrameCapability *shm = luminate_frame_upload_shm(frame);
    require(shm != NULL, "shared-memory frame capability");
    require(luminate_shm_frame_pixel_format_count(shm) == 2, "shared-memory pixel formats");
    require(luminate_shm_frame_pixel_format_at(shm, 0) == LUMINATE_SHM_PIXEL_FORMAT_RGB8 &&
                luminate_shm_frame_pixel_format_at(shm, 1) == LUMINATE_SHM_PIXEL_FORMAT_RGBX8,
            "shared-memory pixel format order");
    require(luminate_shm_pixel_format_bytes_per_pixel(LUMINATE_SHM_PIXEL_FORMAT_RGB8) == 3,
            "shared-memory pixel stride");
    require(luminate_shm_frame_shape_kind(shm) == LUMINATE_SHM_FRAME_SHAPE_MATRIX,
            "shared-memory frame shape");
    uint32_t shm_pixel_count = 0;
    uint32_t shm_matrix_width = 0;
    uint32_t shm_matrix_height = 0;
    require(
        luminate_shm_frame_pixel_count(shm, &shm_pixel_count) && shm_pixel_count == 6 &&
            luminate_shm_frame_matrix_width(shm, &shm_matrix_width) && shm_matrix_width == 3 &&
            luminate_shm_frame_matrix_height(shm, &shm_matrix_height) && shm_matrix_height == 2,
        "shared-memory matrix dimensions");
    require(luminate_shm_frame_has_max_rate_hz(shm) && luminate_shm_frame_max_rate_hz(shm) == 120,
            "shared-memory frame rate");
    require(luminate_capability_set_persistence_kind(capabilities) ==
                LUMINATE_PERSISTENCE_PROFILES,
            "persistence kind");
    uint32_t persistence_requirement = LUMINATE_DISCRIMINANT_INVALID;
    uint16_t persistence_slots = 0;
    bool persistence_explicit_commit = false;
    bool persistence_readback = false;
    require(
        luminate_capability_set_persistence_requirement(capabilities, &persistence_requirement) &&
            persistence_requirement == LUMINATE_PERSISTENCE_OPTIONAL,
        "persistence requirement");
    require(luminate_capability_set_persistence_slots(capabilities, &persistence_slots) &&
                persistence_slots == 2,
            "profile slots");
    require(luminate_capability_set_persistence_explicit_commit(capabilities,
                                                                &persistence_explicit_commit) &&
                persistence_explicit_commit,
            "explicit persistence commit");
    require(luminate_capability_set_persistence_readback(capabilities, &persistence_readback) &&
                persistence_readback,
            "persistence readback");
    require(luminate_capability_set_state_readback_kind(capabilities) ==
                LUMINATE_STATE_READBACK_READABLE,
            "state readback kind");
    require(luminate_capability_set_readable_facet_count(capabilities) == 1,
            "readable facet count");
    const LuminateReadableFacet *readable =
        luminate_capability_set_readable_facet_at(capabilities, 0);
    require(luminate_readable_facet_kind(readable) == LUMINATE_FACET_APPEARANCE,
            "readable facet kind");
    require(luminate_readable_facet_fidelity(readable) == LUMINATE_READBACK_EXACT,
            "readback fidelity");
    require(!luminate_capability_set_read_disturbs_output(capabilities),
            "readback does not disturb output");
    require(luminate_capability_set_notifies_external_changes(capabilities),
            "external change notification");
    require(luminate_physical_power_scope(luminate_capability_set_physical_power(capabilities)) ==
                LUMINATE_SCOPE_DEVICE,
            "physical power scope");
    const LuminatePowerDomainRef *power_domain =
        luminate_capability_set_power_domain(capabilities);
    require(luminate_power_domain_kind(power_domain) == LUMINATE_POWER_DOMAIN_DEVICE,
            "power domain kind");
    require(!luminate_string_view_is_present(luminate_power_domain_surface_id(power_domain)),
            "device power domain has no surface");

    const LuminateHardwareEffectsCapability *hardware =
        luminate_capability_set_hardware_effects(capabilities);
    require(luminate_hardware_effects_scope(hardware) == LUMINATE_SCOPE_DEVICE,
            "hardware effect scope");
    require(!luminate_hardware_effects_concurrent_with_streaming(hardware),
            "hardware effects exclude streaming");
    require(luminate_hardware_effects_effect_count(hardware) == 1, "hardware effect count");
    const LuminateHardwareEffectDescriptor *descriptor =
        luminate_hardware_effects_effect_at(hardware, 0);
    require(view_equal(luminate_hardware_effect_descriptor_id(descriptor), "scene"),
            "hardware id");
    require(view_equal(luminate_hardware_effect_descriptor_name(descriptor), "Scene"),
            "hardware name");
    require(luminate_hardware_effect_descriptor_parameter_count(descriptor) == 6,
            "all hardware parameter kinds");
    const LuminateEffectParameter *colour_parameter =
        luminate_hardware_effect_descriptor_parameter_at(descriptor, 0);
    uint8_t minimum_colours = 0;
    uint8_t maximum_colours = 0;
    require(luminate_effect_parameter_colour_count_range(colour_parameter, &minimum_colours,
                                                         &maximum_colours) &&
                minimum_colours == 1 && maximum_colours == 2,
            "colour parameter");
    LuminateU16Range speed = {0};
    require(luminate_effect_parameter_speed_range(
                luminate_hardware_effect_descriptor_parameter_at(descriptor, 1), &speed) &&
                speed.minimum == 1 && speed.maximum == 10 && speed.step == 1,
            "speed parameter");
    const LuminateEffectParameter *direction_parameter =
        luminate_hardware_effect_descriptor_parameter_at(descriptor, 2);
    require(luminate_effect_parameter_direction_count(direction_parameter) == 2 &&
                luminate_effect_parameter_direction_at(direction_parameter, 1) ==
                    LUMINATE_DIRECTION_REVERSE,
            "direction parameter");
    LuminateU32Range duration = {0};
    require(luminate_effect_parameter_duration_range(
                luminate_hardware_effect_descriptor_parameter_at(descriptor, 3), &duration) &&
                duration.minimum == 100 && duration.maximum == 1000 && duration.step == 100,
            "duration parameter");
    uint8_t brightness_bits = 0;
    require(
        luminate_effect_parameter_brightness_bits(
            luminate_hardware_effect_descriptor_parameter_at(descriptor, 4), &brightness_bits) &&
            brightness_bits == 8,
        "brightness parameter");
    const LuminateEffectParameter *choice_parameter =
        luminate_hardware_effect_descriptor_parameter_at(descriptor, 5);
    require(luminate_effect_parameter_kind(choice_parameter) == LUMINATE_EFFECT_PARAMETER_CHOICE,
            "choice parameter kind");
    require(luminate_effect_parameter_choice_count(choice_parameter) == 1, "choice count");
    const LuminateEffectChoice *choice = luminate_effect_parameter_choice_at(choice_parameter, 0);
    require(view_equal(luminate_effect_choice_id(choice), "calm") &&
                view_equal(luminate_effect_choice_name(choice), "Calm"),
            "choice metadata");

    const LuminateSurface *surface = luminate_device_surface_at(device, 0);
    require(view_equal(luminate_surface_id(surface), "panel"), "surface id");
    require(view_equal(luminate_surface_name(surface), "Panel"), "surface name");
    require(luminate_surface_kind(surface) == LUMINATE_SURFACE_MATRIX, "matrix surface");
    LuminateMatrixCell size = {0};
    require(luminate_surface_matrix_size(surface, &size) && size.row == 2 && size.column == 3,
            "matrix size");
    require(luminate_surface_element_count(surface) == 1, "surface element count");
    require(luminate_surface_physical_tag_count(surface) == 2 &&
                view_equal(luminate_surface_physical_tag_at(surface, 0), "layout:grid") &&
                view_equal(luminate_surface_physical_tag_at(surface, 1), "position:front"),
            "surface physical tags preserve order");
    require(!luminate_string_view_is_present(luminate_surface_physical_tag_at(surface, 2)) &&
                luminate_surface_physical_tag_count(NULL) == 0 &&
                !luminate_string_view_is_present(luminate_surface_physical_tag_at(NULL, 0)),
            "surface physical tag bounds and null root");
    LuminateStringView borrowed_surface_tag = luminate_surface_physical_tag_at(surface, 0);
    require(luminate_surface_note_count(surface) == 1 &&
                view_equal(luminate_surface_note_at(surface, 0), "surface note"),
            "surface note");
    require(luminate_surface_warning_count(surface) == 1 &&
                view_equal(luminate_surface_warning_at(surface, 0), "surface warning"),
            "surface warning");
    require(luminate_capability_set_colour_count(luminate_surface_capabilities(surface)) == 0,
            "surface default capabilities");
    const LuminateElement *element = luminate_surface_element_at(surface, 0);
    require(view_equal(luminate_element_id(element), "pixel"), "element id");
    require(view_equal(luminate_element_name(element), "Pixel"), "element name");
    require(luminate_element_kind(element) == LUMINATE_ELEMENT_LED, "element kind");
    require(luminate_element_geometry_kind(element) == LUMINATE_GEOMETRY_RECT, "rect geometry");
    require(luminate_element_physical_tag_count(element) == 2 &&
                view_equal(luminate_element_physical_tag_at(element, 0), "shape:rectangular") &&
                view_equal(luminate_element_physical_tag_at(element, 1), "position:top-left"),
            "element physical tags preserve order");
    require(!luminate_string_view_is_present(luminate_element_physical_tag_at(element, 2)) &&
                luminate_element_physical_tag_count(NULL) == 0 &&
                !luminate_string_view_is_present(luminate_element_physical_tag_at(NULL, 0)),
            "element physical tag bounds and null root");
    LuminateStringView borrowed_element_tag = luminate_element_physical_tag_at(element, 1);
    LuminateRect rect = {0};
    require(luminate_element_geometry_rect(element, &rect) && rect.width == 0.5f &&
                rect.height == 0.5f,
            "rect values");
    require(luminate_element_note_count(element) == 1 &&
                view_equal(luminate_element_note_at(element, 0), "element note"),
            "element note");
    require(luminate_element_warning_count(element) == 1 &&
                view_equal(luminate_element_warning_at(element, 0), "element warning"),
            "element warning");
    require(luminate_capability_set_colour_count(luminate_element_capabilities(element)) == 0,
            "element default capabilities");
    require(view_equal(borrowed_surface_tag, "layout:grid") &&
                view_equal(borrowed_element_tag, "position:top-left"),
            "nested tag views remain valid while the topology snapshot is owned");

    const LuminateGroup *group = luminate_device_group_at(device, 0);
    require(view_equal(luminate_group_id(group), "all"), "group id");
    require(view_equal(luminate_group_name(group), "All"), "group name");
    require(view_equal(luminate_group_description(group), "fixture group"), "group description");
    require(luminate_group_kind(group) == LUMINATE_GROUP_APPLICATION, "group kind");
    require(luminate_group_member_count(group) == 1, "group member count");
    const LuminateGroupMember *member = luminate_group_member_at(group, 0);
    require(luminate_group_member_kind(member) == LUMINATE_GROUP_MEMBER_ELEMENT,
            "group element member");
    require(view_equal(luminate_group_member_surface_id(member), "panel"), "member surface id");
    require(view_equal(luminate_group_member_element_id(member), "pixel"), "member element id");
    require(!luminate_string_view_is_present(luminate_group_member_group_id(member)),
            "element member has no group id");
    require(luminate_group_note_count(group) == 1 &&
                view_equal(luminate_group_note_at(group, 0), "group note"),
            "group note");
    require(luminate_group_warning_count(group) == 1 &&
                view_equal(luminate_group_warning_at(group, 0), "group warning"),
            "group warning");
    require(luminate_capability_set_colour_count(luminate_group_capabilities(group)) == 0,
            "group default capabilities");

    LuminateDeviceSnapshot *device_root = NULL;
    require_ok(luminate_client_get_device(client, "fixture-device", &device_root), "device");
    require(view_equal(luminate_device_id(luminate_device_snapshot_device(device_root)),
                       "fixture-device"),
            "individual device snapshot");
    luminate_device_snapshot_free(device_root);

    LuminateStateSnapshot *state_root = NULL;
    require_ok(luminate_client_get_state(client, "fixture-device", &state_root), "state");
    const LuminateState *state = luminate_state_snapshot_state(state_root);
    require(view_equal(luminate_state_device_id(state), "fixture-device"), "state device id");
    require(luminate_state_reachability(state) == LUMINATE_REACHABILITY_REACHABLE,
            "state reachability");
    require(luminate_state_reconciliation(state) == LUMINATE_RECONCILIATION_COMPLETE,
            "state reconciliation");
    require(luminate_state_observation_count(state) == 5, "all state facets");
    /* Any observation's target works to look the others up by kind; order is
     * display-only and not a lookup contract. */
    const LuminateTargetView *state_target =
        luminate_observation_target(luminate_state_observation_at(state, 0));
    const LuminateFacetObservation *appearance =
        luminate_state_find_observation(state, state_target, LUMINATE_FACET_APPEARANCE);
    require(luminate_facet_value_kind(luminate_observation_value(appearance)) ==
                LUMINATE_FACET_APPEARANCE,
            "appearance facet");
    require(luminate_colour_channel_count(
                luminate_facet_value_colour(luminate_observation_value(appearance))) == 3,
            "state colour channels");
    require(luminate_observation_confidence(appearance) == LUMINATE_CONFIDENCE_CONFIRMED,
            "observation confidence");
    require(luminate_observation_source(appearance) == LUMINATE_SOURCE_READBACK,
            "observation source");
    require(luminate_observation_observed_at_ms(appearance) == 1, "observation timestamp");
    require(!luminate_observation_stale(appearance), "observation freshness");
    require(luminate_target_view_kind(state_target) == LUMINATE_TARGET_DEVICE,
            "state target kind");
    require(view_equal(luminate_target_view_device_id(state_target), "fixture-device"),
            "state target device");
    require(!luminate_string_view_is_present(luminate_target_view_surface_id(state_target)),
            "device target has no surface");
    require(luminate_facet_value_appearance_kind(luminate_observation_value(appearance)) ==
                LUMINATE_APPEARANCE_STATIC,
            "appearance kind");
    const LuminateColour *state_colour =
        luminate_facet_value_colour(luminate_observation_value(appearance));
    require(luminate_colour_encoding(state_colour) == LUMINATE_COLOUR_ENCODING_ADDITIVE,
            "state colour encoding");
    LuminateColourChannel state_channel = LUMINATE_COLOUR_CHANNEL_INTENSITY;
    uint32_t state_value = 0;
    require(luminate_colour_channel_at(state_colour, 0, &state_channel, &state_value) &&
                state_channel == LUMINATE_COLOUR_CHANNEL_RED && state_value == 1,
            "state red channel");

    const LuminateFacetObservation *brightness =
        luminate_state_find_observation(state, state_target, LUMINATE_FACET_BRIGHTNESS);
    uint32_t facet_brightness = 0;
    require(luminate_facet_value_brightness(luminate_observation_value(brightness),
                                            &facet_brightness) &&
                facet_brightness == 50,
            "brightness facet");
    bool slots_complete = true;
    require(!luminate_facet_value_appearance_slots_complete(
                luminate_observation_value(brightness), &slots_complete) &&
                slots_complete,
            "non-slot facet preserves completeness output");
    const LuminateFacetObservation *emission =
        luminate_state_find_observation(state, state_target, LUMINATE_FACET_EMISSION);
    require(luminate_facet_value_emission(luminate_observation_value(emission)) ==
                LUMINATE_EMISSION_EMITTING,
            "emission facet");
    const LuminateFacetObservation *physical_power =
        luminate_state_find_observation(state, state_target, LUMINATE_FACET_PHYSICAL_POWER);
    require(luminate_facet_value_physical_power(luminate_observation_value(physical_power)) ==
                LUMINATE_PHYSICAL_POWER_ON,
            "physical power facet");

    const LuminateFacetObservation *effective_appearance =
        luminate_state_find_observation(state, state_target, LUMINATE_FACET_EFFECTIVE_APPEARANCE);
    require(luminate_facet_value_kind(luminate_observation_value(effective_appearance)) ==
                LUMINATE_FACET_EFFECTIVE_APPEARANCE,
            "effective appearance facet");
    require(luminate_facet_value_effective_appearance_kind(luminate_observation_value(
                effective_appearance)) == LUMINATE_EFFECTIVE_APPEARANCE_EFFECT,
            "effective appearance kind");

    const LuminateEffectView *running =
        luminate_facet_value_effect(luminate_observation_value(effective_appearance));
    require(luminate_effect_view_kind(running) == LUMINATE_EFFECT_BREATHE, "running effect kind");
    LuminateRgb running_colour = {0};
    require(luminate_effect_view_rgb(running, &running_colour), "running effect RGB");
    require(running_colour.r == 7 && running_colour.g == 8 && running_colour.b == 9,
            "running effect colour");
    uint32_t running_period = 0;
    require(luminate_effect_view_period_ms(running, &running_period) && running_period == 250,
            "running effect period");
    require(luminate_facet_value_kind(luminate_observation_value(effective_appearance)) ==
                LUMINATE_FACET_EFFECTIVE_APPEARANCE,
            "state snapshot intact after reading borrowed effect");
    require(luminate_state_adoption_count(state) == 1, "adoption count");
    const LuminateAdoption *adoption = luminate_state_adoption_at(state, 0);
    require(luminate_adoption_status(adoption) == LUMINATE_ADOPTION_DURABLE, "adoption status");
    require(luminate_adoption_facet(adoption) == LUMINATE_FACET_APPEARANCE, "adoption facet");
    require(view_equal(luminate_target_view_device_id(luminate_adoption_target(adoption)),
                       "fixture-device"),
            "adoption target");
    require(view_equal(luminate_state_latest_error(state), "last failure"), "latest error");
    require(luminate_state_has_latest_attempt_ms(state) &&
                luminate_state_latest_attempt_ms(state) == 99,
            "latest attempt");

    LuminateTarget target = luminate_target_device("fixture-device");
    LuminateRgb colours[] = {luminate_rgb(1, 2, 3), luminate_rgb(4, 5, 6)};
    LuminateColourChannelInput static_channels[] = {
        {LUMINATE_COLOUR_CHANNEL_RED, 1},
        {LUMINATE_COLOUR_CHANNEL_GREEN, 2},
        {LUMINATE_COLOUR_CHANNEL_BLUE, 3},
    };
    LuminateColourInput static_colour = {
        .encoding = LUMINATE_COLOUR_ENCODING_ADDITIVE,
        .channels = static_channels,
        .channel_count = 3,
    };
    LuminateEffect *effects[10] = {NULL};
    require_ok(luminate_effect_create_off(&effects[0]), "off");
    require_ok(luminate_effect_create_static(&static_colour, &effects[1]), "static");
    require_ok(luminate_effect_create_breathe(colours[0], 100, &effects[2]), "breathe");
    require_ok(luminate_effect_create_pulse(colours[0], 100, &effects[3]), "pulse");
    require_ok(luminate_effect_create_scanner(colours[0], 100, &effects[4]), "scanner");
    require_ok(luminate_effect_create_morph(colours, 2, 100, &effects[5]), "morph");
    require_ok(luminate_effect_create_spectrum(100, &effects[6]), "spectrum");
    require_ok(luminate_effect_create_rainbow(100, &effects[7]), "rainbow");
    require_ok(luminate_effect_create_hardware("scene", &effects[8]), "hardware");
    require_ok(luminate_effect_hardware_add_colour(effects[8], colours[0]), "hardware colour");
    require_ok(luminate_effect_hardware_set_speed(effects[8], 5), "hardware speed");
    require_ok(luminate_effect_hardware_set_direction(effects[8], LUMINATE_DIRECTION_FORWARD),
               "hardware direction");
    require_ok(luminate_effect_hardware_set_duration_ms(effects[8], 100), "hardware duration");
    require_ok(luminate_effect_hardware_set_brightness(effects[8], 10), "hardware brightness");
    require_ok(luminate_effect_hardware_set_choice(effects[8], "calm"), "hardware choice");
    require(view_equal(luminate_effect_hardware_id(effects[8]), "scene"), "effect hardware id");
    require(luminate_effect_rgb_count(effects[8]) == 1, "hardware effect colour count");
    LuminateRgb hardware_colour = {0};
    require(luminate_effect_rgb_at(effects[8], 0, &hardware_colour),
            "hardware effect colour lookup");
    require(hardware_colour.r == 1 && hardware_colour.g == 2 && hardware_colour.b == 3,
            "hardware effect colour");
    uint16_t hardware_speed = 0;
    uint32_t hardware_duration = 0;
    uint32_t hardware_brightness = 0;
    require(luminate_effect_speed(effects[8], &hardware_speed) && hardware_speed == 5,
            "hardware effect speed");
    require(luminate_effect_direction(effects[8]) == LUMINATE_DIRECTION_FORWARD,
            "hardware effect direction");
    require(luminate_effect_duration_ms(effects[8], &hardware_duration) &&
                hardware_duration == 100,
            "hardware effect duration");
    require(luminate_effect_brightness(effects[8], &hardware_brightness) &&
                hardware_brightness == 10,
            "hardware effect brightness");
    require(view_equal(luminate_effect_choice(effects[8]), "calm"), "hardware effect choice");
    require_ok(luminate_effect_create_strobe(colours[0], 100, &effects[9]), "strobe");
    for (size_t i = 0; i < 10; ++i)
    {
        require(luminate_effect_kind(effects[i]) == i, "effect kind");
        require_ok(luminate_client_set_effect(client, &target, effects[i]), "set effect");
        luminate_effect_free(effects[i]);
    }

    const char invalid_utf8[] = {(char)0xff, '\0'};
    LuminateDeviceSnapshot *invalid = NULL;
    require(luminate_client_get_device(client, invalid_utf8, &invalid) ==
                LUMINATE_STATUS_INVALID_UTF8,
            "invalid UTF-8");

    luminate_state_snapshot_free(state_root);
    luminate_topology_snapshot_free(topology);
    luminate_device_snapshot_free(NULL);
    luminate_state_snapshot_free(NULL);
    luminate_client_free(client);
    return 0;
}
