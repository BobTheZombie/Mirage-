#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    spider_rs::start::spider_main()
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
    println!("Spider-rs PID 1 online");
    println!("Spider-rs: loading built-in default target table");
    for unit in spider_rs::target::activation_order() {
        println!("Spider-rs: activating unit");
        println!("{unit}");
    }
    println!("Spider-rs: basic.target active");
    println!("Spider-rs: default.target active");
    println!("mode: host diagnostic only; no Mirage PID 1 process ABI is claimed");
}
