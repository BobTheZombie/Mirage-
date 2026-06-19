#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]

#[cfg(target_os = "none")]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let _ = spider_rs::syscall::write(1, b"Mirage M1.1 System\n");
    let _ = spider_rs::syscall::write(1, b"hello world\n");
    spider_rs::syscall::exit(0);
}

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! { loop { core::hint::spin_loop(); } }

#[cfg(not(target_os = "none"))]
fn main() {
    println!("Mirage M1.1 System");
    println!("hello world");
}
