//! Watch together: a small room of B-Anime copies that play the same episode in step.
//!
//! Only playback controls travel between devices (play, pause, seek, which episode); each device
//! plays the episode from its own source or library. Devices connect to each other directly with
//! iroh: hole punching where it works, falling back to iroh's public relays (which only forward
//! encrypted traffic) where it doesn't. B-Anime runs no server.
//!
//! The host's device is the room: guests dial it by the room code, which carries the host's
//! endpoint id and its addresses (home relay, direct IPs), so joining doesn't depend on iroh's
//! address lookup service being reachable from both networks. The
//! host keeps the room's state, so a change from anyone goes to the host and the host passes it on.
//! Messages are JSON, each sent as a 4-byte big-endian length followed by the bytes.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, TransportAddr};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{Error, Result};

/// The protocol name; a change that old copies can't read gets a new number.
const ALPN: &[u8] = b"banime/watch/1";
/// No message comes close to this; anything bigger is a misbehaving peer.
const MAX_MESSAGE: usize = 64 * 1024;
/// Guests per room besides the host.
const MAX_GUESTS: usize = 8;
/// How long joining may take before the guest is told it didn't work.
const JOIN_TIMEOUT: Duration = Duration::from_secs(20);

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
}

/// Who's in the room and whether the host has started it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub members: Vec<Member>,
    /// False in the lobby: everyone gathers, says they're ready, and the host starts.
    pub started: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
enum Msg {
    /// A guest's first message.
    Hello { name: String },
    /// Host → guests: the room's state. Guest → host: a change the guest made.
    State(PlayState),
    /// Guest → host: where the guest is.
    Status { position: f64, buffering: bool },
    /// Guest → host: ready to start, or not any more.
    Ready { ready: bool },
    /// Host → guests: everyone in the room, and whether it has started.
    Roster(Roster),
    /// Host → guest: why it was turned away.
    Refused { reason: String },
}

/// What the room tells this device.
#[derive(Debug, Clone)]
pub enum RoomEvent {
    /// Play this (someone else changed it, or the host's regular position update).
    State(PlayState),
    Roster(Roster),
    /// The room ended for this device (the host left, the connection dropped, or it was refused).
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

/// A device's place in a room, host or guest. Dropping it leaves the room.
pub struct Room {
    endpoint: Endpoint,
    code: String,
    me: String,
    role: Role,
}

enum Role {
    Host(Arc<Mutex<HostState>>),
    Guest { to_host: mpsc::UnboundedSender<Msg>, connection: Connection },
}

struct HostState {
    name: String,
    me: String,
    state: Option<PlayState>,
    position: f64,
    buffering: bool,
    ready: bool,
    /// The host has started the room (the lobby is over).
    started: bool,
    guests: HashMap<EndpointId, Guest>,
    events: mpsc::UnboundedSender<RoomEvent>,
}

struct Guest {
    name: String,
    position: f64,
    buffering: bool,
    ready: bool,
    tx: mpsc::UnboundedSender<Msg>,
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
        }];
        list.extend(self.guests.iter().map(|(id, g)| Member {
            id: id.to_string(),
            name: g.name.clone(),
            host: false,
            position: g.position,
            buffering: g.buffering,
            ready: g.ready,
        }));
        list
    }

    /// Everyone's list changed: tell the guests and this device.
    fn announce_members(&self) {
        let roster = Roster { members: self.members(), started: self.started };
        for g in self.guests.values() {
            let _ = g.tx.send(Msg::Roster(roster.clone()));
        }
        let _ = self.events.send(RoomEvent::Roster(roster));
    }

    /// A new state for the room, from a guest (`from_guest`) or the host. In the lobby only the
    /// host starts playback, and picking another episode there asks everyone to get ready again.
    /// Returns the state as the room keeps it.
    fn adopt(&mut self, mut state: PlayState, from_guest: bool) -> PlayState {
        if !self.started {
            if state.playing {
                if from_guest {
                    state.playing = false;
                } else {
                    self.started = true;
                }
            }
            let other_episode =
                self.state.as_ref().is_none_or(|s| s.media_id != state.media_id || s.episode != state.episode);
            if other_episode && !self.started {
                self.ready = false;
                for g in self.guests.values_mut() {
                    g.ready = false;
                }
            }
        }
        self.state = Some(state.clone());
        state
    }

    fn broadcast_state(&self, state: &PlayState, except: Option<&EndpointId>) {
        for (id, g) in &self.guests {
            if Some(id) != except {
                let _ = g.tx.send(Msg::State(state.clone()));
            }
        }
    }
}

async fn bind(accept: bool) -> Result<Endpoint> {
    let builder = Endpoint::builder(presets::N0);
    let builder = if accept { builder.alpns(vec![ALPN.to_vec()]) } else { builder };
    builder.bind().await.map_err(net)
}

impl Room {
    /// Opens a room on this device. Guests join with [`Room::code`].
    pub async fn host(name: String) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let endpoint = bind(true).await?;
        // Reachable (home relay picked, address published) before the code is handed out.
        endpoint.online().await;
        let me = endpoint.id().to_string();
        let (events, rx) = mpsc::unbounded_channel();
        let host = Arc::new(Mutex::new(HostState {
            name,
            me: me.clone(),
            state: None,
            position: 0.0,
            buffering: false,
            ready: false,
            started: false,
            guests: HashMap::new(),
            events,
        }));
        tokio::spawn(accept_loop(endpoint.clone(), host.clone()));
        if let Ok(h) = host.lock() {
            h.announce_members();
        }
        let addr = endpoint.addr();
        tracing::info!(addrs = ?addr.addrs, "watch: room open");
        let code = room_code(&addr);
        Ok((Room { endpoint, code, me, role: Role::Host(host) }, rx))
    }

    /// Joins the room with this code.
    pub async fn join(code: &str, name: String) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let host_addr = parse_code(code)?;
        tracing::info!(host = %host_addr.id.fmt_short(), addrs = ?host_addr.addrs, "watch: joining room");
        let endpoint = bind(false).await?;
        let unreachable = "Couldn't reach the room. Check the code, and that the host still has the room open.";
        let connection = match tokio::time::timeout(JOIN_TIMEOUT, endpoint.connect(host_addr, ALPN)).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "watch: couldn't connect to the room");
                endpoint.close().await;
                return Err(Error::Network(format!("{unreachable} ({e})")));
            }
            Err(_) => {
                tracing::warn!("watch: connecting to the room timed out");
                endpoint.close().await;
                return Err(Error::Network(unreachable.into()));
            }
        };
        tracing::info!("watch: connected to the room");
        let (mut send, mut recv) = connection.open_bi().await.map_err(net)?;
        write_msg(&mut send, &Msg::Hello { name }).await?;

        let (events, rx) = mpsc::unbounded_channel();
        let (to_host, mut outgoing) = mpsc::unbounded_channel::<Msg>();
        // Writer: this device's changes and status, in order.
        tokio::spawn(async move {
            while let Some(msg) = outgoing.recv().await {
                if write_msg(&mut send, &msg).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        });
        // Reader: the host's state and member list.
        let conn = connection.clone();
        tokio::spawn(async move {
            let reason = loop {
                match read_msg(&mut recv).await {
                    Ok(Msg::State(s)) => {
                        let _ = events.send(RoomEvent::State(s));
                    }
                    Ok(Msg::Roster(roster)) => {
                        let _ = events.send(RoomEvent::Roster(roster));
                    }
                    Ok(Msg::Refused { reason }) => break reason,
                    Ok(_) => {}
                    Err(_) => break "The room ended: the host left or the connection dropped.".to_owned(),
                }
            };
            conn.close(0u32.into(), b"bye");
            let _ = events.send(RoomEvent::Closed(reason));
        });

        let me = endpoint.id().to_string();
        Ok((Room { endpoint, code: code.trim().to_owned(), me, role: Role::Guest { to_host, connection } }, rx))
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    /// This device's id in the member list.
    pub fn me(&self) -> &str {
        &self.me
    }

    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host(_))
    }

    /// This device's player changed (or, from the host, its regular position update).
    pub fn send_state(&self, mut state: PlayState) {
        state.by = self.me.clone();
        match &self.role {
            Role::Host(host) => {
                if let Ok(mut h) = host.lock() {
                    let lobby = !h.started;
                    h.position = state.position;
                    let state = h.adopt(state, false);
                    h.broadcast_state(&state, None);
                    // Readiness and "started" only change in the lobby.
                    if lobby {
                        h.announce_members();
                    }
                }
            }
            Role::Guest { to_host, .. } => {
                let _ = to_host.send(Msg::State(state));
            }
        }
    }

    /// Ready to start (or not), for the lobby.
    pub fn set_ready(&self, ready: bool) {
        match &self.role {
            Role::Host(host) => {
                if let Ok(mut h) = host.lock() {
                    h.ready = ready;
                    h.announce_members();
                }
            }
            Role::Guest { to_host, .. } => {
                let _ = to_host.send(Msg::Ready { ready });
            }
        }
    }

    /// The host starts the room: everyone plays the chosen episode from where it is. False when
    /// this device isn't the host or no episode is chosen yet.
    pub fn start(&self) -> bool {
        let Role::Host(host) = &self.role else { return false };
        let Ok(mut h) = host.lock() else { return false };
        let Some(mut state) = h.state.clone() else { return false };
        state.playing = true;
        state.by = self.me.clone();
        let state = h.adopt(state, false);
        h.broadcast_state(&state, None);
        // The host's own player starts through the same path as everyone else's.
        let _ = h.events.send(RoomEvent::State(state));
        h.announce_members();
        true
    }

    /// Where this device is, for the member list.
    pub fn send_status(&self, position: f64, buffering: bool) {
        match &self.role {
            Role::Host(host) => {
                if let Ok(mut h) = host.lock() {
                    let changed = h.buffering != buffering;
                    h.position = position;
                    h.buffering = buffering;
                    if changed || !h.guests.is_empty() {
                        h.announce_members();
                    }
                }
            }
            Role::Guest { to_host, .. } => {
                let _ = to_host.send(Msg::Status { position, buffering });
            }
        }
    }

    /// Leaves the room; for the host, that ends it for everyone.
    pub async fn leave(self) {
        if let Role::Guest { connection, .. } = &self.role {
            connection.close(0u32.into(), b"bye");
        }
        self.endpoint.close().await;
    }
}

async fn accept_loop(endpoint: Endpoint, host: Arc<Mutex<HostState>>) {
    while let Some(incoming) = endpoint.accept().await {
        let host = host.clone();
        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(c) => c,
                Err(e) => return tracing::warn!(error = %e, "watch: a guest's connection failed"),
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
    let Msg::Hello { name } = read_msg(&mut recv).await? else {
        return Err(Error::Network("a guest didn't say hello".into()));
    };
    let name: String = name.trim().chars().take(32).collect();

    let (tx, mut outgoing) = mpsc::unbounded_channel::<Msg>();
    let admitted = {
        let mut h = host.lock().map_err(|_| Error::Network("room state poisoned".into()))?;
        let room = h.guests.len() < MAX_GUESTS;
        if room {
            // A newcomer starts where the room is.
            if let Some(state) = &h.state {
                let _ = tx.send(Msg::State(state.clone()));
            }
            h.guests.insert(id, Guest { name: if name.is_empty() { "Guest".into() } else { name }, position: 0.0, buffering: true, ready: false, tx });
            h.announce_members();
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
                        h.announce_members();
                    }
                }
            }
            Ok(Msg::Ready { ready }) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guests.get_mut(&id) {
                        g.ready = ready;
                    }
                    h.announce_members();
                }
            }
            Ok(Msg::Status { position, buffering }) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guests.get_mut(&id) {
                        g.position = position;
                        g.buffering = buffering;
                    }
                    h.announce_members();
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    writer.abort();
    if let Ok(mut h) = host.lock() {
        h.guests.remove(&id);
        h.announce_members();
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
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""t":"state""#), "{json}");
        assert!(json.contains(r#""mediaId":21"#), "{json}");
        let back: Msg = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Msg::State(s) if s.position == 12.5));
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
        let parsed = parse_code(&format!("  {code}
")).unwrap();
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

    /// Real network (iroh's relays and address lookup):
    /// `cargo test -p banime-core -- --ignored live_room`.
    #[tokio::test]
    #[ignore]
    async fn live_room_syncs_state_between_two_endpoints() {
        let (host, mut host_events) = Room::host("Host".into()).await.unwrap();
        let (guest, mut guest_events) = Room::join(host.code(), "Guest".into()).await.unwrap();

        let state = PlayState { media_id: 1, episode: 2.0, title: String::new(), dub: false, position: 30.0, playing: true, by: String::new() };
        // Wait for the guest to be in the room before the host plays.
        loop {
            if let Some(RoomEvent::Roster(r)) = host_events.recv().await {
                if r.members.len() == 2 {
                    break;
                }
            }
        }
        host.send_state(state.clone());
        loop {
            match guest_events.recv().await.unwrap() {
                RoomEvent::State(s) => {
                    assert_eq!(s.position, 30.0);
                    break;
                }
                RoomEvent::Closed(r) => panic!("{r}"),
                _ => {}
            }
        }
        // And back: a guest's pause reaches the host.
        guest.send_state(PlayState { playing: false, ..state });
        loop {
            if let Some(RoomEvent::State(s)) = host_events.recv().await {
                assert!(!s.playing);
                break;
            }
        }
        guest.leave().await;
        host.leave().await;
    }
}
