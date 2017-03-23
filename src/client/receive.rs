use std::time::Duration;
use futures::{Future, Sink, Stream, Poll, Async};
use super::*;

pub struct Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
}

impl<I, C> Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>) -> Receive<I, C> {
        Receive { client: Some(client) }
    }

    pub fn with_hard_timeout(mut self,
                             duration: Duration,
                             timer: &tokio_timer::Timer)
                             -> ReceiveWithHardTimeout<I, C> {
        ReceiveWithHardTimeout::new(self.client.take().unwrap(), duration, timer)
    }

    pub fn with_soft_timeout(mut self,
                             duration: Duration,
                             timer: &tokio_timer::Timer)
                             -> ReceiveWithSoftTimeout<I, C> {
        ReceiveWithSoftTimeout::new(self.client.take().unwrap(), duration, timer)
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for Receive<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (C::Item, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(Some(msg))) => Ok(Async::Ready((msg, self.take_client()))),
            Ok(Async::Ready(None)) |
            Err(_) => Err(self.take_client()),
        }
    }
}

pub struct ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
    sleep: tokio_timer::Sleep,
}

impl<I, C> ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>,
               timeout_duration: Duration,
               timer: &tokio_timer::Timer)
               -> ReceiveWithHardTimeout<I, C> {
        ReceiveWithHardTimeout {
            client: Some(client),
            sleep: timer.sleep(timeout_duration),
        }
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for ReceiveWithHardTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (C::Item, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => {}
            Ok(Async::Ready(Some(msg))) => {
                return Ok(Async::Ready((msg, self.client.take().unwrap())));
            }
            Ok(Async::Ready(None)) |
            Err(_) => return Err(self.take_client()),
        }

        match self.sleep.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timeout);
                }
                Err(client)
            }
            Err(e) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timer(e));
                }
                Err(client)
            }
        }
    }
}

pub struct ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    client: Option<Client<I, C>>,
    sleep: tokio_timer::Sleep,
}

impl<I, C> ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>,
               timeout_duration: Duration,
               timer: &tokio_timer::Timer)
               -> ReceiveWithSoftTimeout<I, C> {
        ReceiveWithSoftTimeout {
            client: Some(client),
            sleep: timer.sleep(timeout_duration),
        }
    }

    fn take_client(&mut self) -> Client<I, C> {
        self.client.take().expect("Polled after Async::Ready.")
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for ReceiveWithSoftTimeout<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static,
          C::SinkError: Clone,
          C::Error: Clone
{
    type Item = (Option<C::Item>, Client<I, C>);
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let client_poll = {
            let client = self.client.as_mut().unwrap();
            client.poll()
        };
        match client_poll {
            Ok(Async::NotReady) => {}
            Ok(Async::Ready(Some(msg))) => return Ok(Async::Ready((Some(msg), self.take_client()))),
            Ok(Async::Ready(None)) |
            Err(_) => return Err(self.take_client()),
        }

        match self.sleep.poll() {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(_)) => Ok(Async::Ready((None, self.take_client()))),
            Err(e) => {
                let mut client = self.take_client();
                if client.inner.is_ok() {
                    client.inner = Err(Disconnect::Timer(e));
                }
                Err(client)
            }
        }
    }
}