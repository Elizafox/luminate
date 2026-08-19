<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
<!-- SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford -->

# Rust client guide

The `libluminate` package exposes the `luminate` Rust crate. Its prelude keeps
the usual client, topology, target, colour, effect, result, and error types in
one import:

```rust,no_run
use luminate::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let client = Client::connect().await?;
    let devices = client.list_devices().await?;

    if let Some(device) = devices.first() {
        client
            .set_rgb(TargetId::Device(device.id.clone()), 40, 120, 255)
            .await?;
    }

    Ok(())
}
```

Topology discovery is preferable to guessing identifiers. A `TargetId` can
name a whole device, a surface, an element, or a device-scoped group. Once the
target is known, the common static controls are direct:

`Device::physical_tags` describes the fixture or peripheral as a whole;
`Surface::physical_tags` describes only that addressable region or mapping;
and `Element::physical_tags` describes one individually addressable physical
object. They are ordered, open string vocabularies intended for presentation.
Tags are not inherited between scopes. Preserve unknown values and fall back
gracefully rather than treating known tags as an exhaustive registry or as
capabilities. See the [physical-tag
reference](physical-tags.md) for the standard vocabulary and plugin-defined
extensions.

```rust,no_run
# use luminate::prelude::*;
# async fn set_colours(client: &Client, target: TargetId) -> Result<()> {
client.set_rgb(target.clone(), 255, 80, 20).await?;
client.set_cct(target.clone(), 4_000).await?;
client
    .set_colour(target, Colour::hsv(120, 100, 75))
    .await?;
# Ok(())
# }
```

## Authentication and session scope

`Client::connect()` and `connect_path()` are peer-authentication conveniences.
Use `Client::builder()` for another authentication method or a voluntary
allow-only scope:

```rust,no_run
# use luminate::prelude::*;
# async fn connect_with_token(credential: Credential, scope: SessionScope) -> Result<()> {
let client = Client::builder()
    .authentication(Authentication::Bearer { credential })
    .scope(scope)
    .connect()
    .await?;

println!(
    "connected as {}:{}",
    client.session().subject.authority,
    client.session().subject.subject
);
# Ok(())
# }
```

The builder also supports actor-bound attestations and configured external
authentication providers. A failed method never falls back to peer
authentication. `client.session()` returns sanitized subject, verified-group,
source, credential-ID, and expiry metadata; it never returns the credential or
provider continuation. Provider integrations use the distinct redacted,
zeroizing `Continuation` type; provider protocol v1 retains the same transparent
byte encoding as `Credential`. The daemon closes a session when its credential is
revoked, rotated, expires, or cannot be revalidated.

Front ends use `create_principal_attestation` to preserve verified groups;
`create_attestation` remains the identity-only convenience. Attestation
administration requires both actor registration and `AdministerFrontend`.
Token administration remains governed by `ManageAuthentication`.

`SessionScope` is a voluntary reduction applied for the connection's lifetime.
It can grant multiple operation/resource combinations, but cannot add access
that daemon policy or actor admission did not already allow.

Administrative clients use `policy_administration()` and
`authentication_administration()` to read or revision-replace daemon policy
and to create, list, rotate, or revoke daemon credentials. These calls are
ordinary authorized daemon operations. Token secrets are returned only by a
successful create or rotate call.

`set_colour`, `set_rgb`, and `set_cct` are conveniences over
`Effect::Static`. They use the same daemon operation, capability validation,
authorization, and error behaviour as `set_effect`; they are not weaker or
separate colour operations.

## Selectors and collections

Selector helpers apply the same static colours to a direct target or fan out
over a collection:

```rust,no_run
# use luminate::prelude::*;
# async fn set_room(client: &Client, room: CollectionId) -> Result<()> {
let outcome = client
    .set_rgb_selector(
        Selector::Collection(room),
        40,
        120,
        255,
        Some(UnsupportedPolicy::Skip),
    )
    .await?;

for target in outcome.denied {
    eprintln!("not applied to {target:?}");
}
# Ok(())
# }
```

`UnsupportedPolicy::Skip` applies the request to compatible members.
`UnsupportedPolicy::Reject` requires every resolved member to support it.
The returned `CollectionOutcome` reports the leaves that were applied or
denied. For a direct-target selector, both lists are empty.

## Scenes

Scenes are persistent sparse snapshots of intended appearance, brightness, and
emission. Explicit authoring uses `create_scene`/`replace_scene`; capture uses
`capture_scene`/`recapture_scene`. Replacements and deletion take the revision
returned by the preceding read or mutation:

```rust,no_run
# use luminate::prelude::*;
# async fn capture(client: &Client, target: TargetId) -> Result<()> {
let scene = client
    .capture_scene(
        "Evening".to_owned(),
        None,
        SceneCaptureMode::Frozen,
        vec![target],
    )
    .await?;
let outcome = client.apply_scene(scene.id.clone()).await?;
client.delete_scene(scene.id, scene.revision).await?;
assert!(outcome.denied.is_empty());
# Ok(())
# }
```

Dynamic capture uses `SceneCaptureMode::DynamicCollectionMembers`. It records
each current leaf's individual state and later applies only captured leaves
that are still members. Applying a scene never changes its revision.

## Transitions

`client.transitions()` starts daemon-owned transitions. The daemon
finishes preflight before the call returns, so `Error::TransitionImpossible`
means no transition hardware write occurred:

```rust,no_run
# use std::time::Duration;
# use luminate::prelude::*;
# async fn fade(client: &Client, destination: SceneId) -> Result<()> {
let options = TransitionOptions::new(Duration::from_secs(2), None)
    .map_err(|error| Error::InvalidArgument(error.to_string()))?
    .with_function(TransitionFunction::EaseInOut)
    .with_colour_interpolation(TransitionColourInterpolation::Oklab);
let started = client
    .transitions()
    .current_to_scene(destination, options)
    .await?;
let finished = client.transitions().wait(started.id).await?;
assert!(finished.is_terminal());
# Ok(())
# }
```

The four starts are `scene_to_scene`, `current_to_scene`,
`scene_to_states`, and `current_to_states`. `abort` retains the last
successfully applied appearance and waits until no later step can write.
Transition status is in-memory daemon state and is lost on daemon restart;
only the resulting lighting state is persisted.

`TransitionFunction` also provides cubic `EaseIn`, `EaseOut`, and
`EaseInOut` curves. Encoded HSV/HSL interpolation can take the shortest,
increasing, or decreasing hue path through
`TransitionColourInterpolation::Encoded`. Opt-in `Oklab` interpolation gives
smoother perceptual RGB gradients. It assumes sRGB-like emitters and is
accepted for effect RGB values and exact 8-bit RGB static targets; unsupported
static colour models fail preflight.

## Plugin setup workflows

`Client::plugin_setup_workflows` returns the typed setup workflows advertised
by one installed plugin. An empty list is the normal result for a plugin which
does not provide setup through Luminate:

```rust,no_run
# use luminate::prelude::*;
# async fn inspect_setup(client: &Client) -> Result<()> {
let workflows = client
    .plugin_setup_workflows("luminate-plugin-philips-hue")
    .await?;
for workflow in workflows {
    println!("{}: {}", workflow.id, workflow.label);
}
# Ok(())
# }
```

Workflow metadata is protected by the plugin-management authorization
operation. Start a workflow with `Client::start_plugin_setup`, then respond to
each generation according to its typed state:

```rust,no_run
# use luminate::prelude::*;
# use luminate::{PluginSetupInteractionResponse, PluginSetupSessionState};
# async fn pair_hue(client: &Client) -> Result<()> {
let mut session = client
    .start_plugin_setup("luminate-plugin-philips-hue", "push-link")
    .await?;
loop {
    let response = match &session.state {
        PluginSetupSessionState::Choice { choices, .. } => {
            let choice = choices
                .first()
                .ok_or_else(|| Error::Unavailable("setup offered no choices".to_owned()))?;
            PluginSetupInteractionResponse::Choice(choice.id.clone())
        }
        PluginSetupSessionState::PhysicalAction { .. } => {
            PluginSetupInteractionResponse::Confirmed
        }
        PluginSetupSessionState::Applying => {
            session = client.plugin_setup_session(session.id).await?;
            continue;
        }
        PluginSetupSessionState::Completed { .. } => break,
        PluginSetupSessionState::Failed { diagnostic } => {
            return Err(Error::Unavailable(diagnostic.clone()));
        }
        PluginSetupSessionState::Cancelled => break,
    };
    session = client
        .respond_plugin_setup(session.id, session.generation, response)
        .await?;
}
# Ok(())
# }
```

`plugin_setup_session` fetches an authoritative snapshot and
`cancel_plugin_setup` cancels an active session. Session identifiers are
unpredictable and actor-bound. Every response must echo the current generation;
stale responses fail rather than answering a later prompt. Completion reports
only a sanitized summary and committed management revision. Generated plugin
settings, including Hue application keys, are never returned to the client.

## Lower-level effects

Use `set_effect` or `set_effect_selector` for animation, off, or advertised
hardware effects, and whenever code already has an `Effect` value:

```rust,no_run
# use luminate::prelude::*;
# async fn breathe(client: &Client, target: TargetId) -> Result<()> {
client
    .set_effect(
        target,
        Effect::Breathe {
            colour: Rgb::new(255, 30, 100),
            period_ms: 1_500,
        },
    )
    .await?;
# Ok(())
# }
```

The modules at the crate root remain the complete model surface. Import from
them when a specialised capability, state, policy, or protocol-facing type is
not part of the prelude.
