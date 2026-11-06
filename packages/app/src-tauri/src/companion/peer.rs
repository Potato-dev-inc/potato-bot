//! A companion's shell must not answer a card by plain HTTP either.
//!
//! `guard.rs` stops a companion's MCP session. But a companion (an agent with a shell) can also run
//! `curl http://127.0.0.1:<port>/questions/<id>/answer` over loopback, with no MCP session at all. Such a
//! request is "the local app's own tooling" to the bridge, so it needs another way to be told apart: the
//! TCP connection. For a loopback request the app asks the OS which process owns the other end of the
//! socket (`TcpOwner`, on Windows `GetExtendedTcpTable`), then walks that process up its parents with the
//! same code the recall check uses (`caller::decide`). A process in a companion pane's tree is refused with
//! `COMPANION_CANNOT_ANSWER`.
//!
//! Which requests: the loopback routes that answer or dismiss a card, type into a pane, or change a companion
//! (keep or drop a pending memory line, wake, sleep): `guard::is_loopback_guarded_route`. Not these: the desktop
//! window (Tauri commands, no HTTP), a paired phone (a bearer token: `requires_pairing_token`), and every route that
//! is not about a card or a companion.
//!
//! Companion territory is the companion's pane AND every pane started under it (its helpers, by the `parent_pane_id`
//! chain): a helper's shell is the companion's hand too, so its processes count as the companion's.
//!
//! A process the OS names that is in no companion territory is `Other` (today's behaviour) only when its chain of
//! parents is intact up to this app or the person's desktop. An orphan (its parent exited: `Start-Process`, `cmd /c
//! start /b`, a double fork) is not placed: it is treated like a caller nobody could identify.
//!
//! When the caller cannot be identified (no process table, a socket the OS table does not show, another OS, an
//! orphan): refuse only when the request is about a companion's own tree: a pane that is a companion's or has one up
//! its parent chain, a companion's id, the pane a run `handle` or a question names. Fail closed for companion
//! territory, and today's behaviour for the rest. The decisions are pure (`decide`, over a fake process table and a
//! fake TCP owner in the tests).

use std::net::SocketAddr;


/// Both ends of an accepted connection, as the server sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The server's own side (`TcpStream::local_addr`).
    pub local: SocketAddr,
    /// The client's side (`TcpStream::peer_addr`).
    pub remote: SocketAddr,
}

/// Which process owns the client end of a connection.
pub trait TcpOwner {
    /// The process id that owns the socket whose address is `peer.remote` and whose far end is `peer.local`.
    /// `None` when the OS does not say.
    fn owner_pid(&self, peer: &Peer) -> Option<u32>;
}

/// The OS's own table. Windows only today (`GetExtendedTcpTable`); elsewhere nobody is identified, which is
/// the "cannot identify" path of the module doc.
pub struct SystemTcp;

#[cfg(windows)]
impl TcpOwner for SystemTcp {
    fn owner_pid(&self, peer: &Peer) -> Option<u32> {
        windows_owner(peer)
    }
}

#[cfg(not(windows))]
impl TcpOwner for SystemTcp {
    fn owner_pid(&self, _peer: &Peer) -> Option<u32> {
        None
    }
}

#[cfg(windows)]
fn windows_owner(peer: &Peer) -> Option<u32> {
    use std::net::IpAddr;

    #[repr(C)]
    struct Row {
        state: u32,
        local_addr: u32,
        local_port: u32,
        remote_addr: u32,
        remote_port: u32,
        pid: u32,
    }
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetExtendedTcpTable(table: *mut core::ffi::c_void, size: *mut u32, order: i32, family: u32, class: u32, reserved: u32) -> u32;
    }
    const AF_INET: u32 = 2;
    const TCP_TABLE_OWNER_PID_ALL: u32 = 5;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    // Only IPv4 loopback: the bridge listens on 127.0.0.1.
    let (IpAddr::V4(client), IpAddr::V4(server)) = (peer.remote.ip(), peer.local.ip()) else { return None };

    // A u32 buffer keeps the rows aligned.
    let mut words: Vec<u32> = vec![0; 1024];
    for _ in 0..4 {
        let mut size = (words.len() * 4) as u32;
        // SAFETY: `words` holds `size` writable bytes; the call fills it or reports the size it needs.
        let code = unsafe { GetExtendedTcpTable(words.as_mut_ptr().cast(), &mut size, 0, AF_INET, TCP_TABLE_OWNER_PID_ALL, 0) };
        if code == ERROR_INSUFFICIENT_BUFFER {
            words = vec![0; (size as usize).div_ceil(4) + 64];
            continue;
        }
        if code != 0 {
            return None;
        }
        let count = words[0] as usize;
        let rows = words.get(1..)?;
        if rows.len() < count * 6 {
            return None;
        }
        let port = |raw: u32| u16::from_be((raw & 0xFFFF) as u16);
        let addr = |raw: u32| std::net::Ipv4Addr::from(u32::from_be(raw));
        for n in 0..count {
            // SAFETY: `rows` holds `count` rows of six u32 each (checked above); `Row` is six u32.
            let row = unsafe { &*(rows.as_ptr().add(n * 6) as *const Row) };
            // The client's own socket: its address is the peer address, its far end is our listener.
            if addr(row.local_addr) == client
                && port(row.local_port) == peer.remote.port()
                && addr(row.remote_addr) == server
                && port(row.remote_port) == peer.local.port()
            {
                return Some(row.pid);
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn the_windows_table_names_this_process_as_the_owner_of_its_own_loopback_client() {
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_side, _) = listener.accept().unwrap();
        // What the bridge sees for this connection.
        let seen = Peer { local: server_side.local_addr().unwrap(), remote: server_side.peer_addr().unwrap() };
        assert_eq!(seen.remote, client.local_addr().unwrap());
        assert_eq!(SystemTcp.owner_pid(&seen), Some(std::process::id()));
        // A connection that does not exist has no owner.
        let ghost = Peer { local: seen.local, remote: "127.0.0.1:1".parse().unwrap() };
        assert_eq!(SystemTcp.owner_pid(&ghost), None);
    }
}
