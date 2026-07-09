use crate::error::MctxError;
use socket2::Socket;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

const CAPACITY: usize = 16;

#[derive(Debug)]
pub(crate) struct BoundedSocketCache<K> {
    sockets: HashMap<K, Arc<Socket>>,
    insertion_order: VecDeque<K>,
}

impl<K> Default for BoundedSocketCache<K> {
    fn default() -> Self {
        Self {
            sockets: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }
}

impl<K> BoundedSocketCache<K>
where
    K: Copy + Eq + Hash,
{
    pub(crate) fn len(&self) -> usize {
        self.sockets.len()
    }

    pub(crate) fn get_or_try_insert_with(
        &mut self,
        key: K,
        create: impl FnOnce() -> Result<Socket, MctxError>,
    ) -> Result<Arc<Socket>, MctxError> {
        if let Some(socket) = self.sockets.get(&key) {
            return Ok(Arc::clone(socket));
        }

        let socket = Arc::new(create()?);
        if self.sockets.len() == CAPACITY
            && let Some(evicted) = self.insertion_order.pop_front()
        {
            self.sockets.remove(&evicted);
        }

        self.insertion_order.push_back(key);
        self.sockets.insert(key, Arc::clone(&socket));
        Ok(socket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use socket2::{Domain, Protocol, Type};

    #[test]
    fn evicts_old_entries_at_capacity() {
        let mut cache = BoundedSocketCache::default();

        for key in 0..=CAPACITY {
            cache
                .get_or_try_insert_with(key, || {
                    Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
                        .map_err(MctxError::RawSocketCreateFailed)
                })
                .unwrap();
        }

        assert_eq!(cache.len(), CAPACITY);
        assert!(!cache.sockets.contains_key(&0));
        assert!(cache.sockets.contains_key(&CAPACITY));
    }
}
