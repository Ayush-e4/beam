use std::net::Ipv4Addr;

use super::*;

fn ip(n: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, n))
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn unknown_offers_are_limited_per_address_and_minute() {
    let (mut gate, start) = (SenderGate::default(), Instant::now());
    for _ in 0..UNKNOWN_OFFERS_PER_WINDOW {
        assert!(gate.admit_unknown_offer(ip(1), start));
    }
    assert!(!gate.admit_unknown_offer(ip(1), start + secs(1)));
    assert!(
        gate.admit_unknown_offer(ip(2), start + secs(1)),
        "other addresses unaffected"
    );
    assert!(
        gate.admit_unknown_offer(ip(1), start + secs(60)),
        "a minute later it's allowed"
    );
}

#[test]
fn repeated_declines_block_an_address_for_a_while() {
    let (mut gate, start) = (SenderGate::default(), Instant::now());
    for n in 0..DECLINES_BEFORE_BLOCK as u64 {
        assert!(!gate.is_blocked(ip(1), start + secs(n)));
        gate.declined(ip(1), start + secs(n));
    }
    let blocked_at = start + secs(DECLINES_BEFORE_BLOCK as u64 - 1);
    assert!(gate.is_blocked(ip(1), blocked_at));
    assert!(!gate.admit_unknown_offer(ip(1), blocked_at + secs(1)));
    assert!(!gate.is_blocked(ip(2), blocked_at));
    assert!(gate.is_blocked(
        ip(1),
        (blocked_at + BLOCK_FOR).checked_sub(secs(1)).unwrap()
    ));
    assert!(!gate.is_blocked(ip(1), blocked_at + BLOCK_FOR));
    assert!(gate.admit_unknown_offer(ip(1), blocked_at + BLOCK_FOR));
}

#[test]
fn old_declines_expire() {
    let (mut gate, start) = (SenderGate::default(), Instant::now());
    gate.declined(ip(1), start);
    gate.declined(ip(1), start + secs(1));
    gate.declined(ip(1), start + secs(11 * 60));
    assert!(!gate.is_blocked(ip(1), start + secs(11 * 60)));
}

#[test]
fn quiet_addresses_are_forgotten() {
    let (mut gate, start) = (SenderGate::default(), Instant::now());
    gate.admit_unknown_offer(ip(1), start);
    assert!(!gate.is_blocked(ip(1), start + secs(61)));
    assert!(gate.addresses.is_empty());
}

#[test]
fn memory_is_bounded() {
    let (mut gate, start) = (SenderGate::default(), Instant::now());
    for n in 0..=u32::try_from(MAX_ADDRESSES).unwrap() {
        let address = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + n));
        gate.admit_unknown_offer(address, start + Duration::from_millis(u64::from(n)));
    }
    assert_eq!(gate.addresses.len(), MAX_ADDRESSES);
    assert!(
        !gate
            .addresses
            .contains_key(&IpAddr::V4(Ipv4Addr::from(0x0a00_0000)))
    );
}
