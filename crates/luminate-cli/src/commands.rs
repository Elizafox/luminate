// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! CLI command dispatch and daemon interaction.

#![allow(
    clippy::print_stdout,
    reason = "The CLI intentionally writes command output to stdout."
)]

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, BufRead, Write as _};
use std::process;
use std::time::Duration;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use clap::Parser as _;
use luminate::{
    Authentication, Client, DaemonPreferences, DeviceReconciliationPreference, ErrorKind,
    ManagementChange, ManagementChangeSet, ManagementMutation, ManagementPatch,
    PluginSetupInteractionResponse, PluginSetupSession, PluginSetupSessionState,
    PluginSetupWorkflow, PluginSetupWorkflowKind, SettingValue, WriteOnly,
};
use luminate_core::capability::CctEmulation;
use luminate_core::collection::{Collection, CollectionCategory, CollectionId};
use luminate_core::control::ReconciliationPolicy;
use luminate_core::device::{Device, DeviceCategory, DeviceId};
use luminate_core::effect::Effect;
use luminate_core::element::ElementKind;
use luminate_core::group::GroupKind;
use luminate_core::scene::{SceneBinding, SceneCaptureMode, SceneId};
use luminate_core::surface::SurfaceKind;
use luminate_core::target::TargetId;
use luminate_platform::terminal::escape;
use luminate_protocol::{Selector, UnsupportedPolicy};
use tokio::time::sleep;

use crate::cli::{
    Cli, CollectionAction, Command, ConfigAction, DaemonConfigAction, DaemonPreference,
    DaemonPreferenceClearCommand, DaemonPreferenceSetCommand, ListCommand, ListView, PluginAction,
    SceneAction, TargetSelector,
};
use crate::management_output::{format_plugin, format_plugin_list, plugin_json, plugin_list_json};
use crate::output::{
    format_element_kind, format_group_kind, format_group_member, format_surface_kind, print_device,
    print_element, print_group, print_surface, terminal_json_pretty, terminal_safe,
};
use luminate::all_off_plan;

struct SceneDefinition {
    name: String,
    description: Option<String>,
    bindings: Vec<SceneBinding>,
}

fn read_scene_definition(path: &Path) -> anyhow::Result<SceneDefinition> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read scene definition {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("invalid scene definition {}", path.display()))?;
    let object = value
        .as_object()
        .with_context(|| format!("scene definition {} must be an object", path.display()))?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("scene definition {} needs a string name", path.display()))?
        .to_owned();
    let description = object
        .get("description")
        .map(|value| {
            value.as_str().map(str::to_owned).with_context(|| {
                format!(
                    "scene definition {} description must be a string",
                    path.display()
                )
            })
        })
        .transpose()?;
    let bindings = serde_json::from_value(
        object
            .get("bindings")
            .cloned()
            .with_context(|| format!("scene definition {} needs bindings", path.display()))?,
    )
    .with_context(|| format!("invalid bindings in {}", path.display()))?;
    Ok(SceneDefinition {
        name,
        description,
        bindings,
    })
}

fn capture_selection(target: TargetSelector) -> anyhow::Result<(SceneCaptureMode, Vec<TargetId>)> {
    if let Some(collection) = &target.collection {
        return Ok((
            SceneCaptureMode::DynamicCollectionMembers {
                collection: CollectionId::new(collection),
            },
            Vec::new(),
        ));
    }
    Ok((SceneCaptureMode::Frozen, vec![target.into_target()?]))
}

#[allow(
    clippy::exit,
    clippy::too_many_lines,
    reason = "The command match keeps each small CLI operation visible in one place, and all-off reports partial failure through the process status."
)]
pub(crate) async fn run() -> anyhow::Result<()> {
    let arguments = env::args_os().map(|argument| {
        argument.to_str().map_or_else(
            || argument.clone(),
            |argument| OsString::from(escape(argument).into_owned()),
        )
    });
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) if error.use_stderr() => return Err(error.into()),
        Err(error) => {
            error.print()?;
            return Ok(());
        }
    };

    match cli.command {
        Command::Ping => {
            let client = connect(cli.socket_path.as_ref()).await?;
            client.ping().await?;
        }
        Command::List(command) => {
            validate_list_command(&command)?;
            let client = connect(cli.socket_path.as_ref()).await?;

            if command.view == Some(ListView::Collection) {
                let mut collections = client.list_collections().await?;
                collections.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
                print_collection_summaries(&collections, command.json)?;
            } else {
                let mut devices = client.list_devices().await?;
                sort_devices(&mut devices);
                filter_devices(&mut devices, &command);
                match command.view {
                    None => {
                        if command.json {
                            println!("{}", terminal_json_pretty(&devices)?);
                        } else {
                            for device in devices {
                                print_device(&device);
                            }
                        }
                    }
                    Some(view) => print_topology_summaries(&devices, view, command.json)?,
                }
            }
        }
        Command::Inspect(target) => {
            let target = target.into_target()?;
            let client = connect(cli.socket_path.as_ref()).await?;
            inspect_target(&client, &target).await?;
        }
        Command::State {
            device,
            collection,
            refresh,
        } => {
            let client = connect(cli.socket_path.as_ref()).await?;
            if let Some(device) = device {
                if refresh {
                    client.refresh_state(DeviceId::new(&device)).await?;
                }
                let state = client
                    .get_state(DeviceId::new(&device))
                    .await?
                    .with_context(|| format!("device not found: {device}"))?;
                println!("{}", terminal_json_pretty(&state)?);
            } else if let Some(collection) = collection {
                let state = client
                    .get_collection_state(CollectionId::new(&collection))
                    .await?
                    .with_context(|| format!("collection not found: {collection}"))?;
                println!("{}", terminal_json_pretty(&state)?);
            }
        }
        Command::PurgeWithdrawn { device } => {
            let client = connect(cli.socket_path.as_ref()).await?;
            client.purge_withdrawn_device(DeviceId::new(device)).await?;
            println!("ok");
        }
        Command::Rescan => {
            let client = connect(cli.socket_path.as_ref()).await?;
            client.rescan().await?;
            // "scheduled", not "ok": the daemon acknowledges accepting the
            // rescan, and re-enumerating network hardware can outlast this
            // command by a good margin. Claiming completion here would be a
            // lie the user could act on.
            println!("rescan scheduled");
        }
        Command::Version => {
            println!("luminatectl {}", env!("CARGO_PKG_VERSION"));
            println!("libluminate {}", luminate::version());
            match connect(cli.socket_path.as_ref()).await {
                Ok(client) => {
                    let server_info = client.server_info().await?;
                    println!("luminated {}", terminal_safe(&server_info.daemon_version));
                    println!("protocol abi {}", server_info.protocol_abi_version);
                }
                Err(error) => {
                    println!(
                        "luminated unavailable ({})",
                        terminal_safe(&error.to_string())
                    );
                }
            }
        }
        Command::SetBrightness(command) => {
            let value = command.value;
            let policy = command.reject.then_some(UnsupportedPolicy::Reject);
            let selector = command.target.into_selector()?;
            let target_hint = selector_target_hint(&selector);
            let client = connect(cli.socket_path.as_ref()).await?;
            match client
                .set_brightness_selector(selector, value, policy)
                .await
            {
                Ok(outcome) => print_collection_outcome(&outcome),
                Err(error) => {
                    return Err(
                        user_facing_client_error(&client, error, target_hint.as_ref()).await,
                    );
                }
            }
        }
        Command::SetEffect(command) => {
            let (selector, effect, policy) = (*command).into_parts()?;
            let target_hint = selector_target_hint(&selector);
            let client = connect(cli.socket_path.as_ref()).await?;
            match client.set_effect_selector(selector, effect, policy).await {
                Ok(outcome) => print_collection_outcome(&outcome),
                Err(error) => {
                    return Err(
                        user_facing_client_error(&client, error, target_hint.as_ref()).await,
                    );
                }
            }
        }
        Command::SetSlots(command) => {
            let (target, values) = command.into_parts()?;
            let target_hint = Some(target.clone());
            let client = connect(cli.socket_path.as_ref()).await?;
            if let Err(error) = client.set_appearance_slots(target, values).await {
                return Err(user_facing_client_error(&client, error, target_hint.as_ref()).await);
            }
            println!("ok");
        }
        Command::Clear(target) => {
            let selector = target.into_selector()?;
            let target_hint = selector_target_hint(&selector);
            let client = connect(cli.socket_path.as_ref()).await?;
            match client.clear_target_selector(selector).await {
                Ok(outcome) => print_collection_outcome(&outcome),
                Err(error) => {
                    return Err(
                        user_facing_client_error(&client, error, target_hint.as_ref()).await,
                    );
                }
            }
        }
        Command::Collection(command) => {
            let client = connect(cli.socket_path.as_ref()).await?;
            match command.action {
                CollectionAction::Create(create) => {
                    let (name, description, kind, members) = create.into_parts();
                    let id = client
                        .create_collection(name, description, kind, members)
                        .await?;
                    println!("{}", terminal_safe(id.as_str()));
                }
                CollectionAction::Destroy { id } => {
                    client.destroy_collection(CollectionId::new(id)).await?;
                    println!("ok");
                }
                CollectionAction::AddMember(command) => {
                    let id = CollectionId::new(command.id.clone());
                    let member = command.into_member()?;
                    client.add_collection_member(id, member).await?;
                    println!("ok");
                }
                CollectionAction::RemoveMember(command) => {
                    let id = CollectionId::new(command.id.clone());
                    let member = command.into_member()?;
                    client.remove_collection_member(id, member).await?;
                    println!("ok");
                }
                CollectionAction::List => {
                    let collections = client.list_collections().await?;
                    println!("{}", terminal_json_pretty(&collections)?);
                }
                CollectionAction::Show { id } => {
                    let collection = client
                        .get_collection(CollectionId::new(id.clone()))
                        .await?
                        .with_context(|| format!("collection not found: {id}"))?;
                    println!("{}", terminal_json_pretty(&collection)?);
                }
            }
        }
        Command::Scene(command) => {
            let client = connect(cli.socket_path.as_ref()).await?;
            match command.action {
                SceneAction::List => {
                    let mut scenes = client.list_scenes().await?;
                    scenes.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
                    println!("{}", terminal_json_pretty(&scenes)?);
                }
                SceneAction::Show { id } => {
                    let scene = client
                        .get_scene(SceneId::new(&id))
                        .await?
                        .with_context(|| format!("scene not found: {id}"))?;
                    println!("{}", terminal_json_pretty(&scene)?);
                }
                SceneAction::Create { definition } => {
                    let definition = read_scene_definition(&definition)?;
                    let scene = client
                        .create_scene(definition.name, definition.description, definition.bindings)
                        .await?;
                    println!("{}", terminal_json_pretty(&scene)?);
                }
                SceneAction::Replace {
                    id,
                    revision,
                    definition,
                } => {
                    let definition = read_scene_definition(&definition)?;
                    let scene = client
                        .replace_scene(
                            SceneId::new(id),
                            revision,
                            definition.name,
                            definition.description,
                            definition.bindings,
                        )
                        .await?;
                    println!("{}", terminal_json_pretty(&scene)?);
                }
                SceneAction::Capture {
                    name,
                    description,
                    target,
                } => {
                    let (mode, targets) = capture_selection(target)?;
                    let scene = client
                        .capture_scene(name, description, mode, targets)
                        .await?;
                    println!("{}", terminal_json_pretty(&scene)?);
                }
                SceneAction::Recapture {
                    id,
                    revision,
                    target,
                } => {
                    let (mode, targets) = capture_selection(target)?;
                    let scene = client
                        .recapture_scene(SceneId::new(id), revision, mode, targets)
                        .await?;
                    println!("{}", terminal_json_pretty(&scene)?);
                }
                SceneAction::Apply { id } => {
                    let outcome = client.apply_scene(SceneId::new(id)).await?;
                    print_collection_outcome(&outcome);
                }
                SceneAction::Delete { id, revision } => {
                    client.delete_scene(SceneId::new(id), revision).await?;
                    println!("ok");
                }
            }
        }
        Command::Plugin(command) => {
            let client = connect(cli.socket_path.as_ref()).await?;
            match command.action {
                PluginAction::List { json } => {
                    let snapshot = client.get_management().await?;
                    if json {
                        println!(
                            "{}",
                            terminal_json_pretty(&plugin_list_json(
                                snapshot.revision,
                                &snapshot.plugins
                            ))?
                        );
                    } else {
                        print!(
                            "{}",
                            format_plugin_list(snapshot.revision, &snapshot.plugins)
                        );
                    }
                }
                PluginAction::Show { name, json } => {
                    let snapshot = client.get_management().await?;
                    let plugin = snapshot
                        .plugins
                        .iter()
                        .find(|plugin| plugin.name == name)
                        .with_context(|| format!("plugin not found: {}", terminal_safe(&name)))?;
                    if json {
                        println!(
                            "{}",
                            terminal_json_pretty(&plugin_json(snapshot.revision, plugin))?
                        );
                    } else {
                        print!("{}", format_plugin(plugin));
                    }
                }
                PluginAction::Enable(command) => {
                    patch_plugin_activation(
                        &client,
                        command.name,
                        Some(true),
                        command.revision,
                        command.json,
                    )
                    .await?;
                }
                PluginAction::Disable(command) => {
                    patch_plugin_activation(
                        &client,
                        command.name,
                        Some(false),
                        command.revision,
                        command.json,
                    )
                    .await?;
                }
                PluginAction::Reset(command) => {
                    patch_plugin_activation(
                        &client,
                        command.name,
                        None,
                        command.revision,
                        command.json,
                    )
                    .await?;
                }
                PluginAction::Setup {
                    name,
                    workflow,
                    json,
                } => {
                    if let Some(workflow) = workflow {
                        run_plugin_setup(&client, name, workflow, json).await?;
                    } else {
                        let workflows = client.plugin_setup_workflows(name).await?;
                        print_setup_workflows(&workflows, json)?;
                    }
                }
            }
        }
        Command::Config(command) => {
            let client = connect(cli.socket_path.as_ref()).await?;
            match command.action {
                ConfigAction::Set(command) => {
                    let value = read_setting_value(io::stdin().lock())?;
                    patch_management(
                        &client,
                        command.revision,
                        ManagementMutation::SetPluginSetting {
                            plugin: command.name,
                            key: command.key,
                            value: WriteOnly::new(value),
                        },
                        command.json,
                    )
                    .await?;
                }
                ConfigAction::Clear(command) => {
                    patch_management(
                        &client,
                        command.revision,
                        ManagementMutation::ClearPluginSetting {
                            plugin: command.name,
                            key: command.key,
                        },
                        command.json,
                    )
                    .await?;
                }
                ConfigAction::SetReconciliation(command) => {
                    patch_management(
                        &client,
                        command.revision,
                        ManagementMutation::SetPluginReconciliation {
                            plugin: command.name,
                            reconciliation: Some(parse_reconciliation_policy(&command.policy)?),
                        },
                        command.json,
                    )
                    .await?;
                }
                ConfigAction::ClearReconciliation(command) => {
                    patch_management(
                        &client,
                        command.revision,
                        ManagementMutation::SetPluginReconciliation {
                            plugin: command.name,
                            reconciliation: None,
                        },
                        command.json,
                    )
                    .await?;
                }
                ConfigAction::Daemon(command) => match command.action {
                    DaemonConfigAction::Set(command) => {
                        let snapshot = client.get_management().await?;
                        let preferences = set_daemon_preference(snapshot.desired_daemon, &command)?;
                        patch_management(
                            &client,
                            command.revision,
                            ManagementMutation::SetDaemonPreferences(preferences),
                            command.json,
                        )
                        .await?;
                    }
                    DaemonConfigAction::Clear(command) => {
                        let snapshot = client.get_management().await?;
                        let preferences =
                            clear_daemon_preference(snapshot.desired_daemon, &command)?;
                        patch_management(
                            &client,
                            command.revision,
                            ManagementMutation::SetDaemonPreferences(preferences),
                            command.json,
                        )
                        .await?;
                    }
                },
            }
        }
        Command::Off(target) => {
            let selector = target.into_selector()?;
            let target_hint = selector_target_hint(&selector);
            let client = connect(cli.socket_path.as_ref()).await?;
            match client
                .set_effect_selector(selector, Effect::Off, None)
                .await
            {
                Ok(outcome) => print_collection_outcome(&outcome),
                Err(error) => {
                    return Err(
                        user_facing_client_error(&client, error, target_hint.as_ref()).await,
                    );
                }
            }
        }
        Command::AllOff => {
            let client = connect(cli.socket_path.as_ref()).await?;
            let mut devices = client.list_devices().await?;
            sort_devices(&mut devices);

            let mut failures = 0_usize;
            for device in &devices {
                let plan = all_off_plan(device);
                let mut errors = Vec::new();
                for target in plan.targets() {
                    if let Err(error) = client.set_off(target.clone()).await {
                        errors.push(format!("{target:?}: {error}"));
                    }
                }

                let device_id = terminal_safe(&device.id.to_string());
                if plan.targets().is_empty() && plan.skipped_persistent().is_empty() {
                    failures += 1;
                    println!("{device_id}: error: no safe off-capable target");
                } else if errors.is_empty() {
                    if plan.skipped_persistent().is_empty() {
                        println!("{device_id}: ok ({} target(s))", plan.targets().len());
                    } else {
                        println!(
                            "{device_id}: ok ({} target(s); skipped {} required persistent target(s))",
                            plan.targets().len(),
                            plan.skipped_persistent().len()
                        );
                    }
                } else {
                    failures += 1;
                    println!("{device_id}: error: {}", terminal_safe(&errors.join("; ")));
                }
            }

            println!(
                "{}/{} devices processed without errors",
                devices.len() - failures,
                devices.len()
            );

            if failures > 0 {
                process::exit(1);
            }
        }
    }

    Ok(())
}

fn print_setup_workflows(workflows: &[PluginSetupWorkflow], json: bool) -> anyhow::Result<()> {
    if json {
        let workflows = workflows
            .iter()
            .map(setup_workflow_json)
            .collect::<Vec<_>>();
        println!("{}", terminal_json_pretty(&workflows)?);
        return Ok(());
    }

    if workflows.is_empty() {
        println!("No setup workflows available.");
        return Ok(());
    }

    for workflow in workflows {
        println!(
            "{}\t{}\n  {}",
            terminal_safe(&workflow.id),
            terminal_safe(&workflow.label),
            terminal_safe(&workflow.description)
        );
    }
    Ok(())
}

fn setup_workflow_json(workflow: &PluginSetupWorkflow) -> serde_json::Value {
    serde_json::json!({
        "plugin": workflow.plugin,
        "id": workflow.id,
        "label": workflow.label,
        "description": workflow.description,
        "kind": setup_workflow_kind_name(workflow.kind),
    })
}

const fn setup_workflow_kind_name(kind: PluginSetupWorkflowKind) -> &'static str {
    match kind {
        PluginSetupWorkflowKind::Provision => "provision",
        PluginSetupWorkflowKind::Repair => "repair",
        PluginSetupWorkflowKind::Discover => "discover",
        PluginSetupWorkflowKind::Import => "import",
        PluginSetupWorkflowKind::FactoryProvision => "factory_provision",
        _ => "unknown",
    }
}

async fn run_plugin_setup(
    client: &Client,
    plugin: String,
    workflow: String,
    json: bool,
) -> anyhow::Result<()> {
    let mut session = client.start_plugin_setup(plugin, workflow).await?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut last_output = None;

    loop {
        if last_output.as_ref() != Some(&session) {
            if json {
                println!("{}", serde_json::to_string(&setup_session_json(&session))?);
            } else {
                print_setup_session(&session)?;
            }
            last_output = Some(session.clone());
        }

        match &session.state {
            PluginSetupSessionState::Choice { choices, .. } => {
                let response = if json {
                    read_json_setup_response(&mut input, &session.state)?
                } else {
                    read_interactive_choice(&mut input, choices)?
                };
                session = client
                    .respond_plugin_setup(session.id.clone(), session.generation, response)
                    .await?;
            }
            PluginSetupSessionState::PhysicalAction { .. } => {
                let response = if json {
                    read_json_setup_response(&mut input, &session.state)?
                } else {
                    wait_for_interactive_confirmation(&mut input)?
                };
                session = client
                    .respond_plugin_setup(session.id.clone(), session.generation, response)
                    .await?;
            }
            PluginSetupSessionState::Applying => {
                sleep(Duration::from_millis(200)).await;
                session = client.plugin_setup_session(session.id.clone()).await?;
            }
            PluginSetupSessionState::Completed { .. } => return Ok(()),
            PluginSetupSessionState::Failed { diagnostic } => {
                bail!("plugin setup failed: {}", terminal_safe(diagnostic));
            }
            PluginSetupSessionState::Cancelled => bail!("plugin setup was cancelled"),
        }
    }
}

fn setup_session_json(session: &PluginSetupSession) -> serde_json::Value {
    let state = match &session.state {
        PluginSetupSessionState::Choice { prompt, choices } => serde_json::json!({
            "state": "choice",
            "prompt": prompt,
            "choices": choices,
        }),
        PluginSetupSessionState::PhysicalAction { instruction } => serde_json::json!({
            "state": "physical_action",
            "instruction": instruction,
        }),
        PluginSetupSessionState::Applying => serde_json::json!({ "state": "applying" }),
        PluginSetupSessionState::Completed { summary, revision } => serde_json::json!({
            "state": "completed",
            "summary": summary,
            "revision": revision,
        }),
        PluginSetupSessionState::Failed { diagnostic } => serde_json::json!({
            "state": "failed",
            "diagnostic": diagnostic,
        }),
        PluginSetupSessionState::Cancelled => serde_json::json!({ "state": "cancelled" }),
    };
    serde_json::json!({
        "session": session.id.as_str(),
        "plugin": session.plugin,
        "workflow": session.workflow,
        "generation": session.generation,
        "status": state,
    })
}

fn print_setup_session(session: &PluginSetupSession) -> anyhow::Result<()> {
    match &session.state {
        PluginSetupSessionState::Choice { prompt, choices } => {
            println!("{}", terminal_safe(prompt));
            for (index, choice) in choices.iter().enumerate() {
                print!("  {}. {}", index + 1, terminal_safe(&choice.label));
                if let Some(description) = &choice.description {
                    print!(" — {}", terminal_safe(description));
                }
                println!();
            }
            print!("Choice: ");
            io::stdout().flush()?;
        }
        PluginSetupSessionState::PhysicalAction { instruction } => {
            println!("{}", terminal_safe(instruction));
            print!("Press Enter when complete: ");
            io::stdout().flush()?;
        }
        PluginSetupSessionState::Applying => println!("Applying configuration…"),
        PluginSetupSessionState::Completed { summary, revision } => println!(
            "{} (configuration revision {revision})",
            terminal_safe(summary)
        ),
        PluginSetupSessionState::Failed { diagnostic } => {
            println!("Setup failed: {}", terminal_safe(diagnostic));
        }
        PluginSetupSessionState::Cancelled => println!("Setup cancelled."),
    }
    Ok(())
}

fn read_interactive_choice(
    input: &mut impl BufRead,
    choices: &[luminate::PluginSetupChoice],
) -> anyhow::Result<PluginSetupInteractionResponse> {
    let value = read_setup_line(input)?;
    if let Ok(index) = value.parse::<usize>() {
        let choice = index
            .checked_sub(1)
            .and_then(|index| choices.get(index))
            .with_context(|| format!("choice number out of range: {index}"))?;
        return Ok(PluginSetupInteractionResponse::Choice(choice.id.clone()));
    }
    let choice = choices
        .iter()
        .find(|choice| choice.id == value)
        .with_context(|| format!("unknown setup choice: {}", terminal_safe(&value)))?;
    Ok(PluginSetupInteractionResponse::Choice(choice.id.clone()))
}

fn wait_for_interactive_confirmation(
    input: &mut impl BufRead,
) -> anyhow::Result<PluginSetupInteractionResponse> {
    read_setup_line(input)?;
    Ok(PluginSetupInteractionResponse::Confirmed)
}

fn read_json_setup_response(
    input: &mut impl BufRead,
    state: &PluginSetupSessionState,
) -> anyhow::Result<PluginSetupInteractionResponse> {
    let line = read_setup_line(input)?;
    let value: serde_json::Value =
        serde_json::from_str(&line).context("invalid plugin setup response JSON")?;
    let object = value
        .as_object()
        .context("plugin setup response must be a JSON object")?;
    match object.get("response").and_then(serde_json::Value::as_str) {
        Some("choice") if matches!(state, PluginSetupSessionState::Choice { .. }) => {
            let choice = object
                .get("choice")
                .and_then(serde_json::Value::as_str)
                .context("choice response needs a string `choice` field")?;
            Ok(PluginSetupInteractionResponse::Choice(choice.to_owned()))
        }
        Some("confirmed") if matches!(state, PluginSetupSessionState::PhysicalAction { .. }) => {
            Ok(PluginSetupInteractionResponse::Confirmed)
        }
        Some(response) => bail!("setup response `{response}` does not match the current state"),
        None => bail!("plugin setup response needs a string `response` field"),
    }
}

fn read_setup_line(input: &mut impl BufRead) -> anyhow::Result<String> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        bail!("plugin setup input ended before the interaction was answered");
    }
    Ok(line.trim().to_owned())
}

async fn patch_plugin_activation(
    client: &Client,
    plugin: String,
    enabled: Option<bool>,
    revision: u64,
    json: bool,
) -> anyhow::Result<()> {
    patch_management(
        client,
        revision,
        ManagementMutation::SetPluginEnabled { plugin, enabled },
        json,
    )
    .await
}

async fn patch_management(
    client: &Client,
    expected_revision: u64,
    mutation: ManagementMutation,
    json: bool,
) -> anyhow::Result<()> {
    let changes = client
        .patch_management(ManagementPatch {
            expected_revision,
            mutations: vec![mutation],
        })
        .await?;
    print_management_changes(&changes, json)?;
    Ok(())
}

fn print_management_changes(changes: &ManagementChangeSet, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", terminal_json_pretty(changes)?);
        return Ok(());
    }

    println!("revision: {}", changes.revision);
    for change in &changes.changes {
        match change {
            ManagementChange::DaemonPreferencesChanged { keys } => {
                println!(
                    "daemon preferences changed: {}",
                    keys.iter()
                        .map(|key| terminal_safe(key))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            ManagementChange::PluginActivationChanged { plugin } => {
                println!("plugin activation changed: {}", terminal_safe(plugin));
            }
            ManagementChange::PluginReconciliationChanged { plugin } => {
                println!("plugin reconciliation changed: {}", terminal_safe(plugin));
            }
            ManagementChange::PluginSettingChanged {
                plugin,
                key,
                sensitive,
            } => {
                println!(
                    "plugin setting changed: {} {}{}",
                    terminal_safe(plugin),
                    terminal_safe(key),
                    if *sensitive { " [REDACTED]" } else { "" }
                );
            }
        }
    }

    Ok(())
}

fn read_setting_value(mut input: impl io::Read) -> anyhow::Result<SettingValue> {
    let mut source = String::new();
    input
        .read_to_string(&mut source)
        .context("failed to read setting value from standard input")?;
    let value: serde_json::Value = serde_json::from_str(&source)
        .context("standard input must contain exactly one JSON value")?;
    setting_value_from_json(value)
}

fn setting_value_from_json(value: serde_json::Value) -> anyhow::Result<SettingValue> {
    match value {
        serde_json::Value::Bool(value) => Ok(SettingValue::Boolean(value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(SettingValue::Integer(value))
            } else {
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .map(SettingValue::Number)
                    .context("setting number must be a finite i64 or floating-point value")
            }
        }
        serde_json::Value::String(value) => Ok(SettingValue::String(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(setting_value_from_json)
            .collect::<anyhow::Result<Vec<_>>>()
            .map(SettingValue::Array),
        serde_json::Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| Ok((key, setting_value_from_json(value)?)))
            .collect::<anyhow::Result<_>>()
            .map(SettingValue::Table),
        serde_json::Value::Null => bail!("null is not a supported plugin setting value"),
    }
}

fn set_daemon_preference(
    mut preferences: DaemonPreferences,
    command: &DaemonPreferenceSetCommand,
) -> anyhow::Result<DaemonPreferences> {
    reject_irrelevant_scope(command.preference, command.device.as_deref())?;

    match command.preference {
        DaemonPreference::DefaultUnsupportedPolicy => {
            preferences.default_unsupported_policy =
                Some(parse_unsupported_policy(&command.value)?);
        }
        DaemonPreference::ReconciliationPolicy => {
            preferences.reconciliation_policy = Some(parse_reconciliation_policy(&command.value)?);
        }
        DaemonPreference::DeviceReconciliation => {
            let device = command
                .device
                .as_deref()
                .context("--device is required for device-reconciliation")?;
            let policy = parse_reconciliation_policy(&command.value)?;
            if let Some(existing) = preferences
                .device_reconciliation
                .iter_mut()
                .find(|entry| entry.device.as_str() == device)
            {
                existing.policy = policy;
            } else {
                preferences
                    .device_reconciliation
                    .push(DeviceReconciliationPreference {
                        device: DeviceId::new(device),
                        policy,
                    });
            }
        }
        DaemonPreference::CctEmulation => {
            preferences.cct_emulation = Some(parse_cct_emulation(&command.value)?);
        }
        DaemonPreference::PreferShm => {
            preferences.prefer_shm = Some(parse_boolean(&command.value)?);
        }
        DaemonPreference::PreferClientShm => {
            preferences.prefer_client_shm = Some(parse_boolean(&command.value)?);
        }
    }

    Ok(preferences)
}

fn clear_daemon_preference(
    mut preferences: DaemonPreferences,
    command: &DaemonPreferenceClearCommand,
) -> anyhow::Result<DaemonPreferences> {
    reject_irrelevant_scope(command.preference, command.device.as_deref())?;

    match command.preference {
        DaemonPreference::DefaultUnsupportedPolicy => {
            preferences.default_unsupported_policy = None;
        }
        DaemonPreference::ReconciliationPolicy => {
            preferences.reconciliation_policy = None;
        }
        DaemonPreference::DeviceReconciliation => {
            let device = command
                .device
                .as_deref()
                .context("--device is required for device-reconciliation")?;
            preferences
                .device_reconciliation
                .retain(|entry| entry.device.as_str() != device);
        }
        DaemonPreference::CctEmulation => preferences.cct_emulation = None,
        DaemonPreference::PreferShm => preferences.prefer_shm = None,
        DaemonPreference::PreferClientShm => preferences.prefer_client_shm = None,
    }

    Ok(preferences)
}

fn reject_irrelevant_scope(
    preference: DaemonPreference,
    device: Option<&str>,
) -> anyhow::Result<()> {
    if device.is_some() && !matches!(preference, DaemonPreference::DeviceReconciliation) {
        bail!("--device is only valid for device-reconciliation");
    }
    Ok(())
}

fn parse_unsupported_policy(value: &str) -> anyhow::Result<UnsupportedPolicy> {
    match value {
        "skip" => Ok(UnsupportedPolicy::Skip),
        "reject" => Ok(UnsupportedPolicy::Reject),
        _ => bail!("unsupported-target policy must be `skip` or `reject`"),
    }
}

fn parse_reconciliation_policy(value: &str) -> anyhow::Result<ReconciliationPolicy> {
    match value {
        "restore" => Ok(ReconciliationPolicy::Restore),
        "adopt" => Ok(ReconciliationPolicy::Adopt),
        "leave" => Ok(ReconciliationPolicy::Leave),
        _ => bail!("reconciliation policy must be `restore`, `adopt`, or `leave`"),
    }
}

fn parse_cct_emulation(value: &str) -> anyhow::Result<CctEmulation> {
    match value {
        "auto" => Ok(CctEmulation::Auto),
        "disabled" => Ok(CctEmulation::Disabled),
        _ => bail!("CCT emulation must be `auto` or `disabled`"),
    }
}

fn parse_boolean(value: &str) -> anyhow::Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => bail!("shared-memory preference must be `true` or `false`"),
    }
}

async fn connect(socket_path: Option<&PathBuf>) -> anyhow::Result<Client> {
    let mut builder = Client::builder();
    if let Some(path) = socket_path
        .cloned()
        .or_else(|| env::var_os("LUMINATED_SOCKET_PATH").map(PathBuf::from))
    {
        builder = builder.path(path);
    }
    if let Some(path) = env::var_os("LUMINATE_BEARER_CREDENTIAL_FILE") {
        let credential = fs::read(&path).with_context(|| {
            format!(
                "failed to read bearer credential {}",
                PathBuf::from(path).display()
            )
        })?;
        builder = builder.authentication(Authentication::Bearer {
            credential: luminate::Credential::new(credential)?,
        });
    }
    Ok(builder.connect().await?)
}

#[derive(Debug, PartialEq)]
struct DeviceSummary {
    id: String,
    name: String,
    category: Option<DeviceCategory>,
    host_attached: bool,
}

#[derive(Debug, PartialEq)]
struct SurfaceSummary {
    device: String,
    id: String,
    name: String,
    kind: SurfaceKind,
}

#[derive(Debug, PartialEq)]
struct ElementSummary {
    device: String,
    surface: String,
    id: String,
    name: Option<String>,
    kind: ElementKind,
}

#[derive(Debug, PartialEq)]
struct GroupSummary {
    device: String,
    id: String,
    name: String,
    kind: GroupKind,
}

#[derive(Debug, PartialEq)]
struct CollectionSummary {
    id: String,
    name: String,
    kind: Option<CollectionCategory>,
}

fn validate_list_command(command: &ListCommand) -> anyhow::Result<()> {
    if command.view == Some(ListView::Collection)
        && (command.device.is_some() || command.category.is_some())
    {
        bail!("--device and --category cannot be used with `list collection`");
    }

    Ok(())
}

fn device_summaries(devices: &[Device]) -> Vec<DeviceSummary> {
    devices
        .iter()
        .map(|device| DeviceSummary {
            id: device.id.as_str().to_owned(),
            name: device.name.clone(),
            category: device.category.clone(),
            host_attached: device.host_attached,
        })
        .collect()
}

fn surface_summaries(devices: &[Device]) -> Vec<SurfaceSummary> {
    devices
        .iter()
        .flat_map(|device| {
            device.surfaces.iter().map(|surface| SurfaceSummary {
                device: device.id.as_str().to_owned(),
                id: surface.id.as_str().to_owned(),
                name: surface.name.clone(),
                kind: surface.kind.clone(),
            })
        })
        .collect()
}

fn element_summaries(devices: &[Device]) -> Vec<ElementSummary> {
    devices
        .iter()
        .flat_map(|device| {
            device.surfaces.iter().flat_map(|surface| {
                surface.elements.iter().map(|element| ElementSummary {
                    device: device.id.as_str().to_owned(),
                    surface: surface.id.as_str().to_owned(),
                    id: element.id.as_str().to_owned(),
                    name: element.name.clone(),
                    kind: element.kind.clone(),
                })
            })
        })
        .collect()
}

fn group_summaries(devices: &[Device]) -> Vec<GroupSummary> {
    devices
        .iter()
        .flat_map(|device| {
            device.groups.iter().map(|group| GroupSummary {
                device: device.id.as_str().to_owned(),
                id: group.id.as_str().to_owned(),
                name: group.name.clone(),
                kind: group.kind,
            })
        })
        .collect()
}

fn collection_summaries(collections: &[Collection]) -> Vec<CollectionSummary> {
    collections
        .iter()
        .map(|collection| CollectionSummary {
            id: collection.id.as_str().to_owned(),
            name: collection.name.clone(),
            kind: collection.kind.clone(),
        })
        .collect()
}

fn print_topology_summaries(devices: &[Device], view: ListView, json: bool) -> anyhow::Result<()> {
    match view {
        ListView::Device => print_summaries(&device_summaries(devices), json, |summary| {
            format!(
                "{} ({}) [{}] host-attached={}",
                terminal_safe(&summary.id),
                terminal_safe(&summary.name),
                summary.category.as_ref().map_or_else(
                    || "uncategorized".to_owned(),
                    |category| { terminal_safe(category.as_str()) }
                ),
                summary.host_attached
            )
        }),
        ListView::Surface => print_summaries(&surface_summaries(devices), json, |summary| {
            format!(
                "{}/{} ({}) [{}]",
                terminal_safe(&summary.device),
                terminal_safe(&summary.id),
                terminal_safe(&summary.name),
                format_surface_kind(&summary.kind)
            )
        }),
        ListView::Element => print_summaries(&element_summaries(devices), json, |summary| {
            format!(
                "{}/{}/{} ({}) [{}]",
                terminal_safe(&summary.device),
                terminal_safe(&summary.surface),
                terminal_safe(&summary.id),
                summary
                    .name
                    .as_deref()
                    .map_or_else(|| "unnamed".to_owned(), terminal_safe),
                format_element_kind(&summary.kind)
            )
        }),
        ListView::Group => print_summaries(&group_summaries(devices), json, |summary| {
            format!(
                "{}/{} ({}) [{}]",
                terminal_safe(&summary.device),
                terminal_safe(&summary.id),
                terminal_safe(&summary.name),
                format_group_kind(summary.kind)
            )
        }),
        ListView::Collection => bail!("collection summaries require collection data"),
    }
}

fn print_collection_summaries(collections: &[Collection], json: bool) -> anyhow::Result<()> {
    print_summaries(&collection_summaries(collections), json, |summary| {
        format!(
            "{} ({}) [{}]",
            terminal_safe(&summary.id),
            terminal_safe(&summary.name),
            summary.kind.as_ref().map_or_else(
                || "unclassified".to_owned(),
                |kind| { terminal_safe(kind.as_str()) }
            )
        )
    })
}

trait JsonSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value>;
}

impl JsonSummary for DeviceSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "id": self.id,
            "name": self.name,
            "category": self.category,
            "host_attached": self.host_attached,
        }))
    }
}

impl JsonSummary for SurfaceSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "device": self.device,
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
        }))
    }
}

impl JsonSummary for ElementSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "device": self.device,
            "surface": self.surface,
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
        }))
    }
}

impl JsonSummary for GroupSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "device": self.device,
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
        }))
    }
}

impl JsonSummary for CollectionSummary {
    fn to_json(&self) -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "id": self.id,
            "name": self.name,
            "kind": self.kind,
        }))
    }
}

fn print_summaries<T: JsonSummary>(
    summaries: &[T],
    json: bool,
    format_human: impl Fn(&T) -> String,
) -> anyhow::Result<()> {
    if json {
        let values = summaries
            .iter()
            .map(JsonSummary::to_json)
            .collect::<anyhow::Result<Vec<_>>>()?;
        println!("{}", terminal_json_pretty(&values)?);
    } else {
        for summary in summaries {
            println!("{}", format_human(summary));
        }
    }

    Ok(())
}

fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

    for device in devices {
        device
            .surfaces
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

        device
            .groups
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        for group in &mut device.groups {
            group.members.sort_by_key(format_group_member);
        }
    }
}

fn filter_devices(devices: &mut Vec<Device>, command: &ListCommand) {
    if let Some(device_id) = &command.device {
        devices.retain(|device| device.id.as_str() == device_id);
    }
    if let Some(category) = &command.category {
        devices.retain(|device| {
            device
                .category
                .as_ref()
                .is_some_and(|device_category| device_category.as_str() == category)
        });
    }
}

async fn inspect_target(client: &Client, target: &TargetId) -> anyhow::Result<()> {
    let device_id = target.device_id().as_str();
    let Some(device) = client.get_device(DeviceId::new(device_id)).await? else {
        bail!("unknown device: {device_id}");
    };

    match target {
        TargetId::Device(_) => {
            print_device(&device);
            Ok(())
        }
        TargetId::Surface { surface, .. } => {
            let surface = device
                .surfaces
                .iter()
                .find(|candidate| candidate.id == *surface)
                .with_context(|| format!("unknown surface: {}", surface.as_str()))?;
            print_surface(surface);
            Ok(())
        }
        TargetId::Element {
            surface, element, ..
        } => {
            let surface = device
                .surfaces
                .iter()
                .find(|candidate| candidate.id == *surface)
                .with_context(|| format!("unknown surface: {}", surface.as_str()))?;
            let element = surface
                .elements
                .iter()
                .find(|candidate| candidate.id == *element)
                .with_context(|| format!("unknown element: {}", element.as_str()))?;
            print_element(element);
            Ok(())
        }
        TargetId::Group { group, .. } => {
            let group = device
                .groups
                .iter()
                .find(|candidate| candidate.id == *group)
                .with_context(|| format!("unknown group: {}", group.as_str()))?;
            print_group(group);
            Ok(())
        }
    }
}

/// Prints `ok`. For collection writes, also warns about leaf targets denied by
/// authorization. Concrete-target outcomes never contain denied targets, so a
/// warning always identifies a partially applied, best-effort collection write.
fn print_collection_outcome(outcome: &luminate::CollectionOutcome) {
    println!("ok");
    if !outcome.denied.is_empty() {
        println!(
            "warning: not authorized for {} of the collection's target(s):",
            outcome.denied.len()
        );
        for target in &outcome.denied {
            println!("  {}", terminal_safe(&format!("{target:?}")));
        }
    }
}

/// Returns a concrete target for diagnostics; a collection selector has no
/// single target.
fn selector_target_hint(selector: &Selector) -> Option<TargetId> {
    match selector {
        Selector::Target(target) => Some(target.clone()),
        Selector::Collection(_) | Selector::Targets(_) => None,
    }
}

/// Preserves structured error details and adds topology guidance where useful.
async fn user_facing_client_error(
    client: &Client,
    error: luminate::Error,
    target: Option<&TargetId>,
) -> anyhow::Error {
    let mut message = terminal_safe(&error.to_string());
    if let Some(retry_after_ms) = error.retry_after_ms() {
        let _ = write!(message, "; retry after {retry_after_ms} ms");
    }
    if !error.applied_targets().is_empty() {
        let applied = error
            .applied_targets()
            .iter()
            .map(|target| terminal_safe(&format!("{target:?}")))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(message, "; already applied: {applied}");
    }
    if let Some(hint) = target_hint(client, &error, target).await {
        let _ = write!(message, "; {hint}");
    }
    drop(error);
    anyhow::anyhow!(message)
}

/// Suggests `list` for an unknown device and `inspect` for an invalid target.
///
/// Failure to fetch topology suppresses the hint rather than replacing the
/// original daemon error.
async fn target_hint(
    client: &Client,
    error: &luminate::Error,
    target: Option<&TargetId>,
) -> Option<String> {
    if !matches!(error.kind(), ErrorKind::NotFound | ErrorKind::Unsupported) {
        return None;
    }
    let target = target?;
    let device_id = target.device_id().clone();
    let device_exists = client
        .get_device(device_id.clone())
        .await
        .ok()
        .flatten()
        .is_some();
    Some(if device_exists {
        match target {
            TargetId::Device(_) => format!(
                "run `luminatectl inspect --device {device_id}` to see its surfaces and elements"
            ),
            TargetId::Surface { surface, .. } => format!(
                "run `luminatectl inspect --device {device_id} --surface {}` to see its elements",
                surface.as_str()
            ),
            TargetId::Element { .. } | TargetId::Group { .. } => format!(
                "run `luminatectl inspect --device {device_id}` to review the target's topology"
            ),
        }
    } else {
        "no such device; run `luminatectl list` to see available devices".to_owned()
    })
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
