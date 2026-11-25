//! Discovery beacons for the app's mDNS announcements: making this device's and recognising
//! trusted devices' ones.

use beam_core::{Beacon, DiscoveryKey};

/// The index in `keys` (trusted devices' stored discovery keys) of the device that announced
/// `beacon`, or `None` for anyone else: an unknown device, another app's service or a beacon
/// that isn't well formed. Keys that aren't 32 bytes are skipped.
#[uniffi::export]
#[must_use]
pub fn recognise_beacon(beacon: &str, keys: Vec<Vec<u8>>) -> Option<u32> {
    let beacon = Beacon::parse(beacon).ok()?;
    keys.into_iter()
        .position(|key| DiscoveryKey::from_slice(&key).is_ok_and(|key| beacon.matches(&key)))
        .and_then(|index| u32::try_from(index).ok())
}

#[cfg(test)]
mod tests {
    use beam_core::StaticKeypair;

    use super::*;

    #[test]
    fn finds_the_device_that_announced_it() {
        let key = |n| DiscoveryKey::derive(&StaticKeypair::generate().unwrap(), n);
        let (a, b) = (key(0), key(0));
        let beacon = Beacon::new(&b).unwrap().encode();
        let keys = vec![vec![1; 5], a.as_bytes().to_vec(), b.as_bytes().to_vec()];
        assert_eq!(recognise_beacon(&beacon, keys), Some(2));
        assert_eq!(recognise_beacon(&beacon, vec![a.as_bytes().to_vec()]), None);
        assert_eq!(
            recognise_beacon("My Printer", vec![b.as_bytes().to_vec()]),
            None
        );
    }
}
