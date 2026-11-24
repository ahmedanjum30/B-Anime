//! Watch together: a small room of B-Anime copies that play the same episode in step.
//!
//! Only playback controls travel between devices (play, pause, seek, which episode); each device
//! plays the episode from its own source or library. Devices connect to each other directly with
//! iroh: hole punching where it works, falling back to iroh's public relays (which only forward
//! encrypted traffic) where it doesn't. B-Anime runs no server.
//!
//! The host's device is the room: guests dial it by the room code, which carries the host's
//! endpoint id and home relay, so joining doesn't depend on iroh's address lookup service. The
//! host keeps the room's state, so a change from anyone goes to the host and the host passes it on.
//!
//! Losing the host isn't the end of the room. A guest whose connection drops keeps trying for a
//! minute. If the host itself is gone (it left, or nobody can reach it for a while), the guest who
//! joined first takes over: every device can accept connections, and the roster carries each
//! member's own code, so the others know where to go.
//!
//! Messages are JSON, each sent as a 4-byte big-endian length followed by the bytes.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, TransportAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{Error, Result};

/// The protocol name; a change that old copies can't read gets a new number.
const ALPN: &[u8] = b"banime/watch/2";
/// No message comes close to this; anything bigger is a misbehaving peer.
const MAX_MESSAGE: usize = 64 * 1024;
/// Guests per room besides the host.
const MAX_GUESTS: usize = 8;
/// How long joining may take before the guest is told it didn't work.
const JOIN_TIMEOUT: Duration = Duration::from_secs(20);
/// After losing the host: how long each candidate host (the old host, then each guest in the
/// order they joined) gets before the next one is tried.
const HOST_GRACE: Duration = Duration::from_secs(10);
/// How long a device keeps trying to get back into the room before giving up.
const REJOIN_FOR: Duration = Duration::from_secs(60);
/// One connection attempt while rejoining.
const ATTEMPT: Duration = Duration::from_secs(5);

/// The room code: the host's id and addresses, as URL-safe base64 of a small JSON object.
#[derive(Serialize, Deserialize)]
struct Ticket {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ips: Vec<String>,
}

/// The host's home relay, and its direct addresses only when there's no relay. Direct addresses
/// in the code broke joining: a phone on the host PC's hotspot tried them all at once, some
/// looped back through the PC, and the handshake stalled. Through the relay it connects at once,
/// and iroh then finds a direct path by itself.
fn room_code(addr: &EndpointAddr) -> String {
    let mut t = Ticket { id: addr.id.to_string(), relay: None, ips: Vec::new() };
    for a in &addr.addrs {
        match a {
            TransportAddr::Relay(url) => t.relay = Some(url.to_string()),
            TransportAddr::Ip(ip) => t.ips.push(ip.to_string()),
            _ => {}
        }
    }
    if t.relay.is_some() {
        t.ips.clear();
    }
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&t).unwrap_or_default())
}

/// Reads a room code; a bare endpoint id works too (then the address lookup service finds it).
fn parse_code(code: &str) -> Result<EndpointAddr> {
    // Chat apps wrap long text: spaces and line breaks inside the code are dropped.
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let code = code.as_str();
    let bad = || Error::Config("That isn't a room code. Copy the whole code from the host.".into());
    if let Ok(id) = EndpointId::from_str(code) {
        return Ok(EndpointAddr::from(id));
    }
    let bytes = URL_SAFE_NO_PAD.decode(code).map_err(|_| bad())?;
    let t: Ticket = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let mut addr = EndpointAddr::from(EndpointId::from_str(&t.id).map_err(|_| bad())?);
    if let Some(relay) = t.relay.and_then(|r| r.parse::<RelayUrl>().ok()) {
        addr.addrs.insert(TransportAddr::Relay(relay));
    }
    for ip in t.ips.iter().filter_map(|ip| ip.parse().ok()) {
        addr.addrs.insert(TransportAddr::Ip(ip));
    }
    Ok(addr)
}

/// The extension someone plays the episode with, so others can install the same one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub id: String,
    pub name: String,
    /// Its manifest URL (what installing it takes).
    pub url: String,
}

/// What the room is playing, as the host's player last reported it (or a guest asked for).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayState {
    pub media_id: i64,
    pub episode: f64,
    /// The show's title, for "Sam is watching …" before the guest's player has it.
    #[serde(default)]
    pub title: String,
    /// English dub rather than the original audio.
    #[serde(default)]
    pub dub: bool,
    /// Seconds into the episode.
    pub position: f64,
    pub playing: bool,
    /// Who made the change ("" for the host's own regular position updates).
    #[serde(default)]
    pub by: String,
    /// The host's source for the show, offered to guests whose own sources can't play it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    /// The room's subtitle language (a code like "en", or "off"), set by the host; everyone's
    /// player picks it instead of their own default. None in a change: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subs: Option<String>,
}

/// Whether someone can play the room's episode, and with what.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// None while still checking (or with nothing picked yet).
    pub playable: Option<bool>,
    /// The extension's name (or "Library").
    pub source: Option<String>,
}

/// Someone in the room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub name: String,
    pub host: bool,
    /// Seconds into the episode, as they last reported.
    pub position: f64,
    pub buffering: bool,
    /// Said they're ready to start (the lobby, before the host starts).
    pub ready: bool,
    #[serde(flatten)]
    pub check: Check,
    /// Their link to the host: direct (true) or through a relay (false). None for the host.
    pub direct: Option<bool>,
    /// Round trip to the host, in milliseconds. None for the host.
    pub rtt_ms: Option<u32>,
    /// Their own room code: where the others go if they take over hosting.
    #[serde(default)]
    pub code: String,
}

/// Who's in the room and whether the host has started it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub members: Vec<Member>,
    /// False in the lobby: everyone gathers, says they're ready, and the host starts.
    pub started: bool,
}

/// This device's place in the room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    /// What others enter to join (the host's code).
    pub code: String,
    /// This device's id in the member list.
    pub me: String,
    pub host: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Msg {
    /// A guest's first message; after a reconnect it carries what the guest had said before.
    Hello { name: String, code: String, ready: bool, check: Check },
    /// Host → guests: the room's state. Guest → host: a change the guest made.
    State(PlayState),
    /// Guest → host: where the guest is.
    Status { position: f64, buffering: bool },
    /// Guest → host: ready to start, or not any more.
    Ready { ready: bool },
    /// Guest → host: whether the guest can play the room's episode.
    Check(Check),
    /// Host → guests: everyone in the room, and whether it has started.
    Roster(Roster),
    /// Host → guest: why it was turned away.
    Refused { reason: String },
    /// Host → guests: the host ended the room for everyone.
    Ended,
    /// Host → guests: the host left; the guest who joined first hosts from here.
    HandOver,
}

/// What the room tells this device.
#[derive(Debug, Clone)]
pub enum RoomEvent {
    /// Play this (someone else changed it, or the host's regular position update).
    State(PlayState),
    Roster(Roster),
    /// This device's place changed: it took over hosting, or the room moved to a new host.
    Info(RoomInfo),
    /// The link to the room dropped; this device is trying to get back in.
    Reconnecting,
    /// Back in the room.
    Reconnected,
    /// The room ended for this device (the host ended it, it was refused, or rejoining failed).
    Closed(String),
}

fn net(e: impl std::fmt::Display) -> Error {
    Error::Network(e.to_string())
}

async fn write_msg(send: &mut SendStream, msg: &Msg) -> Result<()> {
    let body = serde_json::to_vec(msg).map_err(net)?;
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    send.write_all(&frame).await.map_err(net)
}

async fn read_msg(recv: &mut RecvStream) -> Result<Msg> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.map_err(net)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(Error::Network(format!("message too large ({len} bytes)")));
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(net)?;
    serde_json::from_slice(&body).map_err(net)
}

/// The link to the host, measured from the host's side: direct or relayed, and the round trip.
fn quality(conn: &Connection) -> (Option<bool>, Option<u32>) {
    let paths = conn.paths();
    match paths.iter().find(|p| p.is_selected()) {
        Some(p) => (Some(p.is_ip()), Some(p.rtt().as_millis().min(u32::MAX as u128) as u32)),
        None => (None, None),
    }
}

/// Every device accepts connections (so any guest can take over hosting); only a host serves them.
async fn bind() -> Result<Endpoint> {
    Endpoint::builder(presets::N0).alpns(vec![ALPN.to_vec()]).bind().await.map_err(net)
}

async fn connect(endpoint: &Endpoint, addr: EndpointAddr, within: Duration) -> Result<Connection> {
    match tokio::time::timeout(within, endpoint.connect(addr, ALPN)).await {
        Ok(Ok(c)) => Ok(c),
        Ok(Err(e)) => Err(net(e)),
        Err(_) => Err(Error::Network("timed out".into())),
    }
}

/// A device's place in a room, host or guest (and a guest can become the host).
pub struct Room {
    shared: Arc<Shared>,
}

struct Shared {
    endpoint: Endpoint,
    me: EndpointId,
    name: String,
    events: mpsc::UnboundedSender<RoomEvent>,
    role: Mutex<Role>,
    /// What this device has said and seen, kept across reconnects and a change of host.
    local: Mutex<Local>,
    closing: AtomicBool,
}

#[derive(Default)]
struct Local {
    ready: bool,
    check: Check,
    position: f64,
    buffering: bool,
    /// The room's last state and roster (a guest that takes over hosting starts from them).
    state: Option<PlayState>,
    roster: Option<Roster>,
}

enum Role {
    Host(Arc<Mutex<HostState>>),
    /// `to_host` feeds whichever connection to the host is current; `code` is the host's.
    Guest { to_host: mpsc::UnboundedSender<Msg>, code: String },
}

struct HostState {
    me: String,
    name: String,
    code: String,
    state: Option<PlayState>,
    position: f64,
    buffering: bool,
    ready: bool,
    check: Check,
    /// The host has started the room (the lobby is over).
    started: bool,
    /// In the order they joined (the order in which they'd take over hosting).
    guests: Vec<Guest>,
    events: mpsc::UnboundedSender<RoomEvent>,
}

struct Guest {
    id: EndpointId,
    name: String,
    code: String,
    position: f64,
    buffering: bool,
    ready: bool,
    check: Check,
    tx: mpsc::UnboundedSender<Msg>,
    conn: Connection,
}

impl HostState {
    fn members(&self) -> Vec<Member> {
        let mut list = vec![Member {
            id: self.me.clone(),
            name: self.name.clone(),
            host: true,
            position: self.position,
            buffering: self.buffering,
            ready: self.ready,
            check: self.check.clone(),
            direct: None,
            rtt_ms: None,
            code: self.code.clone(),
        }];
        list.extend(self.guests.iter().map(|g| {
            let (direct, rtt_ms) = quality(&g.conn);
            Member {
                id: g.id.to_string(),
                name: g.name.clone(),
                host: false,
                position: g.position,
                buffering: g.buffering,
                ready: g.ready,
                check: g.check.clone(),
                direct,
                rtt_ms,
                code: g.code.clone(),
            }
        }));
        list
    }

    /// Everyone's list changed: tell the guests and this device.
    fn announce(&self) {
        let roster = Roster { members: self.members(), started: self.started };
        for g in &self.guests {
            let _ = g.tx.send(Msg::Roster(roster.clone()));
        }
        let _ = self.events.send(RoomEvent::Roster(roster));
    }

    fn guest_mut(&mut self, id: &EndpointId) -> Option<&mut Guest> {
        self.guests.iter_mut().find(|g| &g.id == id)
    }

    /// A new state for the room, from a guest (`from_guest`) or the host. In the lobby only the
    /// host starts playback, and picking another episode there asks everyone to get ready again
    /// (and to check it again). Returns the state as the room keeps it.
    fn adopt(&mut self, mut state: PlayState, from_guest: bool) -> PlayState {
        // The host's source and the room's subtitles stay when a change doesn't name them.
        if state.source.is_none() {
            state.source = self.state.as_ref().and_then(|s| s.source.clone());
        }
        if state.subs.is_none() {
            state.subs = self.state.as_ref().and_then(|s| s.subs.clone());
        }
        // Sub or Dub is part of the pick: a source may have one and not the other.
        let other_episode = self
            .state
            .as_ref()
            .is_none_or(|s| s.media_id != state.media_id || s.episode != state.episode || s.dub != state.dub);
        if !self.started {
            if state.playing {
                if from_guest {
                    state.playing = false;
                } else {
                    self.started = true;
                }
            }
            if other_episode && !self.started {
                self.ready = false;
                for g in &mut self.guests {
                    g.ready = false;
                }
            }
        }
        if other_episode {
            self.check = Check::default();
            for g in &mut self.guests {
                g.check = Check::default();
            }
        }
        self.state = Some(state.clone());
        state
    }

    fn broadcast_state(&self, state: &PlayState, except: Option<&EndpointId>) {
        for g in &self.guests {
            if Some(&g.id) != except {
                let _ = g.tx.send(Msg::State(state.clone()));
            }
        }
    }
}

impl Shared {
    fn new(endpoint: Endpoint, name: String, events: mpsc::UnboundedSender<RoomEvent>, role: Role) -> Arc<Shared> {
        Arc::new(Shared {
            me: endpoint.id(),
            endpoint,
            name,
            events,
            role: Mutex::new(role),
            local: Mutex::new(Local::default()),
            closing: AtomicBool::new(false),
        })
    }

    fn emit(&self, e: RoomEvent) {
        let _ = self.events.send(e);
    }

    fn my_code(&self) -> String {
        room_code(&self.endpoint.addr())
    }

    fn closing(&self) -> bool {
        self.closing.load(Ordering::SeqCst)
    }

    fn info(&self) -> RoomInfo {
        let (code, host) = match &*self.role.lock().unwrap_or_else(|p| p.into_inner()) {
            Role::Host(h) => (h.lock().map(|h| h.code.clone()).unwrap_or_default(), true),
            Role::Guest { code, .. } => (code.clone(), false),
        };
        RoomInfo { code, me: self.me.to_string(), host }
    }

    fn host_state(&self) -> Option<Arc<Mutex<HostState>>> {
        match &*self.role.lock().ok()? {
            Role::Host(h) => Some(h.clone()),
            Role::Guest { .. } => None,
        }
    }

    /// Hosts the room from what this device knows of it (a new room, or taking over).
    fn become_host(&self) {
        let host = {
            let l = self.local.lock().unwrap_or_else(|p| p.into_inner());
            HostState {
                me: self.me.to_string(),
                name: self.name.clone(),
                code: self.my_code(),
                state: l.state.clone(),
                position: l.position,
                buffering: l.buffering,
                ready: l.ready,
                check: l.check.clone(),
                started: l.roster.as_ref().is_some_and(|r| r.started),
                guests: Vec::new(),
                events: self.events.clone(),
            }
        };
        let host = Arc::new(Mutex::new(host));
        if let Ok(mut role) = self.role.lock() {
            *role = Role::Host(host.clone());
        }
        tracing::info!("watch: hosting the room");
        self.emit(RoomEvent::Info(self.info()));
        if let Ok(h) = host.lock() {
            h.announce();
        }
    }

    fn end(&self, reason: String) {
        if !self.closing.swap(true, Ordering::SeqCst) {
            self.emit(RoomEvent::Closed(reason));
        }
    }
}

impl Room {
    /// Opens a room on this device. Guests join with [`Room::info`]'s code.
    pub async fn host(name: String) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let endpoint = bind().await?;
        // Reachable (home relay picked) before the code is handed out.
        endpoint.online().await;
        let (events, rx) = mpsc::unbounded_channel();
        let (to_host, _) = mpsc::unbounded_channel();
        let shared = Shared::new(endpoint, name, events, Role::Guest { to_host, code: String::new() });
        shared.become_host();
        tracing::info!(addrs = ?shared.endpoint.addr().addrs, "watch: room open");
        tokio::spawn(accept_loop(shared.clone()));
        Ok((Room { shared }, rx))
    }

    /// Joins the room with this code.
    pub async fn join(code: &str, name: String) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let host_addr = parse_code(code)?;
        tracing::info!(host = %host_addr.id.fmt_short(), addrs = ?host_addr.addrs, "watch: joining room");
        let endpoint = bind().await?;
        let conn = match connect(&endpoint, host_addr.clone(), JOIN_TIMEOUT).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "watch: couldn't connect to the room");
                endpoint.close().await;
                return Err(Error::Network(
                    "Couldn't reach the room. Check the code, and that the host still has the room open.".into(),
                ));
            }
        };
        tracing::info!("watch: connected to the room");
        // Its own code goes in the hello (for taking over hosting): wait briefly for the relay.
        let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
        let (events, rx) = mpsc::unbounded_channel();
        let (to_host, outgoing) = mpsc::unbounded_channel();
        let shared = Shared::new(endpoint, name, events, Role::Guest { to_host, code: room_code(&host_addr) });
        tokio::spawn(accept_loop(shared.clone()));
        tokio::spawn(guest_loop(shared.clone(), host_addr, conn, outgoing));
        Ok((Room { shared }, rx))
    }

    pub fn info(&self) -> RoomInfo {
        self.shared.info()
    }

    pub fn is_host(&self) -> bool {
        self.shared.host_state().is_some()
    }

    fn to_host(&self, msg: Msg) {
        if let Ok(role) = self.shared.role.lock() {
            if let Role::Guest { to_host, .. } = &*role {
                let _ = to_host.send(msg);
            }
        }
    }

    /// This device's player changed (or, from the host, its regular position update).
    pub fn send_state(&self, mut state: PlayState) {
        state.by = self.shared.me.to_string();
        if let Ok(mut l) = self.shared.local.lock() {
            l.position = state.position;
            l.state = Some(state.clone());
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    let lobby = !h.started;
                    h.position = state.position;
                    let state = h.adopt(state, false);
                    h.broadcast_state(&state, None);
                    // Readiness, checks and "started" only change in the lobby or with the episode.
                    if lobby || h.check == Check::default() {
                        h.announce();
                    }
                }
            }
            None => self.to_host(Msg::State(state)),
        }
    }

    /// Ready to start (or not), for the lobby.
    pub fn set_ready(&self, ready: bool) {
        if let Ok(mut l) = self.shared.local.lock() {
            l.ready = ready;
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    h.ready = ready;
                    h.announce();
                }
            }
            None => self.to_host(Msg::Ready { ready }),
        }
    }

    /// Whether this device can play the room's episode, and with what.
    pub fn set_check(&self, check: Check) {
        if let Ok(mut l) = self.shared.local.lock() {
            l.check = check.clone();
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    h.check = check;
                    h.announce();
                }
            }
            None => self.to_host(Msg::Check(check)),
        }
    }

    /// The host starts the room: everyone plays the chosen episode from where it is. False when
    /// this device isn't the host or no episode is chosen yet.
    pub fn start(&self) -> bool {
        let Some(host) = self.shared.host_state() else { return false };
        let Ok(mut h) = host.lock() else { return false };
        let Some(mut state) = h.state.clone() else { return false };
        state.playing = true;
        state.by = self.shared.me.to_string();
        let state = h.adopt(state, false);
        h.broadcast_state(&state, None);
        // The host's own player starts through the same path as everyone else's.
        let _ = h.events.send(RoomEvent::State(state));
        h.announce();
        true
    }

    /// Where this device is, for the member list.
    pub fn send_status(&self, position: f64, buffering: bool) {
        if let Ok(mut l) = self.shared.local.lock() {
            l.position = position;
            l.buffering = buffering;
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    let changed = h.buffering != buffering;
                    h.position = position;
                    h.buffering = buffering;
                    if changed || !h.guests.is_empty() {
                        h.announce();
                    }
                }
            }
            None => self.to_host(Msg::Status { position, buffering }),
        }
    }

    /// Leaves the room. The host either ends it for everyone (`end`) or hands it over to the
    /// guest who joined first.
    pub async fn leave(self, end: bool) {
        self.shared.closing.store(true, Ordering::SeqCst);
        if let Some(host) = self.shared.host_state() {
            if let Ok(h) = host.lock() {
                let msg = if end { Msg::Ended } else { Msg::HandOver };
                for g in &h.guests {
                    let _ = g.tx.send(msg.clone());
                }
            }
            // Let the goodbye reach them before the connections close.
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        self.shared.endpoint.close().await;
    }
}

enum Outcome {
    /// This device left (the room was dropped).
    Closing,
    /// The host ended the room, or turned this device away.
    Ended(String),
    /// The host left and handed the room over.
    HandedOver,
    /// The connection dropped.
    Dropped,
}

/// A guest's side, for as long as it's in the room: one connection to the host after another.
async fn guest_loop(shared: Arc<Shared>, mut host: EndpointAddr, first: Connection, mut outgoing: mpsc::UnboundedReceiver<Msg>) {
    let mut conn = first;
    loop {
        let outcome = session(&shared, conn, &mut outgoing).await;
        if shared.closing() {
            return;
        }
        let handed_over = match outcome {
            Outcome::Closing => return,
            Outcome::Ended(reason) => return shared.end(reason),
            Outcome::HandedOver => true,
            Outcome::Dropped => false,
        };
        tracing::info!(handed_over, "watch: lost the host; rejoining");
        shared.emit(RoomEvent::Reconnecting);
        match rejoin(&shared, &host, handed_over).await {
            Some(Some((addr, c))) => {
                if addr.id != host.id {
                    if let Ok(mut role) = shared.role.lock() {
                        if let Role::Guest { code, .. } = &mut *role {
                            *code = room_code(&addr);
                        }
                    }
                    shared.emit(RoomEvent::Info(shared.info()));
                }
                host = addr;
                conn = c;
                shared.emit(RoomEvent::Reconnected);
            }
            // Took over hosting.
            Some(None) => {
                shared.emit(RoomEvent::Reconnected);
                return;
            }
            None => {
                if !shared.closing() {
                    shared.end("Lost the room: the host and everyone else in it were out of reach.".into());
                }
                return;
            }
        }
    }
}

/// One connection to the host: hello (with what this device had said), then messages both ways.
async fn session(shared: &Arc<Shared>, conn: Connection, outgoing: &mut mpsc::UnboundedReceiver<Msg>) -> Outcome {
    let Ok((mut send, recv)) = conn.open_bi().await else { return Outcome::Dropped };
    let hello = {
        let l = shared.local.lock().unwrap_or_else(|p| p.into_inner());
        Msg::Hello { name: shared.name.clone(), code: shared.my_code(), ready: l.ready, check: l.check.clone() }
    };
    if write_msg(&mut send, &hello).await.is_err() {
        return Outcome::Dropped;
    }
    // Reading in its own task: a read cut off halfway (by select) would lose the stream's place.
    let (in_tx, mut incoming) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        let mut recv = recv;
        while let Ok(m) = read_msg(&mut recv).await {
            if in_tx.send(m).is_err() {
                break;
            }
        }
    });
    let outcome = loop {
        tokio::select! {
            out = outgoing.recv() => match out {
                Some(m) => {
                    if write_msg(&mut send, &m).await.is_err() {
                        break Outcome::Dropped;
                    }
                }
                None => break Outcome::Closing,
            },
            inc = incoming.recv() => match inc {
                Some(Msg::State(s)) => {
                    if let Ok(mut l) = shared.local.lock() {
                        l.state = Some(s.clone());
                    }
                    shared.emit(RoomEvent::State(s));
                }
                Some(Msg::Roster(r)) => {
                    if let Ok(mut l) = shared.local.lock() {
                        l.roster = Some(r.clone());
                    }
                    shared.emit(RoomEvent::Roster(r));
                }
                Some(Msg::Refused { reason }) => break Outcome::Ended(reason),
                Some(Msg::Ended) => break Outcome::Ended("The host ended the room.".into()),
                Some(Msg::HandOver) => break Outcome::HandedOver,
                Some(_) => {}
                None => break Outcome::Dropped,
            },
        }
    };
    reader.abort();
    conn.close(0u32.into(), b"bye");
    outcome
}

/// Gets back into the room after losing the host. The old host gets [`HOST_GRACE`] (none when it
/// handed the room over), then each guest in the order they joined: everyone works the line out
/// from the same roster, so they meet at the same new host. Reaching our own turn means hosting.
/// `Some(Some(..))`: connected to a host; `Some(None)`: hosting now; `None`: gave up.
async fn rejoin(shared: &Arc<Shared>, old_host: &EndpointAddr, handed_over: bool) -> Option<Option<(EndpointAddr, Connection)>> {
    let line: Vec<(String, String)> = shared
        .local
        .lock()
        .ok()
        .and_then(|l| l.roster.clone())
        .map(|r| r.members.into_iter().filter(|m| !m.host).map(|m| (m.id, m.code)).collect())
        .unwrap_or_default();
    let me = shared.me.to_string();
    let began = Instant::now();
    loop {
        if shared.closing() || began.elapsed() > REJOIN_FOR {
            return None;
        }
        let turn = (began.elapsed().as_secs() / HOST_GRACE.as_secs()) as usize + usize::from(handed_over);
        let target = if turn == 0 {
            Some(old_host.clone())
        } else {
            match line.get(turn - 1) {
                Some((id, _)) if *id == me => {
                    shared.become_host();
                    return Some(None);
                }
                Some((_, code)) => parse_code(code).ok(),
                None => return None,
            }
        };
        if let Some(addr) = target {
            match connect(&shared.endpoint, addr.clone(), ATTEMPT).await {
                Ok(c) => return Some(Some((addr, c))),
                Err(e) => tracing::debug!(error = %e, turn, "watch: rejoin attempt failed"),
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn accept_loop(shared: Arc<Shared>) {
    while let Some(incoming) = shared.endpoint.accept().await {
        let shared = shared.clone();
        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(c) => c,
                Err(e) => return tracing::warn!(error = %e, "watch: a guest's connection failed"),
            };
            // Only a host serves guests (a guest's endpoint accepts so it can take over).
            let Some(host) = shared.host_state() else {
                connection.close(0u32.into(), b"not hosting");
                return;
            };
            tracing::info!(guest = %connection.remote_id().fmt_short(), "watch: guest connected");
            if let Err(e) = serve_guest(connection, host).await {
                tracing::warn!(error = %e, "watch: guest left with an error");
            }
        });
    }
}

async fn serve_guest(connection: Connection, host: Arc<Mutex<HostState>>) -> Result<()> {
    let id = connection.remote_id();
    let (mut send, mut recv) = connection.accept_bi().await.map_err(net)?;
    let Msg::Hello { name, code, ready, check } = read_msg(&mut recv).await? else {
        return Err(Error::Network("a guest didn't say hello".into()));
    };
    let name: String = name.trim().chars().take(32).collect();

    let (tx, mut outgoing) = mpsc::unbounded_channel::<Msg>();
    let admitted = {
        let mut h = host.lock().map_err(|_| Error::Network("room state poisoned".into()))?;
        // Rejoining: the old entry goes (its place in line too: it joins the end).
        h.guests.retain(|g| g.id != id);
        let room = h.guests.len() < MAX_GUESTS;
        if room {
            // A newcomer starts where the room is.
            if let Some(state) = &h.state {
                let _ = tx.send(Msg::State(state.clone()));
            }
            h.guests.push(Guest {
                id,
                name: if name.is_empty() { "Guest".into() } else { name },
                code,
                position: 0.0,
                buffering: true,
                ready,
                check,
                tx,
                conn: connection.clone(),
            });
            h.announce();
        }
        room
    };
    if !admitted {
        let _ = write_msg(&mut send, &Msg::Refused { reason: format!("The room is full ({MAX_GUESTS} guests).") }).await;
        let _ = send.finish();
        return Ok(());
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = outgoing.recv().await {
            if write_msg(&mut send, &msg).await.is_err() {
                break;
            }
        }
    });
    loop {
        match read_msg(&mut recv).await {
            Ok(Msg::State(asked)) => {
                if let Ok(mut h) = host.lock() {
                    let lobby = !h.started;
                    let wanted_play = asked.playing;
                    let state = h.adopt(asked, true);
                    // A guest's play in the lobby is turned down: the guest is told to stay paused too.
                    let except = if wanted_play && !state.playing { None } else { Some(&id) };
                    h.broadcast_state(&state, except);
                    let _ = h.events.send(RoomEvent::State(state));
                    if lobby {
                        h.announce();
                    }
                }
            }
            Ok(Msg::Status { position, buffering }) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guest_mut(&id) {
                        g.position = position;
                        g.buffering = buffering;
                    }
                    h.announce();
                }
            }
            Ok(Msg::Ready { ready }) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guest_mut(&id) {
                        g.ready = ready;
                    }
                    h.announce();
                }
            }
            Ok(Msg::Check(check)) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guest_mut(&id) {
                        g.check = check;
                    }
                    h.announce();
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    writer.abort();
    if let Ok(mut h) = host.lock() {
        // Only this connection's entry: a rejoin may already have replaced it.
        let before = h.guests.len();
        h.guests.retain(|g| !(g.id == id && g.conn.stable_id() == connection.stable_id()));
        if h.guests.len() != before {
            h.announce();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_tagged_json() {
        let msg = Msg::State(PlayState {
            media_id: 21,
            episode: 5.0,
            title: "Show".into(),
            dub: false,
            position: 12.5,
            playing: true,
            by: "x".into(),
            source: Some(SourceRef { id: "src".into(), name: "Src".into(), url: "https://example.com/m.json".into() }),
            subs: Some("en".into()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""t":"state""#), "{json}");
        assert!(json.contains(r#""mediaId":21"#), "{json}");
        let back: Msg = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Msg::State(s) if s.position == 12.5 && s.source.is_some()));
        // A member's check is flat: { "playable": true, "source": "…" }.
        let m = Member {
            id: "a".into(),
            name: "A".into(),
            host: false,
            position: 0.0,
            buffering: false,
            ready: true,
            check: Check { playable: Some(true), source: Some("Src".into()) },
            direct: Some(true),
            rtt_ms: Some(30),
            code: "c".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""playable":true"#) && json.contains(r#""rttMs":30"#), "{json}");
    }

    #[test]
    fn room_codes_carry_the_hosts_addresses() {
        let id = iroh::SecretKey::generate().public();
        let mut addr = EndpointAddr::from(id);
        addr.addrs.insert(TransportAddr::Relay("https://euc1-1.relay.n0.iroh.link./".parse().unwrap()));
        addr.addrs.insert(TransportAddr::Ip("192.168.137.1:50123".parse().unwrap()));
        let code = room_code(&addr);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{code}");
        // With a relay, the code carries only the relay (not the IPs).
        let parsed = parse_code(&format!("  {code}\n")).unwrap();
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.addrs.len(), 1);
        assert!(matches!(parsed.addrs.iter().next(), Some(TransportAddr::Relay(_))));
        // Without one, the IPs.
        let direct = EndpointAddr::from(id).with_ip_addr("192.168.137.1:50123".parse().unwrap());
        assert_eq!(parse_code(&room_code(&direct)).unwrap(), direct);
        // A bare id (the first format) still parses.
        assert_eq!(parse_code(&id.to_string()).unwrap().id, id);
        assert!(parse_code("not a code").is_err());
    }

    async fn until<T>(rx: &mut mpsc::UnboundedReceiver<RoomEvent>, mut pick: impl FnMut(RoomEvent) -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(40), async {
            loop {
                if let Some(t) = pick(rx.recv().await.expect("room events ended")) {
                    return t;
                }
            }
        })
        .await
        .expect("timed out waiting for a room event")
    }

    fn play(position: f64, playing: bool) -> PlayState {
        PlayState { media_id: 1, episode: 2.0, title: String::new(), dub: false, position, playing, by: String::new(), source: None, subs: None }
    }

    /// Real network (iroh's relays): `cargo test -p banime-core -- --ignored live_room`.
    #[tokio::test]
    #[ignore]
    async fn live_room_syncs_state_between_two_endpoints() {
        let (host, mut host_events) = Room::host("Host".into()).await.unwrap();
        let (guest, mut guest_events) = Room::join(&host.info().code, "Guest".into()).await.unwrap();
        until(&mut host_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;

        host.send_state(play(30.0, true));
        let s = until(&mut guest_events, |e| match e {
            RoomEvent::State(s) => Some(s),
            RoomEvent::Closed(r) => panic!("{r}"),
            _ => None,
        })
        .await;
        assert_eq!(s.position, 30.0);
        // And back: a guest's pause reaches the host.
        guest.send_state(play(31.0, false));
        until(&mut host_events, |e| matches!(e, RoomEvent::State(s) if !s.playing).then_some(())).await;
        // The guest's check reaches the host's roster.
        guest.set_check(Check { playable: Some(true), source: Some("Src".into()) });
        until(&mut host_events, |e| match e {
            RoomEvent::Roster(r) => r.members.iter().any(|m| !m.host && m.check.playable == Some(true)).then_some(()),
            _ => None,
        })
        .await;
        guest.leave(false).await;
        host.leave(true).await;
    }

    /// Real network: the host hands over, the first guest hosts, and the second follows it.
    #[tokio::test]
    #[ignore]
    async fn live_room_survives_the_host_leaving() {
        let (host, mut host_events) = Room::host("Host".into()).await.unwrap();
        let code = host.info().code;
        let (first, mut first_events) = Room::join(&code, "First".into()).await.unwrap();
        until(&mut host_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        let (second, mut second_events) = Room::join(&code, "Second".into()).await.unwrap();
        // Both guests know the whole line before the host goes.
        until(&mut second_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 3).then_some(())).await;
        until(&mut first_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 3).then_some(())).await;
        host.send_state(play(10.0, false));
        until(&mut first_events, |e| matches!(e, RoomEvent::State(_)).then_some(())).await;

        host.leave(false).await;
        let info = until(&mut first_events, |e| match e {
            RoomEvent::Info(i) => Some(i),
            _ => None,
        })
        .await;
        assert!(info.host, "the first guest hosts");
        assert!(first.is_host());
        // The second guest finds the new host and is told so.
        let moved = until(&mut second_events, |e| match e {
            RoomEvent::Info(i) => Some(i),
            RoomEvent::Closed(r) => panic!("{r}"),
            _ => None,
        })
        .await;
        assert!(!moved.host);
        assert_eq!(moved.code, info.code);
        until(&mut first_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        // The room's episode carried over.
        first.send_state(play(12.0, true));
        until(&mut second_events, |e| matches!(e, RoomEvent::State(s) if s.position == 12.0).then_some(())).await;
        second.leave(false).await;
        first.leave(true).await;
    }
}
