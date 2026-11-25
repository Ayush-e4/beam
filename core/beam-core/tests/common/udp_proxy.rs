//! A UDP relay between a QUIC client and server that loses, delays and reorders packets, and
//! cuts the link for good after a byte budget.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use super::fault::Rng;

/// How bad the link is.
#[derive(Debug, Clone)]
pub struct LinkProfile {
    /// Packets dropped, in percent.
    pub loss_percent: u64,
    /// Packets held back 1–20 ms so later ones overtake them, in percent.
    pub reorder_percent: u64,
    /// Bytes relayed (both directions) before the link dies.
    pub budget: u64,
}

/// A running relay; clients connect to [`Proxy::addr`]. Stops when dropped.
pub struct Proxy {
    pub addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    /// Relays to `server`. `on_cut` runs once when the budget is spent.
    pub async fn start(
        server: SocketAddr,
        profile: LinkProfile,
        rng: Rng,
        on_cut: impl FnOnce() + Send + 'static,
    ) -> Self {
        let front = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        let back = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        back.connect(server).await.unwrap();
        let addr = front.local_addr().unwrap();
        let task = tokio::spawn(relay(front, back, profile, rng, on_cut));
        Self { addr, task }
    }
}

async fn relay(
    front: Arc<UdpSocket>,
    back: Arc<UdpSocket>,
    profile: LinkProfile,
    mut rng: Rng,
    on_cut: impl FnOnce(),
) {
    let (mut up, mut down) = (vec![0u8; 65_536], vec![0u8; 65_536]);
    let mut client = None;
    let mut relayed: u64 = 0;
    loop {
        let (packet, to_server) = tokio::select! {
            Ok((len, from)) = front.recv_from(&mut up) => {
                client = Some(from);
                (up[..len].to_vec(), true)
            }
            Ok(len) = back.recv(&mut down) => (down[..len].to_vec(), false),
            else => return,
        };
        relayed += packet.len() as u64;
        if relayed > profile.budget {
            on_cut();
            return; // the link is gone; nothing more gets through
        }
        if rng.percent(profile.loss_percent) {
            continue;
        }
        let delay = rng
            .percent(profile.reorder_percent)
            .then(|| Duration::from_millis(rng.between(1..20)));
        let (socket, dest) = if to_server {
            (Arc::clone(&back), None)
        } else {
            (Arc::clone(&front), client)
        };
        let send = async move {
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            // A send that fails is just another lost packet.
            let _ = match dest {
                Some(dest) => socket.send_to(&packet, dest).await,
                None => socket.send(&packet).await,
            };
        };
        if delay.is_some() {
            tokio::spawn(send);
        } else {
            send.await;
        }
    }
}
