//! Compatibility facade for the shared authenticated lifecycle client.
//! Discovery, protocol checks, credentials and managed autostart live in
//! `lifecycle`; no entry point may connect by reading a record alone.
use crate::lifecycle;
use hieronymus::data_root::HieronymusConfig;

pub use crate::client::ClientError as DaemonClientError;

#[derive(Debug)]
pub struct DaemonClient(lifecycle::DaemonClient);

impl DaemonClient {
    pub fn connect(config: &HieronymusConfig) -> Result<Self, DaemonClientError> {
        Self::connect_opt_in(config, false)
    }

    pub fn connect_opt_in(
        config: &HieronymusConfig,
        start_daemon: bool,
    ) -> Result<Self, DaemonClientError> {
        lifecycle::connect(config, start_daemon).map(Self)
    }

    pub fn endpoint(&self) -> std::net::SocketAddr {
        self.0.address()
    }
}

impl std::ops::Deref for DaemonClient {
    type Target = lifecycle::DaemonClient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
