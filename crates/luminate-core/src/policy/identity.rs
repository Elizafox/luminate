// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::BTreeSet;
use std::time::SystemTime;

use serde::de::Error as _;
use serde::{Deserialize, Serialize};

use super::document::{ValidationError, constraints_match, validate_nonempty, validate_strings};
use crate::collection::CollectionId;
use crate::device::DeviceId;

/// Maximum UTF-8 byte length of one authentication identity component.
pub const MAX_IDENTITY_TEXT_BYTES: usize = 4096;

/// Maximum number of verified groups carried by one principal.
pub const MAX_VERIFIED_GROUPS: usize = 256;

/// A semantic operation evaluated by an access policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    /// Observe topology, state, or events.
    Observe,

    /// Refresh state from hardware.
    Refresh,

    /// Control device state.
    Control,

    /// Change hardware-level configuration.
    HardwareAdministration,

    /// Administer the daemon.
    DaemonAdministration,

    /// Manage plugins and their configuration.
    ManagePlugins,

    /// Create a collection.
    CreateCollection,

    /// Destroy a collection.
    DestroyCollection,

    /// Change collection membership.
    ModifyCollection,

    /// Administer a collection without owning it.
    AdministerCollections,

    /// Read or replace the remote access policy.
    ManagePolicy,

    /// Manage front-end authentication records.
    ManageAuthentication,

    /// Administer the front-end process policy independent of daemon policy.
    AdministerFrontend,

    /// Create a persistent scene.
    CreateScene,

    /// Replace or recapture an owned scene.
    ModifyScene,

    /// Delete an owned scene.
    DestroyScene,

    /// Modify or delete a scene without owning it.
    AdministerScenes,
}

/// One concrete device affected by an authorization request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    /// Stable daemon device identifier.
    pub device_id: DeviceId,

    /// Configured provider instance, when known.
    pub provider_instance: Option<String>,

    /// Whether the device is attached directly to the host.
    pub host_attached: bool,

    /// Every collection whose transitive membership encloses the device.
    pub collections: BTreeSet<CollectionId>,
}

/// Stable identity within an authentication authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PrincipalId {
    authority: String,
    subject: String,
}

impl PrincipalId {
    /// Constructs an identity from exact, opaque strings.
    ///
    /// # Errors
    ///
    /// Returns an error when the authority or subject is empty.
    pub fn new(
        authority: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let authority = authority.into();
        validate_nonempty("principal authority", &authority)?;
        let subject = subject.into();
        validate_nonempty("principal subject", &subject)?;

        Ok(Self { authority, subject })
    }

    /// Returns the identity provider or trust domain.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// Returns the authority-local subject identifier.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

impl<'de> Deserialize<'de> for PrincipalId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WirePrincipalId {
            authority: String,
            subject: String,
        }

        let wire = WirePrincipalId::deserialize(deserializer)?;
        Self::new(wire.authority, wire.subject).map_err(D::Error::custom)
    }
}

/// An authenticated principal and its verified group claims.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Principal {
    id: PrincipalId,
    groups: BTreeSet<String>,
}

/// Kernel-authenticated process identity retained for access control and audit.
///
/// Process IDs are deliberately metadata only; they are not part of the
/// stable identity used for policy or ownership.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Actor {
    /// Unix peer credentials captured at connection admission.
    Unix {
        /// Kernel-supplied user ID.
        uid: u32,
        /// Kernel-supplied primary group ID.
        gid: u32,
        /// Kernel-reported process ID, when available.
        pid: Option<u32>,
    },

    /// Windows named-pipe credentials captured at connection admission.
    Windows {
        /// Canonical security identifier.
        sid: String,
        /// Kernel-reported process ID, when available.
        pid: Option<u32>,
    },
}

impl Actor {
    /// Returns the stable platform identity represented by this actor.
    ///
    /// # Errors
    ///
    /// Returns an error when a platform identity contains an invalid opaque
    /// identifier.
    pub fn principal_id(&self) -> Result<PrincipalId, ValidationError> {
        match self {
            Self::Unix { uid, .. } => PrincipalId::new("unix", uid.to_string()),
            Self::Windows { sid, .. } => PrincipalId::new("windows", sid.clone()),
        }
    }
}

/// Sanitized authentication method associated with a session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "method")]
pub enum AuthenticationSource {
    /// Direct kernel peer authentication.
    Peer,
    /// Daemon-managed bearer token.
    Bearer,
    /// Configured front-end attestation.
    Attestation {
        /// Configured attestation name.
        name: String,
    },
    /// Supervised external authentication provider.
    External {
        /// Configured provider name.
        provider: String,
    },
}

/// Immutable connection context used for every authorization decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedSession {
    /// Kernel-authenticated process identity.
    pub actor: Actor,
    /// Subject authenticated by the selected method.
    pub subject: Principal,
    /// Sanitized authentication method.
    pub source: AuthenticationSource,
    /// Non-secret credential identifier, when one exists.
    pub credential_id: Option<String>,
    /// Current validity lease.
    pub lease: AuthenticationLease,
    /// Voluntary allow-only reduction of this session's authority.
    pub scope: SessionScope,
}

impl AuthenticatedSession {
    /// Returns whether this session can be used at `now`.
    #[must_use]
    pub fn is_valid_at(&self, now: SystemTime) -> bool {
        self.lease.is_valid_at(now)
    }

    /// Returns whether recovery rules may consider this session.
    #[must_use]
    pub fn is_direct_peer(&self) -> bool {
        matches!(self.source, AuthenticationSource::Peer)
    }
}

impl Serialize for Principal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct as _;

        let mut value = serializer.serialize_struct("Principal", 3)?;
        value.serialize_field("authority", self.authority())?;
        value.serialize_field("subject", self.subject())?;
        value.serialize_field("groups", &self.groups)?;
        value.end()
    }
}

impl Principal {
    /// Constructs a principal from exact, opaque identity strings.
    ///
    /// # Errors
    ///
    /// Returns an error when the authority, subject, or any group is empty.
    pub fn new(
        authority: impl Into<String>,
        subject: impl Into<String>,
        groups: impl IntoIterator<Item = String>,
    ) -> Result<Self, ValidationError> {
        let id = PrincipalId::new(authority, subject)?;
        let groups = groups.into_iter().collect::<BTreeSet<_>>();
        if groups.len() > MAX_VERIFIED_GROUPS {
            return Err(ValidationError::new(format!(
                "principal has {} verified groups; maximum is {MAX_VERIFIED_GROUPS}",
                groups.len()
            )));
        }
        validate_strings("principal group", &groups)?;

        Ok(Self { id, groups })
    }

    /// Constructs a principal from its canonical identity and verified groups.
    ///
    /// # Errors
    ///
    /// Returns an error when any verified group is empty.
    pub fn from_id(
        id: PrincipalId,
        groups: impl IntoIterator<Item = String>,
    ) -> Result<Self, ValidationError> {
        let groups = groups.into_iter().collect::<BTreeSet<_>>();
        if groups.len() > MAX_VERIFIED_GROUPS {
            return Err(ValidationError::new(format!(
                "principal has {} verified groups; maximum is {MAX_VERIFIED_GROUPS}",
                groups.len()
            )));
        }
        validate_strings("principal group", &groups)?;
        Ok(Self { id, groups })
    }

    /// Returns the canonical identity.
    #[must_use]
    pub const fn id(&self) -> &PrincipalId {
        &self.id
    }

    /// Returns the identity provider or trust domain.
    #[must_use]
    pub fn authority(&self) -> &str {
        self.id.authority()
    }

    /// Returns the authority-local subject identifier.
    #[must_use]
    pub fn subject(&self) -> &str {
        self.id.subject()
    }

    /// Returns the exact verified group identifiers.
    #[must_use]
    pub const fn groups(&self) -> &BTreeSet<String> {
        &self.groups
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WirePrincipal {
            authority: String,
            subject: String,
            groups: BTreeSet<String>,
        }

        let wire = WirePrincipal::deserialize(deserializer)?;
        Self::new(wire.authority, wire.subject, wire.groups).map_err(D::Error::custom)
    }
}

/// Transitional name retained while the client surfaces are unified.
pub type RemotePrincipal = Principal;

/// The lifetime of verified authentication for a remote principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticationLease {
    /// Optional instant after which the authentication is no longer valid.
    pub expires_at: Option<SystemTime>,
}

impl AuthenticationLease {
    /// Creates a lease that does not expire.
    #[must_use]
    pub const fn unlimited() -> Self {
        Self { expires_at: None }
    }

    /// Creates a lease with a fixed expiry.
    #[must_use]
    pub const fn until(expires_at: SystemTime) -> Self {
        Self {
            expires_at: Some(expires_at),
        }
    }

    /// Returns whether the lease is valid at `now`.
    #[must_use]
    pub fn is_valid_at(self, now: SystemTime) -> bool {
        self.expires_at.is_none_or(|expiry| now < expiry)
    }
}

/// Optional constraints applied to every supplied resource.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceConstraints {
    /// Permitted device identifiers.
    #[serde(default)]
    pub device_ids: BTreeSet<DeviceId>,

    /// Permitted configured provider instances.
    #[serde(default)]
    pub provider_instances: BTreeSet<String>,

    /// Required host-attachment value.
    pub host_attached: Option<bool>,

    /// Collections of which each resource must be enclosed by at least one.
    #[serde(default)]
    pub collections: BTreeSet<CollectionId>,
}

/// One allow-only operation and resource grant in a caller-requested scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeGrant {
    /// Operations admitted by this grant.
    pub operations: BTreeSet<Operation>,

    /// Optional constraints applied to every supplied resource.
    #[serde(default)]
    pub resources: ResourceConstraints,
}

/// A voluntary, allow-only reduction of a session's effective authority.
///
/// Grants are combined by union. The resulting scope is still intersected
/// with actor policy, subject policy, and credential validity; it can never
/// increase authority.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionScope(Vec<ScopeGrant>);

impl SessionScope {
    /// Constructs a validated scope from one or more allow-only grants.
    ///
    /// # Errors
    ///
    /// Returns an error if the scope or any grant has no operations.
    pub fn new(grants: Vec<ScopeGrant>) -> Result<Self, ValidationError> {
        if grants.is_empty() {
            return Err(ValidationError::new("session scope has no grants"));
        }
        if grants.iter().any(|grant| grant.operations.is_empty()) {
            return Err(ValidationError::new(
                "session scope grant has no operations",
            ));
        }
        Ok(Self(grants))
    }

    /// Returns the grants in declaration order.
    #[must_use]
    pub fn grants(&self) -> &[ScopeGrant] {
        &self.0
    }

    /// Returns whether at least one grant admits the complete request.
    #[must_use]
    pub fn allows(&self, operation: Operation, resources: &[Resource]) -> bool {
        self.0.iter().any(|grant| {
            grant.operations.contains(&operation) && constraints_match(&grant.resources, resources)
        })
    }
}

impl<'de> Deserialize<'de> for SessionScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let grants = Vec::<ScopeGrant>::deserialize(deserializer)?;
        Self::new(grants).map_err(D::Error::custom)
    }
}
