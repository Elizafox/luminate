// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! D-Bus interfaces that proxy control and inspection through `libluminate`.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use luminate::policy::{PolicyRevision, PrincipalId};
use luminate::{
    AppearanceSlotId, AppearanceSlotUpdatePolicy, AppearanceSlotValue, Authentication,
    AuthenticationSource, BrightnessCapability, Client, CollectionId, DeviceId,
    PluginSetupSessionId, SceneCaptureMode, SceneId, TargetId, TransitionId,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use tokio::time;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::message::Header;
use zbus::object_server::{Interface, SignalEmitter};
use zbus::zvariant::{Error as ValueError, OwnedObjectPath, Type};

use crate::access_policy;
use crate::auth::{self, Decision};
use crate::capability;
use crate::collection;
use crate::control;
use crate::error::MethodError;
use crate::frame;
use crate::management::{self, ChangeRecord, Dictionary, dictionary, owned};
use crate::model::{
    Details, DeviceDetails, ElementDetails, GroupDetails, Kind, Object, SurfaceDetails,
};
use crate::path::parse_canonical_id;
use crate::path::{ROOT, canonical_id};
use crate::scene::{SceneBindingRequest, SceneRecord, record as scene_record};
use crate::setup;
use crate::state;
use crate::topology;
use crate::transition;

use crate::convert::{
    DbusEffectDescriptor, DbusStaticColourCapability, effect_descriptor, facet_strings,
    static_colour_capability,
};
use crate::effect_request::{EffectRequest, StaticColourRequest};

const PKCHECK_PATH: &str = "/usr/bin/pkcheck";
const PKCHECK_SEARCH_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/bin"];
const PKCHECK_TIMEOUT: Duration = Duration::from_secs(10);
const ATTESTATION_LIFETIME: Duration = Duration::from_secs(30);
const ATTESTED_CLIENT_REUSE: Duration = Duration::from_secs(25);
const MAX_ATTESTED_CLIENTS: usize = 64;

type DbusAppearanceSlotDescriptor = (
    String,
    String,
    Vec<DbusStaticColourCapability>,
    Vec<DbusEffectDescriptor>,
    Vec<String>,
    Vec<String>,
);

#[derive(Debug)]
pub struct AttestedClient {
    client: Arc<Client>,
    created_at: time::Instant,
}

#[derive(Debug)]
pub struct Shared {
    pub client: RwLock<Option<Arc<Client>>>,
    pub socket_path: Option<PathBuf>,
    pub attestation_sequence: AtomicU64,
    pub attested_clients: Mutex<HashMap<String, AttestedClient>>,
    pub objects: RwLock<BTreeMap<String, Object>>,

    pub required_gid: u32,
    pub polkit: bool,

    /// Root of procfs used to verify a caller's UID, process start time, and
    /// groups. Tests override it with a fake process tree.
    pub proc_root: PathBuf,
}

impl Shared {
    async fn client(&self) -> Result<Arc<Client>, MethodError> {
        self.client.read().await.clone().ok_or_else(|| {
            MethodError::DaemonUnavailable("the Luminate daemon is unavailable".into())
        })
    }

    async fn authorize(
        &self,
        connection: &Connection,
        header: &Header<'_>,
    ) -> Result<Arc<Client>, MethodError> {
        self.client_for_caller(connection, header, true).await
    }

    async fn attest(
        &self,
        connection: &Connection,
        header: &Header<'_>,
    ) -> Result<Arc<Client>, MethodError> {
        self.client_for_caller(connection, header, false).await
    }

    async fn client_for_caller(
        &self,
        connection: &Connection,
        header: &Header<'_>,
        privileged: bool,
    ) -> Result<Arc<Client>, MethodError> {
        let sender = header.sender().ok_or_else(|| {
            MethodError::PermissionDenied("D-Bus caller has no unique bus name".into())
        })?;
        let caller = sender.to_string();

        let proxy = DBusProxy::new(connection)
            .await
            .map_err(|error| MethodError::Internal(error.to_string()))?;

        let credentials = proxy
            .get_connection_credentials(sender.clone().into())
            .await
            .map_err(|error| MethodError::PermissionDenied(error.to_string()))?;

        let pid = credentials.process_id().ok_or_else(|| {
            MethodError::PermissionDenied("D-Bus caller has no Unix process ID".into())
        })?;

        let uid = credentials.unix_user_id().ok_or_else(|| {
            MethodError::PermissionDenied("D-Bus caller has no Unix user ID".into())
        })?;

        let snapshot = auth::process_snapshot(&self.proc_root, pid, uid)
            .map_err(|error| MethodError::PermissionDenied(error.to_string()))?;

        if privileged {
            match Decision::decide(&snapshot.groups, self.required_gid, self.polkit) {
                Decision::AllowGroup => {}
                Decision::Deny => {
                    return Err(MethodError::PermissionDenied(
                        "mutation requires membership in the luminate group".into(),
                    ));
                }
                Decision::ConsultPolkit => {
                    let pkcheck_path = pkcheck_path();
                    authorize_with_polkit(
                        &pkcheck_path,
                        &auth::polkit_process_subject(pid, snapshot.start_time, uid),
                        PKCHECK_TIMEOUT,
                    )
                    .await?;
                }
            }
        }

        let now = time::Instant::now();
        let mut attested_clients = self.attested_clients.lock().await;
        attested_clients
            .retain(|_, entry| now.duration_since(entry.created_at) < ATTESTED_CLIENT_REUSE);
        if let Some(entry) = attested_clients.get(&caller) {
            return Ok(Arc::clone(&entry.client));
        }

        let administrator = self.client().await?;
        let sequence = self.attestation_sequence.fetch_add(1, Ordering::Relaxed);
        let name = format!("dbus-{pid}-{sequence}");
        let expiry = SystemTime::now().checked_add(ATTESTATION_LIFETIME);
        let subject = PrincipalId::new("unix", uid.to_string())
            .map_err(|error| MethodError::Internal(error.to_string()))?;
        let verified_groups = snapshot
            .groups
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>();
        let created = administrator
            .authentication_administration()
            .create_principal_attestation(name.clone(), subject, verified_groups, expiry)
            .await
            .map_err(MethodError::from)?;
        let mut builder = Client::builder().authentication(Authentication::Attestation {
            name: name.clone(),
            credential: created.secret,
        });
        if let Some(path) = &self.socket_path {
            builder = builder.path(path);
        }
        let client = builder
            .connect()
            .await
            .map(Arc::new)
            .map_err(MethodError::from)?;
        if attested_clients.len() >= MAX_ATTESTED_CLIENTS
            && let Some(oldest) = attested_clients
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(caller, _)| caller.clone())
        {
            attested_clients.remove(&oldest);
        }
        attested_clients.insert(
            caller,
            AttestedClient {
                client: Arc::clone(&client),
                created_at: now,
            },
        );
        Ok(client)
    }
}

fn pkcheck_path() -> PathBuf {
    find_executable("pkcheck", PKCHECK_SEARCH_DIRS, Path::new(PKCHECK_PATH))
}

fn find_executable(name: &str, search_dirs: &[&str], fallback: &Path) -> PathBuf {
    search_dirs
        .iter()
        .map(|directory| Path::new(directory).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| fallback.to_path_buf())
}

async fn authorize_with_polkit(
    pkcheck_path: &Path,
    process_subject: &str,
    deadline: Duration,
) -> Result<(), MethodError> {
    run_polkit_command(polkit_command(pkcheck_path, process_subject), deadline).await
}

fn polkit_command(pkcheck_path: &Path, process_subject: &str) -> Command {
    let mut command = Command::new(pkcheck_path);
    command
        .args([
            "--action-id",
            "org.luminate.control",
            "--process",
            process_subject,
        ])
        .kill_on_drop(true);
    command
}

async fn run_polkit_command(mut command: Command, deadline: Duration) -> Result<(), MethodError> {
    let status = time::timeout(deadline, command.status())
        .await
        .map_err(|_| MethodError::PermissionDenied("Polkit authorization timed out".into()))?
        .map_err(|error| MethodError::PermissionDenied(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(MethodError::PermissionDenied(
            "Polkit denied the mutation".into(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct Manager {
    shared: Arc<Shared>,
}

impl Manager {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[derive(Debug, Clone)]
pub struct Manager2 {
    shared: Arc<Shared>,
}

impl Manager2 {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared }
    }
}

#[zbus::interface(name = "org.luminate.Luminate1.Manager2")]
impl Manager2 {
    #[zbus(property)]
    async fn available(&self) -> bool {
        self.shared.client.read().await.is_some()
    }

    async fn server_information(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(String, String, u32), MethodError> {
        let info = self
            .shared
            .attest(connection, &header)
            .await?
            .server_info()
            .await?;
        Ok((
            info.daemon_name,
            info.daemon_version,
            info.protocol_abi_version,
        ))
    }

    async fn ping(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .attest(connection, &header)
            .await?
            .ping()
            .await?;
        Ok(())
    }

    async fn list_withdrawn_devices(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<String>, MethodError> {
        Ok(self
            .shared
            .attest(connection, &header)
            .await?
            .list_withdrawn_devices()
            .await?
            .into_iter()
            .map(|device| device.as_str().to_owned())
            .collect())
    }

    async fn list_devices(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<Dictionary>, MethodError> {
        self.shared
            .attest(connection, &header)
            .await?
            .list_devices()
            .await?
            .into_iter()
            .map(|value| topology::device(&value))
            .collect()
    }

    async fn get_device(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let value = self
            .shared
            .attest(connection, &header)
            .await?
            .get_device(DeviceId::new(id))
            .await?;
        let mut result = dictionary([("Found", owned(value.is_some())?)]);
        if let Some(value) = value {
            result.insert("Device".into(), owned(topology::device(&value)?)?);
        }
        Ok(result)
    }

    async fn get_device_state(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let value = self
            .shared
            .attest(connection, &header)
            .await?
            .get_state(DeviceId::new(id))
            .await?;
        let mut result = dictionary([("HasState", owned(value.is_some())?)]);
        if let Some(value) = value {
            result.insert("State".into(), owned(state::device_state(value)?)?);
        }
        Ok(result)
    }

    async fn rescan(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .rescan()
            .await?;
        Ok(())
    }

    async fn purge_withdrawn_device(
        &self,
        device: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .purge_withdrawn_device(DeviceId::new(device))
            .await?;
        Ok(())
    }

    async fn list_collections(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<Dictionary>, MethodError> {
        self.shared
            .attest(connection, &header)
            .await?
            .list_collections()
            .await?
            .into_iter()
            .map(collection::record)
            .collect()
    }

    async fn get_collection(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let value = self
            .shared
            .attest(connection, &header)
            .await?
            .get_collection(CollectionId::new(id))
            .await?;
        let mut result = dictionary([("Found", owned(value.is_some())?)]);
        if let Some(value) = value {
            result.insert("Collection".into(), owned(collection::record(value)?)?);
        }
        Ok(result)
    }

    async fn get_collection_state(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let value = self
            .shared
            .attest(connection, &header)
            .await?
            .get_collection_state(CollectionId::new(id))
            .await?;
        let mut result = dictionary([("HasState", owned(value.is_some())?)]);
        if let Some(value) = value {
            result.insert("State".into(), owned(state::collection_state(value)?)?);
        }
        Ok(result)
    }

    async fn create_collection(
        &self,
        request: collection::CreateRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<String, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let (name, description, category, members) = request.into_parts()?;
        Ok(client
            .create_collection(name, description, category, members)
            .await?
            .as_str()
            .to_owned())
    }

    async fn destroy_collection(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .destroy_collection(CollectionId::new(id))
            .await?;
        Ok(())
    }

    async fn add_collection_member(
        &self,
        id: String,
        member: collection::MemberRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let member = member.into_member()?;
        self.shared
            .authorize(connection, &header)
            .await?
            .add_collection_member(CollectionId::new(id), member)
            .await?;
        Ok(())
    }

    async fn remove_collection_member(
        &self,
        id: String,
        member: collection::MemberRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let member = member.into_member()?;
        self.shared
            .authorize(connection, &header)
            .await?
            .remove_collection_member(CollectionId::new(id), member)
            .await?;
        Ok(())
    }

    async fn set_effect_selector(
        &self,
        selector: control::SelectorRequest,
        effect: EffectRequest,
        has_on_unsupported: bool,
        on_unsupported: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client
            .set_effect_selector(
                selector.into_selector()?,
                effect.into_effect()?,
                control::unsupported_policy(has_on_unsupported, &on_unsupported)?,
            )
            .await?;
        control::outcome(&outcome)
    }

    async fn set_colour_selector(
        &self,
        selector: control::SelectorRequest,
        colour: StaticColourRequest,
        has_on_unsupported: bool,
        on_unsupported: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client
            .set_colour_selector(
                selector.into_selector()?,
                colour.to_colour()?,
                control::unsupported_policy(has_on_unsupported, &on_unsupported)?,
            )
            .await?;
        control::outcome(&outcome)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the explicit RGB channels and optional-policy pair are compatibility-sensitive D-Bus arguments"
    )]
    async fn set_rgb_selector(
        &self,
        selector: control::SelectorRequest,
        red: u8,
        green: u8,
        blue: u8,
        has_on_unsupported: bool,
        on_unsupported: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client
            .set_rgb_selector(
                selector.into_selector()?,
                red,
                green,
                blue,
                control::unsupported_policy(has_on_unsupported, &on_unsupported)?,
            )
            .await?;
        control::outcome(&outcome)
    }

    async fn set_cct_selector(
        &self,
        selector: control::SelectorRequest,
        kelvin: u32,
        has_on_unsupported: bool,
        on_unsupported: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client
            .set_cct_selector(
                selector.into_selector()?,
                kelvin,
                control::unsupported_policy(has_on_unsupported, &on_unsupported)?,
            )
            .await?;
        control::outcome(&outcome)
    }

    async fn set_brightness_selector(
        &self,
        selector: control::SelectorRequest,
        value: u32,
        has_on_unsupported: bool,
        on_unsupported: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client
            .set_brightness_selector(
                selector.into_selector()?,
                value,
                control::unsupported_policy(has_on_unsupported, &on_unsupported)?,
            )
            .await?;
        control::outcome(&outcome)
    }

    async fn clear_selector(
        &self,
        selector: control::SelectorRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let outcome = self
            .shared
            .authorize(connection, &header)
            .await?
            .clear_target_selector(selector.into_selector()?)
            .await?;
        control::outcome(&outcome)
    }

    async fn save_current_selector(
        &self,
        selector: control::SelectorRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let outcome = self
            .shared
            .authorize(connection, &header)
            .await?
            .save_current_selector(selector.into_selector()?)
            .await?;
        control::outcome(&outcome)
    }

    async fn restore_appearance_selector(
        &self,
        selector: control::SelectorRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let outcome = self
            .shared
            .authorize(connection, &header)
            .await?
            .restore_appearance_selector(selector.into_selector()?)
            .await?;
        control::outcome(&outcome)
    }

    async fn set_emission_selector(
        &self,
        selector: control::SelectorRequest,
        state: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let state = control::emission_state(&state)?;
        let outcome = self
            .shared
            .authorize(connection, &header)
            .await?
            .set_emission_selector(selector.into_selector()?, state)
            .await?;
        control::outcome(&outcome)
    }

    async fn create_scene_to_scene_transition(
        &self,
        source: String,
        destination: String,
        options: transition::OptionsRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        transition::status(
            client
                .transitions()
                .scene_to_scene(
                    SceneId::new(source),
                    SceneId::new(destination),
                    options.into_options()?,
                )
                .await?,
        )
    }

    async fn create_current_to_scene_transition(
        &self,
        destination: String,
        options: transition::OptionsRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        transition::status(
            client
                .transitions()
                .current_to_scene(SceneId::new(destination), options.into_options()?)
                .await?,
        )
    }

    async fn create_scene_to_states_transition(
        &self,
        source: String,
        destination: Vec<transition::TargetStateRequest>,
        options: transition::OptionsRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        transition::status(
            client
                .transitions()
                .scene_to_states(
                    SceneId::new(source),
                    transition::states(destination)?,
                    options.into_options()?,
                )
                .await?,
        )
    }

    async fn create_current_to_states_transition(
        &self,
        destination: Vec<transition::TargetStateRequest>,
        options: transition::OptionsRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        transition::status(
            client
                .transitions()
                .current_to_states(transition::states(destination)?, options.into_options()?)
                .await?,
        )
    }

    async fn get_transition(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        transition::status(client.transitions().get(TransitionId::new(id)).await?)
    }

    async fn abort_transition(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        transition::status(client.transitions().abort(TransitionId::new(id)).await?)
    }

    async fn wait_transition(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        transition::status(client.transitions().wait(TransitionId::new(id)).await?)
    }

    async fn begin_frame_stream(
        &self,
        target: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<u32, MethodError> {
        Ok(self
            .shared
            .authorize(connection, &header)
            .await?
            .begin_frame_stream(parse_canonical_id(&target)?)
            .await?)
    }

    async fn upload_frame(
        &self,
        target: String,
        request: frame::Request,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(u64, bool), MethodError> {
        let acknowledgement = self
            .shared
            .authorize(connection, &header)
            .await?
            .upload_frame(parse_canonical_id(&target)?, request.into_envelope()?)
            .await?;
        Ok((acknowledgement.sequence, acknowledgement.dropped))
    }

    async fn end_frame_stream(
        &self,
        target: String,
        generation: u32,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .end_frame_stream(parse_canonical_id(&target)?, generation)
            .await?;
        Ok(())
    }

    async fn session_information(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        session_record(client.session())
    }

    async fn plugin_setup_workflows(
        &self,
        plugin: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<Dictionary>, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        setup::workflows(client.plugin_setup_workflows(plugin).await?)
    }

    async fn start_plugin_setup(
        &self,
        plugin: String,
        workflow: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        setup::session(client.start_plugin_setup(plugin, workflow).await?)
    }

    async fn respond_plugin_setup(
        &self,
        session: String,
        generation: u64,
        response: setup::InteractionResponse,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let session = PluginSetupSessionId::parse(session)
            .map_err(|error| MethodError::InvalidArgument(error.into()))?;
        let client = self.shared.authorize(connection, &header).await?;
        setup::session(
            client
                .respond_plugin_setup(session, generation, response.into_response()?)
                .await?,
        )
    }

    async fn plugin_setup_session(
        &self,
        session: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let session = PluginSetupSessionId::parse(session)
            .map_err(|error| MethodError::InvalidArgument(error.into()))?;
        let client = self.shared.attest(connection, &header).await?;
        setup::session(client.plugin_setup_session(session).await?)
    }

    async fn cancel_plugin_setup(
        &self,
        session: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let session = PluginSetupSessionId::parse(session)
            .map_err(|error| MethodError::InvalidArgument(error.into()))?;
        let client = self.shared.authorize(connection, &header).await?;
        setup::session(client.cancel_plugin_setup(session).await?)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method spells out the principal and optional expiry without a loose variant payload"
    )]
    async fn create_attestation(
        &self,
        name: String,
        authority: String,
        subject: String,
        has_expiry: bool,
        expires_at_unix_ms: u64,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(AttestationRecord, Vec<u8>), MethodError> {
        let subject = PrincipalId::new(authority, subject)
            .map_err(|error| MethodError::InvalidArgument(error.to_string()))?;
        let client = self.shared.authorize(connection, &header).await?;
        let created = client
            .authentication_administration()
            .create_attestation(name, subject, dbus_expiry(has_expiry, expires_at_unix_ms)?)
            .await?;
        Ok((
            attestation_record(created.metadata),
            created.secret.expose().to_vec(),
        ))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method spells out principal, verified groups, and optional expiry without a loose variant payload"
    )]
    async fn create_principal_attestation(
        &self,
        name: String,
        authority: String,
        subject: String,
        verified_groups: Vec<String>,
        has_expiry: bool,
        expires_at_unix_ms: u64,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(AttestationRecord, Vec<u8>), MethodError> {
        let subject = PrincipalId::new(authority, subject)
            .map_err(|error| MethodError::InvalidArgument(error.to_string()))?;
        let client = self.shared.authorize(connection, &header).await?;
        let created = client
            .authentication_administration()
            .create_principal_attestation(
                name,
                subject,
                verified_groups,
                dbus_expiry(has_expiry, expires_at_unix_ms)?,
            )
            .await?;
        Ok((
            attestation_record(created.metadata),
            created.secret.expose().to_vec(),
        ))
    }

    async fn list_attestations(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<AttestationRecord>, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        Ok(client
            .authentication_administration()
            .list_attestations()
            .await?
            .into_iter()
            .map(attestation_record)
            .collect())
    }

    async fn revoke_attestation(
        &self,
        name: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .authentication_administration()
            .revoke_attestation(name)
            .await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn topology_changed(
        emitter: &SignalEmitter<'_>,
        devices: Vec<String>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn state_changed(emitter: &SignalEmitter<'_>, devices: Vec<String>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn transitions_changed(
        emitter: &SignalEmitter<'_>,
        transitions: Vec<String>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn shm_stream_ended(
        emitter: &SignalEmitter<'_>,
        target: String,
        generation: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn configuration_changed(
        emitter: &SignalEmitter<'_>,
        revision: u64,
        changes: Vec<ChangeRecord>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn scenes_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// Non-secret daemon token metadata exposed as a typed D-Bus record.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TokenRecord {
    id: String,
    authority: String,
    subject: String,
    has_expiry: bool,
    expires_at_unix_ms: u64,
    revoked: bool,
}

/// Non-secret attestation metadata. The credential itself is returned only by
/// its creation method, mirroring the existing display-once token contract.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AttestationRecord {
    name: String,
    authority: String,
    subject: String,
    verified_groups: Vec<String>,
    credential_id: String,
    has_expiry: bool,
    expires_at_unix_ms: u64,
}

fn token_record(metadata: luminate::TokenMetadata) -> TokenRecord {
    let expires_at_unix_ms = metadata
        .expires_at
        .and_then(|expiry| expiry.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_millis()).ok());
    TokenRecord {
        id: metadata.id,
        authority: metadata.subject.authority().to_owned(),
        subject: metadata.subject.subject().to_owned(),
        has_expiry: expires_at_unix_ms.is_some(),
        expires_at_unix_ms: expires_at_unix_ms.unwrap_or_default(),
        revoked: metadata.revoked,
    }
}

fn attestation_record(metadata: luminate::AttestationMetadata) -> AttestationRecord {
    let expires_at_unix_ms = unix_millis(metadata.expires_at);
    AttestationRecord {
        name: metadata.name,
        authority: metadata.subject.authority().to_owned(),
        subject: metadata.subject.subject().to_owned(),
        verified_groups: metadata.verified_groups,
        credential_id: metadata.credential_id,
        has_expiry: expires_at_unix_ms.is_some(),
        expires_at_unix_ms: expires_at_unix_ms.unwrap_or_default(),
    }
}

fn session_record(metadata: &luminate::SessionMetadata) -> Result<Dictionary, MethodError> {
    let expires_at_unix_ms = unix_millis(metadata.expires_at);
    let (source, source_name) = match &metadata.source {
        AuthenticationSource::Peer => ("peer", None),
        AuthenticationSource::Bearer => ("bearer", None),
        AuthenticationSource::Attestation { name } => ("attestation", Some(name.clone())),
        AuthenticationSource::External { provider } => ("external", Some(provider.clone())),
    };
    let mut result = dictionary([
        ("Authority", owned(metadata.subject.authority().to_owned())?),
        ("Subject", owned(metadata.subject.subject().to_owned())?),
        ("VerifiedGroups", owned(metadata.verified_groups.clone())?),
        ("Source", owned(source)?),
        ("HasSourceName", owned(source_name.is_some())?),
        ("HasCredentialId", owned(metadata.credential_id.is_some())?),
        ("HasExpiry", owned(expires_at_unix_ms.is_some())?),
    ]);
    if let Some(source_name) = source_name {
        result.insert("SourceName".into(), owned(source_name)?);
    }
    if let Some(credential_id) = &metadata.credential_id {
        result.insert("CredentialId".into(), owned(credential_id.clone())?);
    }
    if let Some(expires_at_unix_ms) = expires_at_unix_ms {
        result.insert("ExpiresAtUnixMs".into(), owned(expires_at_unix_ms)?);
    }
    Ok(result)
}

fn unix_millis(value: Option<SystemTime>) -> Option<u64> {
    let duration = value.and_then(|expiry| expiry.duration_since(UNIX_EPOCH).ok())?;
    u64::try_from(duration.as_millis()).ok()
}

fn dbus_expiry(has_expiry: bool, unix_ms: u64) -> Result<Option<SystemTime>, MethodError> {
    if !has_expiry {
        return Ok(None);
    }
    UNIX_EPOCH
        .checked_add(Duration::from_millis(unix_ms))
        .map(Some)
        .ok_or_else(|| MethodError::InvalidArgument("expiry timestamp is out of range".into()))
}

#[zbus::interface(name = "org.luminate.Luminate1.Manager1")]
impl Manager {
    #[zbus(property)]
    async fn available(&self) -> bool {
        self.shared.client.read().await.is_some()
    }

    async fn refresh_topology(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        let _ = client.list_devices().await?;
        Ok(())
    }

    async fn list_scenes(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<SceneRecord>, MethodError> {
        self.shared
            .attest(connection, &header)
            .await?
            .list_scenes()
            .await?
            .into_iter()
            .map(scene_record)
            .collect()
    }

    async fn get_scene(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<SceneRecord, MethodError> {
        let scene = self
            .shared
            .attest(connection, &header)
            .await?
            .get_scene(SceneId::new(&id))
            .await?
            .ok_or_else(|| MethodError::NotFound(format!("scene {id:?}")))?;
        scene_record(scene)
    }

    async fn create_scene(
        &self,
        name: String,
        has_description: bool,
        description: String,
        bindings: Vec<SceneBindingRequest>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<SceneRecord, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let bindings = bindings
            .into_iter()
            .map(SceneBindingRequest::into_binding)
            .collect::<Result<_, _>>()?;
        scene_record(
            client
                .create_scene(name, has_description.then_some(description), bindings)
                .await?,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method carries capture mode and optional-description flags explicitly"
    )]
    async fn capture_scene(
        &self,
        name: String,
        has_description: bool,
        description: String,
        has_dynamic_collection: bool,
        dynamic_collection: String,
        targets: Vec<String>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<SceneRecord, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let mode = if has_dynamic_collection {
            SceneCaptureMode::DynamicCollectionMembers {
                collection: CollectionId::new(dynamic_collection),
            }
        } else {
            SceneCaptureMode::Frozen
        };
        let targets = targets
            .iter()
            .map(|target| parse_canonical_id(target))
            .collect::<Result<_, _>>()?;
        scene_record(
            client
                .capture_scene(name, has_description.then_some(description), mode, targets)
                .await?,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method mirrors the revisioned scene definition"
    )]
    async fn replace_scene(
        &self,
        id: String,
        expected_revision: u64,
        name: String,
        has_description: bool,
        description: String,
        bindings: Vec<SceneBindingRequest>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<SceneRecord, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let bindings = bindings
            .into_iter()
            .map(SceneBindingRequest::into_binding)
            .collect::<Result<_, _>>()?;
        scene_record(
            client
                .replace_scene(
                    SceneId::new(id),
                    expected_revision,
                    name,
                    has_description.then_some(description),
                    bindings,
                )
                .await?,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method mirrors revisioned scene capture"
    )]
    async fn recapture_scene(
        &self,
        id: String,
        expected_revision: u64,
        has_dynamic_collection: bool,
        dynamic_collection: String,
        targets: Vec<String>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<SceneRecord, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let mode = if has_dynamic_collection {
            SceneCaptureMode::DynamicCollectionMembers {
                collection: CollectionId::new(dynamic_collection),
            }
        } else {
            SceneCaptureMode::Frozen
        };
        let targets = targets
            .iter()
            .map(|target| parse_canonical_id(target))
            .collect::<Result<_, _>>()?;
        scene_record(
            client
                .recapture_scene(SceneId::new(id), expected_revision, mode, targets)
                .await?,
        )
    }

    async fn delete_scene(
        &self,
        id: String,
        expected_revision: u64,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        client
            .delete_scene(SceneId::new(id), expected_revision)
            .await?;
        Ok(())
    }

    async fn apply_scene(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(Vec<String>, Vec<String>), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let outcome = client.apply_scene(SceneId::new(id)).await?;
        Ok((
            outcome.applied.iter().map(canonical_id).collect(),
            outcome.denied.iter().map(canonical_id).collect(),
        ))
    }

    async fn get_management(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let snapshot = client.get_management().await?;
        management::snapshot(snapshot)
    }

    async fn get_access_policy(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let document = client.policy_administration().get().await?;
        access_policy::encode(&document)
    }

    async fn replace_access_policy(
        &self,
        expected_revision: u64,
        replacement: Dictionary,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let replacement = access_policy::decode(replacement)?;
        let client = self.shared.authorize(connection, &header).await?;
        let document = client
            .policy_administration()
            .replace(PolicyRevision(expected_revision), replacement)
            .await?;
        access_policy::encode(&document)
    }

    async fn patch_management(
        &self,
        expected_revision: u64,
        mutations: Vec<Dictionary>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(u64, Vec<ChangeRecord>), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let patch = management::patch(expected_revision, mutations)?;
        let changes = client.patch_management(patch).await?;
        Ok(management::changes(changes))
    }

    async fn list_tokens(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<TokenRecord>, MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        Ok(client
            .authentication_administration()
            .list_tokens()
            .await?
            .into_iter()
            .map(token_record)
            .collect())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the D-Bus method spells out the principal and optional expiry without a loose variant payload"
    )]
    async fn create_token(
        &self,
        id: String,
        authority: String,
        subject: String,
        has_expiry: bool,
        expires_at_unix_ms: u64,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(TokenRecord, Vec<u8>), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let subject = PrincipalId::new(authority, subject)
            .map_err(|error| MethodError::InvalidArgument(error.to_string()))?;
        let created = client
            .authentication_administration()
            .create_token(id, subject, dbus_expiry(has_expiry, expires_at_unix_ms)?)
            .await?;
        Ok((
            token_record(created.metadata),
            created.secret.expose().to_vec(),
        ))
    }

    async fn rotate_token(
        &self,
        id: String,
        has_expiry: bool,
        expires_at_unix_ms: u64,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(TokenRecord, Vec<u8>), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let created = client
            .authentication_administration()
            .rotate_token(id, dbus_expiry(has_expiry, expires_at_unix_ms)?)
            .await?;
        Ok((
            token_record(created.metadata),
            created.secret.expose().to_vec(),
        ))
    }

    async fn revoke_token(
        &self,
        id: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .authentication_administration()
            .revoke_token(id)
            .await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn configuration_changed(
        emitter: &SignalEmitter<'_>,
        revision: u64,
        changes: Vec<ChangeRecord>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn scenes_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[derive(Debug, Clone)]
pub struct Target {
    shared: Arc<Shared>,
    object: Object,
}

impl Target {
    pub fn new(shared: Arc<Shared>, object: Object) -> Self {
        Self { shared, object }
    }

    fn device(&self) -> DeviceId {
        match &self.object.target {
            TargetId::Device(device)
            | TargetId::Surface { device, .. }
            | TargetId::Group { device, .. }
            | TargetId::Element { device, .. } => device.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target3 {
    shared: Arc<Shared>,
    object: Object,
    capabilities: Dictionary,
}

impl Target3 {
    fn new(shared: Arc<Shared>, object: Object) -> zbus::Result<Self> {
        let capabilities = capability::capabilities(&object.capabilities)
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        Ok(Self {
            shared,
            object,
            capabilities,
        })
    }

    async fn update(connection: &Connection, object: Object) -> zbus::Result<()> {
        let capabilities = capability::capabilities(&object.capabilities)
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        let interface_ref = connection
            .object_server()
            .interface::<_, Self>(object.path.as_str())
            .await?;
        let mut interface = interface_ref.get_mut().await;
        interface.object = object;
        interface.capabilities = capabilities;
        let emitter = interface_ref.signal_emitter();
        interface.identifier_changed(emitter).await?;
        interface.capabilities_changed(emitter).await
    }
}

#[zbus::interface(name = "org.luminate.Target3")]
impl Target3 {
    #[zbus(property)]
    fn identifier(&self) -> String {
        canonical_id(&self.object.target)
    }

    #[zbus(property)]
    fn capabilities(&self) -> Dictionary {
        self.capabilities.clone()
    }

    async fn state(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Dictionary, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        let status = client.get_state(self.device()).await?;
        let mut result = dictionary([("HasState", owned(status.is_some())?)]);
        if let Some(status) = status {
            result.insert(
                "State".into(),
                owned(state::target_state(status, &self.object.target)?)?,
            );
        }
        Ok(result)
    }

    async fn set_colour(
        &self,
        colour: StaticColourRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .set_colour(self.object.target.clone(), colour.to_colour()?)
            .await?;
        Ok(())
    }

    async fn set_rgb(
        &self,
        red: u8,
        green: u8,
        blue: u8,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .set_rgb(self.object.target.clone(), red, green, blue)
            .await?;
        Ok(())
    }

    async fn set_cct(
        &self,
        kelvin: u32,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .set_cct(self.object.target.clone(), kelvin)
            .await?;
        Ok(())
    }

    async fn save_current(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .save_current(self.object.target.clone())
            .await?;
        Ok(())
    }

    async fn restore_appearance(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        self.shared
            .authorize(connection, &header)
            .await?
            .restore_appearance(self.object.target.clone())
            .await?;
        Ok(())
    }

    async fn set_emission(
        &self,
        state: String,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let state = control::emission_state(&state)?;
        self.shared
            .authorize(connection, &header)
            .await?
            .set_emission(self.object.target.clone(), state)
            .await?;
        Ok(())
    }
}

impl Target3 {
    fn device(&self) -> DeviceId {
        match &self.object.target {
            TargetId::Device(device)
            | TargetId::Surface { device, .. }
            | TargetId::Group { device, .. }
            | TargetId::Element { device, .. } => device.clone(),
        }
    }
}

#[zbus::interface(name = "org.luminate.Target2")]
impl Target {
    #[zbus(property)]
    fn identifier(&self) -> String {
        canonical_id(&self.object.target)
    }

    #[zbus(property)]
    fn static_colour_capabilities(&self) -> Vec<DbusStaticColourCapability> {
        self.object
            .capabilities
            .colour
            .iter()
            .map(static_colour_capability)
            .collect()
    }

    #[zbus(property)]
    fn can_set_brightness(&self) -> bool {
        matches!(
            self.object.capabilities.brightness,
            BrightnessCapability::Independent { .. }
        )
    }

    #[zbus(property)]
    fn brightness_maximum(&self) -> u32 {
        self.object.brightness_maximum()
    }

    #[zbus(property)]
    fn can_set_off(&self) -> bool {
        self.object.capabilities.emission || self.object.capabilities.physical_power.is_some()
    }

    #[zbus(property)]
    fn can_set_effect(&self) -> bool {
        !self.object.capabilities.colour.is_empty()
            || self
                .object
                .capabilities
                .hardware_effects
                .as_ref()
                .is_some_and(|effects| !effects.effects.is_empty())
    }

    #[zbus(property)]
    fn effect_descriptors(&self) -> Vec<DbusEffectDescriptor> {
        self.object
            .capabilities
            .hardware_effects
            .as_ref()
            .map(|effects| effects.effects.iter().map(effect_descriptor).collect())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn appearance_slot_update_policy(&self) -> String {
        self.object
            .capabilities
            .appearance_slots
            .as_ref()
            .map_or_else(String::new, |value| {
                match value.update_policy {
                    AppearanceSlotUpdatePolicy::Independent => "independent",
                    AppearanceSlotUpdatePolicy::PartialIfKnown => "partial-if-known",
                    AppearanceSlotUpdatePolicy::CompleteSet => "complete-set",
                }
                .to_owned()
            })
    }

    #[zbus(property)]
    fn appearance_slot_descriptors(&self) -> Vec<DbusAppearanceSlotDescriptor> {
        self.object
            .capabilities
            .appearance_slots
            .as_ref()
            .map_or_else(Vec::new, |capability| {
                capability
                    .slots
                    .iter()
                    .map(|slot| {
                        (
                            slot.id.as_str().to_owned(),
                            slot.name.clone(),
                            slot.appearance
                                .colour
                                .iter()
                                .map(static_colour_capability)
                                .collect(),
                            slot.appearance
                                .hardware_effects
                                .as_ref()
                                .map(|effects| {
                                    effects.effects.iter().map(effect_descriptor).collect()
                                })
                                .unwrap_or_default(),
                            slot.notes.clone(),
                            slot.warnings.clone(),
                        )
                    })
                    .collect()
            })
    }

    async fn state(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<Vec<(String, String, String, bool, u64)>, MethodError> {
        let client = self.shared.attest(connection, &header).await?;
        let state = client.get_state(self.device()).await?;
        Ok(state
            .into_iter()
            .flat_map(|state| state.observations)
            .filter(|observation| observation.target == self.object.target)
            .map(|observation| {
                let (facet, value) = facet_strings(&observation.value);
                (
                    facet,
                    value,
                    format!("{:?}", observation.confidence),
                    observation.stale,
                    observation.observed_at_ms,
                )
            })
            .collect())
    }

    async fn refresh_state(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        client.refresh_state(self.device()).await?;
        Ok(())
    }

    async fn set_brightness(
        &self,
        value: u32,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        client
            .set_brightness(self.object.target.clone(), value)
            .await?;
        Ok(())
    }

    async fn set_effect(
        &self,
        request: EffectRequest,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let effect = request.into_effect()?;
        client
            .set_effect(self.object.target.clone(), effect)
            .await?;
        Ok(())
    }

    async fn set_appearance_slots(
        &self,
        values: Vec<(String, EffectRequest)>,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        let values = values
            .into_iter()
            .map(|(slot, effect)| {
                Ok(AppearanceSlotValue {
                    slot: AppearanceSlotId::new(slot),
                    effect: effect.into_effect()?,
                })
            })
            .collect::<Result<Vec<_>, MethodError>>()?;
        client
            .set_appearance_slots(self.object.target.clone(), values)
            .await?;
        Ok(())
    }

    async fn off(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        client.set_off(self.object.target.clone()).await?;
        Ok(())
    }

    async fn clear_desired_state(
        &self,
        #[zbus(connection)] connection: &Connection,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(), MethodError> {
        let client = self.shared.authorize(connection, &header).await?;
        client.clear_target(self.object.target.clone()).await?;
        Ok(())
    }
}

macro_rules! typed_interface {
    ($type:ident, $interface:literal) => {
        #[derive(Debug, Clone)]
        pub struct $type {
            name: String,
        }

        impl $type {
            pub fn new(name: String) -> Self {
                Self { name }
            }
        }

        #[zbus::interface(name = $interface)]
        impl $type {
            #[zbus(property)]
            #[allow(
                clippy::same_name_method,
                reason = "Name is the conventional D-Bus display-name property"
            )]
            fn name(&self) -> &str {
                &self.name
            }
        }
    };
}

typed_interface!(DeviceInterface, "org.luminate.Luminate1.Device1");
typed_interface!(SurfaceInterface, "org.luminate.Luminate1.Surface1");
typed_interface!(GroupInterface, "org.luminate.Luminate1.Group1");
typed_interface!(ElementInterface, "org.luminate.Luminate1.Element1");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device2 {
    name: String,
    details: DeviceDetails,
    surfaces: Vec<OwnedObjectPath>,
    groups: Vec<OwnedObjectPath>,
}

impl Device2 {
    fn new(name: String, details: DeviceDetails) -> Result<Self, ValueError> {
        let surfaces = details
            .surfaces
            .iter()
            .map(|path| OwnedObjectPath::try_from(path.clone()))
            .collect::<Result<_, _>>()?;
        let groups = details
            .groups
            .iter()
            .map(|path| OwnedObjectPath::try_from(path.clone()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            name,
            details,
            surfaces,
            groups,
        })
    }

    async fn update(
        connection: &Connection,
        path: &str,
        name: String,
        details: DeviceDetails,
    ) -> zbus::Result<()> {
        let next = Self::new(name, details)?;
        let interface_ref = connection
            .object_server()
            .interface::<_, Self>(path)
            .await?;
        let mut interface = interface_ref.get_mut().await;
        *interface = next;
        let emitter = interface_ref.signal_emitter();
        interface.id_changed(emitter).await?;
        interface.name_changed(emitter).await?;
        interface.has_vendor_changed(emitter).await?;
        interface.vendor_changed(emitter).await?;
        interface.has_model_changed(emitter).await?;
        interface.model_changed(emitter).await?;
        interface.has_provider_instance_changed(emitter).await?;
        interface.provider_instance_changed(emitter).await?;
        interface.has_category_changed(emitter).await?;
        interface.category_changed(emitter).await?;
        interface.physical_tags_changed(emitter).await?;
        interface.host_attached_changed(emitter).await?;
        interface.notes_changed(emitter).await?;
        interface.warnings_changed(emitter).await?;
        interface.surfaces_changed(emitter).await?;
        interface.groups_changed(emitter).await
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Surface2 {
    name: String,
    details: SurfaceDetails,
    elements: Vec<OwnedObjectPath>,
}

impl Surface2 {
    fn new(name: String, details: SurfaceDetails) -> Result<Self, ValueError> {
        let elements = details
            .elements
            .iter()
            .map(|path| OwnedObjectPath::try_from(path.clone()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            name,
            details,
            elements,
        })
    }

    async fn update(
        connection: &Connection,
        path: &str,
        name: String,
        details: SurfaceDetails,
    ) -> zbus::Result<()> {
        let next = Self::new(name, details)?;
        let interface_ref = connection
            .object_server()
            .interface::<_, Self>(path)
            .await?;
        let mut interface = interface_ref.get_mut().await;
        *interface = next;
        let emitter = interface_ref.signal_emitter();
        interface.id_changed(emitter).await?;
        interface.name_changed(emitter).await?;
        interface.kind_changed(emitter).await?;
        interface.has_length_changed(emitter).await?;
        interface.length_changed(emitter).await?;
        interface.has_dimensions_changed(emitter).await?;
        interface.width_changed(emitter).await?;
        interface.height_changed(emitter).await?;
        interface.has_matrix_changed(emitter).await?;
        interface.rows_changed(emitter).await?;
        interface.columns_changed(emitter).await?;
        interface.physical_tags_changed(emitter).await?;
        interface.elements_changed(emitter).await?;
        interface.notes_changed(emitter).await?;
        interface.warnings_changed(emitter).await
    }
}

#[zbus::interface(name = "org.luminate.Luminate1.Surface2")]
impl Surface2 {
    #[zbus(property)]
    fn id(&self) -> &str {
        &self.details.id
    }

    #[zbus(property)]
    #[allow(
        clippy::same_name_method,
        reason = "Name is the conventional D-Bus display-name property"
    )]
    fn name(&self) -> &str {
        &self.name
    }

    #[zbus(property)]
    fn kind(&self) -> &str {
        &self.details.kind
    }

    #[zbus(property)]
    fn has_length(&self) -> bool {
        self.details.length.is_some()
    }

    #[zbus(property)]
    fn length(&self) -> f64 {
        self.details.length.unwrap_or_default()
    }

    #[zbus(property)]
    fn has_dimensions(&self) -> bool {
        self.details.dimensions.is_some()
    }

    #[zbus(property)]
    fn width(&self) -> f64 {
        self.details.dimensions.map_or(0.0, |value| value.0)
    }

    #[zbus(property)]
    fn height(&self) -> f64 {
        self.details.dimensions.map_or(0.0, |value| value.1)
    }

    #[zbus(property)]
    fn has_matrix(&self) -> bool {
        self.details.matrix.is_some()
    }

    #[zbus(property)]
    fn rows(&self) -> u16 {
        self.details.matrix.map_or(0, |value| value.0)
    }

    #[zbus(property)]
    fn columns(&self) -> u16 {
        self.details.matrix.map_or(0, |value| value.1)
    }

    #[zbus(property)]
    fn physical_tags(&self) -> &[String] {
        &self.details.physical_tags
    }

    #[zbus(property)]
    fn elements(&self) -> &[OwnedObjectPath] {
        &self.elements
    }

    #[zbus(property)]
    fn notes(&self) -> &[String] {
        &self.details.notes
    }

    #[zbus(property)]
    fn warnings(&self) -> &[String] {
        &self.details.warnings
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element2 {
    details: ElementDetails,
    surface: OwnedObjectPath,
}

impl Element2 {
    fn new(details: ElementDetails) -> Result<Self, ValueError> {
        let surface = OwnedObjectPath::try_from(details.surface.clone())?;
        Ok(Self { details, surface })
    }

    async fn update(
        connection: &Connection,
        path: &str,
        details: ElementDetails,
    ) -> zbus::Result<()> {
        let next = Self::new(details)?;
        let interface_ref = connection
            .object_server()
            .interface::<_, Self>(path)
            .await?;
        let mut interface = interface_ref.get_mut().await;
        *interface = next;
        let emitter = interface_ref.signal_emitter();
        interface.id_changed(emitter).await?;
        interface.has_name_changed(emitter).await?;
        interface.name_changed(emitter).await?;
        interface.kind_changed(emitter).await?;
        interface.has_geometry_changed(emitter).await?;
        interface.geometry_kind_changed(emitter).await?;
        interface.x_changed(emitter).await?;
        interface.y_changed(emitter).await?;
        interface.width_changed(emitter).await?;
        interface.height_changed(emitter).await?;
        interface.position_changed(emitter).await?;
        interface.row_changed(emitter).await?;
        interface.column_changed(emitter).await?;
        interface.surface_changed(emitter).await?;
        interface.physical_tags_changed(emitter).await?;
        interface.notes_changed(emitter).await?;
        interface.warnings_changed(emitter).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group2 {
    name: String,
    details: GroupDetails,
    members: Vec<OwnedObjectPath>,
}

impl Group2 {
    fn new(name: String, details: GroupDetails) -> Result<Self, ValueError> {
        let members = details
            .members
            .iter()
            .map(|path| OwnedObjectPath::try_from(path.clone()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            name,
            details,
            members,
        })
    }

    async fn update(
        connection: &Connection,
        path: &str,
        name: String,
        details: GroupDetails,
    ) -> zbus::Result<()> {
        let next = Self::new(name, details)?;
        let interface_ref = connection
            .object_server()
            .interface::<_, Self>(path)
            .await?;
        let mut interface = interface_ref.get_mut().await;
        *interface = next;
        let emitter = interface_ref.signal_emitter();
        interface.id_changed(emitter).await?;
        interface.name_changed(emitter).await?;
        interface.has_description_changed(emitter).await?;
        interface.description_changed(emitter).await?;
        interface.kind_changed(emitter).await?;
        interface.members_changed(emitter).await?;
        interface.notes_changed(emitter).await?;
        interface.warnings_changed(emitter).await
    }
}

#[zbus::interface(name = "org.luminate.Luminate1.Group2")]
impl Group2 {
    #[zbus(property)]
    fn id(&self) -> &str {
        &self.details.id
    }

    #[zbus(property)]
    #[allow(
        clippy::same_name_method,
        reason = "Name is the conventional D-Bus display-name property"
    )]
    fn name(&self) -> &str {
        &self.name
    }

    #[zbus(property)]
    fn has_description(&self) -> bool {
        self.details.description.is_some()
    }

    #[zbus(property)]
    fn description(&self) -> &str {
        self.details.description.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn kind(&self) -> &str {
        &self.details.kind
    }

    #[zbus(property)]
    fn members(&self) -> &[OwnedObjectPath] {
        &self.members
    }

    #[zbus(property)]
    fn notes(&self) -> &[String] {
        &self.details.notes
    }

    #[zbus(property)]
    fn warnings(&self) -> &[String] {
        &self.details.warnings
    }
}

#[zbus::interface(name = "org.luminate.Luminate1.Element2")]
impl Element2 {
    #[zbus(property)]
    fn id(&self) -> &str {
        &self.details.id
    }

    #[zbus(property)]
    fn has_name(&self) -> bool {
        self.details.name.is_some()
    }

    #[zbus(property)]
    #[allow(
        clippy::same_name_method,
        reason = "Name is the conventional D-Bus display-name property"
    )]
    fn name(&self) -> &str {
        self.details.name.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn kind(&self) -> &str {
        &self.details.kind
    }

    #[zbus(property)]
    fn has_geometry(&self) -> bool {
        self.details.geometry_kind.is_some()
    }

    #[zbus(property)]
    fn geometry_kind(&self) -> &str {
        self.details.geometry_kind.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn x(&self) -> f64 {
        self.details.x.unwrap_or_default()
    }

    #[zbus(property)]
    fn y(&self) -> f64 {
        self.details.y.unwrap_or_default()
    }

    #[zbus(property)]
    fn width(&self) -> f64 {
        self.details.width.unwrap_or_default()
    }

    #[zbus(property)]
    fn height(&self) -> f64 {
        self.details.height.unwrap_or_default()
    }

    #[zbus(property)]
    fn position(&self) -> f64 {
        self.details.position.unwrap_or_default()
    }

    #[zbus(property)]
    fn row(&self) -> u16 {
        self.details.matrix_cell.map_or(0, |value| value.0)
    }

    #[zbus(property)]
    fn column(&self) -> u16 {
        self.details.matrix_cell.map_or(0, |value| value.1)
    }

    #[zbus(property)]
    fn surface(&self) -> OwnedObjectPath {
        self.surface.clone()
    }

    #[zbus(property)]
    fn physical_tags(&self) -> &[String] {
        &self.details.physical_tags
    }

    #[zbus(property)]
    fn notes(&self) -> &[String] {
        &self.details.notes
    }

    #[zbus(property)]
    fn warnings(&self) -> &[String] {
        &self.details.warnings
    }
}

#[zbus::interface(name = "org.luminate.Luminate1.Device2")]
impl Device2 {
    #[zbus(property)]
    fn id(&self) -> &str {
        &self.details.id
    }

    #[zbus(property)]
    #[allow(
        clippy::same_name_method,
        reason = "Name is the conventional D-Bus display-name property"
    )]
    fn name(&self) -> &str {
        &self.name
    }

    #[zbus(property)]
    fn has_vendor(&self) -> bool {
        self.details.vendor.is_some()
    }

    #[zbus(property)]
    fn vendor(&self) -> &str {
        self.details.vendor.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn has_model(&self) -> bool {
        self.details.model.is_some()
    }

    #[zbus(property)]
    fn model(&self) -> &str {
        self.details.model.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn has_provider_instance(&self) -> bool {
        self.details.provider_instance.is_some()
    }

    #[zbus(property)]
    fn provider_instance(&self) -> &str {
        self.details
            .provider_instance
            .as_deref()
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn has_category(&self) -> bool {
        self.details.category.is_some()
    }

    #[zbus(property)]
    fn category(&self) -> &str {
        self.details.category.as_deref().unwrap_or_default()
    }

    #[zbus(property)]
    fn physical_tags(&self) -> &[String] {
        &self.details.physical_tags
    }

    #[zbus(property)]
    fn host_attached(&self) -> bool {
        self.details.host_attached
    }

    #[zbus(property)]
    fn notes(&self) -> &[String] {
        &self.details.notes
    }

    #[zbus(property)]
    fn warnings(&self) -> &[String] {
        &self.details.warnings
    }

    #[zbus(property)]
    fn surfaces(&self) -> &[OwnedObjectPath] {
        &self.surfaces
    }

    #[zbus(property)]
    fn groups(&self) -> &[OwnedObjectPath] {
        &self.groups
    }
}

pub async fn add_object(
    connection: &Connection,
    shared: Arc<Shared>,
    object: Object,
) -> zbus::Result<()> {
    let path = object.path.clone();
    let name = object.name.clone();
    let kind = object.kind;
    let details = object.details.clone();
    let target3 = Target3::new(Arc::clone(&shared), object.clone())?;
    let _ = connection
        .object_server()
        .at(path.clone(), Target::new(shared, object))
        .await?;
    let _ = connection.object_server().at(path.clone(), target3).await?;
    match kind {
        Kind::Device => {
            let Details::Device(details) = details else {
                return Err(zbus::Error::Failure(
                    "device object is missing device topology details".into(),
                ));
            };
            let device2 = Device2::new(name.clone(), *details)?;
            let _ = connection
                .object_server()
                .at(path.clone(), DeviceInterface::new(name))
                .await?;
            let _ = connection.object_server().at(path, device2).await?;
        }
        Kind::Surface => {
            let Details::Surface(details) = details else {
                return Err(zbus::Error::Failure(
                    "surface object is missing surface topology details".into(),
                ));
            };
            let surface2 = Surface2::new(name.clone(), *details)?;
            let _ = connection
                .object_server()
                .at(path.clone(), SurfaceInterface::new(name))
                .await?;
            let _ = connection.object_server().at(path, surface2).await?;
        }
        Kind::Group => {
            let Details::Group(details) = details else {
                return Err(zbus::Error::Failure(
                    "group object is missing group topology details".into(),
                ));
            };
            let group2 = Group2::new(name.clone(), *details)?;
            let _ = connection
                .object_server()
                .at(path.clone(), GroupInterface::new(name))
                .await?;
            let _ = connection.object_server().at(path, group2).await?;
        }
        Kind::Element => {
            let Details::Element(details) = details else {
                return Err(zbus::Error::Failure(
                    "element object is missing element topology details".into(),
                ));
            };
            let element2 = Element2::new(*details)?;
            let _ = connection
                .object_server()
                .at(path.clone(), ElementInterface::new(name))
                .await?;
            let _ = connection.object_server().at(path, element2).await?;
        }
    }
    Ok(())
}

pub async fn update_object(connection: &Connection, object: Object) -> zbus::Result<()> {
    Target3::update(connection, object.clone()).await?;
    let target_ref = connection
        .object_server()
        .interface::<_, Target>(object.path.as_str())
        .await?;
    {
        let mut target = target_ref.get_mut().await;
        target.object = object.clone();
        target
            .identifier_changed(target_ref.signal_emitter())
            .await?;
        target
            .static_colour_capabilities_changed(target_ref.signal_emitter())
            .await?;
        target
            .can_set_brightness_changed(target_ref.signal_emitter())
            .await?;
        target
            .brightness_maximum_changed(target_ref.signal_emitter())
            .await?;
        target
            .can_set_off_changed(target_ref.signal_emitter())
            .await?;
        target
            .can_set_effect_changed(target_ref.signal_emitter())
            .await?;
        target
            .effect_descriptors_changed(target_ref.signal_emitter())
            .await?;
        target
            .appearance_slot_update_policy_changed(target_ref.signal_emitter())
            .await?;
        target
            .appearance_slot_descriptors_changed(target_ref.signal_emitter())
            .await?;
    };
    match object.kind {
        Kind::Device => {
            let Details::Device(details) = object.details else {
                return Err(zbus::Error::Failure(
                    "device object is missing device topology details".into(),
                ));
            };
            Device2::update(connection, &object.path, object.name.clone(), *details).await?;
            update_name::<DeviceInterface>(connection, &object.path, object.name).await?;
        }
        Kind::Surface => {
            let Details::Surface(details) = object.details else {
                return Err(zbus::Error::Failure(
                    "surface object is missing surface topology details".into(),
                ));
            };
            Surface2::update(connection, &object.path, object.name.clone(), *details).await?;
            update_name::<SurfaceInterface>(connection, &object.path, object.name).await?;
        }
        Kind::Group => {
            let Details::Group(details) = object.details else {
                return Err(zbus::Error::Failure(
                    "group object is missing group topology details".into(),
                ));
            };
            Group2::update(connection, &object.path, object.name.clone(), *details).await?;
            update_name::<GroupInterface>(connection, &object.path, object.name).await?;
        }
        Kind::Element => {
            let Details::Element(details) = object.details else {
                return Err(zbus::Error::Failure(
                    "element object is missing element topology details".into(),
                ));
            };
            Element2::update(connection, &object.path, *details).await?;
            update_name::<ElementInterface>(connection, &object.path, object.name).await?;
        }
    }
    Ok(())
}

async fn update_name<I>(connection: &Connection, path: &str, name: String) -> zbus::Result<()>
where
    I: Interface + NamedInterface,
{
    I::update_name(connection, path, name).await
}

trait NamedInterface: Interface {
    fn update_name<'a>(
        connection: &'a Connection,
        path: &'a str,
        name: String,
    ) -> impl Future<Output = zbus::Result<()>> + Send + 'a;
}

macro_rules! impl_named_interface {
    ($type:ident) => {
        impl NamedInterface for $type {
            async fn update_name(
                connection: &Connection,
                path: &str,
                name: String,
            ) -> zbus::Result<()> {
                let interface_ref = connection
                    .object_server()
                    .interface::<_, Self>(path)
                    .await?;
                let mut interface = interface_ref.get_mut().await;
                interface.name = name;
                interface.name_changed(interface_ref.signal_emitter()).await
            }
        }
    };
}

impl_named_interface!(DeviceInterface);
impl_named_interface!(SurfaceInterface);
impl_named_interface!(GroupInterface);
impl_named_interface!(ElementInterface);

pub async fn available_changed(connection: &Connection) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager>(ROOT)
        .await?;
    let manager = interface_ref.get().await;
    manager
        .available_changed(interface_ref.signal_emitter())
        .await?;

    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    let manager = interface_ref.get().await;
    manager
        .available_changed(interface_ref.signal_emitter())
        .await
}

pub async fn configuration_changed(
    connection: &Connection,
    changes: luminate::ManagementChangeSet,
) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager>(ROOT)
        .await?;
    let (revision, changes) = management::changes(changes);
    Manager::configuration_changed(interface_ref.signal_emitter(), revision, changes.clone())
        .await?;

    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::configuration_changed(interface_ref.signal_emitter(), revision, changes).await
}

pub async fn scenes_changed(connection: &Connection) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager>(ROOT)
        .await?;
    Manager::scenes_changed(interface_ref.signal_emitter()).await?;

    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::scenes_changed(interface_ref.signal_emitter()).await
}

pub async fn topology_changed(connection: &Connection, devices: Vec<DeviceId>) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::topology_changed(
        interface_ref.signal_emitter(),
        devices
            .into_iter()
            .map(|device| device.as_str().to_owned())
            .collect(),
    )
    .await
}

pub async fn state_changed(connection: &Connection, devices: Vec<DeviceId>) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::state_changed(
        interface_ref.signal_emitter(),
        devices
            .into_iter()
            .map(|device| device.as_str().to_owned())
            .collect(),
    )
    .await
}

pub async fn transitions_changed(
    connection: &Connection,
    transitions: Vec<TransitionId>,
) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::transitions_changed(
        interface_ref.signal_emitter(),
        transitions
            .into_iter()
            .map(|transition| transition.as_str().to_owned())
            .collect(),
    )
    .await
}

pub async fn shm_stream_ended(
    connection: &Connection,
    target: TargetId,
    generation: u32,
) -> zbus::Result<()> {
    let interface_ref = connection
        .object_server()
        .interface::<_, Manager2>(ROOT)
        .await?;
    Manager2::shm_stream_ended(
        interface_ref.signal_emitter(),
        canonical_id(&target),
        generation,
    )
    .await
}

pub async fn remove_object(connection: &Connection, object: &Object) -> zbus::Result<()> {
    let _ = connection
        .object_server()
        .remove::<Target3, _>(object.path.as_str())
        .await?;
    match object.kind {
        Kind::Device => {
            let _ = connection
                .object_server()
                .remove::<Device2, _>(object.path.as_str())
                .await?;
            let _ = connection
                .object_server()
                .remove::<DeviceInterface, _>(object.path.as_str())
                .await?;
        }
        Kind::Surface => {
            let _ = connection
                .object_server()
                .remove::<Surface2, _>(object.path.as_str())
                .await?;
            let _ = connection
                .object_server()
                .remove::<SurfaceInterface, _>(object.path.as_str())
                .await?;
        }
        Kind::Group => {
            let _ = connection
                .object_server()
                .remove::<Group2, _>(object.path.as_str())
                .await?;
            let _ = connection
                .object_server()
                .remove::<GroupInterface, _>(object.path.as_str())
                .await?;
        }
        Kind::Element => {
            let _ = connection
                .object_server()
                .remove::<Element2, _>(object.path.as_str())
                .await?;
            let _ = connection
                .object_server()
                .remove::<ElementInterface, _>(object.path.as_str())
                .await?;
        }
    }
    let _ = connection
        .object_server()
        .remove::<Target, _>(object.path.as_str())
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_unit_tests.rs"]
mod service_unit_tests;
