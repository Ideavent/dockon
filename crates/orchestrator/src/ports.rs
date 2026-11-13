use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub(crate) struct PortAllocator(Arc<Mutex<BTreeSet<u16>>>);
pub(crate) struct PortLease {
    pub port: u16,
    owner: Arc<Mutex<BTreeSet<u16>>>,
}
impl PortAllocator {
    pub fn reserve(&self, requested: u16) -> Result<PortLease> {
        let mut ports = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Port allocator unavailable"))?;
        if requested != 0 && ports.contains(&requested) {
            bail!("Port {requested} is already leased to another Dockon workload");
        }
        for _ in 0..128 {
            let listener = std::net::TcpListener::bind(("127.0.0.1", requested))
                .context("Port unavailable")?;
            let port = listener.local_addr()?.port();
            if ports.insert(port) {
                return Ok(PortLease {
                    port,
                    owner: self.0.clone(),
                });
            }
        }
        bail!("Unable to allocate an unleased local port")
    }
}
impl Drop for PortLease {
    fn drop(&mut self) {
        self.owner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.port);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservation_covers_child_startup_gap_and_releases_after_lifecycle() {
        let allocator = PortAllocator::default();
        let first = allocator.reserve(0).unwrap();
        let port = first.port;
        assert!(allocator.reserve(port).is_err());
        drop(first);
        assert!(allocator.reserve(port).is_ok());
    }
}
