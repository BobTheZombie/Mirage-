//! KSO policy data. The supervisor owns real policy; this module stores
//! deterministic policy decisions supplied to the kernel service object runner.

/// Kind of kernel service object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoNodeKind {
    KernelMechanism,
    MtssMechanism,
    SupervisorService,
    DriverService,
    UserspaceBootstrap,
    Application,
}

/// Capability token required or produced by a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KsoCapability(pub &'static str);

/// Startup function identifier. Dispatch tables outside KSO map this to code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KsoStartupFnId(pub u16);

/// Failure handling policy for a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KsoFailurePolicy {
    Fatal,
    AllowDegraded,
    Skip,
    Disable,
    MarkFailedNonFatal,
}

/// Static policy attached to a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KsoPolicy {
    pub required: bool,
    pub allow_missing_wants: bool,
    pub failure: KsoFailurePolicy,
}

impl KsoPolicy {
    pub const REQUIRED: Self = Self {
        required: true,
        allow_missing_wants: false,
        failure: KsoFailurePolicy::Fatal,
    };

    pub const OPTIONAL_DRIVER: Self = Self {
        required: false,
        allow_missing_wants: true,
        failure: KsoFailurePolicy::AllowDegraded,
    };
}
