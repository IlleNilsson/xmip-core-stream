//! Why a Stream's content could not be had: not read, or read and not text.

use std::fmt;
use std::io;
use std::str::Utf8Error;

/// Why kept content could not be read whole: the kind of the failure, as
/// [`io::ErrorKind`] names it, and what was met. Content that is not as long
/// as kept is [`io::ErrorKind::UnexpectedEof`] where it is shorter and
/// [`io::ErrorKind::InvalidData`] where it is longer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    kind: io::ErrorKind,
    message: String,
}

impl Unread {
    #[must_use]
    pub fn new(kind: io::ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// The kind of the failure, as the read that met it named it.
    #[must_use]
    pub const fn kind(&self) -> io::ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Unread {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Unread {}

/// An I/O error of the same kind, for a caller that answers in I/O.
impl From<Unread> for io::Error {
    fn from(unread: Unread) -> Self {
        Self::new(unread.kind, unread.message)
    }
}

/// Why a Stream's content is not text: it could not be read, or it was read
/// and is not UTF-8. The two are never one: a read that failed says nothing
/// of what the content is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextError {
    /// The content could not be read whole.
    Unread(Unread),
    /// The content was read and is not UTF-8; the error says where it stops
    /// being it.
    NotUtf8(Utf8Error),
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unread(unread) => unread.fmt(formatter),
            Self::NotUtf8(refused) => write!(formatter, "the Stream is not UTF-8: {refused}"),
        }
    }
}

impl std::error::Error for TextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unread(unread) => Some(unread),
            Self::NotUtf8(refused) => Some(refused),
        }
    }
}

impl From<Unread> for TextError {
    fn from(unread: Unread) -> Self {
        Self::Unread(unread)
    }
}

/// An I/O error: a read failure of its own kind, text that is not UTF-8
/// [`io::ErrorKind::InvalidData`].
impl From<TextError> for io::Error {
    fn from(refused: TextError) -> Self {
        match refused {
            TextError::Unread(unread) => unread.into(),
            TextError::NotUtf8(refused) => Self::new(io::ErrorKind::InvalidData, refused),
        }
    }
}
