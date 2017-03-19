use std::collections::VecDeque;
use futures::{stream, sink, Sink, Stream, Poll, Async, AsyncSink, StartSend};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CappedError<E> {
    CapExceeded,
    StreamError(E),
    SinkError(E),
}

/// A wrapper around a buffer that
pub struct CappedBufferedStream<S>
    where S: Stream
{
    rx: stream::Fuse<S>,
    will_not_read_again: bool,
    buffer_size: usize,
    buffer: VecDeque<Result<S::Item, CappedError<S::Error>>>,
}

impl<S> CappedBufferedStream<S>
    where S: Stream
{
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    #[doc(hidden)]
    pub fn into_inner(self) -> (S, VecDeque<Result<S::Item, CappedError<S::Error>>>) {
        (self.rx.into_inner(), self.buffer)
    }
}

impl<S> Stream for CappedBufferedStream<S>
    where S: Stream
{
    type Item = S::Item;
    type Error = CappedError<S::Error>;

    fn poll(&mut self) -> Poll<Option<S::Item>, CappedError<S::Error>> {
        if !self.will_not_read_again {
            if let Some(item) = match self.rx.poll() {
                Ok(Async::NotReady) => None,
                Ok(Async::Ready(Some(item))) => Some(Ok(item)),
                Ok(Async::Ready(None)) => {
                    self.will_not_read_again = true;
                    None
                }
                Err(e) => Some(Err(CappedError::StreamError(e))),
            } {
                // Even if the cap is exceeded, buffer this final item.
                self.buffer.push_back(item);
                // If the updated queue exceeds the buffer size, drop the Stream.
                if self.buffer.len() > self.buffer_size {
                    // Drop receive channel if cap exceeded.
                    self.will_not_read_again = true;
                    return Err(CappedError::CapExceeded);
                }
            }
        }

        match self.buffer.pop_front() {
            Some(Ok(item)) => Ok(Async::Ready(Some(item))),
            Some(Err(e)) => Err(e),
            None => {
                if self.will_not_read_again {
                    Ok(Async::Ready(None))
                } else {
                    Ok(Async::NotReady)
                }
            }
        }
    }
}

pub struct CappedBufferedSink<S>
    where S: Sink
{
    buffer_size: usize,
    buffered_tx: sink::Buffer<S>,
}

impl<S> CappedBufferedSink<S>
    where S: Sink
{
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }
}

impl<S> Sink for CappedBufferedSink<S>
    where S: Sink
{
    type SinkItem = S::SinkItem;
    type SinkError = CappedError<S::SinkError>;

    fn start_send(&mut self,
                  item: S::SinkItem)
                  -> StartSend<S::SinkItem, CappedError<S::SinkError>> {
        match self.buffered_tx.start_send(item) {
            Ok(AsyncSink::NotReady(_)) => Err(CappedError::CapExceeded),
            Ok(AsyncSink::Ready) => Ok(AsyncSink::Ready),
            Err(e) => Err(CappedError::SinkError(e)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), CappedError<S::SinkError>> {
        self.buffered_tx.poll_complete().map_err(CappedError::SinkError)
    }
}
