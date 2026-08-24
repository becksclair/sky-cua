use std::net::IpAddr;

#[allow(unused_imports)]
use super::{is_private_ip, validate_bind_addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanCandidate {
    pub iface: String,
    pub ip: IpAddr,
}

/// Enumerate private LAN/tether candidates for direct advertisement.
///
/// - Includes loopback-excluded private IPs (RFC1918, CGNAT, link-local, ULA, fe80).
/// - Excludes `tun*` interfaces unless they are the only candidates (VPN-only host).
/// - Callers should highlight the default-route interface via `ip route get` separately.
pub fn enumerate_lan_candidates() -> Vec<LanCandidate> {
    let mut out = Vec::new();
    let ifaces = match if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(_) => return out,
    };
    for iface in ifaces {
        // Skip virtual bridge/container nets that are never the tether/WiFi LAN
        // the QR should advertise (virbr* libvirt, docker*, vmnet*, br-*, veth*).
        if iface.name.starts_with("virbr")
            || iface.name.starts_with("docker")
            || iface.name.starts_with("vmnet")
            || iface.name.starts_with("br-")
            || iface.name.starts_with("veth")
        {
            continue;
        }
        let ip = iface.addr.ip();
        if ip.is_loopback() {
            continue;
        }
        if !is_private_ip(ip) {
            continue;
        }
        out.push(LanCandidate {
            iface: iface.name.clone(),
            ip,
        });
    }
    // Prefer non-VPN interfaces; drop tun*/tailscale* if we have LAN/tether alternatives.
    let has_non_vpn = out
        .iter()
        .any(|c| !c.iface.starts_with("tun") && !c.iface.starts_with("tailscale"));
    if has_non_vpn {
        out.retain(|c| !c.iface.starts_with("tun") && !c.iface.starts_with("tailscale"));
    }
    sort_candidates(&mut out);
    out
}

fn score_ip(ip: &IpAddr) -> u8 {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if o[0] == 192 && o[1] == 168 {
                0 // LAN/tether/hotspot highest
            } else if o[0] == 10 {
                1
            } else if o[0] == 172 && (16..=31).contains(&o[1]) {
                2
            } else if o[0] == 100 && (64..=127).contains(&o[1]) {
                3 // CGNAT
            } else if o[0] == 169 && o[1] == 254 {
                4 // link-local lowest
            } else {
                5
            }
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                let o = v4.octets();
                if o[0] == 192 && o[1] == 168 {
                    0
                } else if o[0] == 10 {
                    1
                } else if o[0] == 172 && (16..=31).contains(&o[1]) {
                    2
                } else if o[0] == 100 && (64..=127).contains(&o[1]) {
                    3
                } else if o[0] == 169 && o[1] == 254 {
                    4
                } else {
                    5
                }
            } else {
                let o = v6.octets();
                if o[0] == 0xfc || o[0] == 0xfd {
                    1 // ULA ~ like 10/8
                } else if o[0] == 0xfe && (o[1] & 0xc0) == 0x80 {
                    4
                } else {
                    5
                }
            }
        }
    }
}

fn sort_candidates(candidates: &mut [LanCandidate]) {
    candidates.sort_by_key(|c| (score_ip(&c.ip), c.iface.clone(), c.ip));
}

/// Validate that each candidate can be used as a bind address (0.0.0.0 coverage also valid).
/// Used by the `0.0.0.0` bind-probe tests to ensure every enumerated LAN candidate
/// would pass `validate_bind_addr`.
#[cfg(test)]
pub fn candidates_are_bindable(candidates: &[LanCandidate], port: u16) -> bool {
    candidates.iter().all(|c| {
        let addr = std::net::SocketAddr::new(c.ip, port);
        validate_bind_addr(addr).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn scoring_prefers_192_168() {
        let mut cands = vec![
            LanCandidate {
                iface: "eth0".into(),
                ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            },
            LanCandidate {
                iface: "wlan0".into(),
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 42, 10)),
            },
            LanCandidate {
                iface: "eth1".into(),
                ip: IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
            },
        ];
        sort_candidates(&mut cands);
        assert_eq!(cands[0].ip, IpAddr::V4(Ipv4Addr::new(192, 168, 42, 10)));
    }

    #[test]
    fn ula_and_link_local_sorted() {
        let mut cands = vec![
            LanCandidate {
                iface: "wlan0".into(),
                ip: IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)),
            },
            LanCandidate {
                iface: "wlan0".into(),
                ip: IpAddr::V6(Ipv6Addr::new(0xfd12, 0, 0, 0, 0, 0, 0, 1)),
            },
        ];
        sort_candidates(&mut cands);
        assert!(matches!(cands[0].ip, IpAddr::V6(v) if v.octets()[0]==0xfd));
    }
}
