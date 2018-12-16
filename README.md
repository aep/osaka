osaka is async for rust, inspired by go and the clay programming language

Its designed around continuations rather than combinators,
allowing a much more readable flow.


Why
------

rust's tokio/futures ecosystem is a complex monolith that doesn't work well for constrained devices such as
the 2MB flash mips boxes i work on (tokio is 1Mb alone, plus all the futures combinators)
osaka is more of a hack that works for me rather than an attempt to overtake futures.rs.

Continuations are much easier to understand than combinators and require no specific runtime.


what it looks like
----

originally i was planning to implement a proc macro that would allow for golang style chans

```rust
#[osaka]
pub fn something(bar: chan String) {
    let foo <- bar;
}
```

however, due to lack of interest in alternatives to tokio, i decided to just roll with the absolut minimum effort,
so it looks like this:


```rust
#[osaka]
pub fn something(bar: Channel<String>) {
    let foo = sync!(bar);
}
```


in real code you will probably want to register something to a Poll instance to re-activate the closure when the poll is ready.


```rust
#[osaka]
pub fn something(poll: Poll) -> Result<Vec<String>, std::io::Error> {
    let sock = mio::UdpSocket::bind(&"0.0.0.0:0".parse().unwrap())?;
    let token = poll.register(&sock, mio::Ready::readable(), mio::PollOpt::level()).unwrap();

    loop {
        let mut buf = vec![0; 1024];
        if let Err(e) = sock.recv_from(&mut buf) {
            if e.kind() == std::io::ErrorKind::WouldBlock {
                yield Again::again(token, None);
            }
        }
    }
}
```

note that there is no singleton runtime in the background. poll is explicitly passed as argument. 
osaka is significantly more simplistic than futures.rs on purpose.


differences to async/await
---------------------------

One of the most important features is that all behaviour is well defined.
A panic is always a bug in osaka, not in your code.
Osaka is generally more designed for the "it compiles, ship it" workflow.
and more biased towards explicitness and "easy to argue about" rather
than trying to hide the event flow from the user for the sake of "easy to write" code.

- osaka does not have implicit dependencies
- osaka::Again contains a continuation token instead of a hidden singleton "task" registry.
- readyness is explicit, making the code easier to argue about in terms of "what is happening here"
- all errors are explicit
- there is no undefined behaviour. a panic is a bug in osaka, not in your code.
- "hot functions" as described in RFC2394 work fine in osaka, since continuation points are explicit.
