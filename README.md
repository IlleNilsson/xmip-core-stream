# xmip-core-stream

The Stream: what arrives, byte for byte, before and during Message creation.
A Stream is an identity, an optional media type and the bytes, and nothing
else.

A Stream's content is held in memory, or **kept** where it was written, the
Ledger's chunks: `Stream::kept` is a handle — identity, length, media type and
the `Content` its bytes are read from — and nothing holds the whole Stream
until a gate asks for it whole. `Stream::reader` reads it a piece at a time;
`Stream::load` reads it whole once, shared by every clone, and is the one way
to the whole content. Where kept content cannot be read, or is not as long as
kept, it answers `Unread`, which keeps the `io::ErrorKind` of the failure and
converts into an `io::Error` of that kind; a failure is never empty content,
and it is not kept as the answer: the next call reads again.
`Stream::unread` says why the last whole read failed, where none has
succeeded since, so a caller whose gate refused can tell a Stream that could
not be read from content that was refused.

`Stream::text` reads the bytes as UTF-8 once, on the first call that reads
them, and keeps the answer for every clone of the Stream: a contract's
`identify` and `validate` and a path's read share one decoding. Its error,
`TextError`, has two cases that are never one: `Unread`, the content could
not be read, which is not kept; and `NotUtf8`, it was read and is not UTF-8,
which is.

A Stream is never modified. What arrived is what is kept, and that is what
makes replay, audit and preservation mean anything. A Stream is not a Message
— a Message is created over it once a Contract accepts it — and it belongs to
the sender until Xmip accepts it.

`doc/architecture/runtime-model.md` sections 2 and 3 and ADR-0003 govern it;
`architecture.toml` carries the maturity.
