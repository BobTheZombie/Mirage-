#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let _ = spider_rs::syscall::write(1, b"SYSTEM DISPATCHER [ONLINE]\n");
    let _ = spider_rs::syscall::write(1, b"DEFAULT TARGET [STARTED]\n");
    match spider_rs::syscall::spawn("/usr/bin/m1-terminal", &["/usr/bin/m1-terminal"], &[]) {
        Ok(pid) => {
            let _ = spider_rs::syscall::write(1, b"M1 TERMINAL [STARTED]\n");
            match spider_rs::syscall::wait(pid) {
                Ok(0) => { let _ = spider_rs::syscall::write(1, b"M1 TERMINAL [EXITED: 0]\n"); }
                Ok(_) => { let _ = spider_rs::syscall::write(1, b"M1 TERMINAL [FAILED: nonzero exit]\n"); }
                Err(_) => { let _ = spider_rs::syscall::write(1, b"M1 TERMINAL [PENDING: wait syscall unavailable]\n"); }
            }
        }
        Err(_) => { let _ = spider_rs::syscall::write(1, b"M1 TERMINAL [FAILED: /usr/bin/m1-terminal missing or spawn unavailable]\n"); }
    }
    loop { spider_rs::syscall::yield_now(); }
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! { loop { core::hint::spin_loop(); } }

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
