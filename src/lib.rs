#![forbid(unsafe_code)]

//! The Stream: what arrived, byte for byte.
//!
//! A Stream's content is held in memory, or **kept** where it was written —
//! the Ledger's chunks (`runtime-model.md` section 3: *A Stream is written in
//! chunks, never whole in memory*). A kept Stream is a handle: its identity,
//! its length, its media type and the [`Content`] its bytes are read from,
//! a piece at a time through [`Stream::reader`], or whole, once, by the
//! first [`Stream::load`] — what a gate that reads the whole content asks
//! for, and nothing else does.

use std::fmt;
use std::io::{self, Read};
use std::str::Utf8Error;
use std::sync::{Arc, OnceLock};
use xcore::StreamId;

/// Where a kept Stream's content is read from: the Ledger's chunks, for one.
pub trait Content: Send + Sync {
    /// The content from its start, read in pieces as the reader asks, so no
    /// more of it is in memory than the reader holds.
    ///
    /// # Errors
    /// Where the content cannot be reached.
    fn reader(&self) -> io::Result<Box<dyn Read + Send + '_>>;
}

/// What arrived, byte for byte: an identity, an optional media type and the
/// content. A clone shares the content, and the text read from it.
#[derive(Clone)]
pub struct Stream {
    id: StreamId,
    media_type: Option<String>,
    body: Body,
    // The bytes read as UTF-8, decided on the first `text()` and shared by
    // every clone, so a contract's `identify` and `validate` and a path's
    // read decode the Stream once between them.
    text: Arc<OnceLock<Result<Box<str>, Utf8Error>>>,
}

#[derive(Clone)]
enum Body {
    Held(Arc<[u8]>),
    Kept(Arc<Kept>),
}

struct Kept {
    length: u64,
    content: Arc<dyn Content>,
    /// The content read whole, by the first [`Stream::load`], or why not.
    loaded: OnceLock<Result<Arc<[u8]>, String>>,
}

impl Stream {
    /// A Stream whose content is `bytes`, in memory.
    #[must_use]
    pub fn new(id: StreamId, bytes: impl Into<Arc<[u8]>>, media_type: Option<String>) -> Self {
        Self {
            id,
            media_type,
            body: Body::Held(bytes.into()),
            text: Arc::default(),
        }
    }

    /// A Stream of `length` bytes kept where `content` reads it from.
    #[must_use]
    pub fn kept(
        id: StreamId,
        length: u64,
        media_type: Option<String>,
        content: Arc<dyn Content>,
    ) -> Self {
        Self {
            id,
            media_type,
            body: Body::Kept(Arc::new(Kept {
                length,
                content,
                loaded: OnceLock::new(),
            })),
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

    /// Its length in bytes, read from nothing.
    #[must_use]
    pub fn length(&self) -> u64 {
        match &self.body {
            Body::Held(bytes) => bytes.len() as u64,
            Body::Kept(kept) => kept.length,
        }
    }

    /// Its length in bytes, as this machine counts them.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::try_from(self.length()).unwrap_or(usize::MAX)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.length() == 0
    }

    /// The content from its start, a piece at a time: held bytes as they
    /// are, kept content as [`Content::reader`] reads it — never whole.
    ///
    /// # Errors
    /// Where kept content cannot be reached.
    pub fn reader(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        match &self.body {
            Body::Held(bytes) => Ok(Box::new(&bytes[..])),
            Body::Kept(kept) => match kept.loaded.get() {
                Some(Ok(bytes)) => Ok(Box::new(&bytes[..])),
                _ => kept.content.reader(),
            },
        }
    }

    /// The content whole, in memory: held bytes as they are, kept content
    /// read once, by the first call, and shared by every clone. What a gate
    /// that reads the whole content asks for; a gate that reads in pieces
    /// asks [`Stream::reader`].
    ///
    /// # Errors
    /// Where kept content could not be read, or was not as long as kept;
    /// every later call says the same.
    pub fn load(&self) -> io::Result<&[u8]> {
        match &self.body {
            Body::Held(bytes) => Ok(bytes),
            Body::Kept(kept) => kept
                .loaded
                .get_or_init(|| kept.read_whole())
                .as_deref()
                .map_err(|why| io::Error::other(why.clone())),
        }
    }

    /// Why kept content a gate asked for whole could not be read, where it
    /// was asked for and could not: what a caller that let a gate read
    /// [`Stream::bytes`] asks afterwards, so an empty read is never taken
    /// for the content.
    #[must_use]
    pub fn unread(&self) -> Option<&str> {
        match &self.body {
            Body::Kept(kept) => match kept.loaded.get() {
                Some(Err(why)) => Some(why),
                _ => None,
            },
            Body::Held(_) => None,
        }
    }

    /// The content whole, as [`Stream::load`] reads it, or nothing where
    /// kept content could not be read: a caller that must tell the two apart
    /// asks [`Stream::load`] first.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.load().unwrap_or(&[])
    }

    /// The bytes as UTF-8 text. They are decoded on the first call and the
    /// answer is kept, so every later call, on this Stream or a clone of it,
    /// costs nothing.
    ///
    /// # Errors
    /// The bytes are not UTF-8; the error says where they stop being it.
    pub fn text(&self) -> Result<&str, Utf8Error> {
        self.text
            .get_or_init(|| std::str::from_utf8(self.bytes()).map(Box::from))
            .as_deref()
            .map_err(|refused| *refused)
    }
}

impl Kept {
    fn read_whole(&self) -> Result<Arc<[u8]>, String> {
        let mut bytes = Vec::with_capacity(usize::try_from(self.length).unwrap_or(0));
        self.content
            .reader()
            .and_then(|mut reader| reader.read_to_end(&mut bytes))
            .map_err(|failed| format!("the Stream could not be read: {failed}"))?;
        if bytes.len() as u64 == self.length {
            Ok(bytes.into())
        } else {
            Err(format!(
                "the Stream is {} bytes, and {} were kept",
                bytes.len(),
                self.length
            ))
        }
    }
}

// The text is the bytes read once; it is not part of what a Stream is.
impl PartialEq for Stream {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.media_type == other.media_type
            && self.length() == other.length()
            && self.bytes() == other.bytes()
    }
}

impl Eq for Stream {}

impl fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("Stream");
        debug
            .field("id", &self.id)
            .field("media_type", &self.media_type);
        match &self.body {
            Body::Held(bytes) => debug.field("bytes", bytes),
            Body::Kept(kept) => debug.field("kept", &kept.length),
        };
        debug.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    /// Content in memory that counts how often it is read.
    struct Counted(Vec<u8>, AtomicUsize);

    impl Content for Counted {
        fn reader(&self) -> io::Result<Box<dyn Read + Send + '_>> {
            self.1.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(&self.0[..]))
        }
    }

    #[test]
    fn kept_content_is_read_in_pieces_or_whole_once() {
        let content = Arc::new(Counted(b"<Order/>".to_vec(), AtomicUsize::new(0)));
        let kept = Stream::kept(StreamId::new(2), 8, None, Arc::clone(&content) as _);
        assert_eq!(kept.len(), 8, "its length reads nothing");
        assert_eq!(content.1.load(Ordering::Relaxed), 0);
        let mut piece = [0u8; 3];
        kept.reader()
            .expect("a reader")
            .read_exact(&mut piece)
            .expect("read");
        assert_eq!(&piece, b"<Or");
        assert_eq!(kept.load().expect("whole"), b"<Order/>");
        assert_eq!(kept.clone().bytes(), b"<Order/>");
        assert_eq!(content.1.load(Ordering::Relaxed), 2, "whole once, shared");
        assert_eq!(
            kept,
            Stream::new(StreamId::new(2), b"<Order/>".to_vec(), None)
        );
    }

    #[test]
    fn kept_content_that_cannot_be_read_says_so() {
        let short = Stream::kept(
            StreamId::new(3),
            9,
            None,
            Arc::new(Counted(b"short".to_vec(), AtomicUsize::new(0))),
        );
        assert!(short.unread().is_none(), "nothing asked for it yet");
        let refused = short.load().expect_err("not as long as kept");
        assert!(refused.to_string().contains("5 bytes, and 9"), "{refused}");
        assert!(short.bytes().is_empty());
        assert!(short.unread().is_some_and(|why| why.contains("5 bytes")));
    }
}
