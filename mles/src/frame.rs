/**
 *   Mles server frame handling
 *
 *   Copyright (C) 2017  Juhamatti Kuusisaari / Mles developers
 *
 *   This program is free software: you can redistribute it and/or modify
 *   it under the terms of the GNU General Public License as published by
 *   the Free Software Foundation, either version 3 of the License, or
 *   (at your option) any later version.
 *
 *   This program is distributed in the hope that it will be useful,
 *   but WITHOUT ANY WARRANTY; without even the implied warranty of
 *   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *   GNU General Public License for more details.
 *
 *   You should have received a copy of the GNU General Public License
 *   along with this program.  If not, see <http://www.gnu.org/licenses/>.
 */
extern crate tokio_core;
extern crate tokio_io;
extern crate futures;
extern crate mles_utils;

use std::io::{Error, ErrorKind};
use std::net::{SocketAddr};

use tokio_core::net::TcpStream;
use tokio_io::io;

use mles_utils::*;

const HDRL: usize = 4; //hdr len

pub fn process_hdr_dummy_key(reader: io::ReadHalf<TcpStream>, hdr_key: Vec<u8>) -> Result<(io::ReadHalf<TcpStream>, Vec<u8>, usize), Error> {
    process_hdr(reader, hdr_key)
}

pub fn process_hdr(reader: io::ReadHalf<TcpStream>, hdr: Vec<u8>) -> Result<(io::ReadHalf<TcpStream>, Vec<u8>, usize), Error> {
    if hdr.len() == 0 {
        return Err(Error::new(ErrorKind::BrokenPipe, "broken pipe"));
    }
    if read_hdr_type(hdr.as_slice()) != 'M' as u32 {
        return Err(Error::new(ErrorKind::BrokenPipe, "incorrect header"));
    }
    let hdr_len = read_hdr_len(hdr.as_slice());
    if 0 == hdr_len {
        return Err(Error::new(ErrorKind::BrokenPipe, "incorrect header len"));
    }
    Ok((reader, hdr, hdr_len))
}

pub fn process_msg(reader: io::ReadHalf<TcpStream>, hdr_key: Vec<u8>, message: Vec<u8>) -> Result<(io::ReadHalf<TcpStream>, Vec<u8>, Vec<u8>), Error> { 
    if 0 == message.len() { 
        return Err(Error::new(ErrorKind::BrokenPipe, "incorrect message len"));
    }
    Ok((reader, hdr_key, message))
}

pub fn process_key(reader: io::ReadHalf<TcpStream>, mut hdr_key: Vec<u8>, hdr_len: usize, keyval: String, peer_addr: SocketAddr) -> Result<(io::ReadHalf<TcpStream>, Vec<u8>, usize), Error> { 
    let hkey;
    let key = hdr_key.split_off(HDRL);
    let keyx = read_key(&key);
    if 0 == keyval.len() {
        hkey = do_hash(&vec![&peer_addr]);
    }
    else {
        hkey = do_hash(&vec![&keyval]);
    }
    if hkey != keyx {
        return Err(Error::new(ErrorKind::BrokenPipe, "incorrect remote key"));
    }
    hdr_key.extend(key);
    Ok((reader, hdr_key, hdr_len))
}


