#![forbid(unsafe_code)]

use std::fmt;
use std::str::Utf8Error;
use std::sync::{Arc, OnceLock};
use xcore::StreamId;

/// What arrived, byte for byte: an identity, an optional media type and the
/// bytes. A clone shares the bytes and the text read from them.
#[derive(Clone)]
pub struct Stream {
    id: StreamId,
    media_type: Option<String>,
    bytes: Arc<[u8]>,
    // The bytes read as UTF-8, decided on the first `text()` and shared by
    // every clone, so a contract's `identify` and `validate` and a path's
    // read decode the Stream once between them.
    text: Arc<OnceLock<Result<Box<str>, Utf8Error>>>,
}

impl Stream {
    #[must_use]
    pub fn new(id: StreamId, bytes: impl Into<Arc<[u8]>>, media_type: Option<String>) -> Self {
        Self {
            id,
            media_type,
            bytes: bytes.into(),
            text: Arc::default(),
        }
    }

    #[must_use]
    pub const fn id(&self) -> StreamId {
        self.id
    }
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The bytes as UTF-8 text. They are decoded on the first call and the
    /// answer is kept, so every later call, on this Stream or a clone of it,
    /// costs nothing.
    ///
    /// # Errors
    /// The bytes are not UTF-8; the error says where they stop being it.
    pub fn text(&self) -> Result<&str, Utf8Error> {
        self.text
            .get_or_init(|| std::str::from_utf8(&self.bytes).map(Box::from))
            .as_deref()
            .map_err(|refused| *refused)
    }
}

// The text is the bytes read once; it is not part of what a Stream is.
impl PartialEq for Stream {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.media_type == other.media_type && self.bytes == other.bytes
    }
}

impl Eq for Stream {}

impl fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Stream")
            .field("id", &self.id)
            .field("media_type", &self.media_type)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
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

    #[test]
    fn text_is_decoded_once_and_shared_by_every_clone() {
        let stream = Stream::new(StreamId::new(1), b"plain".to_vec(), None);
        let clone = stream.clone();
        let first = stream.text().expect("text");
        assert_eq!(first, "plain");
        // One decoding: the clone reads the very text the first call kept.
        assert!(std::ptr::eq(first, clone.text().expect("text")));
        assert_eq!(stream, clone);
    }

    #[test]
    fn bytes_that_are_not_utf8_say_where_they_stop() {
        let stream = Stream::new(StreamId::new(1), vec![b'a', 0xff], None);
        let refused = stream.text().expect_err("not text");
        assert_eq!(refused.valid_up_to(), 1);
        assert_eq!(stream.text().expect_err("kept"), refused);
    }
}
