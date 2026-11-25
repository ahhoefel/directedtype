use crate::compiler::expand::builtins::{static_type_from_canonical_name, SpecialBuiltinSpec};
use crate::compiler::expand::typecheck::StaticType;

/// Authoritative port & property schema for `\Timer(duration: Duration)`.
pub const TIMER_SPEC: SpecialBuiltinSpec = SpecialBuiltinSpec {
    name: "Timer",
    accepted_ports: &[("duration", "Duration")],
    output_ports: &[
        ("time", "Duration"),
        ("duration", "Duration"),
        ("elapsed", "Float"),
        ("dt", "Duration"),
        ("frames", "Integer"),
    ],
};

/// Authoritative port & property schema for `\IndefiniteTimer()`.
pub const INDEFINITE_TIMER_SPEC: SpecialBuiltinSpec = SpecialBuiltinSpec {
    name: "IndefiniteTimer",
    accepted_ports: &[],
    output_ports: &[
        ("time", "Duration"),
        ("dt", "Duration"),
        ("frames", "Integer"),
    ],
};

/// Returns the static type of a readable property on `Timer`, backed strictly by `TIMER_SPEC`.
pub(in crate::compiler::expand) fn timer_port_type(port_name: &str) -> Option<StaticType> {
    TIMER_SPEC
        .output_port_type(port_name)
        .map(static_type_from_canonical_name)
}

/// Returns the static type of a readable property on `IndefiniteTimer`, backed strictly by `INDEFINITE_TIMER_SPEC`.
pub(in crate::compiler::expand) fn indefinite_timer_port_type(
    port_name: &str,
) -> Option<StaticType> {
    INDEFINITE_TIMER_SPEC
        .output_port_type(port_name)
        .map(static_type_from_canonical_name)
}
