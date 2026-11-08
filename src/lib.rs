#![forbid(unsafe_code)]

//! The Stream: what arrived, byte for byte.
//!
//! A Stream's content is held in memory, or **kept** where it was written —
//! the Ledger's chunks (`runtime-model.md` section 3: *A Stream is written in
//! chunks, never whole in memory*). A kept Stream is a handle: its identity,
//! its length, its media type and the [`Content`] its bytes are read from,
//! a piece at a time through [`Stream::reader`], or whole, once, by the
//! first [`Stream::load`] that reads it — what a gate that reads the whole
//! content asks for, and nothing else does. A read that fails says so, as
//! [`Unread`]; it is never taken for content.

mod error;

pub use error::{TextError, Unread};

use std::fmt;
use std::io::{self, Read};
use std::str::Utf8Error;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
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
    // The bytes read as UTF-8, decided on the first `text()` that reads them
    // and shared by every clone, so a contract's `identify` and `validate`
    // and a path's read decode the Stream once between them. A read that
    // failed decides nothing and is not kept here.
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
    /// The content read whole, by the first [`Stream::load`] that read it.
    loaded: OnceLock<Arc<[u8]>>,
    /// Why the last whole read failed, where none has succeeded since; held
    /// while the content is read whole, so it is read once at a time.
    failed: Mutex<Option<Unread>>,
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
                failed: Mutex::new(None),
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
                Some(bytes) => Ok(Box::new(&bytes[..])),
                None => kept.content.reader(),
            },
        }
    }

    /// The content whole, in memory: held bytes as they are, kept content
    /// read once, by the first call that reads it, and shared by every
    /// clone. The one way to the whole content: a gate that reads in pieces
    /// asks [`Stream::reader`].
    ///
    /// # Errors
    /// Where kept content could not be read, or was not as long as kept. A
    /// failure is not kept as the answer: the next call reads again, since
    /// what failed once may be read the next time.
    pub fn load(&self) -> Result<&[u8], Unread> {
        match &self.body {
            Body::Held(bytes) => Ok(bytes),
            Body::Kept(kept) => kept.load(),
        }
    }

    /// Why the last whole read of kept content failed, where it failed and
    /// none has succeeded since: what a caller asks after a gate refused, to
    /// tell a Stream that could not be read from content that was refused.
    #[must_use]
    pub fn unread(&self) -> Option<Unread> {
        match &self.body {
            Body::Kept(kept) => kept.failed().clone(),
            Body::Held(_) => None,
        }
    }

    /// The bytes as UTF-8 text. They are decoded on the first call that
    /// reads them and the answer is kept, so every later call, on this
    /// Stream or a clone of it, costs nothing.
    ///
    /// # Errors
    /// [`TextError::Unread`] where the content could not be read, as
    /// [`Stream::load`] says, and nothing is kept, so a later call reads
    /// again; [`TextError::NotUtf8`] where it was read and is not UTF-8,
    /// which is kept.
    pub fn text(&self) -> Result<&str, TextError> {
        let decoded = match self.text.get() {
            Some(decoded) => decoded,
            None => {
                let bytes = self.load()?;
                self.text
                    .get_or_init(|| std::str::from_utf8(bytes).map(Box::from))
            }
        };
        decoded
            .as_deref()
            .map_err(|refused| TextError::NotUtf8(*refused))
    }
}

impl Kept {
    fn load(&self) -> Result<&[u8], Unread> {
        if let Some(bytes) = self.loaded.get() {
            return Ok(bytes);
        }
        // Held while reading, so clones asking at once read the content once.
        let mut failed = self.failed();
        if let Some(bytes) = self.loaded.get() {
            return Ok(bytes);
        }
        match self.read_whole() {
            Ok(bytes) => {
                *failed = None;
                Ok(self.loaded.get_or_init(|| bytes))
            }
            Err(unread) => {
                *failed = Some(unread.clone());
                Err(unread)
            }
        }
    }

    fn failed(&self) -> std::sync::MutexGuard<'_, Option<Unread>> {
        self.failed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_whole(&self) -> Result<Arc<[u8]>, Unread> {
        let mut bytes = Vec::with_capacity(usize::try_from(self.length).unwrap_or(0));
        self.content
            .reader()
            .and_then(|mut reader| reader.read_to_end(&mut bytes))
            .map_err(|failed| {
                Unread::new(
                    failed.kind(),
                    format!("the Stream could not be read: {failed}"),
                )
            })?;
        let read = bytes.len() as u64;
        if read == self.length {
            return Ok(bytes.into());
        }
        let kind = if read < self.length {
            io::ErrorKind::UnexpectedEof
        } else {
            io::ErrorKind::InvalidData
        };
        Err(Unread::new(
            kind,
            format!("the Stream is {read} bytes, and {} were kept", self.length),
        ))
    }
}

// The text is the bytes read once; it is not part of what a Stream is. Two
// Streams sharing their content are equal without reading it; otherwise both
// are read, and content that cannot be read equals no other.
impl PartialEq for Stream {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.media_type == other.media_type
            && self.length() == other.length()
            && (self.body.shares(&other.body)
                || matches!((self.load(), other.load()), (Ok(one), Ok(two)) if one == two))
    }
}

impl Body {
    fn shares(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Held(one), Self::Held(two)) => Arc::ptr_eq(one, two),
            (Self::Kept(one), Self::Kept(two)) => Arc::ptr_eq(one, two),
            _ => false,
        }
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn stream_is_immutable_and_shareable() {
        let stream = Stream::new(
            StreamId::new(1),
            vec![1, 2, 3],
            Some("application/octet-stream".into()),
        );
        let clone = stream.clone();
        assert_eq!(stream.load(), clone.load());
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
        let Err(TextError::NotUtf8(refused)) = stream.text() else {
            panic!("read, and not text");
        };
        assert_eq!(refused.valid_up_to(), 1);
        assert_eq!(stream.text(), Err(TextError::NotUtf8(refused)), "kept");
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
        assert_eq!(kept.clone().load().expect("shared"), b"<Order/>");
        assert_eq!(content.1.load(Ordering::Relaxed), 2, "whole once, shared");
        assert_eq!(
            kept,
            Stream::new(StreamId::new(2), b"<Order/>".to_vec(), None)
        );
    }

    #[test]
    fn kept_content_not_as_long_as_kept_says_so() {
        let short = Stream::kept(
            StreamId::new(3),
            9,
            None,
            Arc::new(Counted(b"short".to_vec(), AtomicUsize::new(0))),
        );
        assert!(short.unread().is_none(), "nothing asked for it yet");
        let refused = short.load().expect_err("not as long as kept");
        assert_eq!(refused.kind(), io::ErrorKind::UnexpectedEof);
        assert!(refused.message().contains("5 bytes, and 9"), "{refused}");
        assert_eq!(short.text(), Err(TextError::Unread(refused.clone())));
        assert_eq!(short.unread(), Some(refused));
        assert_eq!(short, short.clone(), "a Stream equals itself unread");
    }

    /// Content whose first read fails, and every later one succeeds.
    struct Flaky(AtomicBool);

    impl Content for Flaky {
        fn reader(&self) -> io::Result<Box<dyn Read + Send + '_>> {
            if self.0.swap(true, Ordering::Relaxed) {
                Ok(Box::new(&b"later"[..]))
            } else {
                Err(io::Error::new(io::ErrorKind::TimedOut, "not yet"))
            }
        }
    }

    #[test]
    fn a_failed_read_is_said_with_its_kind_and_read_again() {
        let kept = Stream::kept(
            StreamId::new(4),
            5,
            None,
            Arc::new(Flaky(AtomicBool::new(false))),
        );
        let Err(TextError::Unread(unread)) = kept.text() else {
            panic!("the first read fails");
        };
        assert_eq!(unread.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            io::Error::from(unread.clone()).kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(kept.unread(), Some(unread));
        assert_eq!(
            kept.text(),
            Ok("later"),
            "read again, not kept as the answer"
        );
        assert!(kept.unread().is_none(), "read since");
    }
}
