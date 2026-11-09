use super::{HEADROOM, MIN_ROOM};

pub(super) trait Storage {
    fn used(&self) -> usize;
    fn allocated(&self) -> usize;
    fn trim(&mut self, capacity: usize);
    fn extend_by(&mut self, additional: usize);
}

impl<T> Storage for Vec<T> {
    fn used(&self) -> usize {
        self.len()
    }

    fn allocated(&self) -> usize {
        self.capacity()
    }

    fn trim(&mut self, capacity: usize) {
        self.shrink_to(capacity);
    }

    fn extend_by(&mut self, additional: usize) {
        self.reserve_exact(additional);
    }
}

impl Storage for String {
    fn used(&self) -> usize {
        self.len()
    }

    fn allocated(&self) -> usize {
        self.capacity()
    }

    fn trim(&mut self, capacity: usize) {
        self.shrink_to(capacity);
    }

    fn extend_by(&mut self, additional: usize) {
        self.reserve_exact(additional);
    }
}

pub(super) fn fit(storage: &mut impl Storage) {
    let target = storage.used() + storage.used() / HEADROOM;
    if storage.allocated() > target {
        storage.trim(target);
    } else {
        storage.extend_by(target - storage.used());
    }
}

pub(super) fn room(storage: &mut impl Storage, more: usize) {
    if storage.allocated() - storage.used() < more {
        storage.extend_by(more.max(storage.allocated() / HEADROOM).max(MIN_ROOM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILT: usize = 80_000;

    #[test]
    fn a_built_array_grows_by_an_eighth_not_double() {
        let mut items = vec![0u32; BUILT];
        fit(&mut items);
        assert_eq!(items.capacity(), BUILT + BUILT / HEADROOM);
        while items.len() < items.capacity() {
            room(&mut items, 1);
            items.push(0);
        }
        room(&mut items, 1);
        items.push(0);
        let grown = (BUILT + BUILT / HEADROOM) * (HEADROOM + 1) / HEADROOM;
        assert!(items.capacity() <= grown, "grew to {} instead of about {grown}", items.capacity());
    }

    #[test]
    fn fitting_trims_an_array_doubled_while_building() {
        let mut names = String::with_capacity(BUILT * 2);
        names.push_str(&"a".repeat(BUILT));
        fit(&mut names);
        assert!(names.capacity() < BUILT * 2);
        assert!(names.capacity() >= BUILT + BUILT / HEADROOM);
    }
}
