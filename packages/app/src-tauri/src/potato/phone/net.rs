//! Who may connect, which Host names we answer to, and which addresses a phone can try.
//!
//! None of this is trust. A request is trusted only by its phone token. These checks keep the port from
//! answering the wrong callers at all: strangers on the internet (a router's port forward), and web pages
//! that reach us by a DNS-rebinding name.

use std::net::{IpAddr, Ipv4Addr};

/// Peers that may talk to the phone server: this machine (a tunnel program on it), and home or office networks.
/// Not the open internet: a public peer is dropped before a byte is read.
pub fn peer_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_private(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return v4_private(mapped);
            }
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn v4_private(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || o[0] == 10
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254)
        // 100.64.0.0/10: carrier-grade NAT, which phone hotspots and Tailscale use.
        || (o[0] == 100 && (64..=127).contains(&o[1]))
}

/// The host part of a `Host` header or an origin's authority: no port, no brackets, lowercase, no trailing dot.
pub fn host_only(authority: &str) -> String {
    let a = authority.trim();
    let host = if let Some(rest) = a.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else if a.matches(':').count() == 1 {
        a.split(':').next().unwrap_or("")
    } else {
        a
    };
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// The names this server answers to. Anything else in a `Host` header is refused with 421.
#[derive(Debug, Clone, Default)]
pub struct Hosts {
    /// The host of the person's own address (reverse proxy or named tunnel), if set.
    pub custom: Option<String>,
    /// The host of the running quick tunnel, if any.
    pub tunnel: Option<String>,
    /// This computer's name.
    pub machine: Option<String>,
    /// The quick tunnel is on but its address is not known yet: the tunnel's own first health check arrives with
    /// the new name, so a quick-tunnel name is let in until the address is known. Only this PC's tunnel can carry such a
    /// request to this port.
    pub tunnel_starting: bool,
}

impl Hosts {
    pub fn allows(&self, authority: &str) -> bool {
        let host = host_only(authority);
        if host.is_empty() {
            return false;
        }
        if host.parse::<IpAddr>().is_ok() || host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
            return true;
        }
        if self.tunnel_starting && host.ends_with(".trycloudflare.com") && host.len() > ".trycloudflare.com".len() {
            return true;
        }
        [&self.custom, &self.tunnel, &self.machine].into_iter().flatten().any(|h| *h == host)
    }
}

/// `scheme://host[:port]` of a candidate or an `Origin` header, lowercase, no path. `None` when it is not an origin.
pub fn origin_of(value: &str) -> Option<String> {
    let v = value.trim();
    let (scheme, rest) = v.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

/// What the person typed as "my own address", made into an origin, and a plain warning when it is not safe.
pub fn normalize_custom_address(raw: &str) -> Result<(String, Option<String>), String> {
    let bad = || "PHONE_BAD_ADDRESS: Use an address like https://bot.example.com or http://myhost:8123.".to_string();
    let trimmed = raw.trim();
    let (scheme_raw, rest) = trimmed.split_once("://").ok_or_else(bad)?;
    let scheme = scheme_raw.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(bad());
    }
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains(['/', '?', '#', '@', ' ']) {
        return Err(bad());
    }
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !rest.ends_with(']') && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h.to_string(), Some(p.to_string())),
        _ => (rest.to_string(), None),
    };
    let bare = host_only(&host);
    if bare.is_empty() || !bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':') {
        return Err(bad());
    }
    if let Some(p) = &port {
        if p.parse::<u16>().ok().filter(|p| *p > 0).is_none() {
            return Err(bad());
        }
    }
    let origin = match &port {
        Some(p) => format!("{scheme}://{bare}:{p}"),
        None => format!("{scheme}://{bare}"),
    };
    let warning = (scheme == "http" && !looks_like_home_network(&bare)).then(|| {
        "This address uses plain http outside your home network. Anyone on the way can read your phone's token and your chats. Use https (a Cloudflare tunnel or a reverse proxy with a certificate).".to_string()
    });
    Ok((origin, warning))
}

/// A name or address that is on a home network: a private IP, localhost, `.local`, or a bare machine name.
fn looks_like_home_network(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return peer_allowed(ip);
    }
    host == "localhost" || host.ends_with(".local") || host.ends_with(".localhost") || !host.contains('.')
}

/// IPv4 addresses on lines of `ipconfig` output. The word "IPv4" is in every language's output; the
/// number is the last dotted quad on the line. Loopback and link-local addresses are left out.
pub fn parse_ipconfig(text: &str) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.contains("IPv4") {
            continue;
        }
        let Some(quad) = line.split(|c: char| !(c.is_ascii_digit() || c == '.')).filter(|s| s.matches('.').count() == 3).last() else { continue };
        if let Ok(ip) = quad.parse::<Ipv4Addr>() {
            if !ip.is_loopback() && !ip.is_unspecified() && !(ip.octets()[0] == 169 && ip.octets()[1] == 254) && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// Home addresses first: 192.168, then 10, then 172.16/12, then the rest; the address the OS would use to reach
/// the internet (`primary`) leads its group.
pub fn rank_lan(mut ips: Vec<Ipv4Addr>, primary: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    fn class(ip: &Ipv4Addr) -> u8 {
        let o = ip.octets();
        match (o[0], o[1]) {
            (192, 168) => 0,
            (10, _) => 1,
            (172, 16..=31) => 2,
            _ => 3,
        }
    }
    if let Some(p) = primary {
        if !ips.contains(&p) && !p.is_loopback() && !p.is_unspecified() {
            ips.push(p);
        }
    }
    ips.sort_by_key(|ip| (class(ip), Some(*ip) != primary));
    ips
}

/// This computer's IPv4 addresses on its networks, best first. Reads `ipconfig` on Windows (hidden), and asks
/// the OS which address it would use to leave the network. Never fails: it may come back empty.
pub fn lan_ipv4s() -> Vec<Ipv4Addr> {
    let primary = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").map(|_| s))
        .and_then(|s| s.local_addr())
        .ok()
        .and_then(|a| match a.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        });
    #[cfg(windows)]
    let found = crate::proc_util::hidden_command("ipconfig").output().map(|o| parse_ipconfig(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
    #[cfg(not(windows))]
    let found = Vec::new();
    rank_lan(found, primary)
}

/// `http://<ip>:<port>` for each address.
pub fn lan_origins(ips: &[Ipv4Addr], port: u16) -> Vec<String> {
    ips.iter().map(|ip| format!("http://{ip}:{port}")).collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_this_machine_and_private_networks_may_connect() {
        for ok in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.9", "169.254.1.1", "100.64.0.1", "::1", "fd12::1", "fe80::1", "::ffff:192.168.1.5"] {
            assert!(peer_allowed(ip(ok)), "{ok}");
        }
        for bad in ["8.8.8.8", "172.32.0.1", "172.15.0.1", "100.128.0.1", "192.169.0.1", "2001:db8::1", "::ffff:8.8.8.8", "1.1.1.1"] {
            assert!(!peer_allowed(ip(bad)), "{bad}");
        }
    }

    #[test]
    fn host_names_are_an_allow_list() {
        let hosts = Hosts { custom: Some("bot.example.com".into()), tunnel: Some("quiet-fox.trycloudflare.com".into()), machine: Some("ren-pc".into()), tunnel_starting: false };
        for ok in ["192.168.1.5:1420", "localhost:1420", "[::1]:1420", "10.0.0.2", "phone.local", "bot.example.com", "BOT.example.com:443", "quiet-fox.trycloudflare.com", "ren-pc:1420"] {
            assert!(hosts.allows(ok), "{ok}");
        }
        for bad in ["evil.example.com", "evil.com:1420", "bot.example.com.evil.com", "trycloudflare.com", "other.trycloudflare.com", "", "ren-pc.evil.com"] {
            assert!(!hosts.allows(bad), "{bad}");
        }
        let starting = Hosts { tunnel_starting: true, ..Hosts::default() };
        assert!(starting.allows("new-words.trycloudflare.com") && !starting.allows("trycloudflare.com") && !starting.allows("evil.com") && !starting.allows("x.trycloudflare.com.evil.com"));
        assert!(!hosts.allows("other-fox.trycloudflare.com"), "not while the tunnel is not starting");
        let none = Hosts::default();
        assert!(!none.allows("bot.example.com"), "a custom host counts only when it is set");
    }

    #[test]
    fn origins_are_compared_by_scheme_host_and_port() {
        assert_eq!(origin_of("HTTP://192.168.1.5:1420/path?x"), Some("http://192.168.1.5:1420".into()));
        assert_eq!(origin_of("https://a.b"), Some("https://a.b".into()));
        assert_eq!(origin_of("null"), None);
        assert_eq!(origin_of("ftp://x"), None);
        assert_eq!(origin_of("https://user@x"), None);
    }

    #[test]
    fn a_custom_address_is_cleaned_and_plain_http_outside_the_home_is_warned() {
        let (o, w) = normalize_custom_address(" https://Bot.Example.com/ ").unwrap();
        assert_eq!((o.as_str(), w), ("https://bot.example.com", None));
        let (o, w) = normalize_custom_address("http://myhost:8123").unwrap();
        assert_eq!((o.as_str(), w), ("http://myhost:8123", None));
        let (_, w) = normalize_custom_address("http://192.168.1.4:8123").unwrap();
        assert!(w.is_none());
        let (_, w) = normalize_custom_address("http://bot.example.com").unwrap();
        assert!(w.unwrap().contains("plain http"));
        let (_, w) = normalize_custom_address("http://8.8.8.8:80").unwrap();
        assert!(w.is_some());
        for bad in ["", "bot.example.com", "ftp://x", "https://a.b/path", "https://u@a.b", "https://a b", "https://a.b:0", "https://a.b:99999", "https://"] {
            assert!(normalize_custom_address(bad).unwrap_err().starts_with("PHONE_BAD_ADDRESS:"), "{bad}");
        }
    }

    #[test]
    fn ipconfig_lines_become_addresses_in_any_language() {
        let text = "Ethernet adapter Wi-Fi:\r\n   IPv4 Address. . . . . . . . . . . : 192.168.1.23\r\n   Subnet Mask . . . . . . . . . . . : 255.255.255.0\r\n\
                    Ethernet adapter vEthernet:\r\n   IPv4 位址 . . . . . . . . . . . . : 172.20.0.1\r\n   Autoconfiguration IPv4 Address. . : 169.254.3.3\r\n   IPv4 Address. . . : 192.168.1.23\r\n";
        assert_eq!(parse_ipconfig(text), vec!["192.168.1.23".parse::<Ipv4Addr>().unwrap(), "172.20.0.1".parse().unwrap()]);
    }

    #[test]
    fn the_home_address_comes_first() {
        let ips = vec!["172.20.0.1".parse().unwrap(), "10.0.0.5".parse().unwrap(), "192.168.1.23".parse().unwrap()];
        let ranked = rank_lan(ips, Some("10.0.0.5".parse().unwrap()));
        assert_eq!(ranked[0].to_string(), "192.168.1.23");
        assert_eq!(ranked[2].to_string(), "172.20.0.1");
        assert_eq!(lan_origins(&ranked[..1], 1420), vec!["http://192.168.1.23:1420"]);
    }
}
