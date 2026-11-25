//! Progress as both sides report it, using the sender tests' folder and in-process pump.

use super::*;

fn total_of(sender: &Sender, items: &[u32]) -> u64 {
    let manifest = sender.manifest().items();
    manifest
        .iter()
        .filter(|i| items.is_empty() || items.contains(&i.item_id))
        .map(|i| i.size)
        .sum()
}

#[test]
fn both_sides_count_up_to_the_whole_transfer() {
    let mut t = setup(DEFAULT_WINDOW);
    let mut r = Receiver::open(t.sender.manifest().clone(), &t.dest, &[]).unwrap();
    let (sent, received) = (t.sender.progress(), r.progress());
    t.sender.accept(&accept_all()).unwrap();
    let total = total_of(&t.sender, &[]);
    assert_eq!((sent.bytes_done(), sent.bytes_total()), (0, total));
    assert_eq!((received.bytes_done(), received.bytes_total()), (0, total));

    pump(&mut t.sender, &mut r, |_| true);
    assert_eq!(sent.bytes_done(), total);
    assert_eq!(received.bytes_done(), total);
}

#[test]
fn only_the_selection_counts() {
    let mut t = setup(DEFAULT_WINDOW);
    let wanted = vec![2, 4, 5];
    let mut r = Receiver::open(t.sender.manifest().clone(), &t.dest, &wanted).unwrap();
    t.sender
        .accept(&Accept {
            transfer_id: ID,
            items: wanted.clone(),
        })
        .unwrap();
    let total = total_of(&t.sender, &wanted);
    assert_eq!(t.sender.progress().bytes_total(), total);
    assert_eq!(r.progress().bytes_total(), total);

    pump(&mut t.sender, &mut r, |_| true);
    assert_eq!(t.sender.progress().bytes_done(), total);
    assert_eq!(r.progress().bytes_done(), total);
}

#[test]
fn a_resume_starts_from_what_the_receiver_already_has() {
    let mut t = setup(DEFAULT_WINDOW);
    let manifest = t.sender.manifest().clone();
    let mut r = Receiver::open(manifest.clone(), &t.dest, &[]).unwrap();
    t.sender.accept(&accept_all()).unwrap();
    let mut delivered = 0;
    pump(&mut t.sender, &mut r, |_| {
        delivered += 1;
        delivered <= 3
    });
    let acknowledged = t.sender.progress().bytes_done();
    drop(r);

    // The reopened receiver counts what its journal holds, and the sender agrees on resume.
    let mut r = Receiver::open(manifest, &t.dest, &[]).unwrap();
    let held = r.progress().bytes_done();
    let total = total_of(&t.sender, &[]);
    assert!(held > 0 && held < total, "held {held} of {total}");
    assert_eq!(held, acknowledged);
    t.sender
        .resume(&Resume {
            transfer_id: ID,
            items: Vec::new(),
            have: r.have(),
        })
        .unwrap();
    assert_eq!(t.sender.progress().bytes_done(), held);

    pump(&mut t.sender, &mut r, |_| true);
    assert_eq!(t.sender.progress().bytes_done(), total);
    assert_eq!(r.progress().bytes_done(), total);
}
