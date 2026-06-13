use crate::units::{default_units, UnitDescriptor};

pub fn builtins() -> &'static [UnitDescriptor] {
    default_units()
}

pub fn activation_order() -> [&'static str; 3] {
    ["spider-init.service", "basic.target", "default.target"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_activation_order_reaches_basic_before_default() {
        assert_eq!(
            activation_order(),
            ["spider-init.service", "basic.target", "default.target"]
        );
    }

    #[test]
    fn builtins_include_emergency_target() {
        assert!(builtins()
            .iter()
            .any(|unit| unit.name == "emergency.target"));
    }
}
