use crate::{log, target};

pub fn activate_builtin_graph() {
    log::info("Spider-rs: loading built-in default target table");
    for name in target::activation_order() {
        log::info("Spider-rs: activating unit");
        log::info(name);
    }
    log::info("Spider-rs: basic.target active");
    log::info("Spider-rs: default.target active");
}
