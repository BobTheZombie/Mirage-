use crate::units::{LoadedUnit, RestartPolicy, ServiceUnit, Unit, UnitKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitParseError {
    MissingUnitName,
    UnknownUnitKind(String),
    InvalidLine { line: usize, text: String },
    UnknownSection { line: usize, section: String },
    InvalidRestartPolicy(String),
    MissingExecStart(String),
    ServiceSectionOnNonService(String),
}

pub fn parse_unit(name: &str, source: &str) -> Result<LoadedUnit, UnitParseError> {
    let kind = UnitKind::from_name(name)
        .ok_or_else(|| UnitParseError::UnknownUnitKind(name.to_string()))?;
    let mut section = String::new();
    let mut unit = Unit {
        name: name.to_string(),
        description: String::new(),
        kind: kind.clone(),
        after: Vec::new(),
        before: Vec::new(),
        requires: Vec::new(),
        wants: Vec::new(),
        wanted_by: Vec::new(),
    };
    let mut exec_start = String::new();
    let mut restart = RestartPolicy::No;

    if name.trim().is_empty() {
        return Err(UnitParseError::MissingUnitName);
    }

    for (index, raw) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            match section.as_str() {
                "Unit" | "Service" | "Target" | "Install" => {}
                _ => {
                    return Err(UnitParseError::UnknownSection {
                        line: line_no,
                        section,
                    })
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(UnitParseError::InvalidLine {
                line: line_no,
                text: raw.to_string(),
            });
        };
        let key = key.trim();
        let value = value.trim();
        match (section.as_str(), key) {
            ("Unit", "Name") => { if unit.description.is_empty() { unit.description = value.to_string(); } }
            ("Unit", "Description") => unit.description = value.to_string(),
            ("Unit", "After") => unit.after = split_unit_list(value),
            ("Unit", "Before") => unit.before = split_unit_list(value),
            ("Unit", "Requires") => unit.requires = split_unit_list(value),
            ("Unit", "Wants") => unit.wants = split_unit_list(value),
            ("Service", "ExecStart") => exec_start = value.to_string(),
            ("Service", "Restart") => {
                restart = RestartPolicy::parse(value)
                    .ok_or_else(|| UnitParseError::InvalidRestartPolicy(value.to_string()))?;
            }
            ("Install", "WantedBy") => unit.wanted_by = split_unit_list(value),
            ("", _) => {
                return Err(UnitParseError::InvalidLine {
                    line: line_no,
                    text: raw.to_string(),
                })
            }
            _ => {}
        }
    }

    let service = if kind == UnitKind::Service {
        if exec_start.is_empty() {
            return Err(UnitParseError::MissingExecStart(name.to_string()));
        }
        Some(ServiceUnit {
            exec_start,
            restart,
        })
    } else {
        if !exec_start.is_empty() {
            return Err(UnitParseError::ServiceSectionOnNonService(name.to_string()));
        }
        None
    };

    Ok(LoadedUnit::new(unit, service))
}

fn split_unit_list(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_service() {
        let source = "[Unit]\nDescription=Mirage shell\nAfter=basic.target\n\n[Service]\nExecStart=/bin/msh --login\nRestart=on-failure\n\n[Install]\nWantedBy=multi-user.target\n";
        let loaded = parse_unit("shell.service", source).unwrap();
        assert_eq!(loaded.unit.kind, UnitKind::Service);
        assert_eq!(loaded.unit.after, vec!["basic.target"]);
        assert_eq!(loaded.unit.wanted_by, vec!["multi-user.target"]);
        let service = loaded.service.unwrap();
        assert_eq!(service.exec_start, "/bin/msh --login");
        assert_eq!(service.restart, RestartPolicy::OnFailure);
    }

    #[test]
    fn parse_target() {
        let source = "[Unit]\nDescription=Basic userspace target\nWants=shell.service\n";
        let loaded = parse_unit("basic.target", source).unwrap();
        assert_eq!(loaded.unit.kind, UnitKind::Target);
        assert!(loaded.service.is_none());
        assert_eq!(loaded.unit.wants, vec!["shell.service"]);
    }
}
