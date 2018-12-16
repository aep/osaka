#![feature(generators, generator_trait, custom_attribute)]
use std::ops::Generator;
use std::time::Instant;

extern crate osaka;
use std::time::Duration;
use osaka::mio;
use osaka::mio::net::UdpSocket;
use osaka::{osaka, Again, Poll};
use std::mem;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum Error {
    NameTooLong,
    OutOfOptions,
    Io(std::io::Error),
}

#[repr(C)]
pub struct DnsPacket {
    /// query id
    id: u16,
    /// flags
    flags: u16,
    ///  number of queries
    queries: u16,
    /// number of answers
    answers: u16,
    /// number of authority something
    authorities: u16,
    /// some crap
    additionals: u16,
}

fn send_query(name: &str, sock: &UdpSocket, to: &SocketAddr) -> Result<(), Error> {
    let mut pkt: DnsPacket = unsafe { mem::zeroed() };
    pkt.id = 0x1337u16.to_be();
    pkt.flags = 0x100u16.to_be(); //request recursion
    pkt.queries = 1u16.to_be();
    pkt.answers = 0;
    pkt.authorities = 0;
    pkt.additionals = 0;

    if name.as_bytes().len() > 512 {
        return Err(Error::NameTooLong);
    }

    let mut payload = unsafe {
        std::slice::from_raw_parts(
            (&pkt as *const DnsPacket) as *const u8,
            mem::size_of::<DnsPacket>(),
        )
    }
    .to_vec();

    for label in name.split('.') {
        payload.push(label.as_bytes().len() as u8);
        payload.extend(label.as_bytes());
    }

    payload.extend(&[
        0,    //end of labels
        0,    //16bit padding
        0x10, //request TXT
        0,    //16bit padding
        1,    //inet class
    ]);

    sock.send_to(&payload, &to).unwrap();

    Ok(())
}

#[osaka]
pub fn resolve(poll: Poll, names: Vec<String>) -> Result<Vec<String>, Error> {
    let dns_servers = vec![
        "1.1.1.1:53".parse().unwrap(),
        "8.8.8.8:53".parse().unwrap(),
        "9.9.9.9:53".parse().unwrap(),
        "78.35.40.149:53".parse().unwrap(),
    ];
    for to in dns_servers {
        for name in names.clone() {
            let now = Instant::now();
            let sock = UdpSocket::bind(&"0.0.0.0:0".parse().unwrap()).map_err(|e| Error::Io(e))?;
            let token = poll
                .register(&sock, mio::Ready::readable(), mio::PollOpt::level())
                .unwrap();
            send_query(&name, &sock, &to)?;
            let pkt = match loop {
                yield Again::again(token, Some(Duration::from_secs(5)));
                if now.elapsed() >= Duration::from_secs(5) {
                    //timeout
                    break None;
                }

                let mut buf = vec![0; 1024];
                let (len, from) = match sock.recv_from(&mut buf) {
                    Ok(v) => v,
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock {
                            continue;
                        }
                        return Err(Error::Io(e));
                    }
                };
                if from == to && len >= mem::size_of::<DnsPacket>() {
                    buf.truncate(len);
                    break Some(buf);
                }
            } {
                Some(v) => v,
                None => continue,
            };

            let header: &DnsPacket = unsafe { mem::transmute(pkt.as_ptr() as *const DnsPacket) };
            let answers = u16::from_be(header.answers);

            if answers < 1 {
                continue;
            }

            let mut at = mem::size_of::<DnsPacket>();
            // skip the query sections
            for _ in 0..u16::from_be(header.queries) {
                while at < pkt.len() - 1 {
                    // find end of labels
                    if pkt[at] == 0 {
                        // from here the section is 5 more bytes long
                        at += 5;
                        break;
                    }
                    at += 1;
                }
            }

            let mut answers = Vec::new();

            for _ in 0..u16::from_be(header.answers) {
                // find start of answer header
                while at < pkt.len() - 1 {
                    if pkt[at] == 0 {
                        break;
                    }
                    if pkt[at] == 0xc0 {
                        at += 1;
                        break;
                    }
                    at += 1;
                }
                at += 1;
                if at >= pkt.len() {
                    break;
                }

                let record_type: *const u16 =
                    unsafe { mem::transmute(pkt[at..].as_ptr() as *const u16) };
                let record_type = u16::from_be(unsafe { *record_type });
                at += 2;
                if at >= pkt.len() {
                    break;
                }

                let record_class: *const u16 =
                    unsafe { mem::transmute(pkt[at..].as_ptr() as *const u16) };
                let record_class = u16::from_be(unsafe { *record_class });
                at += 6;
                if at >= pkt.len() {
                    break;
                }

                let record_len: *const u16 =
                    unsafe { mem::transmute(pkt[at..].as_ptr() as *const u16) };
                let record_len = u16::from_be(unsafe { *record_len }) as usize;
                at += 2;
                if at + record_len > pkt.len() {
                    break;
                }

                if record_type == 0x10 && record_class == 0x01 {
                    // wtf is the additional text length?
                    answers
                        .push(String::from_utf8_lossy(&pkt[at + 1..at + record_len]).to_string());
                }
                at += record_len;
            }

            if answers.len() > 0 {
                return Ok(answers);
            }
        }
    }
    Err(Error::OutOfOptions)
}

#[osaka]
#[cfg(test)]
pub fn r(poll: Poll) -> Result<(), Error> {
    let mut a = resolve(
        poll.clone(),
        vec![
            "3.carrier.devguard.io".into(),
            "x.carrier.devguard.io".into(),
        ],
    );
    let y = osaka::sync!(a);
    println!("{:?}", y);
    Ok(())
}

#[test]
pub fn main() {
    let mut ex = osaka::Executor::new();
    ex.with(|poll| r(poll));
    ex.run().unwrap();
}
