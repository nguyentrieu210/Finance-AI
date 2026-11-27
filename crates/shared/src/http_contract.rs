//! Wire-level G01 draft. Parsing a version token never authorizes a command.
use crate::{contracts::Version, ids::ResourceId};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionTarget {
    Dossier,
    Organization,
    Membership,
    Policy,
    Rule,
    Form,
    Template,
    Workflow,
    Reference,
    Risk,
    Reconciliation,
}
impl VersionTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dossier => "dossier",
            Self::Organization => "organization",
            Self::Membership => "membership",
            Self::Policy => "policy",
            Self::Rule => "rule",
            Self::Form => "form",
            Self::Template => "template",
            Self::Workflow => "workflow",
            Self::Reference => "reference",
            Self::Risk => "risk",
            Self::Reconciliation => "reconciliation",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "dossier" => Self::Dossier,
            "organization" => Self::Organization,
            "membership" => Self::Membership,
            "policy" => Self::Policy,
            "rule" => Self::Rule,
            "form" => Self::Form,
            "template" => Self::Template,
            "workflow" => Self::Workflow,
            "reference" => Self::Reference,
            "risk" => Self::Risk,
            "reconciliation" => Self::Reconciliation,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionToken {
    pub target: VersionTarget,
    pub resource_id: ResourceId,
    pub version: Version,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderError {
    InvalidSyntax,
    WrongTarget,
    StaleVersion,
}

impl VersionToken {
    pub fn parse(raw: &str) -> Result<Self, HeaderError> {
        let bad = HeaderError::InvalidSyntax;
        // Exactly one strong quoted token. Reject *, W/, lists, whitespace and noncanonical forms.
        let text = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or(bad)?;
        let mut fields = text.split(':');
        if fields.next() != Some("v1") {
            return Err(bad);
        }
        let target = VersionTarget::parse(fields.next().ok_or(bad)?).ok_or(bad)?;
        let resource_id =
            ResourceId::try_from(Uuid::parse_str(fields.next().ok_or(bad)?).map_err(|_| bad)?)
                .map_err(|_| bad)?;
        let version = Version::try_from(fields.next().ok_or(bad)?.parse::<i64>().map_err(|_| bad)?)
            .map_err(|_| bad)?;
        if fields.next().is_some() {
            return Err(bad);
        }
        let token = Self {
            target,
            resource_id,
            version,
        };
        if token.to_string() != raw {
            return Err(bad);
        }
        Ok(token)
    }
    pub fn check(
        self,
        target: VersionTarget,
        id: ResourceId,
        current: Version,
    ) -> Result<(), HeaderError> {
        if self.target != target || self.resource_id != id {
            return Err(HeaderError::WrongTarget);
        }
        if self.version != current {
            return Err(HeaderError::StaleVersion);
        }
        Ok(())
    }
}
impl std::fmt::Display for VersionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "\"v1:{}:{}:{}\"",
            self.target.as_str(),
            self.resource_id,
            i64::from(self.version)
        )
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct IdempotencyKey(String);
impl IdempotencyKey {
    pub fn parse(value: &str) -> Result<Self, HeaderError> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
        {
            return Err(HeaderError::InvalidSyntax);
        }
        Ok(Self(value.to_owned()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for IdempotencyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdempotencyKey([redacted])")
    }
}
