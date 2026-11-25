//! Abuse limits for senders that aren't trusted devices (docs/security.md, Abuse): each
//! address may make a few unknown offers a minute, and an address whose offers keep being
//! declined is blocked for a while. Kept per IP address, since anyone can make new keys.
//! Trusted devices are never limited, and their declines don't count.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Unknown offers allowed from one address per [`OFFER_WINDOW`].
pub(crate) const UNKNOWN_OFFERS_PER_WINDOW: usize = 5;
const OFFER_WINDOW: Duration = Duration::from_secs(60);

/// Declines within [`DECLINE_WINDOW`] that block an address for [`BLOCK_FOR`].
pub(crate) const DECLINES_BEFORE_BLOCK: usize = 3;
const DECLINE_WINDOW: Duration = Duration::from_mins(10);
pub(crate) const BLOCK_FOR: Duration = Duration::from_mins(10);

/// Addresses remembered at most; the stalest are forgotten first.
const MAX_ADDRESSES: usize = 1024;

#[derive(Debug, Default)]
struct Record {
    offers: VecDeque<Instant>,
    declines: VecDeque<Instant>,
    blocked_until: Option<Instant>,
    last_seen: Option<Instant>,
}

impl Record {
    fn forget_before(&mut self, now: Instant) {
        drop_older(&mut self.offers, now, OFFER_WINDOW);
        drop_older(&mut self.declines, now, DECLINE_WINDOW);
        if self.blocked_until.is_some_and(|until| until <= now) {
            self.blocked_until = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.offers.is_empty() && self.declines.is_empty() && self.blocked_until.is_none()
    }
}

fn drop_older(times: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while times
        .front()
        .is_some_and(|&at| now.saturating_duration_since(at) >= window)
    {
        times.pop_front();
    }
}

/// What the listener knows about each address that sent unknown offers.
#[derive(Debug, Default)]
pub(crate) struct SenderGate {
    addresses: HashMap<IpAddr, Record>,
}

impl SenderGate {
    /// Whether connections from `ip` are refused right now.
    pub(crate) fn is_blocked(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.tidy(ip, now)
            .is_some_and(|record| record.blocked_until.is_some())
    }

    /// Counts an offer from an unknown device at `ip`; false if it is over the limit and
    /// should be dropped without bothering anyone.
    pub(crate) fn admit_unknown_offer(&mut self, ip: IpAddr, now: Instant) -> bool {
        let record = self.record(ip, now);
        if record.blocked_until.is_some() || record.offers.len() >= UNKNOWN_OFFERS_PER_WINDOW {
            return false;
        }
        record.offers.push_back(now);
        true
    }

    /// Counts a declined unknown offer from `ip`, blocking it after too many.
    pub(crate) fn declined(&mut self, ip: IpAddr, now: Instant) {
        let record = self.record(ip, now);
        record.declines.push_back(now);
        if record.declines.len() >= DECLINES_BEFORE_BLOCK {
            record.declines.clear();
            record.blocked_until = Some(now + BLOCK_FOR);
        }
    }

    /// The record for `ip` with expired entries removed, if there is anything left.
    fn tidy(&mut self, ip: IpAddr, now: Instant) -> Option<&mut Record> {
        let record = self.addresses.get_mut(&ip)?;
        record.forget_before(now);
        if record.is_empty() {
            self.addresses.remove(&ip);
            return None;
        }
        self.addresses.get_mut(&ip)
    }

    fn record(&mut self, ip: IpAddr, now: Instant) -> &mut Record {
        if !self.addresses.contains_key(&ip) && self.addresses.len() >= MAX_ADDRESSES {
            self.forget_stalest(now);
        }
        let record = self.addresses.entry(ip).or_default();
        record.forget_before(now);
        record.last_seen = Some(now);
        record
    }

    fn forget_stalest(&mut self, now: Instant) {
        self.addresses.retain(|_, record| {
            record.forget_before(now);
            !record.is_empty()
        });
        while self.addresses.len() >= MAX_ADDRESSES {
            let Some(stalest) = self
                .addresses
                .iter()
                .min_by_key(|(_, record)| record.last_seen)
                .map(|(&ip, _)| ip)
            else {
                break;
            };
            self.addresses.remove(&stalest);
        }
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
