extern crate futures;
extern crate tokio_timer;

/// A single client connection.
pub mod client;
/// Rooms (groups) of client connections.
pub mod room;

pub use self::client::Client;
pub use self::room::Room;

use std::io;
use std::error;
use std::fmt::{self, Debug};

/// Possible causes for a disconnection.
#[derive(Clone, Debug, PartialEq)]
pub enum Disconnect<T, R> {
    /// Closed with `Client::close` or similar.
    Closed,
    /// The `Sink` or `Stream` dropped.
    Dropped,
    /// Closed because of a timeout strategy.
    Timeout,
    /// Error in a `tokio_timer::Timer` being used for timeout.
    Timer(tokio_timer::TimerError),
    /// Error in the client's `Sink`.
    Sink(T),
    /// Error in the client's `Stream`.
    Stream(R),
}

/// Whether a client is connected or disconnected.
#[derive(Clone, Debug, PartialEq)]
pub enum Status<T, R> {
    /// The Client appears to be connected.
    Connected,
    /// The client is disconnected (and the `Disconnect` explaining why).
    Disconnected(Disconnect<T, R>),
}

impl<T, R> Status<T, R> {
    /// Is the client connected?
    pub fn is_connected(&self) -> bool {
        if let Status::Connected = *self {
            true
        } else {
            false
        }
    }

    /// Is the client disconnected.
    pub fn is_disconnected(&self) -> Option<&Disconnect<T, R>> {
        if let Status::Disconnected(ref e) = *self {
            Some(e)
        } else {
            None
        }
    }
}

/// Convert `io::Error` to a `Clone` representation.
///
/// `Client` and `Room` store the most recent error to conveniently keep track of connection
/// status. This requires those errors to be `Clone`. This offers an easy way to do that for
/// a common error type.
///
/// The easiest way to convert a `Stream<Error = io::Error>` or `Sink<SinkError = io::Error>`
/// is using `futures::Stream::from_err` and `futures::Sink::sink_from_err`.
///
/// ```rust,ignore
/// extern crate futures;
/// use std::io;
/// use futures::{stream, Stream};
///
/// let stream = stream::iter(vec![Ok(5), Err(io::Error::new(io::ErrorKind::Other, "oh no!"))]);
/// let stream_with_clone_error: Stream<Item = u64, Error = IoErrorString> = stream.from_err();
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorString(String);

impl From<io::Error> for ErrorString {
    fn from(e: io::Error) -> ErrorString {
        ErrorString(format!("{}", e))
    }
}

impl fmt::Display for ErrorString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for ErrorString {
    fn description(&self) -> &str {
        &self.0
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

// Utilities for testing `Communicator` implementations.
#[cfg(test)]
mod test {
    use super::*;
    use client::MpscClient;
    use std::sync::Arc;
    use futures::executor;
    use futures::sync::mpsc;

    #[derive(Clone, PartialEq, Debug)]
    pub enum TinyMsg {
        A,
        B(String),
    }

    pub fn unpark_noop() -> Arc<executor::Unpark> {
        struct Foo;

        impl executor::Unpark for Foo {
            fn unpark(&self) {}
        }

        Arc::new(Foo)
    }

    pub fn mock_client
        (id: &str,
         buffer_size: usize)
         -> (mpsc::Receiver<TinyMsg>, mpsc::Sender<TinyMsg>, MpscClient<String, TinyMsg>) {
        let (tx, rx_from_client) = mpsc::channel(buffer_size);
        let (tx_to_client, rx) = mpsc::channel(buffer_size);
        let client = Client::new_from_split(id.to_string(), tx, rx);
        (rx_from_client, tx_to_client, client)
    }
}
