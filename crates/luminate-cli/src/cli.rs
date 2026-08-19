// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Command-line arguments and conversion into Luminate domain types.

use std::path::PathBuf;

use anyhow::{Context as _, bail};
use clap::{Args, ColorChoice, Parser, Subcommand, ValueEnum};
use luminate_core::appearance_slot::{AppearanceSlotId, AppearanceSlotValue};
use luminate_core::capability::{ColourChannel, EffectDirection, HardwareEffectId};
use luminate_core::collection::{CollectionCategory, CollectionId, CollectionMember};
use luminate_core::colour::{Colour, ColourChannelValue};
use luminate_core::effect::{Effect, EffectArguments};
use luminate_core::rgb::Rgb;
use luminate_core::target::TargetId;
use luminate_protocol::{Selector, UnsupportedPolicy};

#[derive(Debug, Parser)]
#[command(name = "luminatectl")]
#[command(color = ColorChoice::Always)]
#[command(about = "Control lighting devices through luminated")]
#[command(
    after_long_help = "TARGETING:\n  Use --device alone, add exactly one of --surface or --group, or add\n  --element to --surface. --key KEY is element shorthand and defaults the\n  surface to 'keyboard'. --collection COLLECTION is exclusive with every\n  concrete target selector and addresses every leaf target the collection\n  (transitively) covers.\n\nEFFECTS:\n  --effect names the effect: a portable typed effect (e.g. `breathe`) or an\n  effect ID advertised by `luminatectl list` (e.g. `demo-scene-show`). By default\n  a built-in effect of that name is used if one exists, otherwise it is treated\n  as a hardware effect ID. Pass --kind builtin or --kind hardware to force one\n  interpretation. Hardware argument flags are valid only when the effect\n  resolves to a hardware effect.\n\nEXAMPLES:\n  luminatectl list\n  luminatectl list element --device demo-keyboard\n  luminatectl list collection --json\n  luminatectl inspect --device demo-keyboard --surface keys --key escape\n  luminatectl set-effect --device demo-bulb --effect static --rgb '#ff5010'\n  luminatectl set-effect --device demo-bulb --effect breathe --rgb 'rgb(0, 80, 255)' --period-ms 1500\n  luminatectl set-effect --device demo-bulb --effect demo-scene-show --choice sunset\n  luminatectl set-effect --collection living-room --effect static --rgb orange\n  luminatectl all-off\n\n`all-off` (alias `init`) is best-effort: required persistent targets are reported\nand skipped to avoid unnecessary nonvolatile-memory wear."
)]
pub(crate) struct Cli {
    /// Override the daemon socket path
    #[arg(long)]
    pub(crate) socket_path: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Check whether the daemon's control connection is responsive
    Ping,

    /// List the full topology or compact typed views
    #[command(alias = "devices")]
    List(ListCommand),

    /// Inspect one target in the topology
    Inspect(TargetSelector),

    /// Show observed state for one device or collection
    State {
        /// Device identifier
        #[arg(
            long,
            conflicts_with = "collection",
            required_unless_present = "collection"
        )]
        device: Option<String>,
        /// Collection identifier
        #[arg(long, conflicts_with = "device", required_unless_present = "device")]
        collection: Option<String>,
        /// Read hardware before displaying the snapshot
        #[arg(long, requires = "device")]
        refresh: bool,
    },

    /// Permanently remove retained state for a device absent from topology
    PurgeWithdrawn {
        /// Withdrawn device identifier
        #[arg(long)]
        device: String,
    },

    /// Re-enumerate hardware and reconcile whatever changed
    ///
    /// The daemon normally does this automatically after resume on supported
    /// platforms. Run it manually if a hardware change was missed, or call it
    /// from a platform sleep hook. The command returns once the rescan is
    /// scheduled, before it finishes.
    Rescan,

    /// Show daemon/client version info
    Version,

    /// Set a target brightness
    SetBrightness(SetBrightnessCommand),

    /// Set a target effect
    SetEffect(Box<SetEffectCommand>),

    /// Store named appearance programs on one surface
    SetSlots(SetSlotsCommand),

    /// Clear a target state
    Clear(TargetSelector),

    /// Manage user-created target collections
    Collection(CollectionCommand),

    /// Manage persistent intended-state scenes
    Scene(SceneCommand),

    /// Inspect and configure installed plugins
    Plugin(PluginCommand),

    /// Change managed plugin settings and daemon preferences
    Config(ConfigCommand),

    /// Turn a target off
    Off(TargetSelector),

    /// Turn off every safe transient output, best-effort; required persistent
    /// targets are reported and skipped (alias: `init`)
    #[command(alias = "init")]
    AllOff,
}

#[derive(Debug, Args)]
pub(crate) struct ListCommand {
    /// Compact object kind to list; omit for the full topology
    #[command(subcommand)]
    pub(crate) view: Option<ListView>,

    /// Emit machine-readable JSON
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// Only show one device ID
    #[arg(long, global = true)]
    pub(crate) device: Option<String>,

    /// Only show devices with this category
    #[arg(long, global = true)]
    pub(crate) category: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub(crate) enum ListView {
    /// List compact device summaries
    Device,

    /// List compact surface summaries
    Surface,

    /// List compact element summaries
    Element,

    /// List compact group summaries
    Group,

    /// List compact collection summaries
    Collection,
}

#[derive(Debug, Args)]
pub(crate) struct TargetSelector {
    /// Device identifier
    #[arg(long, required_unless_present = "collection")]
    pub(crate) device: Option<String>,

    /// Apply the mutation to every resolved leaf target in this collection.
    /// Cannot be combined with a concrete target selector.
    #[arg(long, conflicts_with_all = ["device", "surface", "element", "key", "group"])]
    pub(crate) collection: Option<String>,

    /// Surface identifier
    #[arg(long)]
    pub(crate) surface: Option<String>,

    /// Element identifier
    #[arg(long)]
    pub(crate) element: Option<String>,

    /// Keyboard key shorthand. Defaults --surface to "keyboard" if omitted.
    #[arg(long, conflicts_with = "element")]
    pub(crate) key: Option<String>,

    /// Group identifier
    #[arg(long)]
    pub(crate) group: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct CollectionCommand {
    #[command(subcommand)]
    pub(crate) action: CollectionAction,
}

#[derive(Debug, Args)]
pub(crate) struct SceneCommand {
    #[command(subcommand)]
    pub(crate) action: SceneAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum SceneAction {
    /// List observable scenes
    List,

    /// Show one observable scene as JSON
    Show {
        /// Scene identifier
        id: String,
    },

    /// Create a scene from a typed JSON definition
    Create {
        /// JSON file containing name, optional description, and bindings
        definition: PathBuf,
    },

    /// Replace a scene from a typed JSON definition
    Replace {
        /// Scene identifier
        id: String,
        /// Revision on which the replacement is based
        #[arg(long)]
        revision: u64,
        /// JSON file containing name, optional description, and bindings
        definition: PathBuf,
    },

    /// Capture one fixed target or the current leaf targets of a collection
    Capture {
        /// Human-readable scene name
        name: String,
        /// Optional human-readable description
        #[arg(long)]
        description: Option<String>,
        /// Target to capture. Collection targets remain conditional on
        /// collection membership.
        #[command(flatten)]
        target: TargetSelector,
    },

    /// Recapture one fixed target or the current leaf targets of a collection
    Recapture {
        /// Scene identifier
        id: String,
        /// Revision on which recapture is based
        #[arg(long)]
        revision: u64,
        /// Target to capture. Collection targets remain conditional on
        /// collection membership.
        #[command(flatten)]
        target: TargetSelector,
    },

    /// Apply a scene immediately
    Apply {
        /// Scene identifier
        id: String,
    },

    /// Delete a scene
    Delete {
        /// Scene identifier
        id: String,
        /// Revision on which deletion is based
        #[arg(long)]
        revision: u64,
    },
}

#[derive(Debug, Args)]
pub(crate) struct PluginCommand {
    #[command(subcommand)]
    pub(crate) action: PluginAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum PluginAction {
    /// List installed plugins and their current activation state
    List {
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Show one installed plugin and its manageable settings
    Show {
        /// Canonical plugin name
        name: String,

        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },

    /// Enable a plugin through managed configuration
    Enable(PluginActivationCommand),

    /// Disable a plugin through managed configuration
    Disable(PluginActivationCommand),

    /// Return plugin activation to administrator policy
    Reset(PluginActivationCommand),

    /// Discover or run a plugin-provided setup workflow
    Setup {
        /// Canonical plugin name
        name: String,

        /// Plugin-local workflow identifier; omit to list available workflows
        workflow: Option<String>,

        /// Emit JSON and read interaction responses as JSON lines
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Args)]
pub(crate) struct PluginActivationCommand {
    /// Canonical plugin name
    pub(crate) name: String,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigCommand {
    #[command(subcommand)]
    pub(crate) action: ConfigAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigAction {
    /// Set a plugin setting from one JSON value read from standard input
    Set(PluginSettingCommand),

    /// Clear a managed plugin setting override
    Clear(PluginSettingCommand),

    /// Set a managed plugin reconciliation override
    SetReconciliation(PluginReconciliationSetCommand),

    /// Clear a managed plugin reconciliation override
    ClearReconciliation(PluginReconciliationClearCommand),

    /// Change managed daemon preferences
    Daemon(DaemonConfigCommand),
}

#[derive(Debug, Args)]
pub(crate) struct PluginReconciliationSetCommand {
    /// Canonical plugin name
    pub(crate) name: String,

    /// New reconciliation policy
    pub(crate) policy: String,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PluginReconciliationClearCommand {
    /// Canonical plugin name
    pub(crate) name: String,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PluginSettingCommand {
    /// Canonical plugin name
    pub(crate) name: String,

    /// Canonical dotted setting key
    pub(crate) key: String,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct DaemonConfigCommand {
    #[command(subcommand)]
    pub(crate) action: DaemonConfigAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum DaemonConfigAction {
    /// Set one managed daemon preference
    Set(DaemonPreferenceSetCommand),

    /// Clear one managed daemon preference
    Clear(DaemonPreferenceClearCommand),
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum DaemonPreference {
    DefaultUnsupportedPolicy,
    ReconciliationPolicy,
    DeviceReconciliation,
    CctEmulation,
    PreferShm,
    PreferClientShm,
}

#[derive(Debug, Args)]
pub(crate) struct DaemonPreferenceSetCommand {
    /// Managed daemon preference
    #[arg(value_enum)]
    pub(crate) preference: DaemonPreference,

    /// New preference value
    pub(crate) value: String,

    /// Device ID for a device-reconciliation preference
    #[arg(long, required_if_eq("preference", "device-reconciliation"))]
    pub(crate) device: Option<String>,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct DaemonPreferenceClearCommand {
    /// Managed daemon preference
    #[arg(value_enum)]
    pub(crate) preference: DaemonPreference,

    /// Device ID for a device-reconciliation preference
    #[arg(long, required_if_eq("preference", "device-reconciliation"))]
    pub(crate) device: Option<String>,

    /// Managed revision on which this change is based
    #[arg(long)]
    pub(crate) revision: u64,

    /// Emit the redacted committed change set as JSON
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CollectionAction {
    /// Create a new collection owned by the caller
    Create(CollectionCreateCommand),

    /// Delete a collection. Fails if another collection references it or the
    /// caller does not own it.
    Destroy {
        /// Collection identifier
        id: String,
    },

    /// Add a member to a collection's explicit membership
    AddMember(CollectionMemberCommand),

    /// Remove a member from a collection's explicit membership
    RemoveMember(CollectionMemberCommand),

    /// List every collection currently registered
    List,

    /// Show one collection by id
    Show {
        /// Collection identifier
        id: String,
    },
}

#[derive(Debug, Args)]
pub(crate) struct CollectionCreateCommand {
    /// Human-readable name
    #[arg(long)]
    pub(crate) name: String,

    /// Optional human-readable description
    #[arg(long)]
    pub(crate) description: Option<String>,

    /// Display hint for grouping the collection or choosing its icon, such as
    /// `location`, `logical-grouping`, or `zone`. This never affects behaviour.
    #[arg(long)]
    pub(crate) kind: Option<String>,

    /// A whole device to include as a member, repeatable
    #[arg(long = "member-device")]
    pub(crate) member_devices: Vec<String>,

    /// Another collection to nest as a member, repeatable
    #[arg(long = "member-collection")]
    pub(crate) member_collections: Vec<String>,
}

impl CollectionCreateCommand {
    pub(crate) fn into_parts(
        self,
    ) -> (
        String,
        Option<String>,
        Option<CollectionCategory>,
        Vec<CollectionMember>,
    ) {
        let Self {
            name,
            description,
            kind,
            member_devices,
            member_collections,
        } = self;

        let mut members: Vec<CollectionMember> = member_devices
            .into_iter()
            .map(|device| CollectionMember::Target(TargetId::device(device)))
            .collect();
        members.extend(
            member_collections
                .into_iter()
                .map(|id| CollectionMember::Collection(CollectionId::new(id))),
        );

        (
            name,
            description,
            kind.map(CollectionCategory::new),
            members,
        )
    }
}

#[derive(Debug, Args)]
pub(crate) struct CollectionMemberCommand {
    /// Collection identifier
    pub(crate) id: String,

    /// A whole device member (mutually exclusive with --member-collection)
    #[arg(long, conflicts_with = "member_collection")]
    pub(crate) member_device: Option<String>,

    /// A nested collection member (mutually exclusive with --member-device)
    #[arg(
        long,
        conflicts_with = "member_device",
        required_unless_present = "member_device"
    )]
    pub(crate) member_collection: Option<String>,
}

impl CollectionMemberCommand {
    pub(crate) fn into_member(self) -> anyhow::Result<CollectionMember> {
        match (self.member_device, self.member_collection) {
            (Some(device), None) => Ok(CollectionMember::Target(TargetId::device(device))),
            (None, Some(id)) => Ok(CollectionMember::Collection(CollectionId::new(id))),
            // clap's conflicts_with/required_unless_present already rule out
            // every other combination before parsing succeeds.
            _ => bail!("exactly one of --member-device or --member-collection is required"),
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct SetBrightnessCommand {
    #[command(flatten)]
    pub(crate) target: TargetSelector,

    /// Brightness value
    pub(crate) value: u32,

    /// Reject a collection command unless every current member can apply it.
    #[arg(long)]
    pub(crate) reject: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum EffectKindArg {
    Off,
    Static,
    Breathe,
    Breathing,
    Pulse,
    Strobe,
    Scanner,
    Morph,
    Spectrum,
    Rainbow,
}

/// CLI spelling of [`EffectDirection`], for an `--direction` hardware argument.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum DirectionArg {
    Forward,
    Reverse,
    Clockwise,
    CounterClockwise,
    Inward,
    Outward,
    Random,
}

impl DirectionArg {
    const fn into_core(self) -> EffectDirection {
        match self {
            Self::Forward => EffectDirection::Forward,
            Self::Reverse => EffectDirection::Reverse,
            Self::Clockwise => EffectDirection::Clockwise,
            Self::CounterClockwise => EffectDirection::CounterClockwise,
            Self::Inward => EffectDirection::Inward,
            Self::Outward => EffectDirection::Outward,
            Self::Random => EffectDirection::Random,
        }
    }
}

/// Disambiguates an `--effect` name when it could be read either way.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum EffectKindSelector {
    Hardware,
    Builtin,
}

#[derive(Debug, Args)]
pub(crate) struct SetEffectCommand {
    #[command(flatten)]
    pub(crate) target: TargetSelector,

    /// Effect name: a portable typed effect (e.g. `breathe`) or a hardware
    /// effect id advertised by `luminatectl list` (e.g. `demo-scene-show`, see
    /// the `hw-effects` line). A built-in effect of this name is used if one
    /// exists, otherwise it is treated as a hardware effect id; use --kind to
    /// force one interpretation.
    #[arg(long)]
    pub(crate) effect: String,

    /// Force --effect to be read as `builtin` or `hardware` rather than
    /// trying built-in first. Hardware argument flags (--speed, --direction,
    /// --duration-ms, --brightness, --choice) are valid only when the effect
    /// resolves to a hardware effect.
    #[arg(long, value_enum)]
    pub(crate) kind: Option<EffectKindSelector>,

    /// Colour, CSS-style: `#rrggbb`, `#rgb`, `rgb(r, g, b)`, or a colour name
    /// (e.g. `red`, `white`)
    #[arg(long)]
    pub(crate) rgb: Option<String>,

    /// Static HSV components as HUE,SATURATION,VALUE
    #[arg(long, conflicts_with_all = ["rgb", "hsl", "kelvin", "intensity", "additive_channel"])]
    pub(crate) hsv: Option<String>,

    /// Static HSL components as HUE,SATURATION,LIGHTNESS
    #[arg(long, conflicts_with_all = ["rgb", "hsv", "kelvin", "intensity", "additive_channel"])]
    pub(crate) hsl: Option<String>,

    /// Static correlated colour temperature in kelvin
    #[arg(long, conflicts_with_all = ["rgb", "hsv", "hsl", "intensity", "additive_channel"])]
    pub(crate) kelvin: Option<u32>,

    /// Static monochrome intensity
    #[arg(long, conflicts_with_all = ["rgb", "hsv", "hsl", "kelvin", "additive_channel"])]
    pub(crate) intensity: Option<u32>,

    /// Static additive emitter assignment as NAME=VALUE; repeat as needed
    #[arg(long, conflicts_with_all = ["rgb", "hsv", "hsl", "kelvin", "intensity"])]
    pub(crate) additive_channel: Vec<String>,

    /// Additional colour for morph and hardware effects, same syntax as
    /// --rgb; repeat as needed
    #[arg(long = "extra-rgb")]
    pub(crate) extra_rgb: Vec<String>,

    /// Effect period in milliseconds (typed effects only)
    #[arg(long)]
    pub(crate) period_ms: Option<u32>,

    /// Hardware effect speed argument
    #[arg(long)]
    pub(crate) speed: Option<u16>,

    /// Hardware effect direction argument
    #[arg(long, value_enum)]
    pub(crate) direction: Option<DirectionArg>,

    /// Hardware effect duration argument, in milliseconds
    #[arg(long)]
    pub(crate) duration_ms: Option<u32>,

    /// Hardware effect brightness argument
    #[arg(long)]
    pub(crate) brightness: Option<u32>,

    /// Hardware effect choice option id
    #[arg(long)]
    pub(crate) choice: Option<String>,

    /// Reject a collection command unless every current member can apply it.
    #[arg(long)]
    pub(crate) reject: bool,
}

#[derive(Debug, Args)]
pub(crate) struct SetSlotsCommand {
    #[command(flatten)]
    pub(crate) target: TargetSelector,

    /// Slot assignment as `ID=static:COLOUR` or `ID=EFFECT_JSON`; repeat as needed
    #[arg(long = "slot", required = true)]
    pub(crate) slots: Vec<String>,
}

impl SetSlotsCommand {
    pub(crate) fn into_parts(self) -> anyhow::Result<(TargetId, Vec<AppearanceSlotValue>)> {
        let target = self.target.into_target()?;
        if !matches!(target, TargetId::Surface { .. }) {
            bail!("set-slots requires --device and --surface only");
        }
        let values = self
            .slots
            .iter()
            .map(|assignment| parse_slot_assignment(assignment))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok((target, values))
    }
}

fn parse_slot_assignment(assignment: &str) -> anyhow::Result<AppearanceSlotValue> {
    let (id, definition) = assignment
        .split_once('=')
        .with_context(|| format!("slot assignment '{assignment}' must use ID=EFFECT"))?;
    if id.is_empty() {
        bail!("slot assignment '{assignment}' has an empty ID");
    }
    let effect = if definition == "off" {
        Effect::Off
    } else if let Some(colour) = definition.strip_prefix("static:") {
        Effect::Static {
            colour: Colour::rgb(parse_css_colour(colour)?),
        }
    } else {
        serde_json::from_str(definition).with_context(|| {
            format!("slot '{id}' effect must be off, static:COLOUR, or a JSON Effect value")
        })?
    };
    Ok(AppearanceSlotValue {
        slot: AppearanceSlotId::new(id),
        effect,
    })
}

impl TargetSelector {
    pub(crate) fn into_target(self) -> anyhow::Result<TargetId> {
        let Self {
            device,
            collection,
            mut surface,
            mut element,
            key,
            group,
        } = self;
        if collection.is_some() {
            bail!("a collection selector cannot be used where one concrete target is required");
        }
        let device = device.ok_or_else(|| anyhow::anyhow!("--device is required"))?;
        if let Some(key) = key {
            if element.is_some() {
                bail!("--key cannot be combined with --element");
            }
            element = Some(key);
            surface.get_or_insert_with(|| "keyboard".to_owned());
        }

        TargetId::from_parts(device, surface, element, group)
            .map_err(|reason| anyhow::anyhow!(reason))
    }

    pub(crate) fn into_selector(self) -> anyhow::Result<Selector> {
        if let Some(collection) = self.collection {
            return Ok(Selector::Collection(CollectionId::new(collection)));
        }
        self.into_target().map(Selector::Target)
    }
}

impl SetEffectCommand {
    pub(crate) fn into_parts(
        mut self,
    ) -> anyhow::Result<(Selector, Effect, Option<UnsupportedPolicy>)> {
        let builtin_kind = match self.kind {
            Some(EffectKindSelector::Hardware) => None,
            Some(EffectKindSelector::Builtin) => {
                Some(EffectKindArg::from_str(&self.effect, true).map_err(|_| {
                    anyhow::anyhow!("unknown built-in effect kind '{}'", self.effect)
                })?)
            }
            None => EffectKindArg::from_str(&self.effect, true).ok(),
        };

        let effect = if let Some(kind) = builtin_kind {
            self.hardware_only_args_must_be_absent()?;
            if kind != EffectKindArg::Static {
                self.static_only_args_must_be_absent()?;
            }
            build_effect_with_static(
                kind,
                EffectBuildInput {
                    rgb: self.rgb.as_deref(),
                    hsv: self.hsv.as_deref(),
                    hsl: self.hsl.as_deref(),
                    kelvin: self.kelvin,
                    intensity: self.intensity,
                    additive_channels: &self.additive_channel,
                    extra_rgb: &self.extra_rgb,
                    period_ms: self.period_ms,
                },
            )?
        } else {
            require_none("period-ms", self.period_ms.is_some())?;
            self.static_only_args_must_be_absent()?;
            let arguments = EffectArguments {
                colours: collect_colours(self.rgb.as_deref(), &self.extra_rgb)?,
                speed: self.speed,
                direction: self.direction.map(DirectionArg::into_core),
                duration_ms: self.duration_ms,
                brightness: self.brightness,
                choice: self.choice.take(),
            };
            Effect::Hardware {
                id: HardwareEffectId::new(self.effect.clone()),
                arguments,
            }
        };

        let selector = self.target.into_selector()?;
        Ok((
            selector,
            effect,
            self.reject.then_some(UnsupportedPolicy::Reject),
        ))
    }

    // Hardware arguments have no meaning for portable effects. Rejecting them
    // prevents a mistyped invocation from appearing to succeed.
    fn hardware_only_args_must_be_absent(&self) -> anyhow::Result<()> {
        require_none("speed", self.speed.is_some())?;
        require_none("direction", self.direction.is_some())?;
        require_none("duration-ms", self.duration_ms.is_some())?;
        require_none("brightness", self.brightness.is_some())?;
        require_none("choice", self.choice.is_some())?;
        Ok(())
    }

    fn static_only_args_must_be_absent(&self) -> anyhow::Result<()> {
        require_none("hsv", self.hsv.is_some())?;
        require_none("hsl", self.hsl.is_some())?;
        require_none("kelvin", self.kelvin.is_some())?;
        require_none("intensity", self.intensity.is_some())?;
        require_none("additive-channel", !self.additive_channel.is_empty())
    }
}

fn parse_static_colour(
    rgb: Option<Rgb>,
    hsv: Option<&str>,
    hsl: Option<&str>,
    kelvin: Option<u32>,
    intensity: Option<u32>,
    additive_channels: &[String],
) -> anyhow::Result<Colour> {
    if let Some(rgb) = rgb {
        return Ok(Colour::rgb(rgb));
    }
    if let Some(hsv) = hsv {
        let [hue, saturation, value] = parse_u32_triplet("--hsv", hsv)?;
        return Ok(Colour::hsv(hue, saturation, value));
    }
    if let Some(hsl) = hsl {
        let [hue, saturation, lightness] = parse_u32_triplet("--hsl", hsl)?;
        return Ok(Colour::hsl(hue, saturation, lightness));
    }
    if let Some(kelvin) = kelvin {
        return Ok(Colour::cct(kelvin));
    }
    if let Some(intensity) = intensity {
        return Ok(Colour::monochrome(intensity));
    }
    if !additive_channels.is_empty() {
        let channels = additive_channels
            .iter()
            .map(|assignment| {
                let (name, value) = assignment
                    .split_once('=')
                    .context("--additive-channel must use NAME=VALUE")?;
                let channel = match name.to_ascii_lowercase().as_str() {
                    "red" => ColourChannel::Red,
                    "green" => ColourChannel::Green,
                    "blue" => ColourChannel::Blue,
                    "white" => ColourChannel::White,
                    "warm-white" => ColourChannel::WarmWhite,
                    "cool-white" => ColourChannel::CoolWhite,
                    "amber" => ColourChannel::Amber,
                    "ultraviolet" | "uv" => ColourChannel::Ultraviolet,
                    _ => bail!("unknown additive channel {name:?}"),
                };
                Ok(ColourChannelValue::new(channel, value.parse()?))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        return Colour::additive(channels).map_err(anyhow::Error::from);
    }
    bail!(
        "static effects require one of --rgb, --hsv, --hsl, --kelvin, --intensity, or --additive-channel"
    )
}

fn parse_u32_triplet(option: &str, value: &str) -> anyhow::Result<[u32; 3]> {
    let values = value
        .split(',')
        .map(str::trim)
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()?;
    values
        .try_into()
        .map_err(|_| anyhow::anyhow!("{option} requires exactly three comma-separated values"))
}

#[derive(Clone, Copy)]
struct EffectBuildInput<'a> {
    rgb: Option<&'a str>,
    hsv: Option<&'a str>,
    hsl: Option<&'a str>,
    kelvin: Option<u32>,
    intensity: Option<u32>,
    additive_channels: &'a [String],
    extra_rgb: &'a [String],
    period_ms: Option<u32>,
}

fn build_effect_with_static(
    kind: EffectKindArg,
    input: EffectBuildInput<'_>,
) -> anyhow::Result<Effect> {
    let rgb = input.rgb.map(parse_css_colour).transpose()?;

    let extra_colours = input
        .extra_rgb
        .iter()
        .map(|value| parse_css_colour(value))
        .collect::<anyhow::Result<Vec<_>>>()?;

    match kind {
        EffectKindArg::Off => {
            require_none("rgb", rgb.is_some())?;
            require_none("extra-rgb", !extra_colours.is_empty())?;
            require_none("period-ms", input.period_ms.is_some())?;
            Ok(Effect::Off)
        }
        EffectKindArg::Static => Ok(Effect::Static {
            colour: parse_static_colour(
                rgb,
                input.hsv,
                input.hsl,
                input.kelvin,
                input.intensity,
                input.additive_channels,
            )?,
        }),
        EffectKindArg::Breathe | EffectKindArg::Breathing => Ok(Effect::Breathe {
            colour: rgb.context("--rgb is required for breathe effects")?,
            period_ms: input
                .period_ms
                .context("--period-ms is required for breathe effects")?,
        }),
        EffectKindArg::Pulse => Ok(Effect::Pulse {
            colour: rgb.context("--rgb is required for pulse effects")?,
            period_ms: input
                .period_ms
                .context("--period-ms is required for pulse effects")?,
        }),
        EffectKindArg::Strobe => Ok(Effect::Strobe {
            colour: rgb.context("--rgb is required for strobe effects")?,
            period_ms: input
                .period_ms
                .context("--period-ms is required for strobe effects")?,
        }),
        EffectKindArg::Scanner => Ok(Effect::Scanner {
            colour: rgb.context("--rgb is required for scanner effects")?,
            period_ms: input
                .period_ms
                .context("--period-ms is required for scanner effects")?,
        }),
        EffectKindArg::Morph => {
            let base = rgb.context("--rgb is required for morph effects")?;
            if extra_colours.is_empty() {
                bail!("morph effects require at least one --extra-rgb");
            }

            let mut colours = Vec::with_capacity(extra_colours.len() + 1);
            colours.push(base);
            colours.extend(extra_colours);

            Ok(Effect::Morph {
                colours,
                period_ms: input
                    .period_ms
                    .context("--period-ms is required for morph effects")?,
            })
        }
        EffectKindArg::Spectrum => {
            require_none("rgb", rgb.is_some())?;
            require_none("extra-rgb", !extra_colours.is_empty())?;
            Ok(Effect::Spectrum {
                period_ms: input
                    .period_ms
                    .context("--period-ms is required for spectrum effects")?,
            })
        }
        EffectKindArg::Rainbow => {
            require_none("rgb", rgb.is_some())?;
            require_none("extra-rgb", !extra_colours.is_empty())?;
            Ok(Effect::Rainbow {
                period_ms: input
                    .period_ms
                    .context("--period-ms is required for rainbow effects")?,
            })
        }
    }
}

/// Collects the primary and additional colours in command-line order.
fn collect_colours(rgb: Option<&str>, extra_rgb: &[String]) -> anyhow::Result<Vec<Rgb>> {
    let mut colours = Vec::with_capacity(usize::from(rgb.is_some()) + extra_rgb.len());
    if let Some(rgb) = rgb {
        colours.push(parse_css_colour(rgb)?);
    }
    for extra in extra_rgb {
        colours.push(parse_css_colour(extra)?);
    }
    Ok(colours)
}

fn require_none(name: &str, present: bool) -> anyhow::Result<()> {
    if present {
        bail!("--{name} is not valid for this effect");
    }

    Ok(())
}

/// Parses a CSS colour as RGB. Alpha is accepted but ignored because Luminate
/// colours do not carry an alpha channel.
fn parse_css_colour(value: &str) -> anyhow::Result<Rgb> {
    let colour =
        csscolorparser::parse(value).with_context(|| format!("invalid colour '{value}'"))?;
    let [r, g, b, _] = colour.to_rgba8();
    Ok(Rgb::new(r, g, b))
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
