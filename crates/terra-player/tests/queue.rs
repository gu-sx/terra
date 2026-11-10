//! Every operation of the pure queue.

use terra_player::{Queue, QueueItemId, Repeat};

/// A queue of `names`, with no current item.
fn queue(names: &[&'static str]) -> (Queue<&'static str>, Vec<QueueItemId>) {
    let mut queue = Queue::new();
    let ids = names.iter().map(|name| queue.append(*name)).collect();
    (queue, ids)
}

fn order(queue: &Queue<&'static str>) -> Vec<&'static str> {
    queue.iter().map(|(_, name)| *name).collect()
}

fn current(queue: &Queue<&'static str>) -> Option<&'static str> {
    queue.current().map(|(_, name)| *name)
}

#[test]
fn a_new_queue_is_empty_with_no_current_item_and_repeat_off() {
    let mut queue: Queue<&str> = Queue::new();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.current(), None);
    assert_eq!(queue.repeat(), Repeat::Off);
    assert_eq!(queue.next(), None);
    assert_eq!(queue.previous(), None);
}

#[test]
fn append_adds_at_the_end_with_a_new_id_each_time() {
    let (queue, ids) = queue(&["a", "b", "a"]);
    assert_eq!(order(&queue), ["a", "b", "a"]);
    assert_eq!(queue.len(), 3);
    assert_ne!(ids[0], ids[2]);
    assert_eq!(queue.get(ids[1]), Some(&"b"));
    assert!(queue.contains(ids[2]));
}

#[test]
fn next_starts_at_the_first_item_and_walks_the_queue() {
    let (mut queue, ids) = queue(&["a", "b"]);
    assert_eq!(queue.peek_next(), Some(ids[0]));
    assert_eq!(queue.next(), Some(ids[0]));
    assert_eq!(current(&queue), Some("a"));
    assert_eq!(queue.current_id(), Some(ids[0]));
    assert_eq!(queue.next(), Some(ids[1]));
    assert_eq!(current(&queue), Some("b"));
}

#[test]
fn next_at_the_end_without_repeat_leaves_no_current_item() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.select(ids[1]);
    assert_eq!(queue.peek_next(), None);
    assert_eq!(queue.next(), None);
    assert_eq!(queue.current(), None);
    // It stays past the end, and an item appended there comes next.
    assert_eq!(queue.next(), None);
    let c = queue.append("c");
    assert_eq!(queue.next(), Some(c));
}

#[test]
fn next_at_the_end_with_repeat_wraps_to_the_first_item() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.set_repeat(Repeat::All);
    assert_eq!(queue.repeat(), Repeat::All);
    queue.select(ids[1]);
    assert_eq!(queue.peek_next(), Some(ids[0]));
    assert_eq!(queue.next(), Some(ids[0]));
    assert_eq!(current(&queue), Some("a"));
}

#[test]
fn repeat_on_a_single_item_repeats_it() {
    let (mut queue, ids) = queue(&["a"]);
    queue.set_repeat(Repeat::All);
    queue.next();
    assert_eq!(queue.next(), Some(ids[0]));
}

#[test]
fn previous_goes_back_and_stops_at_the_first_item_without_repeat() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.select(ids[1]);
    assert_eq!(queue.previous(), Some(ids[0]));
    assert_eq!(queue.previous(), None);
    assert_eq!(current(&queue), Some("a"));
}

#[test]
fn previous_at_the_first_item_with_repeat_wraps_to_the_last() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.set_repeat(Repeat::All);
    queue.select(ids[0]);
    assert_eq!(queue.previous(), Some(ids[1]));
}

#[test]
fn previous_past_the_end_goes_to_the_last_item() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.select(ids[1]);
    queue.next();
    assert_eq!(queue.previous(), Some(ids[1]));
}

#[test]
fn insert_next_puts_the_item_after_the_current_one() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.select(ids[0]);
    let c = queue.insert_next("c");
    assert_eq!(order(&queue), ["a", "c", "b"]);
    assert_eq!(current(&queue), Some("a"));
    assert_eq!(queue.next(), Some(c));
}

#[test]
fn insert_next_with_no_current_item_puts_it_first_to_play() {
    let (mut queue, _) = queue(&["a", "b"]);
    let c = queue.insert_next("c");
    assert_eq!(order(&queue), ["c", "a", "b"]);
    assert_eq!(queue.next(), Some(c));
}

#[test]
fn remove_returns_the_item_and_keeps_the_current_one() {
    let (mut queue, ids) = queue(&["a", "b", "c"]);
    queue.select(ids[2]);
    assert_eq!(queue.remove(ids[0]), Some("a"));
    assert_eq!(order(&queue), ["b", "c"]);
    assert_eq!(current(&queue), Some("c"));
    assert_eq!(queue.remove(ids[0]), None);
}

#[test]
fn removing_the_current_item_leaves_the_one_that_followed_next() {
    let (mut queue, ids) = queue(&["a", "b", "c"]);
    queue.select(ids[1]);
    assert_eq!(queue.remove(ids[1]), Some("b"));
    assert_eq!(queue.current(), None);
    assert_eq!(queue.peek_next(), Some(ids[2]));
    assert_eq!(queue.next(), Some(ids[2]));
}

#[test]
fn removing_the_last_current_item_leaves_the_queue_at_its_end() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.select(ids[1]);
    queue.remove(ids[1]);
    assert_eq!(queue.next(), None);
    queue.set_repeat(Repeat::All);
    assert_eq!(queue.next(), Some(ids[0]));
}

#[test]
fn removing_the_only_item_leaves_an_empty_queue() {
    let (mut queue, ids) = queue(&["a"]);
    queue.next();
    queue.remove(ids[0]);
    assert!(queue.is_empty());
    assert_eq!(queue.current(), None);
    assert_eq!(queue.next(), None);
}

#[test]
fn move_to_reorders_and_the_current_item_stays_current() {
    let (mut queue, ids) = queue(&["a", "b", "c", "d"]);
    queue.select(ids[1]);
    assert!(queue.move_to(ids[3], 0));
    assert_eq!(order(&queue), ["d", "a", "b", "c"]);
    assert_eq!(current(&queue), Some("b"));
    assert!(queue.move_to(ids[1], 3));
    assert_eq!(order(&queue), ["d", "a", "c", "b"]);
    assert_eq!(current(&queue), Some("b"));
    assert_eq!(queue.peek_next(), None);
}

#[test]
fn move_to_past_the_end_moves_to_the_end() {
    let (mut queue, ids) = queue(&["a", "b", "c"]);
    assert!(queue.move_to(ids[0], 99));
    assert_eq!(order(&queue), ["b", "c", "a"]);
}

#[test]
fn move_to_after_the_current_item_makes_it_next() {
    let (mut queue, ids) = queue(&["a", "b", "c"]);
    queue.select(ids[0]);
    queue.move_to(ids[2], 1);
    assert_eq!(queue.peek_next(), Some(ids[2]));
}

#[test]
fn move_to_an_unknown_item_does_nothing() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.remove(ids[0]);
    assert!(!queue.move_to(ids[0], 0));
    assert_eq!(order(&queue), ["b"]);
}

#[test]
fn clear_empties_the_queue_and_keeps_repeat() {
    let (mut queue, ids) = queue(&["a", "b"]);
    queue.set_repeat(Repeat::All);
    queue.select(ids[0]);
    queue.clear();
    assert!(queue.is_empty());
    assert_eq!(queue.current(), None);
    assert_eq!(queue.repeat(), Repeat::All);
    let c = queue.append("c");
    assert_eq!(queue.next(), Some(c));
}

#[test]
fn select_makes_an_item_current() {
    let (mut queue, ids) = queue(&["a", "b", "c"]);
    assert!(queue.select(ids[1]));
    assert_eq!(current(&queue), Some("b"));
    assert_eq!(queue.next(), Some(ids[2]));
    queue.remove(ids[0]);
    assert!(!queue.select(ids[0]));
}
