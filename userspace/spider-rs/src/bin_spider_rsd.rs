#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(any(target_os = "none", test))]
#[cfg_attr(test, allow(dead_code))]
mod mirage_dispatcher {
    #[cfg(target_os = "none")]
    use alloc::{
        collections::BTreeMap,
        string::{String, ToString},
        vec::Vec,
    };
    #[cfg(target_os = "none")]
    use spider_rs::syscall;
    #[cfg(test)]
    mod syscall {
        pub fn write(_fd: usize, _bytes: &[u8]) -> isize {
            0
        }
        pub fn spawn(_path: &str, _argv: &[&str], _env: &[(&str, &str)]) -> Result<isize, isize> {
            Err(-38)
        }
        pub fn wait(_pid: isize) -> Result<isize, isize> {
            Err(-38)
        }
    }
    #[cfg(target_os = "none")]
    use spider_rs::graph::resolve_startup_order;
    use spider_rs::parse_unit;
    use spider_rs::units::{LoadedUnit, RestartPolicy, UnitKind, UnitState};
    #[cfg(test)]
    use std::{
        collections::BTreeMap,
        string::{String, ToString},
        vec::Vec,
    };

    const UNIT_DIRS: [&str; 2] = ["/etc/spider/units", "/usr/lib/spider/units"];
    const REQUIRED_UNITS: [(&str, &str); 3] = [
        ("/etc/spider/units/default.target", "default.target"),
        ("/etc/spider/units/basic.target", "basic.target"),
        (
            "/etc/spider/units/m1-terminal.service",
            "m1-terminal.service",
        ),
    ];

    #[cfg(target_os = "none")]
    pub fn run() -> ! {
        let mut dispatcher = Dispatcher {
            units: BTreeMap::new(),
        };
        if !dispatcher.load_units(&SyscallIo) {
            loop {
                syscall::yield_now();
            }
        }
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
        fn load_units<I: DispatcherIo>(&mut self, io: &I) -> bool {
            for (path, name) in REQUIRED_UNITS {
                let Some(bytes) = read_file(io, path) else {
                    report_required_unit_failure(io, path, "missing or unreadable");
                    return false;
                };
                let Ok(source) = core::str::from_utf8(&bytes) else {
                    report_required_unit_failure(io, path, "not valid UTF-8");
                    return false;
                };
                if !self.insert_unit(name, source) {
                    report_required_unit_failure(io, path, "parse failed");
                    return false;
                }
            }

            for dir in UNIT_DIRS {
                self.load_dir(io, dir);
            }
            true
        }

        fn load_dir<I: DispatcherIo>(&mut self, io: &I, dir: &str) -> bool {
            let Ok(fd) = io.open(dir) else {
                return false;
            };
            let mut saw_entries = false;
            let mut buffer = [0u8; 1024];
            loop {
                let Ok(read) = io.read_dir(fd as usize, &mut buffer) else {
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
                            if let Some(bytes) = read_file(io, &path) {
                                if let Ok(source) = core::str::from_utf8(&bytes) {
                                    self.insert_unit(name, source);
                                }
                            }
                        }
                    }
                    offset += reclen;
                }
            }
            let _ = io.close(fd as usize);
            saw_entries
        }

        fn insert_unit(&mut self, name: &str, source: &str) -> bool {
            match parse_unit(name, source) {
                Ok(unit) => {
                    self.units.insert(name.to_string(), unit);
                    true
                }
                Err(_) => false,
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

    trait DispatcherIo {
        fn open(&self, path: &str) -> Result<isize, isize>;
        fn read(&self, fd: usize, buffer: &mut [u8]) -> Result<usize, isize>;
        fn read_dir(&self, fd: usize, buffer: &mut [u8]) -> Result<usize, isize>;
        fn close(&self, fd: usize) -> Result<(), isize>;
        fn write(&self, fd: usize, bytes: &[u8]) -> isize;
    }

    #[cfg(target_os = "none")]
    struct SyscallIo;

    #[cfg(target_os = "none")]
    impl DispatcherIo for SyscallIo {
        fn open(&self, path: &str) -> Result<isize, isize> {
            syscall::open(path)
        }
        fn read(&self, fd: usize, buffer: &mut [u8]) -> Result<usize, isize> {
            syscall::read(fd, buffer)
        }
        fn read_dir(&self, fd: usize, buffer: &mut [u8]) -> Result<usize, isize> {
            syscall::read_dir(fd, buffer)
        }
        fn close(&self, fd: usize) -> Result<(), isize> {
            syscall::close(fd)
        }
        fn write(&self, fd: usize, bytes: &[u8]) -> isize {
            syscall::write(fd, bytes)
        }
    }

    fn read_file<I: DispatcherIo>(io: &I, path: &str) -> Option<Vec<u8>> {
        let fd = io.open(path).ok()?;
        let mut out = Vec::new();
        let mut buffer = [0u8; 512];
        loop {
            let read = io.read(fd as usize, &mut buffer).ok()?;
            if read == 0 {
                break;
            }
            out.extend_from_slice(&buffer[..read]);
        }
        let _ = io.close(fd as usize);
        Some(out)
    }

    fn report_required_unit_failure<I: DispatcherIo>(io: &I, path: &str, reason: &str) {
        let _ = io.write(1, b"SYSTEM DISPATCHER [FAILED]: required unit ");
        let _ = io.write(1, path.as_bytes());
        let _ = io.write(1, b" ");
        let _ = io.write(1, reason.as_bytes());
        let _ = io.write(1, b"\n");
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::cell::{Cell, RefCell};
        use std::collections::BTreeMap as StdBTreeMap;

        const DEFAULT_TARGET: &[u8] = b"[Unit]\nDescription=Default userspace target\nRequires=basic.target m1-terminal.service\nAfter=basic.target\n";
        const BASIC_TARGET: &[u8] = b"[Unit]\nDescription=Basic userspace target\n";
        const M1_TERMINAL_SERVICE: &[u8] = b"[Unit]\nDescription=M1 Terminal\nAfter=basic.target\n\n[Service]\nExecStart=/usr/bin/m1-terminal\nRestart=no\n";

        #[derive(Default)]
        struct MockIo {
            files: StdBTreeMap<&'static str, &'static [u8]>,
            next_fd: Cell<isize>,
            open_files: RefCell<StdBTreeMap<usize, (&'static [u8], usize)>>,
            writes: RefCell<Vec<u8>>,
            opened_paths: RefCell<Vec<String>>,
        }

        impl MockIo {
            fn with_required_units() -> Self {
                let mut io = Self {
                    next_fd: Cell::new(3),
                    ..Self::default()
                };
                io.files
                    .insert("/etc/spider/units/default.target", DEFAULT_TARGET);
                io.files
                    .insert("/etc/spider/units/basic.target", BASIC_TARGET);
                io.files
                    .insert("/etc/spider/units/m1-terminal.service", M1_TERMINAL_SERVICE);
                io
            }
        }

        impl DispatcherIo for MockIo {
            fn open(&self, path: &str) -> Result<isize, isize> {
                self.opened_paths.borrow_mut().push(path.to_string());
                let Some(bytes) = self.files.get(path).copied() else {
                    return Err(-2);
                };
                let fd = self.next_fd.get();
                self.next_fd.set(fd + 1);
                self.open_files.borrow_mut().insert(fd as usize, (bytes, 0));
                Ok(fd)
            }

            fn read(&self, fd: usize, buffer: &mut [u8]) -> Result<usize, isize> {
                let mut open_files = self.open_files.borrow_mut();
                let Some((bytes, offset)) = open_files.get_mut(&fd) else {
                    return Err(-9);
                };
                let available = bytes.len().saturating_sub(*offset);
                let len = available.min(buffer.len()).min(7);
                buffer[..len].copy_from_slice(&bytes[*offset..*offset + len]);
                *offset += len;
                Ok(len)
            }

            fn read_dir(&self, _fd: usize, _buffer: &mut [u8]) -> Result<usize, isize> {
                Ok(0)
            }

            fn close(&self, fd: usize) -> Result<(), isize> {
                self.open_files.borrow_mut().remove(&fd);
                Ok(())
            }

            fn write(&self, _fd: usize, bytes: &[u8]) -> isize {
                self.writes.borrow_mut().extend_from_slice(bytes);
                bytes.len() as isize
            }
        }

        #[test]
        fn loads_required_units_from_syscall_vfs_bytes() {
            let io = MockIo::with_required_units();
            let mut dispatcher = Dispatcher {
                units: BTreeMap::new(),
            };

            assert!(dispatcher.load_units(&io));

            assert!(dispatcher.units.contains_key("default.target"));
            assert!(dispatcher.units.contains_key("basic.target"));
            assert_eq!(
                dispatcher
                    .units
                    .get("m1-terminal.service")
                    .and_then(|unit| unit.service.as_ref())
                    .map(|service| service.exec_start.as_str()),
                Some("/usr/bin/m1-terminal")
            );
            let opened = io.opened_paths.borrow();
            assert!(opened
                .iter()
                .any(|path| path == "/etc/spider/units/default.target"));
            assert!(opened
                .iter()
                .any(|path| path == "/etc/spider/units/basic.target"));
            assert!(opened
                .iter()
                .any(|path| path == "/etc/spider/units/m1-terminal.service"));
        }

        #[test]
        fn missing_required_unit_emits_explicit_sys_write_failure() {
            let mut io = MockIo::with_required_units();
            io.files.remove("/etc/spider/units/basic.target");
            let mut dispatcher = Dispatcher {
                units: BTreeMap::new(),
            };

            assert!(!dispatcher.load_units(&io));

            let message = String::from_utf8(io.writes.borrow().clone()).unwrap();
            assert!(message.contains("SYSTEM DISPATCHER [FAILED]"));
            assert!(message.contains("/etc/spider/units/basic.target"));
            assert!(message.contains("missing or unreadable"));
        }
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

#[cfg(all(feature = "host-tests", not(target_os = "none")))]
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

#[cfg(all(not(feature = "host-tests"), not(target_os = "none")))]
fn main() {
    eprintln!("host diagnostic mode is disabled; rebuild with --features host-tests to run host-only Spider diagnostics");
}
