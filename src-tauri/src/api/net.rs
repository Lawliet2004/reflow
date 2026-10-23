use std::net::{IpAddr, Ipv4Addr};

use qrcode::render::svg;
use qrcode::QrCode;

pub fn lan_ipv4_addrs() -> Vec<String> {
    let mut addrs = Vec::new();
    let routed = guessed_lan_ip();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let IpAddr::V4(v4) = iface.ip() {
                if is_rfc1918(v4) {
                    addrs.push((
                        interface_rank(&iface.name, &v4.to_string(), routed.as_deref()),
                        v4.to_string(),
                    ));
                }
            }
        }
    }
    if addrs.is_empty() {
        if let Some(guess) = guessed_lan_ip() {
            addrs.push((0, guess));
        }
    }
    addrs.sort();
    addrs.dedup();
    addrs.into_iter().map(|(_, address)| address).collect()
}

fn interface_rank(name: &str, address: &str, routed: Option<&str>) -> u8 {
    let name = name.to_ascii_lowercase();
    if [
        "docker",
        "veth",
        "virbr",
        "br-",
        "vmnet",
        "vbox",
        "tun",
        "tap",
        "wg",
        "tailscale",
        "utun",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
    {
        3
    } else if routed == Some(address) {
        0
    } else {
        1
    }
}

pub fn is_rfc1918(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
}

fn guessed_lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().and_then(|addr| match addr.ip() {
        IpAddr::V4(ip) if is_rfc1918(ip) => Some(ip.to_string()),
        _ => None,
    })
}

pub fn pair_uri(host: &str, port: u16, code: &str, fingerprint: &str) -> String {
    format!("reflow://pair?host={host}&port={port}&code={code}&v=2&cert_sha256={fingerprint}")
}

pub fn qr_svg(payload: &str) -> Option<String> {
    let code = QrCode::new(payload.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color<'_>>()
            .min_dimensions(128, 128)
            .build(),
    )
}

pub fn bind_ip(mode: &str) -> &'static str {
    if mode.eq_ignore_ascii_case("localhost") {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_lan_wins_over_docker_vm_and_vpn_even_if_vpn_owns_default_route() {
        let wifi = interface_rank("wlan0", "192.168.1.2", Some("10.0.0.2"));
        for interface in ["docker0", "veth123", "virbr0", "tun0", "wg0", "utun3"] {
            assert!(wifi < interface_rank(interface, "10.0.0.2", Some("10.0.0.2")));
        }
        assert_eq!(
            interface_rank("eth0", "192.168.1.2", Some("192.168.1.2")),
            0
        );
    }

    #[test]
    fn rfc1918_ranges() {
        assert!(is_rfc1918(Ipv4Addr::new(192, 168, 1, 10)));
        assert!(is_rfc1918(Ipv4Addr::new(10, 0, 0, 2)));
        assert!(!is_rfc1918(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_rfc1918(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn pair_uri_shape() {
        let uri = pair_uri("192.168.1.5", 7840, "123456", &"a".repeat(64));
        assert!(uri.contains("host=192.168.1.5"));
        assert!(uri.contains("code=123456"));
        assert!(uri.contains("v=2&cert_sha256="));
    }
}
