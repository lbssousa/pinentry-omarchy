use nix::sys::mman::{MlockAllFlags, mlockall};
use nix::sys::prctl;

/// Keeps the PIN out of swap and core dumps. Best effort: a low
/// RLIMIT_MEMLOCK only loses the swap guarantee.
pub fn harden() {
    let _ = prctl::set_dumpable(false);
    let _ = mlockall(MlockAllFlags::MCL_CURRENT | MlockAllFlags::MCL_FUTURE);
}
