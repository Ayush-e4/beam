//! The listener's background half. It accepts connections and, for many devices at once, runs
//! the handshake and reads the offer, queueing the offers the app should see. A device that
//! connects and stays silent only holds its own slot, never the whole listener.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use beam_core::tcp::{self, TcpAcceptor};
use beam_core::{Cipher, Handshake, confirm_received, read_offer, reject_offer};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::engine::Finished;
use crate::gate::{SenderGate, lock};
use crate::incoming::{IncomingOffer, Sender};
use crate::{BeamError, DeclineReason, DeviceIdentity, TrustedKeys};

/// How long a connected device has to send its offer.
const OFFER_TIMEOUT: Duration = Duration::from_secs(30);

/// Pause after accepting fails, so a broken socket can't spin.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// Connections in their handshake or offer at once, in total and from one address. More are
/// closed straight away.
pub(crate) const MAX_IN_PROGRESS: usize = 32;
pub(crate) const MAX_PER_ADDRESS: usize = 4;

/// Offers ready for the app, waiting for `next_offer`.
pub(crate) const QUEUED_OFFERS: usize = 8;

/// What the queue carries: an offer, or the error that stopped the listener.
pub(crate) type Queued = Result<Arc<IncomingOffer>, BeamError>;

/// Everything a connection needs to become an offer.
pub(crate) struct Inbox {
    pub(crate) handle: Handle,
    pub(crate) acceptor: TcpAcceptor,
    pub(crate) identity: Arc<DeviceIdentity>,
    pub(crate) cipher: Cipher,
    pub(crate) finished: Arc<Finished>,
    pub(crate) trusted: Arc<dyn TrustedKeys>,
    pub(crate) gate: Arc<Mutex<SenderGate>>,
}

impl Inbox {
    /// Accepts until aborted; aborting also drops every connection still in progress.
    pub(crate) async fn run(self: Arc<Self>, queue: mpsc::Sender<Queued>) {
        let counts = Arc::new(Mutex::new(InProgress::default()));
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = self.acceptor.accept_socket() => {
                    let Ok((stream, from)) = accepted else {
                        tokio::time::sleep(RETRY_PAUSE).await;
                        continue;
                    };
                    // Dropping the stream closes it before any handshake work.
                    if lock(&self.gate).is_blocked(from.ip(), Instant::now()) {
                        continue;
                    }
                    let Some(slot) = Slot::claim(&counts, from.ip()) else {
                        continue;
                    };
                    let handshake =
                        match Handshake::known_responder(self.identity.keypair(), self.cipher) {
                            Ok(handshake) => handshake,
                            Err(error) => {
                                // This device's own key is unusable: nothing can be received.
                                // If the send fails, the listener is closed and nobody waits.
                                let _ = queue.send(Err(error.into())).await;
                                return;
                            }
                        };
                    let (inbox, queue) = (self.clone(), queue.clone());
                    connections.spawn(async move {
                        if let Some(offer) = inbox.admit(stream, from, handshake).await {
                            // Keeps its slot while the queue is full, so the caps still hold.
                            // Fails only once the listener closed, which drops the offer.
                            let _ = queue.send(Ok(offer)).await;
                        }
                        drop(slot);
                    });
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    }

    /// Turns a connection into an offer for the app, or `None` if it should go no further.
    async fn admit(
        &self,
        stream: TcpStream,
        from: SocketAddr,
        handshake: Handshake,
    ) -> Option<Arc<IncomingOffer>> {
        // A failed handshake is not a Beam device, or one that doesn't know this device's key.
        let mut conn = tcp::respond(stream, handshake).await.ok()?;
        let manifest = timeout(OFFER_TIMEOUT, read_offer(&mut conn))
            .await
            .ok()?
            .ok()?;
        if self.finished.contains(manifest.transfer_id()) {
            // A repeat of a saved transfer: confirm so the sender finishes. If that fails, it
            // asks again on its next attempt.
            let _ = confirm_received(&mut conn, &manifest).await;
            return None;
        }
        let trusted = self.trusted.is_trusted(conn.peer().as_bytes().to_vec());
        if !trusted && !lock(&self.gate).admit_unknown_offer(from.ip(), Instant::now()) {
            // Best effort: a sender over its limit learns it should wait.
            let busy = DeclineReason::Busy.into();
            let _ = reject_offer(&mut conn, manifest.transfer_id(), busy).await;
            return None;
        }
        let sender = Sender {
            ip: from.ip(),
            trusted,
            gate: self.gate.clone(),
        };
        let finished = self.finished.clone();
        Some(IncomingOffer::new(
            self.handle.clone(),
            conn,
            manifest,
            finished,
            sender,
        ))
    }
}

/// Connections in progress, in total and per address.
#[derive(Debug, Default)]
struct InProgress {
    total: usize,
    per_address: HashMap<IpAddr, usize>,
}

/// One connection's place in [`InProgress`], given back when dropped.
#[derive(Debug)]
struct Slot {
    counts: Arc<Mutex<InProgress>>,
    ip: IpAddr,
}

impl Slot {
    /// A slot for a connection from `ip`, or `None` if either cap is reached.
    fn claim(counts: &Arc<Mutex<InProgress>>, ip: IpAddr) -> Option<Self> {
        let mut now = counts.lock().unwrap_or_else(PoisonError::into_inner);
        let here = now.per_address.get(&ip).copied().unwrap_or(0);
        if now.total >= MAX_IN_PROGRESS || here >= MAX_PER_ADDRESS {
            return None;
        }
        now.total += 1;
        *now.per_address.entry(ip).or_default() += 1;
        Some(Self {
            counts: counts.clone(),
            ip,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut now = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        now.total = now.total.saturating_sub(1);
        if let Entry::Occupied(mut here) = now.per_address.entry(self.ip) {
            *here.get_mut() -= 1;
            if *here.get() == 0 {
                here.remove();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))
    }

    #[test]
    fn slots_are_capped_per_address_and_in_total() {
        let counts = Arc::new(Mutex::new(InProgress::default()));
        let mine: Vec<_> = (0..MAX_PER_ADDRESS)
            .map(|_| Slot::claim(&counts, ip(1)).unwrap())
            .collect();
        assert!(Slot::claim(&counts, ip(1)).is_none());
        drop(mine);
        assert!(counts.lock().unwrap().per_address.is_empty());

        let all: Vec<_> = (0..MAX_IN_PROGRESS)
            .map(|n| Slot::claim(&counts, ip(u8::try_from(n).unwrap())).unwrap())
            .collect();
        assert!(Slot::claim(&counts, ip(200)).is_none());
        drop(all);
        assert_eq!(counts.lock().unwrap().total, 0);
        assert!(Slot::claim(&counts, ip(200)).is_some());
    }
}
