// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::identity::{
    AuthenticatedSession, MAX_IDENTITY_TEXT_BYTES, Operation, PrincipalId, RemotePrincipal,
    Resource, ResourceConstraints,
};

/// Maximum cache lifetime a policy decision may request.
pub const MAX_CACHE_HINT: Duration = Duration::from_secs(300);

/// Maximum UTF-8 length of a caller-safe decision reason.
pub const MAX_REASON_BYTES: usize = 512;
/// Stable revision of an activated policy document.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct PolicyRevision(pub u64);

/// Stable identifier of a policy rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RuleId(String);

impl RuleId {
    /// Creates a rule identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is empty.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        validate_nonempty("rule ID", &value)?;
        Ok(Self(value))
    }

    /// Returns the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for RuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// A rule's effect when it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuleEffect {
    /// Permit the operation.
    Allow,

    /// Deny the operation.
    Deny,
}

/// Independent resource ceilings applied to connections and live work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitCeilings {
    /// Maximum live connections.
    pub connections: Option<u32>,

    /// Maximum simultaneous event subscriptions.
    pub subscriptions: Option<u32>,
}

impl LimitCeilings {
    /// Intersects two matching ceilings by selecting the tighter value in
    /// every dimension. An absent value means that source adds no ceiling.
    #[must_use]
    pub fn most_restrictive(self, other: Self) -> Self {
        Self {
            connections: least_option(self.connections, other.connections),
            subscriptions: least_option(self.subscriptions, other.subscriptions),
        }
    }
}

/// One allow or deny rule belonging to a role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Stable audit identifier.
    pub id: RuleId,

    /// Whether a match allows or denies.
    pub effect: RuleEffect,

    /// Operations matched by this rule.
    pub operations: BTreeSet<Operation>,

    /// Optional constraints applied to all resources.
    #[serde(default)]
    pub resources: ResourceConstraints,

    /// Safe diagnostic suitable for callers and audit records.
    pub reason: Option<String>,

    /// Requested decision cache lifetime, clamped to [`MAX_CACHE_HINT`].
    pub cache_hint: Option<Duration>,
}

/// A named role with inherited roles and rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    /// Parent role names.
    #[serde(default)]
    pub parents: BTreeSet<String>,

    /// Rules evaluated for this role.
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// Built-in materialized role presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    /// Read-only topology and state access.
    Viewer,
    /// Normal lighting control and owned-object access.
    User,
    /// User access plus daemon and hardware administration.
    Operator,
    /// Full policy and authentication administration.
    Administrator,
}

impl Preset {
    /// Returns the stable role name used when materializing this preset.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::User => "user",
            Self::Operator => "operator",
            Self::Administrator => "administrator",
        }
    }

    /// Materializes this preset as an ordinary role.
    #[must_use]
    pub fn role(self) -> Role {
        let operations = match self {
            Self::Viewer => BTreeSet::from([Operation::Observe]),
            Self::User => BTreeSet::from([
                Operation::Observe,
                Operation::Refresh,
                Operation::Control,
                Operation::CreateCollection,
                Operation::DestroyCollection,
                Operation::ModifyCollection,
                Operation::CreateScene,
                Operation::ModifyScene,
                Operation::DestroyScene,
            ]),
            Self::Operator => BTreeSet::from([
                Operation::Observe,
                Operation::Refresh,
                Operation::Control,
                Operation::HardwareAdministration,
                Operation::DaemonAdministration,
                Operation::ManagePlugins,
                Operation::CreateCollection,
                Operation::DestroyCollection,
                Operation::ModifyCollection,
                Operation::AdministerCollections,
                Operation::CreateScene,
                Operation::ModifyScene,
                Operation::DestroyScene,
                Operation::AdministerScenes,
            ]),
            Self::Administrator => all_operations(),
        };
        Role {
            parents: BTreeSet::new(),
            rules: vec![Rule {
                id: RuleId(format!("preset-{}", self.name())),
                effect: RuleEffect::Allow,
                operations,
                resources: ResourceConstraints::default(),
                reason: None,
                cache_hint: None,
            }],
        }
    }
}

/// Returns every operation that a built-in administrator can perform.
fn all_operations() -> BTreeSet<Operation> {
    [
        Operation::Observe,
        Operation::Refresh,
        Operation::Control,
        Operation::HardwareAdministration,
        Operation::DaemonAdministration,
        Operation::ManagePlugins,
        Operation::CreateCollection,
        Operation::DestroyCollection,
        Operation::ModifyCollection,
        Operation::AdministerCollections,
        Operation::ManagePolicy,
        Operation::ManageAuthentication,
        Operation::AdministerFrontend,
        Operation::CreateScene,
        Operation::ModifyScene,
        Operation::DestroyScene,
        Operation::AdministerScenes,
    ]
    .into_iter()
    .collect()
}

/// Materializes selected built-in roles into a policy source.
#[must_use]
pub fn materialize_presets(
    revision: PolicyRevision,
    presets: impl IntoIterator<Item = Preset>,
) -> PolicyDocumentSource {
    PolicyDocumentSource {
        revision,
        roles: presets
            .into_iter()
            .map(|preset| (preset.name().to_owned(), preset.role()))
            .collect(),
        bindings: Vec::new(),
    }
}

/// An exact authority binding from subjects or verified groups to roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Exact authority matched by this binding.
    pub authority: String,

    /// Exact subjects matched within the authority.
    #[serde(default)]
    pub subjects: BTreeSet<String>,

    /// Exact verified groups matched within the authority.
    #[serde(default)]
    pub groups: BTreeSet<String>,

    /// Roles granted by this binding.
    pub roles: BTreeSet<String>,
}

/// Unvalidated serialized representation of a policy document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDocumentSource {
    /// Revision used for compare-and-swap replacement and cache invalidation.
    pub revision: PolicyRevision,

    /// Roles keyed by stable name.
    pub roles: BTreeMap<String, Role>,

    /// Principal-to-role bindings.
    pub bindings: Vec<Binding>,
}

/// A completely validated policy document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PolicyDocument(PolicyDocumentSource);

impl PolicyDocument {
    /// Validates and constructs a policy document.
    ///
    /// Validation checks names and identifiers, references, role cycles,
    /// binding selectors, rule uniqueness, and bounded diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic for the first invalid policy invariant.
    pub fn new(mut source: PolicyDocumentSource) -> Result<Self, ValidationError> {
        validate_document(&source)?;
        for role in source.roles.values_mut() {
            role.rules.sort_by(|left, right| left.id.cmp(&right.id));
        }
        source.bindings.sort_by(|left, right| {
            (&left.authority, &left.subjects, &left.groups, &left.roles).cmp(&(
                &right.authority,
                &right.subjects,
                &right.groups,
                &right.roles,
            ))
        });
        Ok(Self(source))
    }

    /// Returns the document revision.
    #[must_use]
    pub const fn revision(&self) -> PolicyRevision {
        self.0.revision
    }

    /// Returns the roles keyed by name.
    #[must_use]
    pub const fn roles(&self) -> &BTreeMap<String, Role> {
        &self.0.roles
    }

    /// Returns the principal bindings.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.0.bindings
    }

    /// Returns the deterministic serialized source.
    #[must_use]
    pub const fn source(&self) -> &PolicyDocumentSource {
        &self.0
    }

    /// Evaluates one semantic request using deny-overrides-allow semantics.
    ///
    /// A constrained rule matches only when at least one resource is supplied
    /// and every supplied resource satisfies its constraints. The default is
    /// deny when no rule matches.
    #[must_use]
    pub fn evaluate(
        &self,
        principal: &RemotePrincipal,
        operation: Operation,
        resources: &[Resource],
    ) -> AuthorizationDecision {
        let roles = self.effective_roles(principal);
        let mut allow = None;
        let mut deny = None;

        for role_name in &roles {
            let Some(role) = self.0.roles.get(role_name) else {
                continue;
            };
            for rule in &role.rules {
                if rule_matches(rule, operation, resources) {
                    let candidate = rule_decision(rule, self.revision());
                    match rule.effect {
                        RuleEffect::Allow => select_decision(&mut allow, candidate),
                        RuleEffect::Deny => select_decision(&mut deny, candidate),
                    }
                }
            }
        }

        deny.or(allow).unwrap_or_else(|| AuthorizationDecision {
            outcome: DecisionOutcome::Deny,
            reason: Some("access denied by default".to_owned()),
            audit_rule: None,
            cache_hint: None,
            revision: self.revision(),
        })
    }

    /// Evaluates a request against the session's lease, allow-only scope, and
    /// subject policy in that order.
    #[must_use]
    pub fn evaluate_session(
        &self,
        session: &AuthenticatedSession,
        operation: Operation,
        resources: &[Resource],
        now: SystemTime,
    ) -> AuthorizationDecision {
        if !session.is_valid_at(now) {
            return AuthorizationDecision::denied(self.revision(), "authentication lease expired");
        }
        if !session.scope.allows(operation, resources) {
            return AuthorizationDecision::denied(self.revision(), "session scope denied access");
        }
        self.evaluate(&session.subject, operation, resources)
    }

    fn effective_roles(&self, principal: &RemotePrincipal) -> BTreeSet<String> {
        let mut roles = self
            .0
            .bindings
            .iter()
            .filter(|binding| binding_matches(binding, principal))
            .flat_map(|binding| binding.roles.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut pending = roles.iter().cloned().collect::<Vec<_>>();

        while let Some(name) = pending.pop() {
            let Some(role) = self.0.roles.get(&name) else {
                continue;
            };
            for parent in &role.parents {
                if roles.insert(parent.clone()) {
                    pending.push(parent.clone());
                }
            }
        }

        roles
    }
}

impl TryFrom<PolicyDocumentSource> for PolicyDocument {
    type Error = ValidationError;

    fn try_from(source: PolicyDocumentSource) -> Result<Self, Self::Error> {
        Self::new(source)
    }
}

impl<'de> Deserialize<'de> for PolicyDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let source = PolicyDocumentSource::deserialize(deserializer)?;
        Self::new(source).map_err(D::Error::custom)
    }
}

/// A policy validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Whether an authorization request is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionOutcome {
    /// The request is permitted.
    Allow,

    /// The request is denied.
    Deny,
}

/// Complete result of evaluating an authorization request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationDecision {
    /// Allow or deny outcome.
    pub outcome: DecisionOutcome,

    /// Validated caller-safe diagnostic.
    pub reason: Option<String>,

    /// Stable identifier of the matching rule, or none for default denial.
    pub audit_rule: Option<RuleId>,

    /// Cache lifetime clamped to [`MAX_CACHE_HINT`].
    pub cache_hint: Option<Duration>,

    /// Policy revision under which the decision was made.
    pub revision: PolicyRevision,
}

impl AuthorizationDecision {
    fn denied(revision: PolicyRevision, reason: &'static str) -> Self {
        Self {
            outcome: DecisionOutcome::Deny,
            reason: Some(reason.to_owned()),
            audit_rule: None,
            cache_hint: None,
            revision,
        }
    }

    /// Returns whether the request is allowed.
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self.outcome, DecisionOutcome::Allow)
    }
}

/// Exact principals allowed to perform policy and authentication recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    /// Recovery principals. Group membership never grants recovery access.
    pub principals: BTreeSet<PrincipalId>,
}

impl RecoveryPolicy {
    /// Creates an exact-principal recovery policy.
    #[must_use]
    pub const fn new(principals: BTreeSet<PrincipalId>) -> Self {
        Self { principals }
    }

    /// Returns whether the session may recover policy or authentication.
    ///
    /// Recovery is intentionally restricted to direct peer authentication;
    /// a bearer token, attestation, or provider cannot bootstrap recovery.
    #[must_use]
    pub fn allows(&self, session: &AuthenticatedSession, operation: Operation) -> bool {
        matches!(
            operation,
            Operation::ManagePolicy | Operation::ManageAuthentication
        ) && session.is_direct_peer()
            && self.principals.contains(session.subject.id())
    }
}

fn validate_document(source: &PolicyDocumentSource) -> Result<(), ValidationError> {
    let mut rule_ids = BTreeSet::new();
    for (name, role) in &source.roles {
        validate_nonempty("role name", name)?;
        validate_strings("parent role name", &role.parents)?;
        for parent in &role.parents {
            if !source.roles.contains_key(parent) {
                return Err(ValidationError::new(format!(
                    "role {name:?} refers to missing parent {parent:?}"
                )));
            }
        }
        for rule in &role.rules {
            validate_nonempty("rule ID", rule.id.as_str())?;
            if rule.operations.is_empty() {
                return Err(ValidationError::new(format!(
                    "rule {:?} matches no operations",
                    rule.id.as_str()
                )));
            }
            if !rule_ids.insert(rule.id.clone()) {
                return Err(ValidationError::new(format!(
                    "duplicate rule ID {:?}",
                    rule.id.as_str()
                )));
            }
            validate_strings(
                "provider instance constraint",
                &rule.resources.provider_instances,
            )?;
            if rule.reason.as_ref().is_some_and(String::is_empty) {
                return Err(ValidationError::new(format!(
                    "rule {:?} has an empty reason",
                    rule.id.as_str()
                )));
            }
            if rule
                .reason
                .as_ref()
                .is_some_and(|reason| reason.len() > MAX_REASON_BYTES)
            {
                return Err(ValidationError::new(format!(
                    "rule {:?} reason exceeds {MAX_REASON_BYTES} bytes",
                    rule.id.as_str()
                )));
            }
        }
    }

    validate_acyclic_roles(&source.roles)?;

    for binding in &source.bindings {
        validate_nonempty("binding authority", &binding.authority)?;
        validate_strings("binding subject", &binding.subjects)?;
        validate_strings("binding group", &binding.groups)?;
        if binding.subjects.is_empty() && binding.groups.is_empty() {
            return Err(ValidationError::new(
                "binding must contain at least one subject or group",
            ));
        }
        if binding.roles.is_empty() {
            return Err(ValidationError::new("binding must grant at least one role"));
        }
        validate_strings("binding role", &binding.roles)?;
        for role in &binding.roles {
            if !source.roles.contains_key(role) {
                return Err(ValidationError::new(format!(
                    "binding refers to missing role {role:?}"
                )));
            }
        }
    }

    Ok(())
}

fn binding_matches(binding: &Binding, principal: &RemotePrincipal) -> bool {
    binding.authority == principal.authority()
        && (binding.subjects.contains(principal.subject())
            || !binding.groups.is_disjoint(principal.groups()))
}

fn rule_matches(rule: &Rule, operation: Operation, resources: &[Resource]) -> bool {
    rule.operations.contains(&operation) && constraints_match(&rule.resources, resources)
}

pub(super) fn constraints_match(constraints: &ResourceConstraints, resources: &[Resource]) -> bool {
    if constraints_are_empty(constraints) {
        true
    } else {
        !resources.is_empty()
            && resources
                .iter()
                .all(|resource| resource_matches(constraints, resource))
    }
}

fn constraints_are_empty(constraints: &ResourceConstraints) -> bool {
    constraints.device_ids.is_empty()
        && constraints.provider_instances.is_empty()
        && constraints.host_attached.is_none()
        && constraints.collections.is_empty()
}

fn resource_matches(constraints: &ResourceConstraints, resource: &Resource) -> bool {
    (constraints.device_ids.is_empty() || constraints.device_ids.contains(&resource.device_id))
        && (constraints.provider_instances.is_empty()
            || resource
                .provider_instance
                .as_ref()
                .is_some_and(|instance| constraints.provider_instances.contains(instance)))
        && constraints
            .host_attached
            .is_none_or(|required| resource.host_attached == required)
        && (constraints.collections.is_empty()
            || !constraints.collections.is_disjoint(&resource.collections))
}

fn rule_decision(rule: &Rule, revision: PolicyRevision) -> AuthorizationDecision {
    AuthorizationDecision {
        outcome: match rule.effect {
            RuleEffect::Allow => DecisionOutcome::Allow,
            RuleEffect::Deny => DecisionOutcome::Deny,
        },
        reason: rule.reason.clone(),
        audit_rule: Some(rule.id.clone()),
        cache_hint: rule.cache_hint.map(|hint| hint.min(MAX_CACHE_HINT)),
        revision,
    }
}

fn select_decision(selected: &mut Option<AuthorizationDecision>, candidate: AuthorizationDecision) {
    let replace = selected.as_ref().is_none_or(|current| {
        candidate.audit_rule.as_ref().map(RuleId::as_str)
            < current.audit_rule.as_ref().map(RuleId::as_str)
    });
    if replace {
        *selected = Some(candidate);
    }
}

fn validate_acyclic_roles(roles: &BTreeMap<String, Role>) -> Result<(), ValidationError> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Visit {
        Active,
        Complete,
    }

    fn visit<'a>(
        name: &'a str,
        roles: &'a BTreeMap<String, Role>,
        visits: &mut BTreeMap<&'a str, Visit>,
    ) -> Result<(), ValidationError> {
        match visits.get(name) {
            Some(Visit::Active) => {
                return Err(ValidationError::new(format!(
                    "role inheritance contains a cycle at {name:?}"
                )));
            }
            Some(Visit::Complete) => return Ok(()),
            None => {}
        }

        visits.insert(name, Visit::Active);
        if let Some(role) = roles.get(name) {
            for parent in &role.parents {
                visit(parent, roles, visits)?;
            }
        }
        visits.insert(name, Visit::Complete);
        Ok(())
    }

    let mut visits = BTreeMap::new();
    for name in roles.keys() {
        visit(name, roles, &mut visits)?;
    }
    Ok(())
}

pub(super) fn validate_nonempty(kind: &str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        Err(ValidationError::new(format!("{kind} must not be empty")))
    } else if value.len() > MAX_IDENTITY_TEXT_BYTES {
        Err(ValidationError::new(format!(
            "{kind} is {} bytes; maximum is {MAX_IDENTITY_TEXT_BYTES}",
            value.len()
        )))
    } else {
        Ok(())
    }
}

pub(super) fn validate_strings(
    kind: &str,
    values: &BTreeSet<String>,
) -> Result<(), ValidationError> {
    for value in values {
        validate_nonempty(kind, value)?;
    }
    Ok(())
}

fn least_option<T: Ord>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}
