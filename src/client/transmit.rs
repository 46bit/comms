use futures::{Future, Sink, Stream, Poll, Async};
use super::*;

pub struct Transmit<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static
{
    client: Option<Client<I, C>>,
    msg: Option<C::SinkItem>,
}

impl<I, C> Transmit<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static
{
    #[doc(hidden)]
    pub fn new(client: Client<I, C>, msg: C::SinkItem) -> Transmit<I, C> {
        Transmit {
            client: Some(client),
            msg: Some(msg),
        }
    }

    pub fn into_inner(mut self) -> Option<Client<I, C>> {
        self.client.take()
    }
}

impl<I, C> Future for Transmit<I, C>
    where I: Clone + Send + Debug + 'static,
          C: Sink + Stream + 'static
{
    type Item = Client<I, C>;
    type Error = Client<I, C>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if let Some(msg) = self.msg.take() {
            let start_send = {
                let client = self.client.as_mut().expect("Polled after Async::Ready.");
                client.start_send(msg)
            };
            match start_send {
                Ok(AsyncSink::NotReady(msg)) => {
                    self.msg = Some(msg);
                    return Ok(Async::NotReady);
                }
                Ok(AsyncSink::Ready) => {}
                Err(()) => return Err(self.client.take().unwrap()),
            }
        }

        let poll_complete = {
            let client = self.client.as_mut().expect("Polled after Async::Ready.");
            client.poll_complete()
        };
        match poll_complete {
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Ok(Async::Ready(())) => Ok(Async::Ready(self.client.take().unwrap())),
            Err(()) => Err(self.client.take().unwrap()),
        }
    }
}
