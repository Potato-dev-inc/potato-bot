//! Environment guards for issue runs are not available in this build. The spawn refuses a guard
//! name instead of starting unguarded.

use crate::tui::LaunchEnv;

pub fn launch_env_for(guard: &str) -> Result<LaunchEnv, String> {
    Err(format!("env guard {guard:?} is not available in this build; the launch is refused"))
}
