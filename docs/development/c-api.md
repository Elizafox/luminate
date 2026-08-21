<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# C client API

`libluminate` exposes a strict C11 interface in
`crates/libluminate/luminate.h`. The same header can be included from C23 and
C++17. The C ABI is versioned independently from the private daemon protocol;
`LUMINATE_C_ABI_VERSION` and `luminate_c_abi_version()` currently report 28.

Enum-like arguments, return values, output pointers, and public input fields
use their semantic `Luminate*` typedefs. These remain `uint32_t` at the ABI
level. Raw fixed-width integers are reserved for quantities such as revisions,
brightness values, dimensions, and generations.

`luminate_client_ping` checks the existing control connection and daemon
request loop without returning metadata or invoking resource authorization.
It does not create another connection or session. Excess
calls return `LUMINATE_STATUS_RATE_LIMITED` with retry guidance available
through the ordinary error-metadata accessors.

## Scenes

The C API represents scenes as owned `LuminateSceneSnapshot` and
`LuminateSceneList` roots with borrowed scene and binding accessors. Explicit
authoring uses `LuminateSceneBindingInput`; capture and recapture accept
concrete targets and an optional dynamic collection ID. Create, capture,
replace, recapture, delete, list, get, and immediate apply operations mirror
the ordinary Rust client. Release owned roots with
`luminate_scene_snapshot_free` and `luminate_scene_list_free`.
Changing this client API does not by itself change the daemon protocol ABI.

Use `luminate_scene_builder_from_scene` when editing an existing scene without
reconstructing unchanged bindings. The builder copies the scene ID, expected
revision, text, and every binding. Its binding views are borrowed until the
next mutation. A typical edit is:

```c
LuminateSceneBuilder *builder = NULL;
LuminateSceneSnapshot *replacement = NULL;

luminate_scene_builder_from_scene(scene, &builder);
const LuminateSceneBinding *old =
    luminate_scene_builder_binding_at(builder, binding_index);
/* Read old, clone any effect payload needed by the replacement input. */
luminate_scene_builder_replace_binding(builder, binding_index, &input);
luminate_client_replace_scene_from_builder(client, builder, &replacement);

luminate_effect_free(cloned_effect);
luminate_scene_builder_free(builder);
luminate_scene_snapshot_free(replacement);
```

`luminate_scene_builder_set_description(builder, NULL)` clears the optional
description. Replacement does not consume the builder. A revision conflict is
reported normally, so a client can fetch the current scene, seed a fresh
builder, and replay its edit. Binding order is preserved and observable, but
does not currently change scene application behaviour; ABI 27 has no binding
move operation. `luminate_client_replace_scene_from_builder_async` copies the
builder during submission, following the ordinary asynchronous lifetime rules.

## Ownership and borrowing

Topology, individual devices, state, and events are returned as owned opaque
roots. Each root has one matching null-safe free function. Nested devices,
surfaces, elements, groups, capabilities, observations, and other model values
are borrowed views into their root. Never free a nested view. Every nested
pointer and string view becomes invalid when the owning root is freed.

`LuminateStringView` is length-delimited UTF-8, not a C string. A null `data`
pointer means an optional value is absent. A present empty string has non-null
`data` and zero `len`. String views do not need freeing and are not guaranteed
to have a trailing NUL byte.

Collections use a `*_count`/`*_at` pair. An out-of-range `*_at` returns null
(or an absent string view) and deliberately leaves the thread-local last error
unchanged. Snapshots are immutable and exclusively own the storage borrowed by
their views.

Device physical tags describe the fixture or peripheral as a whole. Surface
physical tags describe only that addressable region or mapping, and element
physical tags describe only the individual control or light. Tags do not
inherit between these scopes.

Use the `luminate_device_physical_tag_*`,
`luminate_surface_physical_tag_*`, and `luminate_element_physical_tag_*`
count/at pairs to enumerate tags in provider order. The returned string views
borrow from the owning snapshot and become invalid when that snapshot is freed,
just like other nested string views.
The current C API exposes topology through these typed owned roots and borrowed
views; it has no parallel JSON topology entry point requiring a second
physical-tag accessor or lifetime contract.

The [physical-tag reference](physical-tags.md) documents the standard
vocabulary and plugin-defined extensions. Unknown tags remain valid and should
be ignored safely by consumers that do not recognize them.

## Structured error metadata

The existing last-error message remains the general human-readable diagnostic.
Callers can also inspect permission-denied and protocol-incompatibility errors
without parsing that message. The `luminate_last_error_*` accessors expose a
safe policy reason, the incompatible daemon version and optional reason, and
the supported primary or event protocol version when applicable. String views
borrow from the current thread's last-error storage. Numeric accessors return
false without changing the output when the metadata does not apply.

Asynchronous operations expose the same fields through
`luminate_async_operation_error_*`. Those views borrow from the completed
operation and remain valid until its final reference is released. Cancellation
has no error metadata. These accessors never expose credentials, hidden
resources, filesystem details, or provider continuation data.

State observations are the one collection with a second access path:
`luminate_state_find_observation` looks one up by target and
`LUMINATE_FACET_*` kind. `observations`' `*_count`/`*_at` order is for
display only and carries no lookup contract, so this is the only supported
way to find a specific facet.

## Thread safety

Operations on one `LuminateClient` may be called concurrently from multiple
threads. The caller must still ensure `luminate_client_free` does not overlap
another use of that exact external handle. One event subscription permits only
one active next-event wait. Different handles, and a client alongside its own
event subscription, may be used concurrently without coordination. Snapshots
are immutable once returned, so reading from one on multiple threads is safe as
long as no thread frees it while another is still reading.

`luminate_client_server_info` returns an opaque owned `LuminateServerInfo`.
Its daemon name and version accessors return borrowed string views, and its
protocol accessor returns the negotiated primary protocol ABI version. Release
the object once with `luminate_server_info_free`; null is a no-op.

Ordinary client operations are deadline-bounded when communicating with the
daemon. Each request applies the protocol I/O deadline separately to sending
the request and receiving its response. A daemon that accepts a request but
stops responding therefore returns `LUMINATE_STATUS_TIMEOUT` instead of
blocking indefinitely; because the interrupted stream may be between frames,
later operations on that handle return `LUMINATE_STATUS_CONNECTION_POISONED`.
Free the handle and reconnect to recover. `luminate_client_free` joins the
client's worker thread before returning. Under the no-overlap rule above this
is prompt; even if it overlaps a stalled request, the request's I/O deadline
bounds that join.

## Events

`luminate_event_subscription_next` remains a blocking call and deliberately has
no idle deadline: a healthy topology may remain unchanged indefinitely. Do not
free its subscription concurrently to try to cancel that wait. Inspect its
result with `luminate_event_kind`: `LUMINATE_EVENT_TOPOLOGY_CHANGED` invalidates
the listed devices' topology, while `LUMINATE_EVENT_STATE_CHANGED` invalidates
their state. A zero device count requests a full refresh. Fetch authoritative
topology or state through the ordinary client calls after receiving an event;
the event itself is only a bounded dirty bit.

`LUMINATE_EVENT_RESYNC_REQUIRED` means the subscription lost events. Refetch
every authoritative baseline before trusting later incremental events.

For `LUMINATE_EVENT_SHM_STREAM_ENDED`, the target is a borrowed view and the
generation uses a boolean/output accessor. The accessor can successfully write
generation zero; other event kinds leave caller storage unchanged.

## Targets and effects

Mutation functions take one `LuminateTarget`. Prefer the static inline
`luminate_target_device`, `luminate_target_surface`,
`luminate_target_element`, and `luminate_target_group` constructors. They do
not allocate or alter the last-error state. The library validates nulls,
UTF-8, and target shape before contacting the daemon; the daemon remains the
authority for topology and capability validation.

`LuminateEffect` values are mutable, owned builders. Construct a portable effect with its
`luminate_effect_create_*` function, or construct a hardware effect and fill
the arguments advertised by its hardware-effect descriptor. Pass the completed
effect by borrowed pointer to `luminate_client_set_effect`, then release it
with `luminate_effect_free`. Builder setters validate local structure; the
daemon validates argument ranges and the target's advertised schema.

Hardware-effect parameter descriptors expose variant payloads through
boolean/output accessors. Colour parameters write their minimum and maximum
counts together, while speed, duration, and brightness parameters write their
range or bit width. A wrong variant leaves caller storage unchanged.

Independent-brightness capabilities use the same boolean/output convention
for their bit width, maximum value, and scope. Check the brightness kind first;
an absent or different brightness variant leaves caller storage unchanged.

Persistence capabilities retain their kind discriminant and expose their
requirement, profile-slot count, explicit-commit flag, and readback flag through
boolean/output accessors. A successful call may write `false`; the function's
return value distinguishes that from an absent or mismatched variant.

Facet brightness and appearance-slot completeness are also boolean/output
accessors. This preserves zero brightness and incomplete slot state as present
values instead of conflating them with a different facet kind.

Shared-memory frame shapes expose their pixel count and matrix dimensions
through boolean/output accessors. Pixel count applies to either shape; width
and height succeed only for a matrix shape. The shape-kind discriminant remains
the clearest way to select the applicable payload.

Nested models expose effects as borrowed `const LuminateEffectView *` values.
Their read accessors mirror the owned effect surface with the
`luminate_effect_view_*` prefix. A view must not be freed and remains valid only
while its owning snapshot or list root remains alive.

`luminate_effect_view_clone` turns any borrowed portable or hardware effect
into an independent `LuminateEffect`. Likewise,
`luminate_effect_create_static_from_colour` copies a borrowed generic colour
into an owned static effect. These are the lossless bridge from read accessors
back to mutation inputs:

```c
const LuminateEffectView *view = luminate_scene_binding_appearance(binding);
LuminateEffect *copy = NULL;
if (view != NULL)
    luminate_effect_view_clone(view, &copy);
/* `copy` remains valid after the scene snapshot is freed. */
luminate_effect_free(copy);
```

Appearance-slotted surfaces expose their descriptors through
`luminate_capability_set_appearance_slots`. The nested slot and appearance
capabilities follow the ordinary borrowed-view lifetime rules. Observed and
scene slot values expose a stable slot ID and a borrowed effect view. Apply an
ordered logical mutation with `luminate_client_set_appearance_slots`; each
`LuminateAppearanceSlotInput` borrows its NUL-terminated slot ID and effect for
the duration of the call. Scene state inputs use the same array shape.

Static uses the same generic colour contract as the Rust API:
`luminate_effect_create_static` accepts `LuminateColourInput`, and
`luminate_effect_static_colour` returns its borrowed `LuminateColour`.
`luminate_colour_value` looks up fixed-model or additive components by channel;
`luminate_colour_capability_bits` performs the matching capability lookup.
`luminate_colour_channel_count` and `luminate_colour_channel_at` enumerate
every encoding. Additive colours retain their stored order. Fixed models use
canonical order: hue, saturation, value for HSV; hue, saturation, lightness for
HSL; temperature for CCT; and intensity for monochrome.
The RGB effect accessors apply only to animated and hardware fields whose
contract is explicitly RGB.

`luminate_client_restore_appearance` turns a dark target back on by
reapplying its last-known configured colour or non-off effect.
`luminate_client_set_emission` selects that behaviour with
`LUMINATE_EMISSION_EMITTING`, or uses the ordinary off operation with
`LUMINATE_EMISSION_DARK`. Other `LuminateEmissionState` values are rejected
locally as `LUMINATE_STATUS_INVALID_ARGUMENT` and set the thread-local
last-error diagnostic.

`luminate_facet_value_effect` reads a running effect back out of a state
snapshot as a borrowed `LuminateEffectView`. The view becomes invalid when the
snapshot is freed.

Aggregate targets can report `LUMINATE_APPEARANCE_MIXED` and
`LUMINATE_EFFECTIVE_APPEARANCE_MIXED` when their constituents differ.
`luminate_client_get_collection_state` returns the corresponding collection
snapshot; its configured and effective values remain absent when constituent
knowledge is incomplete. Release it with
`luminate_collection_state_snapshot_free`.

After checking the configured or effective kind, use the corresponding
`*_appearance_colour` or `*_appearance_effect` accessor to obtain its payload.
Wrong variants, mixed values, unknown aggregates, and off or streaming
effective states return null. The payload borrows from the collection-state
snapshot; clone it before freeing that root when it will be used for editing.

Device views expose the daemon-assigned configured provider instance through
`luminate_device_provider_instance`. An absent view means ownership is
unknown; provider-constrained authorization therefore fails closed.

`luminate_all_off_plan` builds a wear-safe plan from a borrowed device.
Its ordinary targets are maximal and non-overlapping. Off-capable targets
whose hardware requires persistent writes are reported separately through the
`skipped_persistent` accessors. Plan targets are borrowed
`LuminateTargetView`s and remain valid until `luminate_all_off_plan_free`.

## Capabilities and frame streaming

Capabilities are scoped to the exact device, surface, element, or group that
advertises them. Identifiers, device categories, effect IDs, and choice IDs
remain open strings so plugins can extend them without a C ABI revision.

A target whose capability snapshot reports a frame-upload shape
(`FrameUpdateMode`, `BufferingMode`, frame rate, atomicity) can be streamed
to: `luminate_client_begin_frame_stream` starts a stream and returns a
generation, `luminate_client_upload_frame_full`/`_partial` upload one frame
(echoing that generation and a monotonically increasing sequence, filling a
`LuminateFrameAck` out-param with the accepted sequence and whether the
daemon silently rate-limited the frame instead of forwarding it), and
`luminate_client_end_frame_stream` ends the stream. Starting a stream is
rejected if the target doesn't advertise the capability, another stream is
already active on it, or an active hardware effect isn't marked concurrent
with streaming — check the returned `LuminateStatus` (`Conflict` for the
last two, `Unsupported` for the first). Streamed frames are hardware-only:
they are never persisted and never appear in `luminate_client_get_state`.

Targets may additionally advertise a client-to-daemon shared-memory fast path.
Inspect `luminate_frame_upload_shm` and the borrowed
`LuminateShmFrameCapability` view for its accepted pixel formats, shape, and
frame-rate ceiling. `luminate_client_begin_shm_frame_stream` negotiates an
opaque publisher, `luminate_client_shm_upload_frame_full` publishes a complete
frame, and `luminate_client_end_shm_frame_stream` ends it. The client handle may
be freed after stream creation: the stream retains its originating connection
until teardown. The ordinary request/response stream remains available
regardless of whether this optional capability is present.

The snapshot reports the daemon's authoritative accepted topology. Consumers
may diagnose suspicious declarations (for example readable facets without an
effective read path), but callback/capability contract enforcement belongs at
the daemon/plugin-host boundary.

## Authentication, scope, and administration

`LuminateClientBuilder` configures exactly one authenticated connection. It
defaults to peer authentication and supports bearer credentials, actor-bound
attestations, configured external providers, an explicit daemon path, and an
allow-only session scope. Builder setters deep-copy credentials and strings.
Connecting never falls back to peer authentication after another method fails.

`luminate_client_get_session_metadata` returns an owned sanitized snapshot of
the canonical authority and subject, verified groups, authentication source,
credential ID, and optional expiry. It never exposes a credential or provider
continuation. Free it with `luminate_session_metadata_free`.

Policy-document builders remain local typed construction tools. The daemon is
the authority for evaluation and persistence. Grouped policy and
authentication administration functions read or replace daemon policy and
create, list, rotate, or revoke daemon tokens and attestations. Replacement is
revision-checked and token secrets are returned only on creation or rotation.
Attestation creation has both identity-only and verified-group variants;
created and listed metadata expose borrowed group accessors. Session scopes
support unconstrained and resource-constrained multi-operation grants.
Authentication rejection reports `LUMINATE_STATUS_AUTHENTICATION_FAILED`, and
null session metadata reports `LUMINATE_AUTHENTICATION_SOURCE_UNKNOWN`.

Built policy documents expose roles and bindings as borrowed item views, and
roles expose their rules the same way. Enumerate with the corresponding
`*_count`/`*_at` pair, then use the `luminate_policy_role_*`,
`luminate_policy_rule_*`, or `luminate_policy_binding_*` accessors. The role
name remains a document-level projection because it is the key of the policy's
role map. All policy item views become invalid when the document is freed.

`luminate_policy_document_builder_from_document` seeds a complete independent
copy for editing. ABI 27 also supplies role removal, parent removal, indexed
rule replacement/removal, and indexed binding replacement/removal. Removing a
role does not silently rewrite parent or binding references; make the related
edits before building, when normal document validation reports any remaining
dangling reference.

```c
LuminatePolicyDocumentBuilder *edit = NULL;
LuminatePolicyDocument *updated = NULL;

luminate_policy_document_builder_from_document(current, &edit);
luminate_policy_document_builder_set_revision(edit, next_revision);
luminate_policy_document_builder_role_replace_rule(edit, "operator", 0,
                                                    &replacement_rule);
luminate_policy_document_build(edit, &updated);

luminate_policy_document_builder_free(edit);
luminate_policy_document_free(updated);
```

The original document, builder, and rebuilt document have independent
lifetimes. An unmodified seeded builder rebuilds the same source and therefore
preserves its observable contents and authorization decisions.

Collections and scenes expose their owner through the shared borrowed
`LuminateOwnerIdentity` view. Its kind distinguishes Unix UIDs, Windows SIDs,
and authenticated principals. Principal owners expose both their authority and
subject, while UID access uses boolean/output form so UID zero remains valid.
The owner view has the lifetime of its collection or scene root.

Token and attestation list roots use `*_count`/`*_at` enumeration. Each
selected `LuminateToken` or `LuminateAttestation` is a borrowed item whose
ordinary accessors no longer repeat the list index. Item and nested group views
remain valid until the owning list is freed.

### Managed configuration

The management API is available through `LuminateClient`.
`luminate_client_get_management` returns an owned
`LuminateManagementSnapshot`; its daemon preferences, plugins, schemas,
reported setting values, and recursively nested visible values are borrowed
views and remain valid only until the snapshot is freed.

Patches are assembled with `LuminateManagementPatchBuilder`. The builder holds
an expected revision and an ordered list of mutations. Mutation functions
deep-copy strings, daemon-preference arrays, and `LuminateSettingValue`
contents, so callers may free or reuse their input values immediately.
`LuminateSettingValue` supports Boolean, signed integer, finite floating-point,
UTF-8 string, array, and string-keyed table values. Array and table insertion
also deep-copy; a value may therefore be inserted more than once without
transferring ownership.

Sensitive plugin settings remain write-only. They may be supplied to
`luminate_management_patch_builder_set_plugin_setting`, but neither the patch
builder nor a committed change set has an accessor that reveals the value.
Snapshots return `LUMINATE_REPORTED_SETTING_REDACTED`, and change records
contain only the plugin, key, and sensitivity flag.

Visible values can be copied recursively with
`luminate_setting_value_view_clone` and then used anywhere an owned
`LuminateSettingValue` is accepted. The copy preserves nested arrays, tables,
keys, strings, and scalars after the snapshot is freed. Unset and redacted
reported values return no visible view; inspect
`luminate_reported_setting_value_kind` and reject those cases rather than
inventing a placeholder value.

Successful patch calls return an owned `LuminateManagementChangeSet`. Both
snapshot and change-set roots have matching free functions. A builder is
reusable after a call because patch operations clone its current contents.

Management reads and writes both evaluate
`LUMINATE_POLICY_OP_MANAGE_PLUGINS`. They deliberately do not use ordinary
Observe permission: management snapshots expose installed-plugin metadata,
administrator locks, revisions, and runtime diagnostics. The decision has no
device resources, matching the daemon's resource-free `ManagePlugins`
authorization request.

### Plugin setup workflow discovery

`luminate_client_plugin_setup_workflows` returns an owned
`LuminatePluginSetupWorkflowList` for one installed plugin. A count of
zero means the plugin has no setup workflow, which is the default. Each
borrowed workflow view exposes its owning plugin, stable plugin-local ID,
label, description, and `LuminatePluginSetupWorkflowKind`. Free the root with
`luminate_plugin_setup_workflow_list_free`; every nested view becomes invalid
at that point.

Workflow discovery requires `LUMINATE_POLICY_OP_MANAGE_PLUGINS`.

`luminate_client_plugin_setup_start` returns an owned
`LuminatePluginSetupSession`. Inspect its state, message, generation, and any
choices through `luminate_plugin_setup_session_choice_count` and
`luminate_plugin_setup_session_choice_at`. Each selected
`LuminatePluginSetupChoice` is borrowed from the session and exposes its id,
label, and optional description through `luminate_plugin_setup_choice_*`
accessors. Continue with
`luminate_client_plugin_setup_choose` or
`luminate_client_plugin_setup_confirm`; use the exact session ID and generation
from the snapshot. `luminate_client_plugin_setup_get` refreshes a snapshot and
`luminate_client_plugin_setup_cancel` cancels an active session. Each operation
returns a new owned snapshot which must be released with
`luminate_plugin_setup_session_free`.

`LUMINATE_PLUGIN_SETUP_APPLYING` means the daemon has accepted the plugin's
result and is committing it. Refresh the session until it reaches a terminal
state.

Completed snapshots expose only a sanitized summary and the committed managed
configuration revision. Plugin-generated settings and credentials are not
represented by the C session model. Read the completed revision through the
boolean/output `luminate_plugin_setup_session_revision`; non-completed states
leave caller storage unchanged.

### Rust and C feature parity

ABI 17 adds daemon-managed transitions. `LuminateTransitionOptions` supplies
the required duration and an optional step interval; zero selects the default
interval of approximately 30 Hz. Its function field selects linear, cubic-in,
cubic-out, or cubic-in-out easing. The colour and hue fields select encoded
interpolation with a shortest, increasing, or decreasing hue path, or
Palette-backed `OKLab` interpolation. Zero-initialization retains the linear,
encoded, shortest-path defaults. The four
`luminate_client_transition_*_to_*` starts return an owned
`LuminateTransitionSnapshot`. Get, abort, and wait return new snapshots, and
`luminate_transition_snapshot_free` releases each root. Snapshot accessors
expose the identifier, timing, terminal kind, and controlled targets.

Preflight failures return `LUMINATE_STATUS_TRANSITION_IMPOSSIBLE`. Runtime
hardware failures are instead represented by a terminal failed snapshot.
`LUMINATE_EVENT_TRANSITIONS_CHANGED` is a dirty-bit notification; enumerate
its identifiers with `luminate_event_transition_count` and
`luminate_event_transition_id_at`, then fetch authoritative snapshots.

Earlier ABIs closed operation-level gaps between the Rust client and the C
surface where the concepts have a stable C representation. ABI 14 makes
`Effect::Static` the sole static-colour mutation, and `LuminateColourInput` is
consumed by the Static effect constructor instead.

The current development surface also exposes the client's daemon and session
metadata, device host-attachment and CCT-emulation metadata, shared-memory
frame capability metadata, and the redacted change set carried by
configuration events. These are additive parity repairs discovered while
exercising the generated header from real C consumers.

ABI 26 completes physical-tag projection across the topology hierarchy.
Device, surface, and element tags each use an ordered `count`/`at` pair and
borrow from the same owned topology or device snapshot as their containing
model. No scope inherits tags from another.

The same ABI preserves the structured parts of Rust permission and handshake
errors. Synchronous and asynchronous C callers can inspect the safe permission
reason, incompatible daemon version and optional reason, and the supported
primary or event protocol version without parsing display text. Both paths are
captured from one internal owned diagnostic snapshot so their variant and
sanitization semantics remain aligned.

The C façade intentionally differs where Rust language mechanics have no
useful direct ABI equivalent. Rust futures become blocking worker-dispatched
calls. Iterators and borrowed Rust references become `count`/`at`
accessors and opaque borrowed views. Daemon operations and consumer-visible
models have C representations where their ownership and lifetime contracts are
well-defined.

## Errors and helpers

Fallible exported functions return `LuminateStatus`, a fixed-width `uint32_t`
discriminant in C11 and a fixed-underlying-type enum where C23 permits it.
Retrieve diagnostics with `luminate_last_error_message` or copy them with
`luminate_copy_last_error_message` before another library call on the same
thread.

Structured error metadata is available without parsing that diagnostic.
`luminate_last_error_retry_after_ms` retrieves provider retry guidance.
`luminate_last_error_applied_target_count` and
`luminate_last_error_applied_target` enumerate the successful subset after
`LUMINATE_STATUS_PARTIAL_MUTATION`. Returned target strings borrow the same
thread-local snapshot as the message and are invalidated by the next library
call on that thread.

Hardware mutations report temporarily absent devices as
`LUMINATE_STATUS_UNAVAILABLE` and provider rate limits as
`LUMINATE_STATUS_RATE_LIMITED`; callers need not infer either condition from
the diagnostic string. A fan-out that changed some targets before failing
returns `LUMINATE_STATUS_PARTIAL_MUTATION`; those successful writes were
committed and must not be blindly repeated.

## Asynchronous operations

Asynchronous operation handles are thread-safe and reference-counted. Retain a
handle with `luminate_async_operation_retain` when another owner needs it, and
release each owned reference with `luminate_async_operation_release`. Null
retain and release calls are harmless. Releasing a reference does not cancel
the operation.

`luminate_async_operation_cancel` races once with normal completion. An
accepted cancellation publishes `LUMINATE_STATUS_CANCELLED` and stops this
client from waiting for or delivering the result; it does not roll back a
request which may already have reached the daemon. Repeated cancellation
distinguishes an earlier cancellation from an operation which completed
normally. `luminate_async_operation_status` leaves its output unchanged while
the operation is pending and reports the immutable terminal status afterwards.

Failed asynchronous operations own their diagnostic snapshot rather than
using thread-local last-error storage. The asynchronous error message, retry
guidance, and applied-target accessors borrow immutable data from the operation
and remain valid until its last reference is released. Pending, successful,
cancelled, and null operations have no asynchronous error diagnostic. These
accessors do not alter the calling thread's last error.

Every blocking operation which performs daemon or event I/O has an asynchronous
sibling. Its name adds `_async`, removes blocking output parameters, and
appends the completion context, optional context destructor, typed callback,
and operation output. Purely local builders, accessors, constructors, free
functions, and shared-memory frame uploads remain synchronous. The source
parity test in `tests/async_operation_coverage.rs` derives both sets from the
generated header and keeps this distinction exhaustive as the API grows. A new
external-I/O operation must ship blocking and asynchronous forms together; an
intentional local-only exception must be added to the test's explicit
allow-list.

Successful submission writes the operation handle and transfers responsibility
for callback delivery and context destruction to libluminate. Submission
failure leaves the operation output unchanged, does not call either consumer
function, and leaves the context with the caller. Completion callbacks run on
the connection domain's separate serialized dispatcher rather than inline or
on its I/O runtime. A callback may therefore make a blocking call through its
live client without stalling response routing. Callbacks from one domain run
one at a time in completion-publication order; callbacks from different client
domains may overlap. Consumer code which shares state between domains must
synchronize it, and a C++ callback must not throw across the C boundary. A
long-running callback delays later callbacks for that domain but does not stop
daemon I/O.

Accepted operations deep-copy or otherwise take a safe owned representation of
their inputs before returning, including strings, arrays, targets, effects,
policies, management patches, scene bindings, setup choices, and secrets. The
caller may mutate or free those inputs after submission. Secret copies remain
out of diagnostics and are dropped after use. The exception is
`luminate_client_end_shm_frame_stream_async`: successful submission consumes
the uniquely owned stream handle, while failed submission leaves it with the
caller.

The callback's operation pointer is borrowed for the duration of the callback;
retain it there if it must survive callback return. The context destructor runs
once, after the callback, even when completion reports an error or cancellation.

A status-only operation can use the callback to publish completion into the
consumer's own synchronization primitive:

```c
static void ping_complete(void *context,
                          const LuminateAsyncOperation *operation,
                          LuminateStatus status)
{
    struct completion *completion = context;
    completion->status = status;
    completion->retained_operation = luminate_async_operation_retain(operation);
    signal_completion(completion);
}

LuminateAsyncOperation *operation = NULL;
LuminateStatus submitted = luminate_client_ping_async(
    client, &completion, destroy_completion, ping_complete, &operation);
```

Here `signal_completion` and `destroy_completion` stand for the application's
own thread-safe notification and cleanup functions. Check `submitted` before
waiting: on submission failure the application still owns `completion`, and
neither consumer function will run. The complete POSIX C11 example in
`crates/libluminate/examples/async.c` includes the pthread mutex and
condition-variable plumbing and is compiled and run by the C API smoke suite.

Owned results transfer through the callback rather than borrowing storage from
the operation. Move the pointer into consumer-owned state and eventually use
its ordinary free function:

```c
static void topology_complete(void *context,
                              const LuminateAsyncOperation *operation,
                              LuminateStatus status,
                              LuminateTopologySnapshot *topology)
{
    (void)operation;
    struct topology_completion *completion = context;
    completion->status = status;
    completion->topology = topology;
    signal_topology_completion(completion);
}

/* After the callback has transferred a successful result: */
luminate_topology_snapshot_free(completion.topology);
```

Successful pointer payloads are owned by the callback and use their ordinary
free functions. Failure and cancellation supply null pointer payloads. A
successful payload remains valid after the client, operation, and callback
context are released; nested borrowed views remain valid only until their
owning result root is freed. Scalar and by-value payloads are meaningful only
when the terminal status is `LUMINATE_STATUS_OK`.

A subscription retains its originating client domain, so it and an accepted
event wait may outlive the public client handle. Cancelling an event wait
before reading begins leaves the subscription usable; cancellation after frame
reading begins poisons that subscription, and its next wait reports
`LUMINATE_STATUS_CONNECTION_POISONED`.

To stop an indefinite asynchronous event wait, cancel its operation and still
wait for the callback and context destructor before destroying consumer state:

```c
LuminateAsyncCancelResult cancelled =
    luminate_async_operation_cancel(next_operation);
/* ACCEPTED means the callback will report LUMINATE_STATUS_CANCELLED. */
```

Cancellation remains distinct from daemon-owned lifecycle operations:
cancelling `luminate_client_transition_wait_async` does not abort the
transition, and cancelling an individual plugin-setup request does not cancel
its setup session. Use the corresponding transition-abort or setup-cancel
operation for those effects. Cancelling a subscribe attempt does not restore
an event ticket already consumed by connection establishment. Cancelling an
accepted shared-memory teardown does not return its consumed stream handle;
local resources close and remote teardown continues on a best-effort cleanup
path.

An accepted operation retains its execution domain. The public client handle
may therefore be freed after submission without suppressing its completion.
Event subscriptions and shared-memory streams likewise retain the client domain
needed for their own operations. This does not make overlapping access to the
same external handle safe: finish the submission call before freeing that
handle.

ABI 24's implementation was verified with exhaustive generated-header symbol
parity, compiled C11/C23/C++17 consumers, cancellation and completion races,
ownership and diagnostic lifetime tests, the full workspace workflow, the
major-change workflow, and combined Rust and C/C++ coverage. Coverage work was
closed after the asynchronous submission and completion families received
meaningful stable tests; percentage-only tests coupled to incidental internal
structure are not an ongoing requirement. Platform-specific smoke tests remain
tracked with their respective platform work rather than by the retired ABI 24
implementation plan.

## Migrating from ABI 24

ABI 25 adds the device physical-tag accessors and changes the library SONAME.
Consumers must recompile against the ABI 25 header and relink against
`libluminate.so.25` (or the corresponding versioned macOS library). Existing
function signatures and discriminant values are unchanged.

## Migrating from ABI 25

ABI 26 adds surface and element physical-tag accessors plus structured
permission-denial and protocol-incompatibility metadata. Existing function
signatures and discriminant values are unchanged. Consumers must recompile
against the ABI 26 header and relink against `libluminate.so.26` (or the
corresponding versioned macOS library).

## Migrating from ABI 26

ABI 27 adds lossless GUI editing support: collection aggregate appearance
payloads, owned effect and setting-value copies, policy and scene builder
seeding, policy builder removal and replacement operations, and synchronous
and asynchronous scene replacement. Consumers must recompile against the ABI
27 header and relink against `libluminate.so.27` (or the corresponding
versioned macOS library).

## Migrating from ABI 27

ABI 28 removes the unenforced role-quota surface. `LuminateQuotasInput`,
`luminate_policy_document_builder_role_set_quotas`, and
`luminate_policy_role_quotas` no longer exist. Policy roles now contain only
parents and authorization rules. Consumers using the removed declarations
must stop setting or reading quotas, then recompile against the ABI 28 header
and relink against `libluminate.so.28` (or the corresponding versioned macOS
library).

Header-only status, string-view, RGB, and target helpers are allocation-free,
side-effect-free, and do not touch last-error storage. The header intentionally
does not provide control-flow, cleanup, `_Generic`, field-access, or array-size
macros.
