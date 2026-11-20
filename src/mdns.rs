//! mDNS / zeroconf announcement (`_eink-renderer._tcp.local.`) so Home Assistant can discover the renderer.
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;

pub const SERVICE_TYPE: &str = "_eink-renderer._tcp.local.";

/// True for addresses Docker hands out on its bridge networks (172.17.0.0 - 172.31.255.255) when running
/// inside a container: such an address is not reachable from the LAN, so it is neither shown nor announced.
fn docker_bridge_address(ip: &IpAddr, in_container: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => in_container && v4.octets()[0] == 172 && (17..=31).contains(&v4.octets()[1]),
        IpAddr::V6(_) => false,
    }
}

/// IPv4 addresses worth announcing: not loopback/link-local, not container/VPN bridges.
/// Empty inside a container on Docker's bridge network (use --network host, or set ADVERTISE_IP).
pub fn lan_addresses(override_ip: Option<&str>) -> Vec<IpAddr> {
    if let Some(ip) = override_ip.and_then(|s| s.parse::<IpAddr>().ok()) {
        return vec![ip];
    }
    let in_container = std::path::Path::new("/.dockerenv").exists();
    let skip = [
        "docker",
        "br-",
        "veth",
        "virbr",
        "cni",
        "flannel",
        "cali",
        "tailscale",
        "zt",
        "wg",
        "tun",
        "lo",
    ];
    let mut out: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !skip.iter().any(|p| i.name.starts_with(p)))
        .map(|i| i.ip())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_link_local() && !v4.is_unspecified(),
            IpAddr::V6(_) => false,
        })
        .filter(|ip| !docker_bridge_address(ip, in_container))
        .collect();
    out.sort();
    out.dedup();
    out
}

pub struct Announcer {
    daemon: ServiceDaemon,
    fullname: Option<String>,
}

impl Announcer {
    pub fn new() -> Result<Announcer, String> {
        Ok(Announcer {
            daemon: ServiceDaemon::new().map_err(|e| e.to_string())?,
            fullname: None,
        })
    }

    /// (Re-)announce with the current TXT properties.
    pub fn announce(
        &mut self,
        id: &str,
        name: &str,
        port: u16,
        addrs: &[IpAddr],
        props: &[(&str, String)],
    ) -> Result<(), String> {
        if addrs.is_empty() {
            return Err("no LAN address to announce (use --network host, or set ADVERTISE_IP)".into());
        }
        if let Some(f) = self.fullname.take() {
            let _ = self.daemon.unregister(&f);
        }
        let instance = format!("{name} {}", &id[..id.len().min(6)]);
        let host = format!("eink-renderer-{id}.local.");
        let props: Vec<(&str, &str)> = props.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let info =
            ServiceInfo::new(SERVICE_TYPE, &instance, &host, addrs, port, &props[..]).map_err(|e| e.to_string())?;
        self.fullname = Some(info.get_fullname().to_string());
        self.daemon.register(info).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_bridge_ranges() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(docker_bridge_address(&ip("172.17.0.2"), true));
        assert!(docker_bridge_address(&ip("172.31.255.1"), true));
        assert!(!docker_bridge_address(&ip("172.16.5.5"), true));
        assert!(!docker_bridge_address(&ip("172.17.0.2"), false));
        assert!(!docker_bridge_address(&ip("192.168.0.10"), true));
    }
}
