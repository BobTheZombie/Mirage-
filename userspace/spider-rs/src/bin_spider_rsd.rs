#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(target_os = "none")]
mod mirage_dispatcher {
    use alloc::{
        collections::BTreeMap,
        string::{String, ToString},
        vec::Vec,
    };
    use spider_rs::syscall;
    use spider_rs::units::{LoadedUnit, RestartPolicy, UnitKind, UnitState};
    use spider_rs::{graph::resolve_startup_order, parse_unit};

    const UNIT_DIRS: [&str; 2] = ["/etc/spider/units", "/usr/lib/spider/units"];
    const MANIFEST: [(&str, &str); 3] = [
        ("/etc/spider/units/default.target", "default.target"),
        ("/etc/spider/units/basic.target", "basic.target"),
        (
            "/etc/spider/units/m1-terminal.service",
            "m1-terminal.service",
        ),
    ];
    const BUILTINS: [(&str, &str); 3] = [
        ("basic.target", include_str!("../units/basic.target")),
        ("default.target", include_str!("../units/default.target")),
        (
            "m1-terminal.service",
            include_str!("../units/m1-terminal.service"),
        ),
    ];

    pub fn run() -> ! {
        let mut dispatcher = Dispatcher {
            units: BTreeMap::new(),
        };
        dispatcher.load_units();
        let Ok(plan) = resolve_startup_order(&dispatcher.units, "default.target") else {
            let _ = syscall::write(1, b"SYSTEM DISPATCHER [FAILED]");
            loop {
                syscall::yield_now();
            }
        };
        let _ = syscall::write(1, b"SYSTEM DISPATCHER [ONLINE]\n");
        for name in plan.order {
            dispatcher.start_unit(&name);
        }
        loop {
            syscall::yield_now();
        }
    }

    struct Dispatcher {
        units: BTreeMap<String, LoadedUnit>,
    }

    impl Dispatcher {
        fn load_units(&mut self) {
            let mut listed_any = false;
            for dir in UNIT_DIRS {
                listed_any |= self.load_dir(dir);
            }
            if !listed_any {
                for (path, name) in MANIFEST {
                    if let Some(source) = read_file(path) {
                        self.insert_unit(name, &source);
                    }
                }
            }
            for (name, source) in BUILTINS {
                if !self.units.contains_key(name) {
                    self.insert_unit(name, source);
                }
            }
        }

        fn load_dir(&mut self, dir: &str) -> bool {
            let Ok(fd) = syscall::open(dir) else {
                return false;
            };
            let mut saw_entries = false;
            let mut buffer = [0u8; 1024];
            loop {
                let Ok(read) = syscall::read_dir(fd as usize, &mut buffer) else {
                    break;
                };
                if read == 0 {
                    break;
                }
                let mut offset = 0usize;
                while offset + 19 <= read {
                    let reclen =
                        u16::from_ne_bytes([buffer[offset + 16], buffer[offset + 17]]) as usize;
                    if reclen == 0 || offset + reclen > read {
                        break;
                    }
                    let name_start = offset + 19;
                    let mut name_end = name_start;
                    while name_end < offset + reclen && buffer[name_end] != 0 {
                        name_end += 1;
                    }
                    if let Ok(name) = core::str::from_utf8(&buffer[name_start..name_end]) {
                        if is_unit_name(name) {
                            saw_entries = true;
                            let mut path = String::new();
                            path.push_str(dir);
                            path.push('/');
                            path.push_str(name);
                            if let Some(source) = read_file(&path) {
                                self.insert_unit(name, &source);
                            }
                        }
                    }
                    offset += reclen;
                }
            }
            let _ = syscall::close(fd as usize);
            saw_entries
        }

        fn insert_unit(&mut self, name: &str, source: &str) {
            if let Ok(unit) = parse_unit(name, source) {
                self.units.insert(name.to_string(), unit);
            }
        }

        fn start_unit(&mut self, name: &str) {
            if self.required_failed(name) {
                self.set_state(name, UnitState::Failed);
                return;
            }
            self.set_state(name, UnitState::Waiting);
            let Some(kind) = self.units.get(name).map(|u| u.unit.kind) else {
                return;
            };
            match kind {
                UnitKind::Target => {
                    self.set_state(name, UnitState::Running);
                    if name == "default.target" {
                        let _ = syscall::write(1, b"DEFAULT TARGET [STARTED]\n");
                    }
                }
                UnitKind::Service => self.start_service(name),
                _ => self.set_state(name, UnitState::Skipped),
            }
        }

        fn start_service(&mut self, name: &str) {
            let Some(service) = self.units.get(name).and_then(|u| u.service.clone()) else {
                self.set_state(name, UnitState::Failed);
                return;
            };
            let mut attempts = 0usize;
            loop {
                attempts += 1;
                self.set_state(name, UnitState::Starting);
                let argv = split_exec(&service.exec_start);
                let Some(path) = argv.first() else {
                    self.set_state(name, UnitState::Failed);
                    break;
                };
                let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
                match syscall::spawn(path, &refs, &[]) {
                    Ok(pid) => {
                        self.set_state(name, UnitState::Running);
                        if name == "m1-terminal.service" {
                            let _ = syscall::write(1, b"M1 TERMINAL [STARTED]\n");
                        }
                        match syscall::wait(pid) {
                            Ok(0) => {
                                self.set_state(name, UnitState::Exited);
                                if name == "m1-terminal.service" {
                                    let _ = syscall::write(1, b"M1 TERMINAL [EXITED: 0]\n");
                                }
                            }
                            Ok(_) | Err(_) => {
                                self.set_state(name, UnitState::Failed);
                                if name == "m1-terminal.service" {
                                    let _ = syscall::write(1, b"M1 TERMINAL [FAILED]\n");
                                }
                            }
                        }
                    }
                    Err(_) => {
                        self.set_state(name, UnitState::Failed);
                        if name == "m1-terminal.service" {
                            let _ = syscall::write(1, b"M1 TERMINAL [FAILED]\n");
                        }
                    }
                }
                let state = self
                    .units
                    .get(name)
                    .map(|u| u.state)
                    .unwrap_or(UnitState::Failed);
                let restart = match service.restart {
                    RestartPolicy::No => false,
                    RestartPolicy::OnFailure => state == UnitState::Failed,
                    RestartPolicy::Always => matches!(state, UnitState::Exited | UnitState::Failed),
                };
                if !restart || attempts >= 2 {
                    break;
                }
            }
        }

        fn required_failed(&self, name: &str) -> bool {
            self.units
                .get(name)
                .map(|u| {
                    u.unit.requires.iter().any(|r| {
                        self.units
                            .get(r)
                            .map(|d| d.state == UnitState::Failed)
                            .unwrap_or(true)
                    })
                })
                .unwrap_or(true)
        }
        fn set_state(&mut self, name: &str, state: UnitState) {
            if let Some(unit) = self.units.get_mut(name) {
                unit.state = state;
            }
        }
    }

    fn read_file(path: &str) -> Option<String> {
        let fd = syscall::open(path).ok()?;
        let mut out = String::new();
        let mut buffer = [0u8; 512];
        loop {
            let read = syscall::read(fd as usize, &mut buffer).ok()?;
            if read == 0 {
                break;
            }
            out.push_str(core::str::from_utf8(&buffer[..read]).ok()?);
        }
        let _ = syscall::close(fd as usize);
        Some(out)
    }

    fn split_exec(exec: &str) -> Vec<String> {
        exec.split_whitespace().map(str::to_string).collect()
    }
    fn is_unit_name(name: &str) -> bool {
        matches!(
            name.rsplit_once('.').map(|(_, s)| s),
            Some("service" | "target" | "socket" | "timer" | "mount" | "device" | "path")
        )
    }
}

#[cfg(target_os = "none")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    mirage_dispatcher::run()
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(target_os = "none"))]
fn main() {
    use spider_rs::{SpiderManager, StubSpawner};
    let mut manager = SpiderManager::with_builtin_units().expect("builtin units parse");
    let _ = manager.load_search_paths();
    let plan = manager.resolve_default().expect("default target resolves");
    println!("SYSTEM DISPATCHER [ONLINE]");
    for outcome in manager.start_plan(&plan, &StubSpawner::default()) {
        println!("{} [{:?}] {}", outcome.name, outcome.state, outcome.message);
    }
}
