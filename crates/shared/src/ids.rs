use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! typed_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);
        impl TryFrom<Uuid> for $name {
            type Error = &'static str;
            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                if value.is_nil() {
                    Err("nil identifier")
                } else {
                    Ok(Self(value))
                }
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
typed_id!(TenantId);
typed_id!(CompanyId);
typed_id!(ActorId);
typed_id!(CommandId);
typed_id!(CorrelationId);
typed_id!(ResourceId);
typed_id!(DossierId);
typed_id!(RevisionId);
typed_id!(EvidenceId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompanyScope {
    pub tenant: TenantId,
    pub company: CompanyId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceScope {
    Tenant(TenantId),
    Company(CompanyScope),
}
impl ResourceScope {
    pub fn tenant(self) -> TenantId {
        match self {
            Self::Tenant(id) => id,
            Self::Company(scope) => scope.tenant,
        }
    }
}
