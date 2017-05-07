/*
* Copyright (c) 2016 Alex Crichton
*
* Permission is hereby granted, free of charge, to any
* person obtaining a copy of this software and associated
* documentation files (the "Software"), to deal in the
* Software without restriction, including without
* limitation the rights to use, copy, modify, merge,
* publish, distribute, sublicense, and/or sell copies of
* the Software, and to permit persons to whom the Software
* is furnished to do so, subject to the following
* conditions:
*
* The above copyright notice and this permission notice
* shall be included in all copies or substantial portions
* of the Software.
*
* THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
* ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
* TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
* PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
* SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
* CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
* OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
* IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
* DEALINGS IN THE SOFTWARE.
*
* Mles-support Copyright (c) 2017 Mles developers
*
*/

/*
 * Mles client example based on Tokio core-connect example.
 */

extern crate mles_utils;
extern crate futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate bytes;

/* websocket proxy support */
extern crate websocket;

use std::{env, process};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::thread;
use std::io::{Error, ErrorKind};

use bytes::{BufMut, BytesMut};
use futures::{Sink, Future, Stream};
use futures::sync::mpsc;
use tokio_core::reactor::Core;
use tokio_core::net::TcpStream;
use tokio_io::AsyncRead;
use tokio_io::codec::{Encoder, Decoder};
use mles_utils::*;

/* websocket proxy support */
use websocket::{Server, Message};
use websocket::message::Type;

const SRVPORT: &str = ":8077";
const WSPORT: &str = ":8076";
const KEYL: usize = 8; //key len
const HDRKEYL: usize = 4 + KEYL; //hdr + key len
const USAGE: &str = "Usage: mles-client <server-address> [--use-websockets]";

fn main() {
    let mut ws_enabled: Option<bool> = None;

    // Parse what address we're going to connect to
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| {
                            println!("{}", USAGE);
                            process::exit(1);
                        });

    let raddr = addr + SRVPORT;
    let raddr: Vec<_> = raddr.to_socket_addrs()
        .unwrap_or(vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0)].into_iter())
        .collect();
    let raddr = *raddr.first().unwrap();
    let raddr = Some(raddr).unwrap();
    // check that we really got a proper peer
    if 0 == raddr.port() {
        println!("{}", USAGE);
        process::exit(1);
    }

    if let Some(ws) = env::args().nth(2) {
        if ws == "--use-websockets" {
            ws_enabled = Some(true);
        } else {
            println!("{}", USAGE);
            process::exit(1);
        }
    }

    let keyval = match env::var("MLES_KEY") {
        Ok(val) => val,
        Err(_) => "".to_string(),
    };

    let keyaddr = match env::var("MLES_ADDR_KEY") {
        Ok(val) => val,
        Err(_) => "".to_string(),
    };

    if let Some(_) = ws_enabled {
        /* Websocket proxy support */
        if let Ok(ws_server) = Server::bind("0.0.0.0".to_string() + WSPORT) {
            println!("Listening Websockets clients at port{}", WSPORT);
            for connection in ws_server.filter_map(Result::ok) {
                let keyval = keyval.clone();
                let keyaddr = keyaddr.clone();
                thread::spawn(move || {
                    // Handle stdin in a separate thread
                    let (ws_tx, ws_rx) = mpsc::channel(16);
                    let (mles_tx_ws, mles_rx_ws) = std::sync::mpsc::channel();

                    let mut core = Core::new().unwrap();
                    let handle = core.handle();
                    let tcp = TcpStream::connect(&raddr, &handle);
                    let mut key: Option<u64> = None;
                    let mut keys = Vec::new();

                    if !connection
                            .protocols()
                            .contains(&"mles-websocket".to_string()) {
                        let _ = connection.reject().map_err(|_| ());
                        println!("Protocol rejected");
                        return;
                    }
                    let client = match connection.use_protocol("mles-websocket").accept() {
                        Ok(client) => client,
                        Err(_) => {
                            println!("Connection rejected");
                            return;
                        }
                    };

                    let ip = client.peer_addr().unwrap();
                    println!("Connection from {}", ip);

                    let (mut receiver, mut sender) = client.split().unwrap();
                    let mles_tx_ws_inner = mles_tx_ws.clone();
                    thread::spawn(move || for message in receiver.incoming_messages() {
                        let ws_tx_msg = ws_tx.clone();
                        let mles_tx_ws_msg = mles_tx_ws_inner.clone();
                        let message: Message = match message {
                            Ok(message) => message,
                            Err(_) => {
                                //Sometimes message just fails..
                                let empty_msg = Vec::new();
                                let _ = message_forward_all(mles_tx_ws_msg, ws_tx_msg, empty_msg).map_err(|_| {});
                                println!("Message failed, client {} disconnected", ip);
                                return;
                            }
                        };
                        match message.opcode {
                            Type::Close => {
                                let message = Message::close();
                                let msg = message.payload.into_owned().to_vec();
                                let _ = message_forward_all(mles_tx_ws_msg, ws_tx_msg, msg);
                                println!("Close: Client {} disconnected", ip);
                                return;
                            }
                            Type::Ping => {
                                let message = Message::pong(message.payload);
                                let msg = message.payload.into_owned().to_vec();
                                let _ = message_forward_mles(mles_tx_ws_msg, ws_tx_msg, msg).map_err(|_| {
                                    println!("Client {} disconnected", ip);
                                    return;
                                });
                            }
                            _ => {
                                let msg = message.payload.into_owned().to_vec();
                                let _ = message_forward_all(mles_tx_ws_msg.clone(), ws_tx_msg.clone(), msg).map_err(|_| {
                                    let message = Message::close();
                                    let msg = message.payload.into_owned().to_vec();
                                    let _ = message_forward_all(mles_tx_ws_msg, ws_tx_msg, msg);
                                    println!("Client {} disconnected", ip);
                                    return;
                                });
                            }
                        }
                    });
                    thread::spawn(move || loop {
                        if let Ok(mles_msg) = mles_rx_ws.recv() {
                            if 0 == mles_msg.len() {
                                let message = Message::close();
                                match sender.send_message(&message) {
                                    Ok(_) => {}
                                    Err(err) => {
                                        println!("Client tx error: {}", err);
                                    }
                                }
                                return;
                            }
                            let message: Message = Message::binary(mles_msg);
                            match sender.send_message(&message) {
                                Ok(_) => {}
                                Err(err) => {
                                    println!("Client tx error: {}", err);
                                }
                            }
                        } else {
                            return;
                        }
                    });

                    let client = tcp.and_then(|stream| {
                        let _val = stream
                            .set_nodelay(true)
                            .map_err(|_| panic!("Cannot set to no delay"));
                        let laddr = match stream.local_addr() {
                            Ok(laddr) => laddr,
                            Err(_) => {
                                let addr = "0.0.0.0:0";
                                let addr = addr.parse::<SocketAddr>().unwrap();
                                addr
                            }
                        };
                        if  keyval.len() > 0 {
                            keys.push(KeyInput::Str(keyval.clone()));
                        } else {            
                            keys.push(KeyInput::Addr(laddr));
                            if keyaddr.len() > 0 {
                                keys.push(KeyInput::Str(keyaddr.clone()));
                            }
                        }
                        let (sink, stream) = stream.framed(Bytes).split();
                        let ws_rx = ws_rx.map_err(|_| panic!()); // errors not possible on rx XXX
                        let ws_rx = ws_rx.and_then(|buf| {
                            if 0 == buf.len() {
                                println!("Got broken pipe");
                                return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                            }
                            if None == key {
                                //create hash for verification
                                let decoded_message = message_decode(buf.as_slice());
                                keys.push(KeyInput::Str(decoded_message.get_uid().to_string()));
                                keys.push(KeyInput::Str(decoded_message.get_channel().to_string()));
                                key = Some(do_hash(&keys));
                            }
                            let mut keyv = write_key(key.unwrap());
                            keyv.extend(buf);
                            Ok(keyv)
                        });

                        let send_wsrx = ws_rx.forward(sink);
                        let write_wstx = stream.for_each(move |buf| {
                            // send to websocket
                            match mles_tx_ws.send(buf.to_vec()) {
                                Ok(_) => {}
                                Err(_) => {
                                    return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                                }
                            }
                            if 0 == buf.len() {
                                return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                            }
                            Ok(())
                        });

                        send_wsrx
                            .map(|_| ())
                            .select(write_wstx.map(|_| ()))
                            .then(|_| Ok(()))
                    });

                    let _run = match core.run(client) {
                        Ok(_) => {}
                        Err(err) => {
                            println!("Error: {}", err);
                        }
                    };
                    println!("Returning from main ws thread");
                });
            }
        } else {
            println!("Could not bind to websockets port");
        }
    } else {
        // Handle stdin in a separate thread
        let (stdin_tx, stdin_rx) = mpsc::channel(16);

        let mut core = Core::new().unwrap();
        let handle = core.handle();
        let tcp = TcpStream::connect(&raddr, &handle);
        let mut key: Option<u64> = None;
        let mut keys = Vec::new();

        /* Normal console connection */
        thread::spawn(|| read_stdin(stdin_tx));
        let stdin_rx = stdin_rx.map_err(|_| panic!()); // errors not possible on rx

        let mut stdout = io::stdout();
        let client = tcp.and_then(|stream| {
            let _val = stream
                .set_nodelay(true)
                .map_err(|_| panic!("Cannot set to no delay"));
            let laddr = match stream.local_addr() {
                Ok(laddr) => laddr,
                Err(_) => {
                    let addr = "0.0.0.0:0";
                    let addr = addr.parse::<SocketAddr>().unwrap();
                    addr
                }
            };
            if  keyval.len() > 0 {
                keys.push(KeyInput::Str(keyval.clone()));
            } else {            
                keys.push(KeyInput::Addr(laddr));
                if keyaddr.len() > 0 {
                    keys.push(KeyInput::Str(keyaddr.clone()));
                }
            }
            let (sink, stream) = stream.framed(Bytes).split();
            let stdin_rx = stdin_rx.and_then(|buf| {
                if None == key {
                    //create hash for verification
                    let decoded_message = message_decode(buf.as_slice());
                    keys.push(KeyInput::Str(decoded_message.get_uid().to_string()));
                    keys.push(KeyInput::Str(decoded_message.get_channel().to_string()));
                    key = Some(do_hash(&keys));
                }
                let mut keyv = write_key(key.unwrap());
                keyv.extend(buf);
                Ok(keyv)
            });

            let send_stdin = stdin_rx.forward(sink);
            let write_stdout = stream.for_each(move |buf| {
                let decoded = message_decode(buf.to_vec().as_slice());
                let mut msg = "".to_string();
                if decoded.get_message().len() > 0 {
                    msg.push_str(decoded.get_uid());
                    msg.push_str(":");
                    msg.push_str(String::from_utf8_lossy(decoded.get_message().as_slice())
                                     .into_owned()
                                     .as_str());
                }
                stdout.write_all(&msg.into_bytes())
            });

            send_stdin
                .map(|_| ())
                .select(write_stdout.map(|_| ()))
                .then(|_| Ok(()))
        });

        let _run = match core.run(client) {
            Ok(_) => {}
            Err(err) => {
                println!("Error: {}", err);
                process::exit(1);
            }
        };
    }
}

struct Bytes;

impl Decoder for Bytes {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        if buf.len() >= HDRKEYL {
            // HDRKEYL is header min size
            if read_hdr_type(&buf.to_vec()) != 'M' as u32 {
                let len = buf.len();
                buf.split_to(len);
                return Ok(None);
            }
            let hdr_len = read_hdr_len(&buf.to_vec());
            if 0 == hdr_len {
                let len = buf.len();
                buf.split_to(len);
                return Ok(None);
            }
            let len = buf.len();
            if len < (HDRKEYL + hdr_len) {
                return Ok(None);
            }
            if HDRKEYL + hdr_len < len {
                buf.split_to(HDRKEYL);
                return Ok(Some(buf.split_to(hdr_len)));
            }
            buf.split_to(HDRKEYL);
            Ok(Some(buf.split_to(hdr_len)))
        } else {
            Ok(None)
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        self.decode(buf)
    }
}

impl Encoder for Bytes {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn encode(&mut self, data: Vec<u8>, buf: &mut BytesMut) -> io::Result<()> {
        let mut msgv = write_hdr(data.len() - KEYL);
        msgv.extend(data);
        buf.put(&msgv[..]);
        Ok(())
    }
}

fn read_stdin(mut rx: mpsc::Sender<Vec<u8>>) {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    /* Set user */
    let _val = stdout.write_all(b"User name?\n");
    let mut buf = vec![0; 80];
    let n = match stdin.read(&mut buf) {
        Err(_) | Ok(0) => return,
        Ok(n) => n,
    };
    buf.truncate(n - 1);
    let mut userstr = String::from_utf8_lossy(buf.clone().as_slice()).into_owned();
    if userstr.ends_with("\r") {
        let len = userstr.len();
        userstr.truncate(len - 1);
    }

    /* Set channel */
    let _val = stdout.write_all(b"Channel?\n");
    let mut buf = vec![0; 80];
    let n = match stdin.read(&mut buf) {
        Err(_) | Ok(0) => return,
        Ok(n) => n,             
    };
    buf.truncate(n - 1);
    let mut channelstr = String::from_utf8_lossy(buf.clone().as_slice()).into_owned();
    if channelstr.ends_with("\r") {
        let len = channelstr.len();
        channelstr.truncate(len - 1);
    }

    /* Join channel */
    let msg = message_encode(&Msg::new(userstr.clone(), channelstr.clone(), Vec::new()));
    rx = rx.send(msg).wait().unwrap();

    let mut msg = userstr.clone();
    msg += " to ";
    msg += channelstr.as_str();

    /* Say welcome */
    let mut welcome = "Welcome ".to_string();
    welcome += msg.as_str();
    welcome += "!\n";
    let _val = stdout.write_all(welcome.as_bytes());

    loop {
        let mut buf = vec![0;80];
        let n = match stdin.read(&mut buf) {
            Err(_) | Ok(0) => break,
            Ok(n) => n,
        };
        buf.truncate(n);
        let str = String::from_utf8_lossy(buf.as_slice()).into_owned();
        let msg = Msg::new(userstr.clone(), channelstr.clone(), str.into_bytes());
        let msg = message_encode(&msg);
        rx = rx.send(msg).wait().unwrap();
    }
}

fn message_forward_all(mles_tx: std::sync::mpsc::Sender<Vec<u8>>, ws_tx: futures::sync::mpsc::Sender<Vec<u8>>, msg: Vec<u8>) -> Result<futures::sync::mpsc::Sender<std::vec::Vec<u8>>, Error> {
    match mles_tx.send(msg.clone()) {
        Ok(_) => {
            match ws_tx.send(msg).wait() {
                Ok(val) => { 
                    return Ok(val);
                }
                Err(_) => { 
                    return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                }
            }
        }
        Err(_) => {
            // send empty messages to drop remaining threads
            let empty = Vec::new();
            match ws_tx.send(empty).wait() {
                Ok(val) => {
                  return Ok(val);
                }
                Err(_) => { 
                    return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                }
            }
        }
    };
}


fn message_forward_mles(mles_tx: std::sync::mpsc::Sender<Vec<u8>>, ws_tx: futures::sync::mpsc::Sender<Vec<u8>>, msg: Vec<u8>) -> Result<futures::sync::mpsc::Sender<std::vec::Vec<u8>>, Error> {
    match mles_tx.send(msg.clone()) {
        Ok(_) => {
            return Ok(ws_tx);
        }
        Err(_) => {
            // send empty messages to drop remaining threads
            let empty = Vec::new();
            match ws_tx.send(empty).wait() {
                Ok(val) => {
                  return Ok(val);
                }
                Err(_) => { 
                    return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
                }
            }
        }
    };
}

