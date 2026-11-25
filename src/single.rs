//! One Slipcase at a time: the first instance listens on a fixed local port, and a second one
//! started while it runs asks it to come to the front and quits.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

use bevy::prelude::*;

const PORT: u16 = 47_917;
const ASK: &str = "slipcase raise";
const ACK: &str = "slipcase ok";

/// Set by the listener thread when another instance asked for the window; `app::home_button` raises it.
static RAISE: AtomicBool = AtomicBool::new(false);

pub fn take_raise() -> bool {
    RAISE.swap(false, Relaxed)
}

/// The listener, until `listen` hands it to its thread.
#[derive(Resource)]
pub struct Instance(Mutex<Option<TcpListener>>);

/// `Some` if this is the only instance. `None` if another Slipcase answered and was told to show
/// itself (so this one should quit). If the port is taken by something that is not Slipcase,
/// starts anyway, just without the check.
pub fn acquire() -> Option<Instance> {
    match TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)) {
        Ok(l) => Some(Instance(Mutex::new(Some(l)))),
        Err(_) if ask_other() => None,
        Err(_) => Some(Instance(Mutex::new(None))),
    }
}

fn ask_other() -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, PORT).into(), Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    if writeln!(s, "{ASK}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == ACK
}

/// Startup: answers later instances on a thread of its own, waking the app when one asks.
pub fn listen(inst: Option<Res<Instance>>, proxy: Option<NonSend<bevy::winit::EventLoopProxy<bevy::winit::WakeUp>>>) {
    let (Some(inst), Some(proxy)) = (inst, proxy) else { return };
    let Some(listener) = inst.0.lock().ok().and_then(|mut l| l.take()) else { return };
    let proxy = proxy.clone();
    let _ = std::thread::Builder::new().name("single-instance".into()).spawn(move || {
        for mut s in listener.incoming().flatten() {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(&s).read_line(&mut line).is_ok() && line.trim() == ASK {
                let _ = writeln!(s, "{ACK}");
                RAISE.store(true, Relaxed);
                let _ = proxy.send_event(bevy::winit::WakeUp);
            }
        }
    });
}
