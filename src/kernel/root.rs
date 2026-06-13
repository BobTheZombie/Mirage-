//! Root filesystem selection parser for whole-device and partition-backed roots.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootFsKind {
    Auto,
    Qfs,
    Ext4,
    Raw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootSpec<'a> {
    pub kind: RootFsKind,
    pub device: Option<&'a str>,
}

pub fn parse_root_spec(input: &str) -> Option<RootSpec<'_>> {
    if input == "auto" || input.is_empty() {
        return Some(RootSpec {
            kind: RootFsKind::Auto,
            device: None,
        });
    }
    if let Some(device) = input.strip_prefix("qfs:") {
        return valid_device(device).then_some(RootSpec {
            kind: RootFsKind::Qfs,
            device: Some(device),
        });
    }
    if let Some(device) = input.strip_prefix("ext4:") {
        return valid_device(device).then_some(RootSpec {
            kind: RootFsKind::Ext4,
            device: Some(device),
        });
    }
    valid_device(input).then_some(RootSpec {
        kind: RootFsKind::Raw,
        device: Some(input),
    })
}

pub fn valid_device(name: &str) -> bool {
    matches!(name, "sata0" | "nvme0n1" | "atapi0")
        || name.starts_with("sata0p")
        || name.starts_with("nvme0n1p")
        || name.starts_with("atapi0p")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_parser_accepts_whole_devices_partitions_and_fs_prefixes() {
        assert_eq!(parse_root_spec("auto").unwrap().kind, RootFsKind::Auto);
        assert_eq!(parse_root_spec("sata0p1").unwrap().device, Some("sata0p1"));
        assert_eq!(
            parse_root_spec("qfs:sata0p1").unwrap().kind,
            RootFsKind::Qfs
        );
        assert_eq!(
            parse_root_spec("ext4:nvme0n1p1").unwrap().kind,
            RootFsKind::Ext4
        );
        assert!(parse_root_spec("linux:/dev/sda1").is_none());
    }
}
