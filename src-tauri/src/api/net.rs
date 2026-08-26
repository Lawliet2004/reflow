use std::net::{IpAddr, Ipv4Addr};

use qrcode::render::svg;
use qrcode::QrCode;

pub fn lan_ipv4_addrs() -> Vec<String> {
    let mut addrs = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let IpAddr::V4(v4) = iface.ip() {
                if is_rfc1918(v4) {
                    addrs.push(v4.to_string());
                }
            }
        }
    }
    if addrs.is_empty() {
        if let Some(guess) = guessed_lan_ip() {
            addrs.push(guess);
        }
    }
    addrs.sort();
    addrs.dedup();
    addrs
}

pub fn is_rfc1918(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
}

fn guessed_lan_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip().to_string())
}

pub fn pair_uri(host: &str, port: u16, code: &str) -> String {
    format!("reflow://pair?host={host}&port={port}&code={code}&v=1")
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
    fn rfc1918_ranges() {
        assert!(is_rfc1918(Ipv4Addr::new(192, 168, 1, 10)));
        assert!(is_rfc1918(Ipv4Addr::new(10, 0, 0, 2)));
        assert!(!is_rfc1918(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_rfc1918(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn pair_uri_shape() {
        let uri = pair_uri("192.168.1.5", 7840, "123456");
        assert!(uri.contains("host=192.168.1.5"));
        assert!(uri.contains("code=123456"));
    }
}
