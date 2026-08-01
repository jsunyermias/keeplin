// md:Overview
use uuid::Uuid;

// md:KeyedItem
struct KeyedItem<T> {
    key: (String, Uuid),
    item: T,
}

// md:impl PartialEq for KeyedItem
impl<T> PartialEq for KeyedItem<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
// md:impl Eq for KeyedItem
impl<T> Eq for KeyedItem<T> {}
// md:impl PartialOrd for KeyedItem
impl<T> PartialOrd for KeyedItem<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
// md:impl Ord for KeyedItem
impl<T> Ord for KeyedItem<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

// md:PageCollector
pub(super) struct PageCollector<T> {
    limit: usize,
    cursor: Option<(String, Uuid)>,
    heap: std::collections::BinaryHeap<KeyedItem<T>>,
}

// md:impl PageCollector
impl<T> PageCollector<T> {
    // md:impl PageCollector > fn new
    pub(super) fn new(limit: usize, token: Option<&str>) -> Self {
        let cursor = token
            .filter(|t| !t.is_empty())
            .and_then(|t| t.split_once('|'))
            .and_then(|(ts, id)| Uuid::parse_str(id).ok().map(|id| (ts.to_string(), id)));
        Self {
            limit,
            cursor,
            heap: std::collections::BinaryHeap::with_capacity(limit + 2),
        }
    }

    // md:impl PageCollector > fn push
    pub(super) fn push(&mut self, key: (String, Uuid), item: T) {
        if let Some(cursor) = &self.cursor {
            if (key.0.as_str(), key.1) <= (cursor.0.as_str(), cursor.1) {
                return;
            }
        }
        if self.heap.len() <= self.limit {
            self.heap.push(KeyedItem { key, item });
        } else if let Some(top) = self.heap.peek() {
            if key < top.key {
                self.heap.pop();
                self.heap.push(KeyedItem { key, item });
            }
        }
    }

    // md:impl PageCollector > fn into_page
    pub(super) fn into_page(self) -> (Vec<T>, Option<String>) {
        let mut items = self.heap.into_sorted_vec();
        let has_more = items.len() > self.limit;
        items.truncate(self.limit);
        let next_token = if has_more {
            items
                .last()
                .map(|last| format!("{}|{}", last.key.0, last.key.1))
        } else {
            None
        };
        (items.into_iter().map(|k| k.item).collect(), next_token)
    }
}

// md:fn paginate
pub(super) fn paginate<T, F>(
    items: Vec<T>,
    limit: usize,
    token: Option<&str>,
    key_fn: F,
) -> (Vec<T>, Option<String>)
where
    F: Fn(&T) -> (String, Uuid),
{
    let start = match token.filter(|t| !t.is_empty()) {
        Some(cursor) => {
            if let Some((ts, id_str)) = cursor.split_once('|') {
                if let Ok(cursor_id) = Uuid::parse_str(id_str) {
                    items.partition_point(|item| {
                        let (item_ts, item_id) = key_fn(item);
                        item_ts.as_str() < ts || (item_ts.as_str() == ts && item_id <= cursor_id)
                    })
                } else {
                    0
                }
            } else {
                0
            }
        }
        None => 0,
    };

    let remaining: Vec<T> = items.into_iter().skip(start).collect();
    let has_more = remaining.len() > limit;
    let page: Vec<T> = remaining.into_iter().take(limit).collect();

    let next_token = if has_more {
        page.last().map(|last| {
            let (ts, id) = key_fn(last);
            format!("{ts}|{id}")
        })
    } else {
        None
    };

    (page, next_token)
}
