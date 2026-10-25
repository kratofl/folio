//! Decides which thumbnails to load and which to let go, so whatever the user
//! is looking at loads first and memory stays bounded however big the
//! library is. Pure: the view's photo order and the scroll position in,
//! decisions out.
//!
//! Only a window of photos around the scroll position is kept in memory:
//! `AHEAD` after it and `BEHIND` before it, in the view's reading order.
//! Scrolling moves the window. Photos that fall out of it are evicted and,
//! thanks to the disk cache, come back quickly when scrolled to again.

/// Photos kept loaded after the first visible one. People scroll down more
/// than up, so the window leans ahead.
pub const AHEAD: usize = 240;
/// Photos kept loaded before the first visible one.
pub const BEHIND: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Pending,
    Loading,
    Loaded,
    /// Couldn't be read. Never retried, so a broken file doesn't loop.
    Failed,
}

/// Tracks every photo's thumbnail. Workers call `take_next` for a job and
/// `finish` when it's done. The app calls `evict` after the window moves.
#[derive(Debug, Clone)]
pub struct LoadQueue {
    slots: Vec<Slot>,
}

impl LoadQueue {
    pub fn new(len: usize) -> Self {
        Self { slots: vec![Slot::Pending; len] }
    }

    /// The next photo to load: the pending one in the window closest to the
    /// focus, preferring photos ahead at up to twice the distance. `order`
    /// is the view's photos in reading order and `focus` a position in it.
    pub fn take_next(&mut self, order: &[usize], focus: usize) -> Option<usize> {
        let (start, end) = window(order.len(), focus);
        let focus = focus.min(end.saturating_sub(1));
        let pending = |&position: &usize| self.slots.get(order[position]) == Some(&Slot::Pending);
        let ahead = (focus..end).find(pending);
        let behind = (start..focus).rev().find(pending);
        let position = match (ahead, behind) {
            (Some(a), Some(b)) => {
                if a - focus <= 2 * (focus - b) { a } else { b }
            }
            (a, b) => a.or(b)?,
        };
        let photo = order[position];
        self.slots[photo] = Slot::Loading;
        Some(photo)
    }

    /// Records a finished load. A photo that left the window while loading
    /// is handed back by the next `evict`.
    pub fn finish(&mut self, photo: usize, loaded: bool) {
        if let Some(slot) = self.slots.get_mut(photo) {
            *slot = if loaded { Slot::Loaded } else { Slot::Failed };
        }
    }

    /// Loaded photos outside the window, which the caller should drop from
    /// memory. They become pending again.
    pub fn evict(&mut self, order: &[usize], focus: usize) -> Vec<usize> {
        let (start, end) = window(order.len(), focus);
        let mut keep = vec![false; self.slots.len()];
        for &photo in &order[start..end] {
            if let Some(k) = keep.get_mut(photo) {
                *k = true;
            }
        }
        let mut evicted = Vec::new();
        for (photo, slot) in self.slots.iter_mut().enumerate() {
            if *slot == Slot::Loaded && !keep[photo] {
                *slot = Slot::Pending;
                evicted.push(photo);
            }
        }
        evicted
    }

    /// True when nothing in the window is still waiting or loading.
    pub fn is_settled(&self, order: &[usize], focus: usize) -> bool {
        let (start, end) = window(order.len(), focus);
        order[start..end].iter().all(|&p| matches!(self.slots.get(p), Some(Slot::Loaded | Slot::Failed)))
    }

    /// How many thumbnails are in memory.
    pub fn loaded_count(&self) -> usize {
        self.slots.iter().filter(|s| **s == Slot::Loaded).count()
    }
}

/// The positions in reading order to keep loaded around `focus`.
fn window(len: usize, focus: usize) -> (usize, usize) {
    let focus = focus.min(len);
    (focus.saturating_sub(BEHIND), (focus + AHEAD).min(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(queue: &mut LoadQueue, order: &[usize], focus: usize) -> Vec<usize> {
        std::iter::from_fn(|| {
            let photo = queue.take_next(order, focus)?;
            queue.finish(photo, true);
            Some(photo)
        })
        .collect()
    }

    #[test]
    fn loads_from_the_focus_outward_in_view_order() {
        // The view shows photos in an order of its own.
        let order = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0];
        let mut queue = LoadQueue::new(10);
        assert_eq!(drain(&mut queue, &order, 4), vec![5, 4, 3, 6, 2, 1, 7, 0, 8, 9]);
        assert!(queue.is_settled(&order, 4));
    }

    #[test]
    fn only_the_window_loads_and_the_rest_is_evicted_as_it_moves() {
        let len = AHEAD + BEHIND + 100;
        let order: Vec<usize> = (0..len).collect();
        let mut queue = LoadQueue::new(len);

        let loaded = drain(&mut queue, &order, 0);
        assert_eq!(loaded.len(), AHEAD, "nothing past the window loads");
        assert!(queue.evict(&order, 0).is_empty());

        // Scroll far down: the top falls out of the window.
        let focus = len - 1;
        let evicted = queue.evict(&order, focus);
        assert_eq!(evicted, (0..AHEAD).collect::<Vec<_>>(), "everything loaded is now far behind");
        assert_eq!(queue.loaded_count(), 0);
        drain(&mut queue, &order, focus);
        assert!(queue.loaded_count() <= AHEAD + BEHIND);
    }

    #[test]
    fn photos_outside_the_view_are_evicted_and_failures_never_retry() {
        let mut queue = LoadQueue::new(3);
        let photo = queue.take_next(&[0, 1, 2], 0).unwrap();
        queue.finish(photo, false);
        assert_eq!(queue.take_next(&[0], 0), None, "a failed photo isn't retried");

        drain(&mut queue, &[0, 1, 2], 0);
        assert_eq!(queue.evict(&[2], 0), vec![1], "photo 1 left the view; 0 failed and holds no memory");
        assert!(queue.is_settled(&[2], 0));
        assert!(!queue.is_settled(&[1], 0));
    }
}
