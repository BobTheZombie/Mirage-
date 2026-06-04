use serde::{Deserialize, Serialize};
use std::fmt;

/// Supervisor-consumed manifest describing signed modules available during boot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootModuleManifest {
    pub modules: Vec<BootModule>,
}

impl BootModuleManifest {
    pub fn module(&self, id: &BootModuleId) -> Option<&BootModule> {
        self.modules.iter().find(|module| &module.id == id)
    }
}

/// A single boot module and its supervisor policy metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootModule {
    pub id: BootModuleId,
    pub kind: BootModuleKind,
    pub image: String,
    pub signature: BootModuleSignature,
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default)]
    pub capabilities: Vec<BootModuleCapabilityRequest>,
}

/// Stable manifest identifier for a boot module.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BootModuleId(pub String);

impl BootModuleId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BootModuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Boot module classes understood by the supervisor policy layer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BootModuleKind {
    Supervisor,
    Service,
    DriverService,
    Filesystem,
    Runtime,
    Config,
}

/// Restart behavior requested by a supervised service module.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Never,
    Always,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::Never
    }
}

/// Manifest-carried signature metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootModuleSignature {
    pub value: String,
}

impl BootModuleSignature {
    pub fn is_mock_valid(&self) -> bool {
        self.value == "mock-valid"
    }
}

/// Capability authority requested by a boot module.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BootModuleCapabilityRequest {
    pub object: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub rights: Vec<String>,
}
