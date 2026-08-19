// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! D-Bus compatibility service entry point and runtime setup.

mod access_policy;
mod auth;
mod capability;
mod collection;
mod control;
mod convert;
mod effect_request;
mod error;
mod frame;
mod management;
mod model;
#[cfg(test)]
mod parity_tests;
mod path;
mod scene;
mod service;
mod setup;
mod state;
mod topology;
mod transition;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::io::{self, Write as _};
use std::mem;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use clap::Parser;
use luminate::{Client, Event};
use luminate_platform::terminal::{TerminalSafeFields, escape};
use tokio::signal::ctrl_c;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;
use tracing::{info, warn};
use zbus::Connection;
use zbus::fdo::ObjectManager;

use crate::model::objects;
use crate::path::ROOT;
use crate::service::{Manager, Manager2, Shared};

#[derive(Debug, Parser)]
#[command(about = "Optional system-bus adapter for Luminate")]
struct Args {
    #[arg(long)]
    socket_path: Option<PathBuf>,

    #[arg(long, default_value = "luminate")]
    authorization_group: String,

    #[arg(long, default_value_t = false)]
    polkit: bool,

    /// Overrides the `/etc/group` path consulted for the authorization group's GID.
    #[arg(long, default_value = "/etc/group", hide = true)]
    group_file: PathBuf,

    /// Overrides the `/proc` root consulted for a caller's process groups.
    #[arg(long, default_value = "/proc", hide = true)]
    proc_root: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run_main().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let exit_code = error
                .downcast_ref::<clap::Error>()
                .map_or(1, clap::Error::exit_code);
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                escape(&format!("error: {error}"))
            );
            u8::try_from(exit_code).map_or(ExitCode::FAILURE, ExitCode::from)
        }
    }
}

async fn run_main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .fmt_fields(TerminalSafeFields)
        .init();

    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) if error.use_stderr() => return Err(error.into()),
        Err(error) => {
            error.print()?;
            return Ok(());
        }
    };

    let gid = auth::group_gid(&args.group_file, &args.authorization_group)?;

    let shared = Arc::new(Shared {
        client: RwLock::new(None),
        socket_path: args.socket_path.clone(),
        attestation_sequence: AtomicU64::new(1),
        attested_clients: Mutex::new(HashMap::new()),
        objects: RwLock::new(BTreeMap::new()),
        required_gid: gid,
        polkit: args.polkit,
        proc_root: args.proc_root,
    });

    let connection = Connection::system().await?;

    let _ = connection
        .object_server()
        .at(ROOT, Manager::new(Arc::clone(&shared)))
        .await?;

    let _ = connection
        .object_server()
        .at(ROOT, Manager2::new(Arc::clone(&shared)))
        .await?;

    let _ = connection.object_server().at(ROOT, ObjectManager).await?;

    connection.request_name("org.luminate.Luminate1").await?;

    info!("D-Bus service is ready");

    tokio::select! {
        () = run(connection, shared, args.socket_path) => {}
        result = ctrl_c() => result?,
    }

    Ok(())
}

#[allow(
    clippy::infinite_loop,
    reason = "The companion service retains its bus name and reconnects for its lifetime"
)]
async fn run(connection: Connection, shared: Arc<Shared>, socket_path: Option<PathBuf>) {
    let mut delay = Duration::from_millis(250);
    loop {
        let connected = match &socket_path {
            Some(path) => Client::connect_path(path).await,
            None => Client::connect().await,
        };
        match connected {
            Ok(client) => {
                let client = Arc::new(client);
                match client.subscribe_with_baseline().await {
                    Ok((mut subscription, baseline)) => {
                        if let Err(error) =
                            replace_topology(&connection, &shared, objects(&baseline)).await
                        {
                            warn!(%error, "could not export topology");
                        }
                        *shared.client.write().await = Some(Arc::clone(&client));
                        if let Err(error) = service::available_changed(&connection).await {
                            warn!(%error, "could not signal daemon availability");
                        }
                        delay = Duration::from_millis(250);
                        loop {
                            match subscription.next_event().await {
                                Ok(event) => {
                                    if let Err(error) =
                                        handle_event(&connection, &shared, &client, event).await
                                    {
                                        warn!(%error, "could not apply daemon event");
                                        break;
                                    }
                                }
                                Err(error) => {
                                    warn!(%error, "daemon event connection lost");
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => warn!(%error, "could not establish race-free subscription"),
                }
            }
            Err(error) => warn!(%error, "luminated is unavailable; retrying"),
        }
        *shared.client.write().await = None;
        if let Err(error) = service::available_changed(&connection).await {
            warn!(%error, "could not signal daemon unavailability");
        }
        if let Err(error) = replace_topology(&connection, &shared, BTreeMap::new()).await {
            warn!(%error, "could not withdraw stale topology");
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

async fn handle_event(
    connection: &Connection,
    shared: &Arc<Shared>,
    client: &Arc<Client>,
    event: Event,
) -> Result<(), Box<dyn Error>> {
    match event {
        Event::ResyncRequired => {
            let devices = client.list_devices().await?;
            replace_topology(connection, shared, objects(&devices)).await?;
            service::topology_changed(connection, Vec::new()).await?;
            service::state_changed(connection, Vec::new()).await?;
        }
        Event::TopologyChanged { devices: dirty } => {
            let devices = client.list_devices().await?;
            replace_topology(connection, shared, objects(&devices)).await?;
            service::topology_changed(connection, dirty).await?;
        }
        Event::StateChanged { devices } => service::state_changed(connection, devices).await?,
        Event::ConfigurationChanged { changes } => {
            service::configuration_changed(connection, changes).await?;
        }
        Event::ScenesChanged => service::scenes_changed(connection).await?,
        Event::TransitionsChanged { transitions } => {
            service::transitions_changed(connection, transitions).await?;
        }
        Event::ShmStreamEnded { target, generation } => {
            service::shm_stream_ended(connection, target, generation).await?;
        }
    }
    Ok(())
}

async fn replace_topology(
    connection: &Connection,
    shared: &Arc<Shared>,
    next: BTreeMap<String, model::Object>,
) -> zbus::Result<()> {
    let previous = { mem::take(&mut *shared.objects.write().await) };
    for (path, object) in &previous {
        if !next.contains_key(path) {
            service::remove_object(connection, object).await?;
        }
    }
    for (path, object) in &next {
        match previous.get(path) {
            None => service::add_object(connection, Arc::clone(shared), object.clone()).await?,
            Some(old) if old != object => {
                service::update_object(connection, object.clone()).await?;
            }
            Some(_) => {}
        }
    }
    *shared.objects.write().await = next;
    Ok(())
}
