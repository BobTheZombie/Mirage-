//! `_start` entry for the no_std Spider-rs PID1 userspace ELF.

use crate::syscall;

const DISPATCHER: &str = "/spider-rt/sbin/spider-rsd";
const MAX_RESTARTS: usize = 3;

pub fn spider_main() -> ! {
    let _ = syscall::write(1, b"SPIDER-RS PID1 [RUNNING]\n");
    let mut restarts = 0usize;
    loop {
        let _ = syscall::write(1, b"SPIDER-RSD [STARTING]\n");
        match syscall::spawn(DISPATCHER, &[DISPATCHER], &[]) {
            Ok(pid) => {
                let _ = syscall::write(1, b"SPIDER-RSD [RUNNING]\n");
                match syscall::wait(pid) {
                    Ok(0) => {
                        let _ = syscall::write(1, b"SPIDER-RSD [EXITED: 0]\n");
                    }
                    Ok(_) if restarts < MAX_RESTARTS => {
                        restarts += 1;
                        let _ = syscall::write(1, b"SPIDER-RSD [RESTARTING: failure]\n");
                        continue;
                    }
                    Ok(_) => {
                        let _ = syscall::write(1, b"SPIDER-RSD [FAILED: restart limit reached]\n");
                    }
                    Err(_) if restarts < MAX_RESTARTS => {
                        restarts += 1;
                        let _ = syscall::write(1, b"SPIDER-RSD [RESTARTING: wait failed]\n");
                        continue;
                    }
                    Err(_) => {
                        let _ = syscall::write(1, b"SPIDER-RSD [FAILED: wait failed]\n");
                    }
                }
            }
            Err(_) if restarts < MAX_RESTARTS => {
                restarts += 1;
                let _ = syscall::write(1, b"SPIDER-RSD [RESTARTING: spawn failed]\n");
                continue;
            }
            Err(_) => {
                let _ = syscall::write(1, b"SPIDER-RSD [FAILED: spawn syscall failed]\n");
            }
        }
        syscall::yield_now();
    }
}
