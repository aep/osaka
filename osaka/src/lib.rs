#![feature(generators, generator_trait)]
#![feature(custom_attribute)]
#![feature(termination_trait_lib)]

pub extern crate osaka_macros;
pub extern crate mio;
pub extern crate log;

use std::io::Error;
use std::ops::{Generator, GeneratorState};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use log::{warn, debug};
use std::time::{Duration, Instant};

pub use osaka_macros::osaka;

/**
    convenience macro to wait for another future inside an osaka async fn

    if bar is a future that returns a string, for example:

    ```
    let foo : String = sync!(bar);
    printfn!("yo! {}", foo);
    ```

    this is asynchronous and does not block the current thread.
    Instead it will return the Again activation handle of `bar` if its not ready

*/
#[macro_export]
macro_rules! sync {
    ($task:expr) => {{
        use osaka::FutureResult;
        use osaka::Future;
        loop {
            match $task.poll() {
                FutureResult::Done(y) => {
                    break y;
                }
                FutureResult::Again(y) => {
                    yield y;
                }
            }
        }
    };};
}

#[macro_export]
macro_rules! try{
    ($e:expr) => {
        match $e {
            Err(e) => return $crate::FutureResult::Done(Err(e.into())),
            Ok(v) => v,
        }
    }
}

/// an activation token
#[derive(Clone)]
pub struct Token {
    m: mio::Token,
    active: Arc<AtomicUsize>,
}


/** Activation Handle

Again contains tokens that tell the execution engine when to reactivate this task.
*/

#[derive(Clone)]
pub struct Again {
    tokens:     Vec<Token>,
    deadline:   Option<Instant>,
    poll:       Arc<mio::Poll>,
}

impl Again {
    pub fn merge(&mut self, mut other: Again) {
        if let Some(mut t2) = other.deadline {
            let t = if let Some(ref mut t1) = self.deadline {
                if t1 > &mut t2 {
                    true
                } else {
                    false
                }
            } else {
                true
            };
            if t {
                self.deadline = Some(t2);
            }
        }
        self.tokens.append(&mut other.tokens);
    }
}

/// It's either done or we'll try again later
pub enum FutureResult<F> {
    Done(F),
    Again(Again),
}

/// something that can be resumed
pub trait Future<R> {
    fn poll(&mut self) -> FutureResult<R>;
}

/*
impl<R,X> Future<R> for X
where X: FnMut() -> FutureResult<R> {
    fn poll(&mut self) -> FutureResult<R> {
        (self)()
    }
}
*/

impl<R,X> Future<R> for X
where X: Generator<Yield = Again, Return = R>
{
    fn poll(&mut self) -> FutureResult<R> {
        match unsafe { self.resume() } {
            GeneratorState::Complete(y) => {
                FutureResult::Done(y)
            }
            GeneratorState::Yielded(a) => {
                FutureResult::Again(a)
            }
        }
    }
}

/**
  Task execution engine.

 */
#[derive(Clone)]
pub struct Poll {
    tokens: Arc<AtomicUsize>,
    poll:   Arc<mio::Poll>,
}

impl Poll {
    /// register a mio::Evented as a wake up source
    pub fn register<E: ?Sized>(
        &self,
        handle: &E,
        interest: mio::Ready,
        opts: mio::PollOpt,
    ) -> Result<Token, Error>
    where
        E: mio::Evented,
    {
        let token = mio::Token(self.tokens.fetch_add(1, Ordering::SeqCst));
        self.poll.register(handle, token, interest, opts)?;
        Ok(Token{
            m: token,
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// create an execution engine
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(AtomicUsize::new(0)),
            poll:   Arc::new(mio::Poll::new().unwrap()),
        }
    }

    /// returns an Again that will never be activated because it contains no wakeup sources
    pub fn never(&self) -> Again {
        Again { poll: self.poll.clone(), tokens: Vec::new(), deadline: None }
    }

    /// wake up after the specified time has passed
    pub fn later(&self, deadline: Duration) -> Again {
        Again { poll: self.poll.clone(), tokens: Vec::new(), deadline: Some(Instant::now() + deadline)}
    }

    /// wake up either when the token is ready or after the specified time has passed
    pub fn again(&self, token: Token, deadline: Option<Duration>) -> Again {
        Again { poll: self.poll.clone(), tokens: vec![token], deadline: deadline.map(|v|Instant::now() + v) }
    }

    /// wake up when any of the tokens is ready or after the specified time has passed
    pub fn any(&self, tokens:Vec<Token>, deadline: Option<Duration>) -> Again {
        Again { poll: self.poll.clone(), tokens, deadline: deadline.map(|v|Instant::now() + v) }
    }
}


/**
Something that can be activated

An osaka task is usually constructed by adding the osaka macro to a function, like so:

```
#[osaka]
fn the_answer(poll: osaka::Poll) -> u32 {
    let oracle = Oracle{};
    let token = poll.register(oracle);
    if oracle.is_ready() {
        return 42;
    } else {
        yield poll.again(token);
    }
}
```

*/
pub enum Task<R> {
    Later {
        f: Box<Future<R>>,
        a: Again,
    },
    Immediate {
        r: Option<R>,
    }
}

impl<R> Task<R> {

    /// run a task to completion, blocking the current thread.
    ///
    /// this is not re-entrant, meaning you cannot call this function from some callback.
    /// It is also not thread safe. Basically only ever call this once, prefferably in main.
    pub fn run(&mut self) -> R {
        loop {
            match self {
                Task::Immediate{r} =>  {
                    return r.take().expect("immediate polled after completion");
                }
                Task::Later{f,a} =>  {
                    let mut events = mio::Events::with_capacity(1024);
                    let mut timeout = None;
                    if let Some(deadline) = a.deadline {
                        let now = Instant::now();
                        if now > deadline {
                            warn!("deadline already expired. will loop in 1ms");
                            timeout = Some(Duration::from_millis(1));
                        } else {
                            timeout = Some(deadline - now);
                        }
                    }

                    if a.tokens.len() == 0 {
                        panic!("trying to run() with 0 tokens, this is not going to do anything useful.\n
                       forgot to pass a token with poll.again() ?");
                    }


                    debug!("going to poll with timeout {:?} and {} tokens",
                           timeout,
                           a.tokens.len());

                    a.poll.poll(&mut events, timeout).expect("poll");

                    for token in &a.tokens {
                        token.active.store(0, Ordering::SeqCst);
                    }
                    for event in &events {
                        for token in &a.tokens {
                            if event.token() == token.m {
                                debug!("token {:?} activated", token.m);
                                token.active.store(1, Ordering::SeqCst);
                            }
                        }
                    }

                }
            }
            if let FutureResult::Done(v) = self.poll() {
                return v;
            }
        }
    }

    /// the brave may construct a Task manually from a Future
    ///
    /// the passed Again instance needs to already contain an activation source,
    /// or the task will never be executed
    ///
    ///
    /// for example:
    ///
    /// ```
    /// struct Santa {
    ///     poll: Poll
    /// }
    ///
    /// impl Future for Santa {
    ///     fn poll(&mut self) -> FutureResult<Present> {
    ///         FutureResult::Again(self.poll.never())
    ///     }
    /// }
    ///
    /// fn main() {
    ///     let poll = Poll::new();
    ///     let santa = Santa{poll};
    ///     santa.run().unwrap();
    /// }
    ///
    /// ```
    pub fn new(f: Box<Future<R>>, a: Again) -> Self {
        Task::Later{f,a}
    }



    /// force a wakeup the next time `activate` is called. This is for a poor implementation of
    /// channels and you should probably not use this.
    pub fn wakeup_now(&mut self)  {
        if let Task::Later{f,a} = self {
            a.deadline = Some(Instant::now());
        }
    }


    pub fn immediate(t:R) -> Task<R> {
        Task::Immediate{r:Some(t)}
    }

}

impl<R> Future<R> for Task<R> {
    /// this is called by the execution engine, or a sync macro.
    ///
    /// you can call this by hand, but it won't actually do anything unless the task
    /// contains a token that is ready, or has an expired deadline
    fn poll(&mut self) -> FutureResult<R> {
        match self {
            Task::Immediate{r} =>  {
                return FutureResult::Done(r.take().expect("immediate polled after completion"));
            }
            Task::Later{f,a} =>  {
                let mut ready = false;

                if let Some(deadline) = a.deadline {
                    if Instant::now() >= deadline {
                        debug!("task wakeup caused by deadline");
                        a.deadline = None;
                        ready = true;
                    }
                }

                if !ready {
                    for token in &a.tokens {
                        if token.active.load(Ordering::SeqCst) > 0 {
                            debug!("task wakeup caused by token readyness");
                            ready = true;
                            break;
                        }
                    }
                }

                if ready {
                    match f.poll() {
                        FutureResult::Done(y) => {
                            return FutureResult::Done(y);
                        },
                        FutureResult::Again(a2) => {
                            *a = a2;
                        }
                    }
                }

                FutureResult::Again(a.clone())
            }
        }
    }
}




impl<E:  std::fmt::Debug> std::process::Termination for Task<Result<(), E>> {
    fn report(mut self) -> i32 {
        match self.run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{:?}", e);
                2
            }
        }
    }
}
