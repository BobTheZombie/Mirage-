use crate::manifest::BootModuleManifest;
use std::error::Error;
use std::fmt;

/// Error returned when boot manifest TOML cannot be decoded into policy data.
#[derive(Debug)]
pub struct BootManifestParseError {
    source: toml::de::Error,
}

impl BootManifestParseError {
    pub fn source_error(&self) -> &toml::de::Error {
        &self.source
    }
}

impl fmt::Display for BootManifestParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to parse Mirage boot manifest: {}",
            self.source
        )
    }
}

impl Error for BootManifestParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Parse supervisor boot policy from TOML.
pub fn parse_manifest_toml(input: &str) -> Result<BootModuleManifest, BootManifestParseError> {
    toml::from_str(input).map_err(|source| BootManifestParseError { source })
}
