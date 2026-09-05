#![forbid(unsafe_code)]

use std::sync::Arc;
use xcore::StreamId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    id: StreamId,
    media_type: Option<String>,
    bytes: Arc<[u8]>,
}

impl Stream {
    pub fn new(id: StreamId, bytes: impl Into<Arc<[u8]>>, media_type: Option<String>) -> Self {
        Self {
            id,
            media_type,
            bytes: bytes.into(),
        }
    }

    pub const fn id(&self) -> StreamId {
        self.id
    }
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_immutable_and_shareable() {
        let stream = Stream::new(
            StreamId::new(1),
            vec![1, 2, 3],
            Some("application/octet-stream".into()),
        );
        let clone = stream.clone();
        assert_eq!(stream.bytes(), clone.bytes());
        assert_eq!(stream.len(), 3);
    }
}
