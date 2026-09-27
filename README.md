# xmip-core-stream

The Stream: what arrives, byte for byte, before and during Message creation.
A Stream is an identity, an optional media type and the bytes, and nothing
else.

`Stream::text` reads the bytes as UTF-8 once, on the first call, and keeps the
answer for every clone of the Stream: a contract's `identify` and `validate`
and a path's read share one decoding.

A Stream is never modified. What arrived is what is kept, and that is what
makes replay, audit and preservation mean anything. A Stream is not a Message
— a Message is created over it once a Contract accepts it — and it belongs to
the sender until Xmip accepts it.

`doc/architecture/runtime-model.md` sections 2 and 3 and ADR-0003 govern it;
`architecture.toml` carries the maturity.
