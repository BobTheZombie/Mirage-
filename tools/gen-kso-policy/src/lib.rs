use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const STARTUPS: &[&str] = &[
    "seed_rs",
    "bootinfo",
    "kernel_main",
    "kernel_constructed",
    "architecture",
    "serial",
    "gdt",
    "memory_map",
    "physical_allocator",
    "kernel_mapper",
    "paging",
    "heap",
    "framebuffer",
    "idt",
    "pic",
    "interrupts",
    "pci",
    "block_layer",
    "ahci",
    "nvme",
    "i8042",
    "ps2_keyboard",
    "xhci",
    "usb_core",
    "usb_hid",
    "usb_keyboard",
    "input",
    "boot_runtime",
    "rootfs",
    "supervisor",
    "mtss_pid0",
    "userspace_loader",
    "pid1_handoff",
    "idleloop",
];
const CAPABILITIES: &[&str] = &[
    "boot.seed",
    "boot.info",
    "kernel.main",
    "kernel.constructed",
    "arch.ready",
    "serial.console",
    "gdt.ready",
    "memory.map",
    "memory.physical_allocator",
    "memory.kernel_mapper",
    "paging.ready",
    "heap.ready",
    "framebuffer.ready",
    "idt.ready",
    "pic.ready",
    "interrupts.ready",
    "pci.bus",
    "block.layer",
    "storage.ahci",
    "storage.nvme",
    "i8042.controller",
    "input.keyboard.ps2",
    "usb.xhci",
    "usb.core",
    "usb.hid",
    "input.keyboard.usb",
    "input.ready",
    "boot.runtime.validated",
    "boot.spider_rs_image",
    "rootfs.mounted",
    "supervisor.ready",
    "supervisor.launch_grants",
    "mtss.pid0",
    "mtss.core",
    "mtss.scheduler",
    "mtss.cooperative",
    "mtss.timer",
    "mtss.preemption",
    "userspace.loader",
    "pid1.handoff",
    "idleloop.ready",
];

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyToml {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub startup: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub optional_policy: Option<String>,
    #[serde(default)]
    pub allow_cooperative_mtss: bool,
    #[serde(default)]
    pub require_preemption: bool,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub wants: Vec<String>,
    #[serde(default)]
    pub wants_capabilities: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub optional_provides: Vec<String>,
}

pub fn load_dir(dir: impl AsRef<Path>) -> Result<Vec<PolicyToml>, String> {
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(dir.as_ref()).map_err(|e| format!("read {}: {e}", dir.as_ref().display()))?
    {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            paths.push(path);
        }
    }
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let text =
            fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let policy: PolicyToml =
            toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        out.push(policy);
    }
    Ok(out)
}

pub fn validate(nodes: &[PolicyToml]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for n in nodes {
        if !ids.insert(n.id.as_str()) {
            return Err(format!("duplicate node id `{}`", n.id));
        }
    }
    let ids: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let caps: BTreeSet<&str> = CAPABILITIES.iter().copied().collect();
    let starts: BTreeSet<&str> = STARTUPS.iter().copied().collect();
    for n in nodes {
        for (field, refs) in [
            ("after", &n.after),
            ("before", &n.before),
            ("conflicts", &n.conflicts),
            ("wants", &n.wants),
        ] {
            for r in refs {
                if !ids.contains(r.as_str()) {
                    return Err(format!(
                        "node `{}` has unknown {field} reference `{r}`",
                        n.id
                    ));
                }
            }
        }
        for c in n
            .requires
            .iter()
            .chain(n.wants_capabilities.iter())
            .chain(n.provides.iter())
            .chain(n.optional_provides.iter())
        {
            if !caps.contains(c.as_str()) {
                return Err(format!(
                    "node `{}` references unknown capability `{c}`",
                    n.id
                ));
            }
        }
        if n.requires.iter().any(|c| n.provides.contains(c)) {
            return Err(format!(
                "node `{}` requires its own provided capability",
                n.id
            ));
        }
        match &n.startup {
            Some(s) if !starts.contains(s.as_str()) => {
                return Err(format!(
                    "node `{}` references unknown startup function `{s}`",
                    n.id
                ))
            }
            None if n.required => {
                return Err(format!("required node `{}` has no startup function", n.id))
            }
            _ => {}
        }
        if !n.required && n.failure.as_deref() == Some("Fatal") && n.optional_policy.is_none() {
            return Err(format!(
                "optional fatal node `{}` lacks explicit optional_policy",
                n.id
            ));
        }
    }
    reject_cycles(nodes)
}

fn reject_cycles(nodes: &[PolicyToml]) -> Result<(), String> {
    let map: BTreeMap<&str, &PolicyToml> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    fn visit<'a>(
        id: &'a str,
        map: &BTreeMap<&'a str, &'a PolicyToml>,
        temp: &mut BTreeSet<&'a str>,
        perm: &mut BTreeSet<&'a str>,
    ) -> Result<(), String> {
        if perm.contains(id) {
            return Ok(());
        }
        if !temp.insert(id) {
            return Err(format!("dependency cycle involving `{id}`"));
        }
        let node = map[id];
        for dep in effective_after_refs(id, node, map) {
            visit(dep, map, temp, perm)?;
        }
        temp.remove(id);
        perm.insert(id);
        Ok(())
    }
    let mut temp = BTreeSet::new();
    let mut perm = BTreeSet::new();
    for id in map.keys().copied() {
        visit(id, &map, &mut temp, &mut perm)?;
    }
    Ok(())
}

fn effective_after_refs<'a>(
    id: &str,
    node: &'a PolicyToml,
    map: &BTreeMap<&'a str, &'a PolicyToml>,
) -> Vec<&'a str> {
    let mut deps: Vec<&'a str> = node.after.iter().map(String::as_str).collect();
    for (other_id, other) in map {
        if other.before.iter().any(|b| b == id) {
            deps.push(*other_id);
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

fn effective_after_owned(
    id: &str,
    node: &PolicyToml,
    map: &BTreeMap<&str, &PolicyToml>,
) -> Vec<String> {
    effective_after_refs(id, node, map)
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub fn generate(nodes: &[PolicyToml]) -> Result<String, String> {
    validate(nodes)?;
    let mut nodes = nodes.to_vec();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let idnum: BTreeMap<String, u16> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), (i + 1) as u16))
        .collect();
    let startnum: BTreeMap<&str, u16> = STARTUPS
        .iter()
        .enumerate()
        .map(|(i, s)| (*s, (i + 1) as u16))
        .collect();
    let mut s = String::from("//! Generated by tools/gen-kso-policy. Do not edit by hand.\n\nuse super::graph::{KsoNode, KsoNodeRuntime};\nuse super::policy::{KsoCapability, KsoFailurePolicy, KsoNodeKind, KsoPolicy, KsoStartupFnId};\nuse super::state::KsoNodeId;\n\n");
    for n in &nodes {
        let cname = const_name(&n.id);
        s.push_str(&format!(
            "pub const {cname}: KsoNodeId = KsoNodeId({});\n",
            idnum[&n.id]
        ));
    }
    s.push('\n');
    for n in &nodes {
        let cname = const_name(&n.id);
        let after = effective_after_owned(
            &n.id,
            n,
            &nodes.iter().map(|node| (node.id.as_str(), node)).collect(),
        );
        emit_id_slice(&mut s, &format!("{cname}_AFTER"), &after, &idnum);
        emit_id_slice(&mut s, &format!("{cname}_WANTS"), &n.wants, &idnum);
        emit_cap_slice(&mut s, &format!("{cname}_REQUIRES"), &n.requires);
        emit_cap_slice(
            &mut s,
            &format!("{cname}_WANTS_CAPABILITIES"),
            &n.wants_capabilities,
        );
        emit_cap_slice(&mut s, &format!("{cname}_PROVIDES"), &n.provides);
        emit_cap_slice(
            &mut s,
            &format!("{cname}_OPTIONAL_PROVIDES"),
            &n.optional_provides,
        );
    }
    s.push_str("pub static KSO_NODES: &[KsoNode] = &[\n");
    for n in &nodes {
        let cname = const_name(&n.id);
        let startup = n
            .startup
            .as_ref()
            .map(|x| startnum[x.as_str()])
            .unwrap_or(0);
        s.push_str(&format!("    KsoNode {{ id: {cname}, name: {:?}, kind: KsoNodeKind::{}, startup: KsoStartupFnId({startup}), after: &{cname}_AFTER, wants: &{cname}_WANTS, requires: &{cname}_REQUIRES, wants_capabilities: &{cname}_WANTS_CAPABILITIES, provides: &{cname}_PROVIDES, optional_provides: &{cname}_OPTIONAL_PROVIDES, policy: KsoPolicy {{ required: {}, allow_missing_wants: {}, failure: KsoFailurePolicy::{}, allow_cooperative_mtss: {}, require_preemption: {} }} }},\n", n.name, n.kind, n.required, !n.required, n.failure.as_deref().unwrap_or("Fatal"), n.allow_cooperative_mtss, n.require_preemption));
    }
    s.push_str("];\npub const KSO_RUNTIME_INIT: KsoNodeRuntime = KsoNodeRuntime::new();\n");
    Ok(s)
}

fn emit_id_slice(s: &mut String, name: &str, ids: &[String], map: &BTreeMap<String, u16>) {
    s.push_str(&format!("const {name}: [KsoNodeId; {}] = [", ids.len()));
    for id in ids {
        s.push_str(&format!("KsoNodeId({}),", map[id]));
    }
    s.push_str("];\n");
}
fn emit_cap_slice(s: &mut String, name: &str, caps: &[String]) {
    s.push_str(&format!(
        "const {name}: [KsoCapability; {}] = [",
        caps.len()
    ));
    for c in caps {
        s.push_str(&format!("KsoCapability({:?}),", c));
    }
    s.push_str("];\n");
}
fn const_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub fn generate_from_dir(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), String> {
    let nodes = load_dir(input)?;
    let text = generate(&nodes)?;
    if let Some(parent) = output.as_ref().parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(output, text).map_err(|e| e.to_string())
}
