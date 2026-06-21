//! Streaming primitives: a `Bytes`-chunk → `String`-line adapter (`LineSplit`)
//! plus shared stream type aliases used across the provider code.
//!
//! Today (pre-streaming-refactor) all SSE handling buffers the full upstream
//! response with `resp.bytes().await`. The adapters here let us plumb
//! `reqwest::Response::bytes_stream()` into the line-oriented SSE converters
//! one newline-terminated line at a time, while carrying over partial lines
//! across chunk boundaries.

use bytes::Bytes;
use futures::stream::Stream;
use futures::task::{Context, Poll};
use std::pin::Pin;

/// `Box<dyn std::error::Error + Send + Sync>` — what axum's body expects.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Adapter that turns a `Stream<Item = Result<Bytes, _>>` into a
/// `Stream<Item = Result<String, BoxError>>`, yielding one `String` per
/// newline-terminated line. Partial trailing bytes are buffered in `carry`
/// until the next chunk arrives.
///
/// The inner stream is type-erased via `Pin<Box<dyn Stream + Send>>` so that
/// `LineSplit` itself doesn't need a type parameter for the error type.
pub struct LineSplit {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    /// Bytes left over from the previous chunk that did not yet end with `\n`.
    carry: Vec<u8>,
}

impl LineSplit {
    pub fn new<S>(inner: S) -> Self
    where
        S: Stream<Item = Result<Bytes, BoxError>> + Send + 'static,
    {
        Self {
            inner: Box::pin(inner),
            carry: Vec::new(),
        }
    }
}

impl Stream for LineSplit {
    type Item = Result<String, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // First, drain any complete lines we already have in `carry`.
            if let Some(idx) = self.carry.iter().position(|b| *b == b'\n') {
                let line_bytes: Vec<u8> = self.carry.drain(..=idx).collect();
                // Strip the trailing '\n'.
                let mut line = String::from_utf8_lossy(&line_bytes[..line_bytes.len() - 1])
                    .into_owned();
                // Trim a trailing '\r' if present (CRLF line endings).
                if line.ends_with('\r') {
                    line.pop();
                }
                return Poll::Ready(Some(Ok(line)));
            }

            // Otherwise pull the next chunk from the inner stream.
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    self.carry.extend_from_slice(&chunk);
                    // Loop: either we found a newline above or we'll poll again.
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    // Inner stream ended. Yield any leftover `carry` as the final
                    // (unterminated) line if it's non-empty.
                    if !self.carry.is_empty() {
                        let line = String::from_utf8_lossy(&self.carry).into_owned();
                        self.carry.clear();
                        return Poll::Ready(Some(Ok(line)));
                    }
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::{self, StreamExt};

    fn chunks(items: Vec<Result<Bytes, BoxError>>) -> impl Stream<Item = Result<Bytes, BoxError>> + Send + 'static + Unpin {
        stream::iter(items)
    }

    fn boxify<S>(s: S) -> Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>
    where
        S: Stream<Item = Result<Bytes, BoxError>> + Send + 'static,
    {
        Box::pin(s)
    }

    #[tokio::test]
    async fn splits_simple_lines() {
        let input = boxify(chunks(vec![Ok(Bytes::from_static(b"a\nb\nc\n"))]));
        let mut s = LineSplit::new(input);
        assert_eq!(s.next().await.unwrap().unwrap(), "a");
        assert_eq!(s.next().await.unwrap().unwrap(), "b");
        assert_eq!(s.next().await.unwrap().unwrap(), "c");
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn carries_over_partial_lines() {
        // The first chunk ends mid-line, the second chunk completes it.
        let input = boxify(chunks(vec![
            Ok(Bytes::from_static(b"data: hel")),
            Ok(Bytes::from_static(b"lo\nworld\n")),
        ]));
        let mut s = LineSplit::new(input);
        assert_eq!(s.next().await.unwrap().unwrap(), "data: hello");
        assert_eq!(s.next().await.unwrap().unwrap(), "world");
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn strips_crlf() {
        let input = boxify(chunks(vec![Ok(Bytes::from_static(b"line1\r\nline2\r\n"))]));
        let mut s = LineSplit::new(input);
        assert_eq!(s.next().await.unwrap().unwrap(), "line1");
        assert_eq!(s.next().await.unwrap().unwrap(), "line2");
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn yields_final_unterminated_line() {
        // Some upstreams don't terminate the last line; yield it anyway.
        let input = boxify(chunks(vec![Ok(Bytes::from_static(b"a\nb"))]));
        let mut s = LineSplit::new(input);
        assert_eq!(s.next().await.unwrap().unwrap(), "a");
        assert_eq!(s.next().await.unwrap().unwrap(), "b");
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn propagates_inner_error() {
        let err: BoxError = Box::new(std::io::Error::other("boom"));
        let input = boxify(chunks(vec![Err(err)]));
        let mut s = LineSplit::new(input);
        let got = s.next().await.unwrap().unwrap_err();
        assert_eq!(format!("{got}"), "boom");
    }
}