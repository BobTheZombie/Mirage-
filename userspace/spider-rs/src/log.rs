use crate::syscall;

pub fn info(message: &str) {
    let _ = syscall::write(1, message.as_bytes());
    let _ = syscall::write(1, b"\n");
}
