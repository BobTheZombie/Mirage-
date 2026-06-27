use gen_kso_policy::{generate, validate, PolicyToml};

fn node(id: &str) -> PolicyToml {
    PolicyToml {
        id: id.into(),
        name: id.into(),
        kind: "KernelMechanism".into(),
        startup: Some("seed_rs".into()),
        required: true,
        failure: Some("Fatal".into()),
        optional_policy: None,
        allow_cooperative_mtss: false,
        require_preemption: false,
        after: vec![],
        before: vec![],
        conflicts: vec![],
        wants: vec![],
        wants_capabilities: vec![],
        requires: vec![],
        provides: vec!["boot.seed".into()],
        optional_provides: vec![],
    }
}

#[test]
fn valid_parsing_and_generation() {
    let mut a = node("a");
    a.provides = vec!["boot.seed".into()];
    let mut b = node("b");
    b.startup = Some("bootinfo".into());
    b.after = vec!["a".into()];
    b.requires = vec!["boot.seed".into()];
    b.provides = vec!["boot.info".into()];
    validate(&[a.clone(), b.clone()]).unwrap();
    let out = generate(&[b, a]).unwrap();
    assert!(out.contains("pub static KSO_NODES"));
    assert!(out.find("BOOT SEED").is_none());
}

#[test]
fn rejects_unknown_dependency() {
    let mut a = node("a");
    a.after = vec!["missing".into()];
    assert!(validate(&[a])
        .unwrap_err()
        .contains("unknown after reference"));
}

#[test]
fn rejects_unknown_capability() {
    let mut a = node("a");
    a.requires = vec!["bad.cap".into()];
    assert!(validate(&[a]).unwrap_err().contains("unknown capability"));
}

#[test]
fn rejects_unknown_startup_function() {
    let mut a = node("a");
    a.startup = Some("bad_start".into());
    assert!(validate(&[a])
        .unwrap_err()
        .contains("unknown startup function"));
}

#[test]
fn rejects_cycle() {
    let mut a = node("a");
    a.provides = vec!["boot.seed".into()];
    a.after = vec!["b".into()];
    let mut b = node("b");
    b.startup = Some("bootinfo".into());
    b.provides = vec!["boot.info".into()];
    b.after = vec!["a".into()];
    assert!(validate(&[a, b]).unwrap_err().contains("dependency cycle"));
}

#[test]
fn rejects_duplicate_id() {
    let a = node("a");
    let mut b = node("a");
    b.startup = Some("bootinfo".into());
    b.provides = vec!["boot.info".into()];
    assert!(validate(&[a, b]).unwrap_err().contains("duplicate node id"));
}

#[test]
fn deterministic_generation() {
    let mut a = node("a");
    a.provides = vec!["boot.seed".into()];
    let mut b = node("b");
    b.startup = Some("bootinfo".into());
    b.after = vec!["a".into()];
    b.requires = vec!["boot.seed".into()];
    b.provides = vec!["boot.info".into()];
    assert_eq!(
        generate(&[a.clone(), b.clone()]).unwrap(),
        generate(&[b, a]).unwrap()
    );
}
