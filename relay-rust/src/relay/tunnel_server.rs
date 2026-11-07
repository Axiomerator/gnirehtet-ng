/*
 * Copyright (C) 2017 Genymobile
 * Modified for Gnirehtet NG by the Gnirehtet NG contributors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use log::*;
use mio::net::TcpListener;
use mio::{Event, PollOpt, Ready};
use std::cell::RefCell;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::ptr;
use std::rc::{Rc, Weak};

use super::client::Client;
use super::selector::Selector;

const TAG: &str = "TunnelServer";

fn local_hostname() -> String {
    #[cfg(target_os = "windows")]
    {
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            if !name.trim().is_empty() {
                return name;
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(name) = std::env::var("HOSTNAME") {
            if !name.trim().is_empty() {
                return name;
            }
        }
    }
    "gnirehtet-relay".to_string()
}

pub struct TunnelServer {
    self_weak: Weak<RefCell<TunnelServer>>,
    clients: Vec<Rc<RefCell<Client>>>,
    tcp_listener: TcpListener,
    next_client_id: u32,
    greeting: Vec<u8>,
}

/// Fixed-size greeting sent to every client before any packet: 4-byte client id (big endian),
/// 2-byte relay port (big endian), then a NUL-padded UTF-8 host name.
pub const GREETING_HOSTNAME_LENGTH: usize = 256;

pub fn build_greeting(port: u16, hostname: &str) -> Vec<u8> {
    let mut greeting = Vec::with_capacity(4 + 2 + GREETING_HOSTNAME_LENGTH);
    greeting.extend_from_slice(&0u32.to_be_bytes());
    greeting.extend_from_slice(&port.to_be_bytes());
    let mut name = [0u8; GREETING_HOSTNAME_LENGTH];
    let hostname_bytes = hostname.as_bytes();
    let length = hostname_bytes.len().min(GREETING_HOSTNAME_LENGTH - 1);
    name[..length].copy_from_slice(&hostname_bytes[..length]);
    greeting.extend_from_slice(&name);
    greeting
}

impl TunnelServer {
    pub fn create(port: u16, selector: &mut Selector) -> io::Result<Rc<RefCell<Self>>> {
        let tcp_listener = Self::start_socket(port)?;
        let rc = Rc::new(RefCell::new(Self {
            self_weak: Weak::new(),
            clients: Vec::new(),
            tcp_listener,
            next_client_id: 0,
            greeting: build_greeting(port, &local_hostname()),
        }));

        // keep a shared reference to this
        rc.borrow_mut().self_weak = Rc::downgrade(&rc);

        let rc2 = rc.clone();
        // must anotate selector type: https://stackoverflow.com/a/44004103/1987178
        let handler =
            move |selector: &mut Selector, event| rc2.borrow_mut().on_ready(selector, event);
        selector.register(
            &rc.borrow().tcp_listener,
            handler,
            Ready::readable(),
            PollOpt::edge(),
        )?;
        Ok(rc)
    }

    fn start_socket(port: u16) -> io::Result<TcpListener> {
        let localhost = Ipv4Addr::new(127, 0, 0, 1).into();
        let addr = SocketAddr::new(localhost, port);
        let server = TcpListener::bind(&addr)?;
        Ok(server)
    }

    fn on_ready(&mut self, selector: &mut Selector, _: Event) {
        match self.accept_client(selector) {
            Ok(_) => debug!(target: TAG, "New client accepted"),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                debug!(target: TAG, "Spurious event, ignoring");
            }
            Err(err) => error!(target: TAG, "Cannot accept client: {}", err),
        }
    }

    fn accept_client(&mut self, selector: &mut Selector) -> io::Result<()> {
        let (stream, _) = self.tcp_listener.accept()?;
        let client_id = self.next_client_id;
        self.next_client_id += 1;
        // write the actual client id into this client's greeting copy
        let mut greeting = self.greeting.clone();
        greeting[..4].copy_from_slice(&client_id.to_be_bytes());
        let weak = self.self_weak.clone();
        let on_client_closed = Box::new(move |client: &Client| {
            if let Some(rc) = weak.upgrade() {
                let mut tunnel_server = rc.borrow_mut();
                tunnel_server.remove_client(client);
            } else {
                warn!(
                    target: TAG,
                    "on_client_closed called but no tunnel_server available"
                );
            }
        });
        let client = Client::create(client_id, selector, stream, &greeting, on_client_closed)?;
        self.clients.push(client);
        info!(target: TAG, "Client #{} connected", client_id);
        Ok(())
    }

    fn remove_client(&mut self, client: &Client) {
        info!(target: TAG, "Client #{} disconnected", client.id());
        let index = self
            .clients
            .iter()
            .position(|item| {
                // compare pointers to find the client to remove
                ptr::eq(client, item.as_ptr())
            })
            .expect("Trying to remove an unknown client");
        self.clients.swap_remove(index);
    }

    pub fn clean_up(&mut self, selector: &mut Selector) {
        for client in &self.clients {
            client.borrow_mut().clean_expired_connections(selector);
        }
    }
}
