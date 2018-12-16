#![feature(generators, generator_trait)]
#![feature(custom_attribute)]

pub extern crate osaka_macros;
pub extern crate mio;

use std::io::Error;
use std::mem;
use std::ops::{Generator, GeneratorState};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::time::Duration;
pub use mio::Token;
pub use osaka_macros::osaka;

#[macro_export]
macro_rules! sync {
    ($x:ident) => {{
        use std::ops::GeneratorState;
        loop {
            match unsafe { $x.resume() } {
                GeneratorState::Complete(y) => {
                    let y = y?;
                    break y;
                }
                GeneratorState::Yielded(y) => {
                    yield y;
                }
            }
        }
    };};
}

pub struct Again {
    pub tokens:     Vec<Token>,
    pub timeout:    Option<Duration>,
}

#[derive(Clone)]
pub struct Poll {
    tokens: Arc<AtomicUsize>,
    poll:   Arc<mio::Poll>,
}

impl Poll {
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
        Ok(token)
    }
}

impl Again {
    pub fn empty() -> Self {
        Self { tokens: Vec::new(), timeout: None }
    }

    pub fn later(timeout: Duration) -> Self {
        Self { tokens: Vec::new(), timeout: Some(timeout)}
    }

    pub fn again(token: Token, timeout: Option<Duration>) -> Self {
        Self { tokens: vec![token], timeout }
    }

    pub fn merge(&mut self, mut other: Again) {
        if let Some(mut t2) = other.timeout {
            let t = if let Some(ref mut t1) = self.timeout {
                if t1 > &mut t2 {
                    true
                } else {
                    false
                }
            } else {
                true
            };
            if t {
                self.timeout = Some(t2);
            }
        }

        self.tokens.append(&mut other.tokens);
    }
}

pub struct Executor<Error> {
    exit:       bool,
    tokens:     Arc<AtomicUsize>,
    poll:       Arc<mio::Poll>,
    tasks:      Vec<Box<Generator<Yield = Again, Return = Result<(), Error>>>>,
    timeout:    Option<Duration>,
}

impl<Error> Executor<Error>
where
    Error: core::fmt::Debug,
{
    pub fn new() -> Executor<Error> {
        let poll = Arc::new(mio::Poll::new().unwrap());

        Self {
            exit: false,
            tokens: Arc::new(AtomicUsize::new(1)),
            poll,
            tasks: Vec::default(),
            timeout: None,
        }
    }

    pub fn with<X, F>(&mut self, f: F)
    where
        X: 'static + Generator<Yield = Again, Return = Result<(), Error>> + Sized,
        F: FnOnce(Poll) -> X,
    {
        let fx = f(Poll {
            tokens: self.tokens.clone(),
            poll: self.poll.clone(),
        });
        self.tasks.push(Box::new(fx));
    }

    pub fn activate(&mut self) -> Result<(), Error> {
        self.timeout = None;
        for mut v in mem::replace(&mut self.tasks, Vec::new()) {
            match unsafe { v.resume() } {
                GeneratorState::Complete(y) => {
                    let _: () = y?;
                    break;
                }
                GeneratorState::Yielded(y) => {
                    if let Some(y) = y.timeout {
                        self.timeout = Some(y);
                    }
                    self.tasks.push(v);
                }
            }
        }
        if self.tasks.is_empty() {
            self.exit = true;
        }
        Ok(())
    }

    pub fn run(&mut self) -> Result<(), Error> {
        let mut events = mio::Events::with_capacity(1024);
        loop {
            self.activate()?;
            if self.exit {
                break;
            }
            self.poll.poll(&mut events, self.timeout).unwrap();
        }
        Ok(())
    }
}
