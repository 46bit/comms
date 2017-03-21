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
use std::time::Duration;

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
    Ready,
    /// The client is disconnected (and the `Disconnect` explaining why).
    Gone(Disconnect<T, R>),
}

impl<T, R> Status<T, R> {
    /// Is the client connected?
    pub fn is_ready(&self) -> bool {
        if let Status::Ready = *self {
            true
        } else {
            false
        }
    }

    /// Is the client disconnected.
    pub fn is_gone(&self) -> Option<&Disconnect<T, R>> {
        if let Status::Gone(ref e) = *self {
            Some(e)
        } else {
            None
        }
    }
}

/// Strategies for timeouts.
#[derive(Clone)]
pub enum Timeout {
    /// Don't time out.
    None,
    /// If a client misses the timeout, keep it connected.
    KeepAliveAfter(Duration, tokio_timer::Timer),
    /// If a client misses the timeout, disconnect it.
    DisconnectAfter(Duration, tokio_timer::Timer),
}

impl Debug for Timeout {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Timeout::*;
        match *self {
            None => write!(f, "Timeout::None"),
            KeepAliveAfter(d, _) => {
                write!(f, "Timeout::KeepAliveAfter({:?}, tokio_timer::Timer)", d)
            }
            DisconnectAfter(d, _) => {
                write!(f, "Timeout::DisconnectAfter({:?}, tokio_timer::Timer)", d)
            }
        }
    }
}

impl Timeout {
    /// Construct a new `Timeout` from an `Option<Duration>`, where
    /// if a client misses the timeout, keep it connected.
    ///
    /// `Some(duration)` becomes `Timeout::KeepAliveAfter`.
    /// `None` becomes `Timeout::None`.
    pub fn keep_alive_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> Timeout {
        match maybe_duration {
            Some(duration) => Timeout::KeepAliveAfter(duration, timer),
            None => Timeout::None,
        }
    }

    /// Construct a new `Timeout` from an `Option<Duration>`, where
    /// if a client misses the timeout, disconnect it.
    ///
    /// `Some(duration)` becomes `Timeout::DisconnectAfter`.
    /// `None` becomes `Timeout::None`.
    pub fn disconnect_after(maybe_duration: Option<Duration>,
                            timer: tokio_timer::Timer)
                            -> Timeout {
        match maybe_duration {
            Some(duration) => Timeout::DisconnectAfter(duration, timer),
            None => Timeout::None,
        }
    }

    #[doc(hidden)]
    pub fn to_sleep(&self) -> Option<tokio_timer::Sleep> {
        match *self {
            Timeout::None => None,
            Timeout::KeepAliveAfter(duration, ref timer) |
            Timeout::DisconnectAfter(duration, ref timer) => Some(timer.sleep(duration)),
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
pub struct IoErrorString(String);

impl From<io::Error> for IoErrorString {
    fn from(e: io::Error) -> IoErrorString {
        IoErrorString(format!("{}", e))
    }
}

impl fmt::Display for IoErrorString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl error::Error for IoErrorString {
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

    pub fn mock_client(id: &str,
                       buffer_size: usize)
                       -> (mpsc::Receiver<TinyMsg>,
                           mpsc::Sender<TinyMsg>,
                           Client<String, mpsc::Sender<TinyMsg>, mpsc::Receiver<TinyMsg>>) {
        let (tx, rx_from_client) = mpsc::channel(buffer_size);
        let (tx_to_client, rx) = mpsc::channel(buffer_size);
        (rx_from_client, tx_to_client, Client::new(id.to_string(), Timeout::None, tx, rx))
    }
}
