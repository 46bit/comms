# Design choices

## Can clients be in multiple Rooms simultaneously?

One clever way to make this work is if things are broken into traits. If an Arc<Mutex<_>>
could implement Client or various traits to that effect then the end-user could put a Client
into multiple rooms but the simple case would be zero-cost.

Trickiness would be that the future naturally consumes the Client. As such it'd need to be
taken out of the Arc<Mutex<_>> and suddenly that doesn't work anymore. Now it could have a
work queue inside and perform those work items as a separate Future (oh, hi, ClientRelay).

In short this should require a wrapper that waited until Client was put back, then continued.
IntoFuture and a queue for `task::park`/`task_unpark` could allow for that logic, but it's
getting past a V2 of Client.

## Can multiple actions be taken on a room simultaneously? (e.g., receive and transmit)

``` rust
// An obvious-to-implement API would be:
let a = room1.clone().broadcast(TinyMsg::A);
let b = room1.receive(ClientTimeout::_);
(a, b).and_then(|(room1_a, room1_b)| {
    ...
})
// This naturally requires extra synchronisation primitives. I'm not doing this when it
// seems not to offer anything but convenience - zero-cost abstractions!
```

- Would be preferable. Could require Client or Room to Arc<Mutex<_>> things.
- Killer usecase is my Msg::Room. We want to expect replies immediately from when it is sent.
  Having to wait for completion before looking for replies isn't a killer problem but it does
  cause some extra chaining.

``` rust
// A very JavaScripty Futures API would be:
room1.transmit(TinyMsg::A)
     .receive(ClientTimeout::_)
     .into_future().and_then(|(rx, room1)| { ... })
// vs typical Tokio
room1.transmit(TinyMsg::A)
     .and_then(|room1| room1.receive(ClientTimeout::_))
     .and_then(|room1| { ... })

// This could be possible zero-cost, without extra synchronisation primitives, by having the
// client queue up the futures internally.
//
// Having both APIs would be unpleasant.
//
// Explore the first one:
Room::transmit(self) -> RoomAction<Self>      RoomAction::transmit(self) -> RoomAction<Self>
Room::receive(self) -> RoomAction<Self>       RoomAction::receive(self) -> RoomAction<Self>
impl IntoFuture for RoomAction // ... or I suppose it could already be a Future
//
// could Deref or DerefMut somehow allow just implementing things on RoomAction? sadly seems
// not as they only deref references. DerefMove (#997) is postponed for the time being.
// an Into or IntoFuture could be defined but that'll look ugly. I suggest that I really do
// implement duplicate functions. Room only needs to setup for things, after all.
//
// if doing all this I'd rather have Room not be a Sink nor a Stream, just to direct people
// through this API. I am reimplementing bits of futures, sure, but I think there's value-add
// in that the Room operations cannot fail.
//
// if I want to chain multiple receives there'd be a type problem to build a simple future chain
// inside of RoomAction - what would the future's Item value be? this can be worked around to
// allow multiple receives, maybe, but with one function definition per level.
```

``` rust
// Similar model works for clients
client.transmit(TinyMsg::A)
      .receive(ClientTimeout::_)
      .into_future().and_then(|client| { ... })
```

## Can a single Room be filtered/where many times simultaneously?

``` rust
// Unless Client gets more complicated, there's nothing but name synchronous to filter on. So
// no need for the callbacking one.
let room1_f_a = room1.filter(|c| c.something == something);
let room1_f_b = room1.filter(|c| c.something2 == something2);
...
// This would seem to require extra synchronisation primitives as discussed elsewhere.
```

## What if you didn't merge Stream + Sink, or had easy ways to break them apart?

``` rust
// normally without the `ClientTimeout` we could use split
let (room_tx, room_rx) = room.split();
let msg = TinyMsg::A;
let timeout = ClientTimeout::_;
(room_tx.transmit(msg), room_rx.receive(timeout)).and_then(|(room_tx, room_rx)| {
    let room = Room::unsplit(room_tx, room_rx);
})
```

## Conclusion: types

``` rust
struct Client<I, R, T> {
    id: I,
    inner: Option<ClientInner<T, R>>,
}
struct ClientInner<T, R>
    where T: Sink + 'static,
          R: Stream + 'static
{
    tx: T,
    rx: R,
}
```

``` rust
struct Room<I, R, T> {
    clients: HashMap<I, Client<I, R, T>>
}
```
