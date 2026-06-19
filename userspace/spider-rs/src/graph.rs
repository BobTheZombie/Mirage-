use crate::units::LoadedUnit;
#[cfg(target_os = "none")]
use alloc::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    string::{String, ToString},
    vec,
    vec::Vec,
};
#[cfg(not(target_os = "none"))]
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupPlan {
    pub target: String,
    pub order: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DependencyError {
    MissingUnit(String),
    Cycle { units: Vec<String> },
}

pub fn resolve_startup_order(
    units: &BTreeMap<String, LoadedUnit>,
    target: &str,
) -> Result<StartupPlan, DependencyError> {
    if !units.contains_key(target) {
        return Err(DependencyError::MissingUnit(target.to_string()));
    }

    let included = collect_units(units, target)?;
    let mut edges: BTreeMap<String, BTreeSet<String>> = included
        .iter()
        .map(|name| (name.clone(), BTreeSet::new()))
        .collect();
    let mut indegree: BTreeMap<String, usize> =
        included.iter().map(|name| (name.clone(), 0)).collect();

    for name in &included {
        let unit = &units[name].unit;
        for dep in unit.requires.iter().chain(unit.wants.iter()) {
            if included.contains(dep) {
                add_edge(dep, name, &mut edges, &mut indegree);
            }
        }
        for after in &unit.after {
            if included.contains(after) {
                add_edge(after, name, &mut edges, &mut indegree);
            }
        }
        for before in &unit.before {
            if included.contains(before) {
                add_edge(name, before, &mut edges, &mut indegree);
            }
        }
    }

    let mut ready: VecDeque<String> = indegree
        .iter()
        .filter_map(|(name, count)| (*count == 0).then(|| name.clone()))
        .collect();
    let mut order = Vec::new();

    while let Some(name) = ready.pop_front() {
        order.push(name.clone());
        for dependent in edges.get(&name).into_iter().flatten() {
            let count = indegree.get_mut(dependent).expect("known dependent");
            *count -= 1;
            if *count == 0 {
                ready.push_back(dependent.clone());
            }
        }
    }

    if order.len() != included.len() {
        let cycle_units = indegree
            .into_iter()
            .filter_map(|(name, count)| (count > 0).then_some(name))
            .collect();
        return Err(DependencyError::Cycle { units: cycle_units });
    }

    Ok(StartupPlan {
        target: target.to_string(),
        order,
    })
}

fn collect_units(
    units: &BTreeMap<String, LoadedUnit>,
    target: &str,
) -> Result<BTreeSet<String>, DependencyError> {
    let mut included = BTreeSet::new();
    let mut stack = vec![target.to_string()];

    while let Some(name) = stack.pop() {
        if !included.insert(name.clone()) {
            continue;
        }
        let unit = units
            .get(&name)
            .ok_or_else(|| DependencyError::MissingUnit(name.clone()))?;
        for dep in unit.unit.requires.iter().chain(unit.unit.wants.iter()) {
            if !units.contains_key(dep) {
                return Err(DependencyError::MissingUnit(dep.clone()));
            }
            stack.push(dep.clone());
        }
        for candidate in units.values() {
            if candidate
                .unit
                .wanted_by
                .iter()
                .any(|wanted| wanted == &name)
            {
                stack.push(candidate.unit.name.clone());
            }
        }
    }

    Ok(included)
}

fn add_edge(
    before: &str,
    after: &str,
    edges: &mut BTreeMap<String, BTreeSet<String>>,
    indegree: &mut BTreeMap<String, usize>,
) {
    if edges
        .get_mut(before)
        .expect("known before unit")
        .insert(after.to_string())
    {
        *indegree.get_mut(after).expect("known after unit") += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_unit;

    fn map(items: &[(&str, &str)]) -> BTreeMap<String, LoadedUnit> {
        items
            .iter()
            .map(|(name, source)| ((*name).to_string(), parse_unit(name, source).unwrap()))
            .collect()
    }

    #[test]
    fn dependency_ordering_is_deterministic() {
        let units = map(&[
            ("default.target", "[Unit]\nWants=multi-user.target\nAfter=multi-user.target\n"),
            ("multi-user.target", "[Unit]\nRequires=basic.target\nAfter=basic.target\n"),
            ("basic.target", "[Unit]\nDescription=Basic\n"),
            ("shell.service", "[Unit]\nAfter=basic.target\n[Service]\nExecStart=/bin/msh\n[Install]\nWantedBy=multi-user.target\n"),
        ]);
        let plan = resolve_startup_order(&units, "default.target").unwrap();
        assert_eq!(plan.order[0], "basic.target");
        assert!(
            plan.order
                .iter()
                .position(|name| name == "multi-user.target")
                .unwrap()
                < plan
                    .order
                    .iter()
                    .position(|name| name == "default.target")
                    .unwrap()
        );
    }

    #[test]
    fn detects_cycles_clearly() {
        let units = map(&[
            ("default.target", "[Unit]\nWants=a.service b.service\n"),
            (
                "a.service",
                "[Unit]\nAfter=b.service\n[Service]\nExecStart=/bin/a\n",
            ),
            (
                "b.service",
                "[Unit]\nAfter=a.service\n[Service]\nExecStart=/bin/b\n",
            ),
        ]);
        let err = resolve_startup_order(&units, "default.target").unwrap_err();
        assert!(matches!(err, DependencyError::Cycle { .. }));
    }

    #[test]
    fn default_target_resolution() {
        let units = map(&[
            (
                "default.target",
                "[Unit]\nWants=multi-user.target\nAfter=multi-user.target\n",
            ),
            (
                "multi-user.target",
                "[Unit]\nRequires=basic.target\nAfter=basic.target\n",
            ),
            ("basic.target", "[Unit]\nDescription=Basic\n"),
        ]);
        let plan = resolve_startup_order(&units, "default.target").unwrap();
        assert_eq!(
            plan.order,
            vec!["basic.target", "multi-user.target", "default.target"]
        );
    }
}
