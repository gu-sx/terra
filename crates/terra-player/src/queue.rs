//! The play queue: an ordered list of items and a cursor, with no audio.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Identifies one entry of a [`Queue`].
///
/// Every entry gets a new identifier, unique within the process, so the same
/// track queued twice is two entries. `Display` prints the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QueueItemId(u64);

impl QueueItemId {
    /// A fresh identifier, never handed out before in this process.
    pub(crate) fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for QueueItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// What [`Queue::next`] and [`Queue::previous`] do at either end of the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Repeat {
    /// Stop at the ends.
    #[default]
    Off,
    /// Wrap around: after the last item comes the first, before the first the last.
    All,
}

/// Where the queue stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cursor {
    /// On the item at this index: the current item.
    At(usize),
    /// Between items, with no current item: [`Queue::next`] moves to the item
    /// at this index, which may be one past the end. A new queue, a queue
    /// whose current item was removed and a queue that ran past its last item
    /// stand here.
    Before(usize),
}

/// An ordered list of items with a current one.
///
/// The queue does no I/O; the player keeps one of
/// [`AudioSource`](terra_core::AudioSource)s.
#[derive(Debug, Clone)]
pub struct Queue<T> {
    items: Vec<(QueueItemId, T)>,
    cursor: Cursor,
    repeat: Repeat,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue<T> {
    /// An empty queue, with repeat off.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            cursor: Cursor::Before(0),
            repeat: Repeat::Off,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// The items in order.
    pub fn iter(&self) -> impl Iterator<Item = (QueueItemId, &T)> {
        self.items.iter().map(|(id, item)| (*id, item))
    }

    pub fn get(&self, id: QueueItemId) -> Option<&T> {
        self.index_of(id).map(|i| &self.items[i].1)
    }

    pub fn contains(&self, id: QueueItemId) -> bool {
        self.index_of(id).is_some()
    }

    /// The current item, if the cursor is on one.
    pub fn current(&self) -> Option<(QueueItemId, &T)> {
        match self.cursor {
            Cursor::At(i) => Some((self.items[i].0, &self.items[i].1)),
            Cursor::Before(_) => None,
        }
    }

    pub fn current_id(&self) -> Option<QueueItemId> {
        self.current().map(|(id, _)| id)
    }

    /// Adds `item` at the end.
    pub fn append(&mut self, item: T) -> QueueItemId {
        let id = QueueItemId::fresh();
        self.push(id, item);
        id
    }

    /// Adds `item` so that it comes next: after the current item, or where
    /// [`next`](Self::next) would move to when there is none.
    pub fn insert_next(&mut self, item: T) -> QueueItemId {
        let id = QueueItemId::fresh();
        self.push_next(id, item);
        id
    }

    pub(crate) fn push(&mut self, id: QueueItemId, item: T) {
        self.items.push((id, item));
    }

    pub(crate) fn push_next(&mut self, id: QueueItemId, item: T) {
        let index = match self.cursor {
            Cursor::At(i) => i + 1,
            Cursor::Before(i) => i,
        };
        self.insert_at(index, id, item);
    }

    /// Removes an item and returns it, or `None` if the queue does not hold
    /// it. When it is the current item, the queue is left with no current
    /// item and [`next`](Self::next) moves to the item that followed it.
    pub fn remove(&mut self, id: QueueItemId) -> Option<T> {
        let index = self.index_of(id)?;
        let (_, item) = self.items.remove(index);
        self.cursor = match self.cursor {
            Cursor::At(i) if i == index => Cursor::Before(index),
            Cursor::At(i) if i > index => Cursor::At(i - 1),
            Cursor::Before(i) if i > index => Cursor::Before(i - 1),
            cursor => cursor,
        };
        Some(item)
    }

    /// Moves an item to `index`, counted after it is taken out, so that it
    /// ends up at that index; an index past the end moves it to the end. The
    /// current item stays current. Returns `false` if the queue does not
    /// hold the item.
    pub fn move_to(&mut self, id: QueueItemId, index: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let was_current = self.cursor == Cursor::At(from);
        let item = self.remove(id).expect("the item is in the queue");
        let to = index.min(self.items.len());
        self.insert_at(to, id, item);
        if was_current {
            self.cursor = Cursor::At(to);
        }
        true
    }

    /// Removes every item. Repeat is kept.
    pub fn clear(&mut self) {
        self.items.clear();
        self.cursor = Cursor::Before(0);
    }

    /// Makes an item the current one. Returns `false` if the queue does not
    /// hold it.
    pub fn select(&mut self, id: QueueItemId) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.cursor = Cursor::At(i);
                true
            }
            None => false,
        }
    }

    /// Moves to the next item and returns it. At the end, with repeat off,
    /// it returns `None` and leaves the queue past its last item, with no
    /// current item; with [`Repeat::All`] it moves to the first item.
    #[allow(
        clippy::should_implement_trait,
        reason = "the queue is not an iterator: with repeat it never ends, and it moves back too"
    )]
    pub fn next(&mut self) -> Option<QueueItemId> {
        let index = self.next_index();
        self.cursor = match index {
            Some(i) => Cursor::At(i),
            None => Cursor::Before(self.items.len()),
        };
        index.map(|i| self.items[i].0)
    }

    /// The item [`next`](Self::next) would move to, without moving.
    pub fn peek_next(&self) -> Option<QueueItemId> {
        self.next_index().map(|i| self.items[i].0)
    }

    /// Moves to the previous item and returns it. At the first item, with
    /// repeat off, it returns `None` and stays; with [`Repeat::All`] it moves
    /// to the last item. Past the end it moves to the last item.
    pub fn previous(&mut self) -> Option<QueueItemId> {
        let len = self.items.len();
        let index = match self.cursor {
            Cursor::At(i) | Cursor::Before(i) if i > 0 => Some(i - 1),
            _ if self.repeat == Repeat::All && len > 0 => Some(len - 1),
            _ => None,
        };
        let index = index?;
        self.cursor = Cursor::At(index);
        Some(self.items[index].0)
    }

    fn next_index(&self) -> Option<usize> {
        let len = self.items.len();
        let candidate = match self.cursor {
            Cursor::At(i) => i + 1,
            Cursor::Before(i) => i,
        };
        if candidate < len {
            Some(candidate)
        } else if self.repeat == Repeat::All && len > 0 {
            Some(0)
        } else {
            None
        }
    }

    fn index_of(&self, id: QueueItemId) -> Option<usize> {
        self.items.iter().position(|(item, _)| *item == id)
    }

    fn insert_at(&mut self, index: usize, id: QueueItemId, item: T) {
        self.items.insert(index, (id, item));
        self.cursor = match self.cursor {
            Cursor::At(i) if i >= index => Cursor::At(i + 1),
            Cursor::Before(i) if i > index => Cursor::Before(i + 1),
            cursor => cursor,
        };
    }
}
