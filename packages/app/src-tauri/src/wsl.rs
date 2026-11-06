//! Linux panes (the old Linux Inside feature) are not built. The pane record keeps the optional
//! place field so existing data and callers stay valid; nothing starts a distro here.

pub mod model {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct PanePlace {
        pub distro: String,
    }

    impl PanePlace {
        pub fn startup_extra_ms(&self) -> u64 {
            0
        }
    }
}

pub mod live {
    pub fn stop_pane(_distro: &str, _pane_id: &str) {}
}

pub mod place {
    pub fn agent_runs_in_linux(_agent: &str) -> bool {
        false
    }

}
