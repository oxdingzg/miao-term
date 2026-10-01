//! The Wayland data device: file drops and clipboard reads.
//!
//! winit 0.30 implements drag and drop only for X11, so on a Wayland session
//! a file dragged from a file manager did nothing. This binds `wl_data_device`
//! on winit's own connection, in a queue of its own: winit reads the socket,
//! and `poll` (called every loop turn) dispatches what arrived for us. A drop
//! of `text/uri-list` is read through a pipe on a helper thread and handed
//! back as paths.
//!
//! GNOME's compositor delivers data-device events to one device per client
//! (the newest), so once this one exists egui's clipboard (smithay-clipboard)
//! no longer hears about the selection. Pastes therefore read the selection
//! here too ([`Dnd::selection_text`]); copying still goes through egui, since
//! setting the selection works from any device.

use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

use wayland_client::backend::Backend;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const URI_LIST: &str = "text/uri-list";
/// Text types, best first.
const TEXT_MIMES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
];

/// What the window should do, in surface-local logical coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum DropEvent {
    Hover { x: f64, y: f64 },
    Leave,
    Drop { paths: Vec<PathBuf>, x: f64, y: f64 },
}

#[derive(Default)]
struct Inner {
    /// Offers announced by `data_offer`, with the MIME types they list.
    offers: Vec<(WlDataOffer, Vec<String>)>,
    /// The offer under the pointer, if it carries files.
    current: Option<WlDataOffer>,
    /// The clipboard offer and its MIME types.
    selection: Option<(WlDataOffer, Vec<String>)>,
    pos: (f64, f64),
    events: Vec<DropEvent>,
    /// Drops whose data is still being read.
    reading: Vec<(WlDataOffer, Receiver<String>, (f64, f64))>,
}

pub struct Dnd {
    conn: Connection,
    queue: EventQueue<Inner>,
    inner: Inner,
    _device: WlDataDevice,
}

impl Dnd {
    /// Attach to winit's `wl_display`. `None` when the compositor offers no
    /// data device manager or seat.
    ///
    /// # Safety
    /// `display` must be the live `wl_display*` of this process's Wayland
    /// connection, valid for as long as the returned value.
    pub unsafe fn new(display: *mut std::ffi::c_void) -> Option<Self> {
        let backend = Backend::from_foreign_display(display as *mut _);
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Inner>(&conn).ok()?;
        let qh = queue.handle();
        let manager: WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).ok()?;
        let seat: WlSeat = globals.bind(&qh, 1..=1, ()).ok()?;
        let device = manager.get_data_device(&seat, &qh, ());
        conn.flush().ok()?;
        Some(Self {
            conn,
            queue,
            inner: Inner::default(),
            _device: device,
        })
    }

    /// Handle what arrived since the last call; never blocks.
    pub fn poll(&mut self) -> Vec<DropEvent> {
        let _ = self.queue.dispatch_pending(&mut self.inner);
        let inner = &mut self.inner;
        let mut done = Vec::new();
        inner
            .reading
            .retain(|(offer, rx, pos)| match rx.try_recv() {
                Ok(text) => {
                    done.push((offer.clone(), text, *pos));
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done.push((offer.clone(), String::new(), *pos));
                    false
                }
            });
        for (offer, text, (x, y)) in done {
            if offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
            let paths = parse_uri_list(&text);
            if !paths.is_empty() {
                inner.events.push(DropEvent::Drop { paths, x, y });
            }
        }
        let _ = self.conn.flush();
        std::mem::take(&mut inner.events)
    }
}

impl Dnd {
    /// The clipboard as text, or `None` when it holds no text (or the owner
    /// does not answer within a second). Blocks briefly: call it for a paste.
    pub fn selection_text(&mut self) -> Option<String> {
        let _ = self.queue.dispatch_pending(&mut self.inner);
        let (offer, mimes) = self.inner.selection.as_ref()?;
        let mime = TEXT_MIMES
            .iter()
            .find(|m| mimes.iter().any(|have| have == *m))?;
        let (mut read, write) = std::os::unix::net::UnixStream::pair().ok()?;
        offer.receive(mime.to_string(), write.as_fd());
        drop(write);
        self.conn.flush().ok()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = read.read_to_end(&mut buf);
            let _ = tx.send(buf);
        });
        let buf = rx.recv_timeout(std::time::Duration::from_secs(1)).ok()?;
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
}

/// Local paths from a `text/uri-list` (RFC 2483): `file://` URIs only,
/// comments skipped, percent-escapes decoded.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        .filter_map(|rest| {
            // `file:///path` or `file://host/path`.
            let path = &rest[rest.find('/')?..];
            Some(PathBuf::from(percent_decode(path)))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Inner {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for Inner {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Inner {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataOffer, ()> for Inner {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if let Some((_, mimes)) = state.offers.iter_mut().find(|(o, _)| o == offer) {
                mimes.push(mime_type);
            }
        }
    }
}

impl Dispatch<WlDataDevice, ()> for Inner {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => state.offers.push((id, Vec::new())),
            wl_data_device::Event::Enter {
                serial, x, y, id, ..
            } => {
                state.current = None;
                let Some(offer) = id else {
                    return;
                };
                let files = state
                    .offers
                    .iter()
                    .any(|(o, mimes)| o == &offer && mimes.iter().any(|m| m == URI_LIST));
                state.offers.retain(|(o, _)| o != &offer);
                if files {
                    offer.accept(serial, Some(URI_LIST.into()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    state.pos = (x, y);
                    state.events.push(DropEvent::Hover { x, y });
                    state.current = Some(offer);
                } else {
                    offer.accept(serial, None);
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if state.current.is_some() {
                    state.pos = (x, y);
                    state.events.push(DropEvent::Hover { x, y });
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.current.take() {
                    offer.destroy();
                    state.events.push(DropEvent::Leave);
                }
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = state.current.take() else {
                    return;
                };
                let Ok((mut read, write)) = std::os::unix::net::UnixStream::pair() else {
                    offer.destroy();
                    return;
                };
                offer.receive(URI_LIST.into(), write.as_fd());
                // Our copy of the write end must close, or the read never ends.
                drop(write);
                let (tx, rx) = channel();
                std::thread::spawn(move || {
                    let mut text = String::new();
                    let _ = read.read_to_string(&mut text);
                    let _ = tx.send(text);
                });
                state.reading.push((offer, rx, state.pos));
                state.events.push(DropEvent::Leave);
            }
            wl_data_device::Event::Selection { id } => {
                let new = id.map(|offer| {
                    let mimes = state
                        .offers
                        .iter()
                        .find(|(o, _)| o == &offer)
                        .map(|(_, m)| m.clone())
                        .unwrap_or_default();
                    state.offers.retain(|(o, _)| o != &offer);
                    (offer, mimes)
                });
                if let Some((old, _)) = std::mem::replace(&mut state.selection, new) {
                    old.destroy();
                }
            }
            _ => {}
        }
    }

    event_created_child!(Inner, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_become_local_paths() {
        let text = "# from nautilus\r\nfile:///tmp/a%20file%20%E4%B8%AD.txt\r\nfile://host/srv/b\r\nhttps://example.com/x\r\n\r\n";
        assert_eq!(
            parse_uri_list(text),
            vec![PathBuf::from("/tmp/a file 中.txt"), PathBuf::from("/srv/b")]
        );
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
