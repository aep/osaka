#![feature(generators, generator_trait)]
#![feature(custom_attribute)]

pub extern crate osaka_macros;

pub extern crate mio;

use std::io::Error;
use std::mem;
use std::ops::{Generator, GeneratorState};
use std::sync::Arc;

use core::time::Duration;
pub use mio::Token;
pub use osaka_macros::osaka;
use std::collections::HashMap;

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
    pub token: Option<Token>,
    pub timeout: Option<Duration>,
}

#[derive(Clone)]
pub struct Poll {
    pub token: Token,
    pub poll: Arc<mio::Poll>,
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
        self.poll
            .register(handle, self.token.clone(), interest, opts)?;
        Ok(self.token.clone())
    }
}

impl Again {
    pub fn new(token: Option<Token>, timeout: Option<Duration>) -> Self {
        Self { token, timeout }
    }
}

pub struct Executor<Error> {
    exit: bool,
    poll: Arc<mio::Poll>,
    tasks: HashMap<Token, Box<Generator<Yield = Again, Return = Result<(), Error>>>>,
    timeout: Option<Duration>,
}

impl<Error> Executor<Error>
where
    Error: core::fmt::Debug,
{
    pub fn new() -> Executor<Error> {
        let poll = Arc::new(mio::Poll::new().unwrap());

        Self {
            exit: false,
            poll,
            tasks: HashMap::default(),
            timeout: None,
        }
    }

    pub fn with<X, F>(&mut self, f: F)
    where
        X: 'static + Generator<Yield = Again, Return = Result<(), Error>> + Sized,
        F: FnOnce(Poll) -> X,
    {
        let token = mio::Token(0);
        let fx = f(Poll {
            token: token.clone(),
            poll: self.poll.clone(),
        });
        self.tasks.insert(token, Box::new(fx));
    }

    pub fn activate(&mut self) -> Result<(), Error> {
        self.timeout = None;
        for (k, mut v) in mem::replace(&mut self.tasks, HashMap::new()) {
            match unsafe { v.resume() } {
                GeneratorState::Complete(y) => {
                    let y = y?;
                    eprintln!("completed {:?}", y);
                }
                GeneratorState::Yielded(y) => {
                    if let Some(y) = y.timeout {
                        self.timeout = Some(y);
                    }
                    self.tasks.insert(k, v);
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