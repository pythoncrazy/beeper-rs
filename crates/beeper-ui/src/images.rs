//! The decoded avatars and media by URL, with a budget of bytes. When an insert passes the budget, the images that
//! no view showed for the longest time go, down to three quarters of the budget. Images that a view showed after
//! the last drop stay, so a view that shows more than the budget does not load its images again and again. A view
//! that shows a dropped image asks for it again (`wants.rs`).

use crate::prelude::*;
use iced::widget::image::Handle;

/// The decoded size of a value.
pub trait Weigh {
    fn bytes(&self) -> usize;
}

impl Weigh for Handle {
    fn bytes(&self) -> usize {
        match self {
            Handle::Rgba { pixels, .. } => pixels.len(),
            Handle::Bytes(_, b) => b.len(),
            Handle::Path(..) => 0,
        }
    }
}

impl Weigh for crate::Frames {
    fn bytes(&self) -> usize {
        self.iter().map(|f| f.0.bytes()).sum::<usize>() + self.animation().map_or(0, |a| a.bytes())
    }
}

impl<V: Weigh, E> Weigh for Result<V, E> {
    fn bytes(&self) -> usize {
        self.as_ref().map_or(0, V::bytes)
    }
}

/// A URL maps to `None` while a task loads it.
pub struct Images<V> {
    map: HashMap<String, (Option<V>, usize, Cell<u64>)>,
    bytes: usize,
    budget: usize,
    /// The count of reads, the time of the last read of each image.
    clock: Cell<u64>,
    /// The clock at the last drop.
    mark: u64,
}

impl<V: Weigh> Images<V> {
    pub fn new(budget: usize) -> Self {
        Self { map: HashMap::new(), bytes: 0, budget, clock: Cell::new(0), mark: 0 }
    }

    /// The image of `url`, as a view shows it.
    pub fn get(&self, url: &str) -> Option<&Option<V>> {
        let (v, _, used) = self.map.get(url)?;
        used.set(self.clock.get());
        self.clock.set(self.clock.get() + 1);
        Some(v)
    }

    pub fn contains_key(&self, url: &str) -> bool {
        self.map.contains_key(url)
    }

    pub fn insert(&mut self, url: String, v: Option<V>) {
        let size = v.as_ref().map_or(0, V::bytes);
        self.bytes += size;
        let used = Cell::new(self.clock.get());
        if let Some((_, old, _)) = self.map.insert(url, (v, size, used)) {
            self.bytes -= old;
        }
        if self.bytes > self.budget {
            let mut old: Vec<(u64, String)> = (self.map.iter())
                .filter(|(_, (_, size, used))| *size > 0 && used.get() < self.mark)
                .map(|(url, (_, _, used))| (used.get(), url.clone()))
                .collect();
            old.sort_unstable();
            for (_, url) in old {
                if self.bytes <= self.budget / 4 * 3 {
                    break;
                }
                self.remove(&url);
            }
            self.mark = self.clock.get();
        }
    }

    pub fn remove(&mut self, url: &str) {
        if let Some((_, size, _)) = self.map.remove(url) {
            self.bytes -= size;
        }
    }

    /// Keeps the images for which `keep` holds.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &Option<V>) -> bool) {
        let bytes = &mut self.bytes;
        self.map.retain(|url, (v, size, _)| {
            let kept = keep(url, v);
            if !kept {
                *bytes -= *size;
            }
            kept
        });
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = (String, Option<V>)>) {
        for (url, v) in items {
            self.insert(url, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Weigh for usize {
        fn bytes(&self) -> usize {
            *self
        }
    }

    #[test]
    fn images_drop_the_least_recently_shown_over_the_budget() {
        let mut m = Images::new(100);
        for i in 0..10 {
            m.insert(i.to_string(), Some(10));
        }
        assert_eq!(m.bytes, 100, "the images fill the budget");
        assert!(m.get("0").is_some());
        m.insert("10".into(), Some(10));
        assert!(m.bytes > 100, "the images that a view showed since the last drop stay");
        assert!(m.get("10").is_some());
        m.insert("11".into(), Some(10));
        assert!(m.bytes <= 75, "a drop goes down to three quarters of the budget");
        assert!(m.contains_key("10") && m.contains_key("11"), "the images of the last view stay");
        assert!(!m.contains_key("1"), "the oldest image goes");
    }
}
