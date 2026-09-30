//! Local iroh endpoint.
//!
//! The event log stays the source of truth. With the `p2p` feature (on by
//! default) `bind_local` binds a QUIC endpoint and returns its node id. Relays
//! and port mapping stay off, and nothing is dialed. Without the feature the
//! same function returns no node id and iroh is not linked.

#[cfg(feature = "p2p")]
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEndpoint {
    pub node_id: Option<String>,
}

#[cfg(feature = "p2p")]
pub const TRANSPORT: &str = "iroh";

#[cfg(not(feature = "p2p"))]
pub const TRANSPORT: &str = "off";

pub fn linked() -> bool {
    cfg!(feature = "p2p")
}

#[cfg(feature = "p2p")]
pub fn describe() -> &'static str {
    "iroh endpoint, relays disabled, portmapper disabled, no peers"
}

#[cfg(not(feature = "p2p"))]
pub fn describe() -> &'static str {
    "p2p feature off: iroh is not linked"
}

#[cfg(not(feature = "p2p"))]
pub fn bind_local() -> Result<LocalEndpoint, String> {
    Ok(LocalEndpoint { node_id: None })
}

/// Binds an iroh endpoint and keeps it alive for the process.
///
/// The endpoint runs on a background current-thread runtime so the daemon's
/// HTTP accept loop can stay blocking. The node id is local; no peer is contacted.
#[cfg(feature = "p2p")]
pub fn bind_local() -> Result<LocalEndpoint, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("iroh-endpoint".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                    return;
                }
            };
            runtime.block_on(async move {
                let bound = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
                    .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
                    .bind()
                    .await;
                match bound {
                    Ok(endpoint) => {
                        let id = endpoint.id().to_string();
                        let _ = tx.send(Ok(id));
                        // Hold the socket until the process exits.
                        std::future::pending::<()>().await;
                        drop(endpoint);
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err.to_string()));
                    }
                }
            });
        })
        .map_err(|err| err.to_string())?;

    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(id)) => Ok(LocalEndpoint { node_id: Some(id) }),
        Ok(Err(err)) => Err(err),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("iroh bind timed out after 10s".into())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("iroh bind thread exited before reporting a node id".into())
        }
    }
}

#[cfg(all(test, feature = "p2p"))]
mod tests {
    use super::*;

    #[test]
    fn binds_a_local_node_id() {
        assert!(linked());
        assert_eq!(TRANSPORT, "iroh");
        let endpoint = bind_local().expect("bind");
        let id = endpoint.node_id.expect("node id");
        assert!(id.len() > 8, "{id}");
    }
}

#[cfg(all(test, not(feature = "p2p")))]
mod tests {
    use super::*;

    #[test]
    fn feature_off_does_not_bind() {
        assert!(!linked());
        assert_eq!(TRANSPORT, "off");
        assert!(bind_local().unwrap().node_id.is_none());
    }
}
