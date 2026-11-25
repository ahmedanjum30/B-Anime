//! Watch together: a small room of B-Anime copies that play the same episode in step.
//!
//! Only playback controls travel between devices (play, pause, seek, which episode); each device
//! plays the episode from its own source or library. Devices connect to each other directly with
//! iroh: hole punching where it works, falling back to iroh's public relays (which only forward
//! encrypted traffic) where it doesn't. B-Anime runs no server.
//!
//! The host's device is the room: guests dial it by the room code, a short code like
//! `K7QM-4X2P-9FTE` that the host's key is made from (so the code alone gives its endpoint id),
//! plus a letter for its home relay, so joining doesn't depend on iroh's address lookup service. The
//! host keeps the room's state, so a change from anyone goes to the host and the host passes it on.
//!
//! Losing the host isn't the end of the room. A guest whose connection drops keeps trying for a
//! minute. If the host itself is gone (it left, or nobody can reach it for a while), the guest who
//! joined first takes over: every device can accept connections, and the roster carries each
//! member's own code, so the others know where to go.
//!
//! Messages are JSON, each sent as a 4-byte big-endian length followed by the bytes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
/// Silence this long means the other side is gone (app closed, phone asleep, network lost):
/// guests ping every 2 s and the host answers, so a live link is never quiet for this long.
/// QUIC alone would take 30 s to notice.
const GONE_AFTER: Duration = Duration::from_secs(10);
/// A device that took over hosting and has been alone this long, not playing, ends the room.
const EMPTY_ROOM: Duration = Duration::from_secs(10 * 60);
/// A host that lost its guests looks for them hosting the room elsewhere for this long.
const LOOK_FOR_ROOM: Duration = Duration::from_secs(5 * 60);
/// Guests lost within this of each other went together: the host's own connection dropped.
const LOST_TOGETHER: Duration = Duration::from_secs(20);
/// Rejoining keeps trying this long: a takeover after the host left takes a few seconds.
const REJOIN_TRYING: Duration = Duration::from_secs(20);
/// Positions and connection quality reach the others at most this often: reports only change
/// those, so each guest gets one roster per interval instead of one per member's report. A
/// visible change (loading, ready, joining, leaving…) goes out at once.
const ROSTER_EVERY: Duration = Duration::from_secs(2);
/// Only the host moves the timeline: a guest's change this far from where the room is (more
/// than drift and the host's 2 s reports explain) is a seek, and the room keeps its position.
const GUEST_SEEK: f64 = 5.0;

/// Room codes use Crockford's base 32: no I, L, O or U, so they read out and type without
/// mix-ups (and a typed I or L reads as 1, an O as 0).
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Characters of a code, which make the key (5 bits each: 55 bits, far too many to guess).
const CODE_SEED_CHARS: usize = 11;
/// iroh's default relays: a join tries the host through each.
const RELAYS: [(char, &str); 4] = [
    ('E', "euc1-1.relay.n0.iroh.link."),
    ('N', "use1-1.relay.n0.iroh.link."),
    ('W', "usw1-1.relay.n0.iroh.link."),
    ('S', "aps1-1.relay.n0.iroh.link."),
];

/// A device's identity in rooms: a short random seed, its key made from it. Whoever has the code
/// can work out the key, so the code is what admits people (anyone with it could also stand in
/// as the room's host while the real one is away; fine for a watch party).
#[derive(Clone, Copy)]
struct Seed(u64);

impl Seed {
    fn random() -> Seed {
        Seed(rand::random::<u64>() >> (64 - 5 * CODE_SEED_CHARS))
    }

    fn key(self) -> iroh::SecretKey {
        let digest = Sha256::digest([b"banime/watch/room/1:".as_slice(), &self.0.to_be_bytes()].concat());
        iroh::SecretKey::from_bytes(&digest.into())
    }

    fn chars(self) -> String {
        (0..CODE_SEED_CHARS)
            .rev()
            .map(|i| CODE_ALPHABET[((self.0 >> (5 * i)) & 31) as usize] as char)
            .collect()
    }
}

/// A device's room code: its 11 seed characters, `K7QM-4X2P-9FT`. The same for good for a
/// device that keeps its seed (B-Anime saves one), whichever relay it's on: joining tries them all.
fn short_code(seed: Seed) -> String {
    let chars: Vec<char> = seed.chars().chars().collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// A room code back to the host's address, once through each of iroh's relays (it's on one of
/// them; a join tries all at once).
fn parse_code(code: &str) -> Result<Vec<EndpointAddr>> {
    let bad = || Error::Config("That isn't a room code. Copy the whole code from the host.".into());
    let chars: Vec<char> = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        })
        .collect();
    if chars.len() != CODE_SEED_CHARS {
        return Err(bad());
    }
    let mut seed = 0u64;
    for c in &chars {
        seed = (seed << 5) | CODE_ALPHABET.iter().position(|a| *a as char == *c).ok_or_else(bad)? as u64;
    }
    let id = Seed(seed).key().public();
    Ok(RELAYS
        .iter()
        .filter_map(|(_, host)| format!("https://{host}/").parse::<RelayUrl>().ok())
        .map(|relay| EndpointAddr::from(id).with_relay_url(relay))
        .collect())
}

/// A typed code, tidied: upper case, grouped like `K7QM-4X2P-9FT`.
fn short_code_of(code: &str) -> String {
    let chars: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    chars.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// A new device seed, to keep (the device's room code for good): see [`code_for_seed`].
pub fn new_seed() -> u64 {
    Seed::random().0
}

/// The room code a device with this seed hosts under.
pub fn code_for_seed(seed: u64) -> String {
    short_code(device_seed(Some(seed)))
}

/// Connects to whichever of `addrs` answers first (the same host through different relays, or
/// different members): the others are dropped.
async fn connect_any(endpoint: &Endpoint, addrs: Vec<EndpointAddr>, within: Duration) -> Option<(EndpointAddr, Connection)> {
    let mut tries = tokio::task::JoinSet::new();
    for addr in addrs {
        let endpoint = endpoint.clone();
        tries.spawn(async move { connect(&endpoint, addr.clone(), within).await.ok().map(|c| (addr, c)) });
    }
    while let Some(done) = tries.join_next().await {
        if let Ok(Some(found)) = done {
            tries.abort_all();
            return Some(found);
        }
    }
    None
}

/// What a member answered when this device looked for the room's host there.
enum Reply {
    /// It hosts the room: the connection, to join on.
    Hosting(EndpointAddr, Connection),
    /// It doesn't host, but it's in the room with the host: the host isn't gone.
    HasHost,
    /// It doesn't host, and it has lost the host too.
    LostHost,
    /// No answer: it's gone, or this device is offline.
    Unreachable,
}

/// Why a member that isn't hosting closes a connection: whether it's still with the host.
const NOT_HOSTING_WITH_HOST: &[u8] = b"not hosting; with the host";
const NOT_HOSTING_LOST_HOST: &[u8] = b"not hosting; lost the host";

/// Looks for the room's host at one member (`addrs`: that member through each relay).
async fn ask(endpoint: &Endpoint, addrs: Vec<EndpointAddr>, within: Duration) -> Reply {
    let Some((addr, conn)) = connect_any(endpoint, addrs, within).await else { return Reply::Unreachable };
    // A member that isn't hosting closes the connection at once, saying whether it has the host.
    match tokio::time::timeout(Duration::from_millis(1500), conn.closed()).await {
        Err(_) => Reply::Hosting(addr, conn),
        Ok(ConnectionError::ApplicationClosed(c)) if c.reason.as_ref() == NOT_HOSTING_WITH_HOST => Reply::HasHost,
        Ok(_) => Reply::LostHost,
    }
}

/// Asks several members at once (each a number of the caller's and its addresses), until one of
/// them hosts. Each reply comes with its member's number.
async fn ask_all(endpoint: &Endpoint, members: Vec<(usize, Vec<EndpointAddr>)>, within: Duration) -> Vec<(usize, Reply)> {
    let mut tries = tokio::task::JoinSet::new();
    for (n, addrs) in members {
        let endpoint = endpoint.clone();
        tries.spawn(async move { (n, ask(&endpoint, addrs, within).await) });
    }
    let mut replies = Vec::new();
    while let Some(done) = tries.join_next().await {
        if let Ok(reply) = done {
            let hosting = matches!(reply.1, Reply::Hosting(..));
            replies.push(reply);
            if hosting {
                tries.abort_all();
                break;
            }
        }
    }
    replies
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
    /// Held at the start after the host switched show or episode: everyone loads it, and the host
    /// plays it once they all have. Nobody else's play starts it meanwhile.
    #[serde(default)]
    pub held: bool,
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
    /// The episode their player is on (with `buffering`: whether it has loaded it).
    pub media_id: Option<i64>,
    pub episode: Option<f64>,
    /// How long their copy of that episode is (s). A different cut (a Blu-ray file against a TV
    /// stream) doesn't line up with the others, so the room points it out.
    #[serde(default)]
    pub length: Option<f64>,
    /// False while their connection is lost (they didn't leave): the room waits for them to
    /// come back, for up to [`REJOIN_FOR`].
    #[serde(default = "connected_default")]
    pub connected: bool,
}

fn connected_default() -> bool {
    true
}

/// Who's in the room and whether the host has started it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub members: Vec<Member>,
    /// False in the lobby: everyone gathers, says they're ready, and the host starts.
    pub started: bool,
    /// The room's name ("Ahmed's room" unless the host renamed it).
    #[serde(default)]
    pub name: String,
}

/// Where a room was, as a returning guest last knew it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RoomSoFar {
    pub state: PlayState,
    pub started: bool,
    #[serde(default)]
    pub name: String,
}

/// A room's name when the host hasn't given it one: "<host>'s room".
fn default_room_name(host: &str) -> String {
    let host = host.trim();
    if host.is_empty() { "Watch together".to_owned() } else { format!("{host}'s room") }
}

/// A name as typed: trimmed, at most 32 characters.
fn clean_name(name: &str) -> String {
    name.trim().chars().take(32).collect()
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
    Hello {
        name: String,
        code: String,
        ready: bool,
        check: Check,
        /// A guest coming back: where the room was. A room that has just opened (its host came
        /// back and opened it again under the same code) carries on from here instead of from
        /// its own player.
        #[serde(default)]
        room: Option<RoomSoFar>,
    },
    /// Host → guests: the room's state. Guest → host: a change the guest made.
    State(PlayState),
    /// Guest → host: where the guest is.
    Status {
        position: f64,
        buffering: bool,
        #[serde(default)]
        media_id: Option<i64>,
        #[serde(default)]
        episode: Option<f64>,
        #[serde(default)]
        length: Option<f64>,
    },
    /// Guest → host: ready to start, or not any more.
    Ready { ready: bool },
    /// Guest → host: whether the guest can play the room's episode.
    Check(Check),
    /// Guest → host: the guest's new name.
    Rename { name: String },
    /// Guest → host: leaving on purpose (not a lost connection: nobody waits for them).
    Bye,
    /// Host → guests: everyone in the room, and whether it has started.
    Roster(Roster),
    /// Host → guest: why it was turned away.
    Refused { reason: String },
    /// Host → guests: the host ended the room for everyone.
    Ended,
    /// Host → guests: the host left; the guest who joined first hosts from here.
    HandOver,
    /// Guest → host every 2 s, and the host's immediate answer: the guest measures its ping (its
    /// radio is awake, having just sent, so a phone's Wi-Fi power saving doesn't skew it).
    Ping { n: u32 },
    Pong { n: u32 },
    /// Guest → host: its ping, the median of the last few.
    Latency { ms: u32 },
    /// A message from a newer B-Anime: ignored, rather than ending the connection.
    #[serde(other)]
    Unknown,
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
/// Its key comes from `seed`, so its short code identifies it.
async fn bind(seed: Seed) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(seed.key())
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(net)
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
    seed: Seed,
    me: EndpointId,
    /// This device's name in the room (it can change; a reconnect's hello carries the latest).
    name: Mutex<String>,
    events: mpsc::UnboundedSender<RoomEvent>,
    role: Mutex<Role>,
    /// What this device has said and seen, kept across reconnects and a change of host.
    local: Mutex<Local>,
    closing: AtomicBool,
    /// A guest with a live connection to the host (what it tells members looking for the host).
    linked: AtomicBool,
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

impl Local {
    /// Starts from where a room was (this device was in it before).
    fn carry(&mut self, r: RoomSoFar) {
        self.position = r.state.position;
        self.state = Some(r.state);
        self.roster = Some(Roster { members: Vec::new(), started: r.started, name: r.name });
    }
}

/// This device's seed (kept, so it's the same member and code every time), or a new one.
fn device_seed(seed: Option<u64>) -> Seed {
    seed.map(|s| Seed(s & ((1 << (5 * CODE_SEED_CHARS)) - 1))).unwrap_or_else(Seed::random)
}

enum Role {
    Host(Arc<Mutex<HostState>>),
    /// `to_host` feeds whichever connection to the host is current; `code` is the host's.
    Guest { to_host: mpsc::UnboundedSender<Msg>, code: String },
}

struct HostState {
    me: String,
    name: String,
    room_name: String,
    code: String,
    state: Option<PlayState>,
    position: f64,
    buffering: bool,
    on: Option<(i64, f64)>,
    length: Option<f64>,
    ready: bool,
    check: Check,
    /// The host has started the room (the lobby is over).
    started: bool,
    /// In the order they joined (the order in which they'd take over hosting).
    guests: Vec<Guest>,
    events: mpsc::UnboundedSender<RoomEvent>,
    /// When the roster last went out, and whether a throttled change is waiting for the next.
    roster_at: Instant,
    roster_due: bool,
    /// Guests whose connection was lost (they didn't leave), while they may come back.
    dropped: Vec<DroppedGuest>,
    /// Reopened here by its host (Rejoin, nobody else hosting) and nobody has joined yet: a
    /// returning guest's place wins. Never for a new room (Start a room), which keeps its lobby.
    fresh: bool,
    /// Guests whose connection was lost lately (id, code, when), in the lobby too: where this
    /// host looks for the room, should it turn out to be the one that was cut off.
    lost: Vec<(EndpointId, String, Instant)>,
}

/// A guest whose connection was lost: listed (as not connected) until it's back or gone for good.
struct DroppedGuest {
    id: EndpointId,
    name: String,
    code: String,
    position: f64,
    on: Option<(i64, f64)>,
    since: Instant,
}

struct Guest {
    id: EndpointId,
    name: String,
    code: String,
    position: f64,
    buffering: bool,
    on: Option<(i64, f64)>,
    length: Option<f64>,
    ready: bool,
    check: Check,
    tx: mpsc::UnboundedSender<Msg>,
    conn: Connection,
    /// Its ping to the host, as it measured and reported it.
    latency: Option<u32>,
}

/// How many recent pings the shown one is the median of, and how often a guest pings.
const PINGS_KEPT: usize = 5;
const PING_EVERY: Duration = Duration::from_secs(2);

/// A guest's pings to the host: the one in flight and the last few round trips.
#[derive(Default)]
struct Pinger {
    n: u32,
    sent: Option<(u32, Instant)>,
    rtts: Vec<Duration>,
    reported: Option<u32>,
}

impl Pinger {
    fn next(&mut self) -> Option<Msg> {
        // One at a time; an unanswered one is given up after 10 s.
        if self.sent.is_some_and(|(_, at)| at.elapsed() < Duration::from_secs(10)) {
            return None;
        }
        self.n = self.n.wrapping_add(1);
        self.sent = Some((self.n, Instant::now()));
        Some(Msg::Ping { n: self.n })
    }

    /// The answer to ping `n`: the new median, when it changed (to report).
    fn answered(&mut self, n: u32) -> Option<u32> {
        let (_, at) = self.sent.take().filter(|(sent, _)| *sent == n)?;
        self.rtts.push(at.elapsed());
        if self.rtts.len() > PINGS_KEPT {
            self.rtts.remove(0);
        }
        let mut r = self.rtts.clone();
        r.sort();
        let ms = r.get(r.len() / 2)?.as_millis().min(u32::MAX as u128) as u32;
        (self.reported != Some(ms)).then(|| {
            self.reported = Some(ms);
            ms
        })
    }
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
            media_id: self.on.map(|o| o.0),
            episode: self.on.map(|o| o.1),
            length: self.length,
            connected: true,
        }];
        list.extend(self.guests.iter().map(|g| {
            let (direct, quic_rtt) = quality(&g.conn);
            // Measured pings; QUIC's own estimate until there are some.
            let rtt_ms = g.latency.or(quic_rtt);
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
                media_id: g.on.map(|o| o.0),
                episode: g.on.map(|o| o.1),
                length: g.length,
                connected: true,
            }
        }));
        // Last: they're the last to try if the host leaves meanwhile.
        list.extend(self.dropped.iter().map(|d| Member {
            id: d.id.to_string(),
            name: d.name.clone(),
            host: false,
            position: d.position,
            buffering: false,
            ready: false,
            check: Check::default(),
            direct: None,
            rtt_ms: None,
            code: d.code.clone(),
            media_id: d.on.map(|o| o.0),
            episode: d.on.map(|o| o.1),
            length: None,
            connected: false,
        }));
        list
    }

    /// Everyone's list changed visibly: tell the guests and this device now.
    fn announce(&mut self) {
        let roster = Roster { members: self.members(), started: self.started, name: self.room_name.clone() };
        for g in &self.guests {
            let _ = g.tx.send(Msg::Roster(roster.clone()));
        }
        let _ = self.events.send(RoomEvent::Roster(roster));
        self.roster_at = Instant::now();
        self.roster_due = false;
    }

    /// Only positions (or connection quality) changed: at most once per [`ROSTER_EVERY`]; the
    /// room's ticker sends a change that came too soon.
    fn announce_later(&mut self) {
        if self.roster_at.elapsed() >= ROSTER_EVERY {
            self.announce();
        } else {
            self.roster_due = true;
        }
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
        // Held after a switch: a guest's play (its autoplay, say) doesn't start it; the host does.
        if from_guest && state.playing && self.state.as_ref().is_some_and(|s| s.held) && !other_episode {
            state.playing = false;
            state.held = true;
        }
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
    fn new(endpoint: Endpoint, seed: Seed, name: String, events: mpsc::UnboundedSender<RoomEvent>, role: Role) -> Arc<Shared> {
        Arc::new(Shared {
            me: endpoint.id(),
            seed,
            endpoint,
            name: Mutex::new(name),
            events,
            role: Mutex::new(role),
            local: Mutex::new(Local::default()),
            closing: AtomicBool::new(false),
            linked: AtomicBool::new(false),
        })
    }

    fn my_name(&self) -> String {
        self.name.lock().map(|n| n.clone()).unwrap_or_default()
    }

    fn emit(&self, e: RoomEvent) {
        let _ = self.events.send(e);
    }

    /// This device's room code.
    fn my_code(&self) -> String {
        short_code(self.seed)
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

    /// Hosts the room from what this device knows of it. `fresh`: it has just opened (new, or
    /// reopened by its host), so the first member back brings where everyone is; not when a
    /// guest takes over a room that's going on.
    fn become_host(self: &Arc<Self>, fresh: bool) {
        let host = {
            let l = self.local.lock().unwrap_or_else(|p| p.into_inner());
            let name = self.my_name();
            // Taking over keeps the room's name; a new room is named after its host.
            let room_name = l.roster.as_ref().map(|r| r.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| default_room_name(&name));
            // Taking over from a host that left: the room carries on paused, where this device's
            // player is, until someone plays it (the others stopped while it changed hands).
            let state = l.state.clone().map(|mut s| {
                if !fresh {
                    s.playing = false;
                    // Nobody's pause (no "… paused" notice): the room changed hands.
                    s.by = String::new();
                    if l.position > 0.0 {
                        s.position = l.position;
                    }
                }
                s
            });
            HostState {
                me: self.me.to_string(),
                name,
                room_name,
                code: self.my_code(),
                state,
                position: l.position,
                buffering: l.buffering,
                on: None,
                length: None,
                ready: l.ready,
                check: l.check.clone(),
                started: l.roster.as_ref().is_some_and(|r| r.started),
                guests: Vec::new(),
                events: self.events.clone(),
                roster_at: Instant::now(),
                roster_due: false,
                dropped: Vec::new(),
                fresh,
                lost: Vec::new(),
            }
        };
        let host = Arc::new(Mutex::new(host));
        tokio::spawn(look_for_moved_room(self.clone(), Arc::downgrade(&host)));
        // Sends a throttled roster that's waiting, until the room ends here.
        let ticker = Arc::downgrade(&host);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(ROSTER_EVERY).await;
                let Some(host) = ticker.upgrade() else { break };
                let Ok(mut h) = host.lock() else { break };
                if h.roster_due {
                    h.announce();
                }
            }
        });
        if let Ok(mut role) = self.role.lock() {
            *role = Role::Host(host.clone());
        }
        tracing::info!("watch: hosting the room");
        self.emit(RoomEvent::Info(self.info()));
        // This device's player pauses too.
        if !fresh {
            if let Some(state) = host.lock().ok().and_then(|h| h.state.clone()) {
                self.emit(RoomEvent::State(state));
            }
        }
        if let Ok(mut h) = host.lock() {
            h.announce();
        }
    }

    /// This device stops hosting and joins the room where it went on without it (`addr`, whose
    /// code is `code`): it had lost its connection, and a guest took over meanwhile.
    fn become_guest(self: &Arc<Self>, addr: EndpointAddr, conn: Connection, code: String) {
        if let Some(host) = self.host_state() {
            if let Ok(h) = host.lock() {
                let mut l = self.local.lock().unwrap_or_else(|p| p.into_inner());
                l.state = h.state.clone();
                l.position = h.position;
                l.roster = Some(Roster { members: h.members(), started: h.started, name: h.room_name.clone() });
            }
        }
        let (to_host, outgoing) = mpsc::unbounded_channel();
        if let Ok(mut role) = self.role.lock() {
            *role = Role::Guest { to_host, code };
        }
        self.emit(RoomEvent::Info(self.info()));
        tokio::spawn(guest_loop(self.clone(), addr, conn, outgoing));
    }

    fn end(&self, reason: String) {
        if !self.closing.swap(true, Ordering::SeqCst) {
            self.emit(RoomEvent::Closed(reason));
        }
    }
}

impl Room {
    /// Opens a room on this device. Guests join with [`Room::info`]'s code.
    /// Opens a room on this device, named `room_name` (or "<host>'s room"), under the code of
    /// `seed` (a kept one gives the device the same code every time).
    pub async fn host(name: String, room_name: Option<String>, seed: Option<u64>) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let seed = device_seed(seed);
        let endpoint = bind(seed).await?;
        // Reachable (home relay picked) before the code is handed out.
        endpoint.online().await;
        let (events, rx) = mpsc::unbounded_channel();
        let (to_host, _) = mpsc::unbounded_channel();
        let shared = Shared::new(endpoint, seed, name, events, Role::Guest { to_host, code: String::new() });
        shared.become_host(true);
        if let Some(host) = shared.host_state() {
            if let Ok(mut h) = host.lock() {
                // A new room starts in its lobby: a guest coming back from an earlier room (same
                // code: it's this device's) joins it as it is, rather than carrying that room on.
                h.fresh = false;
                if let Some(room_name) = room_name.filter(|n| !clean_name(n).is_empty()) {
                    h.room_name = clean_name(&room_name);
                }
                h.announce();
            }
        }
        tracing::info!(addrs = ?shared.endpoint.addr().addrs, "watch: room open");
        tokio::spawn(accept_loop(shared.clone()));
        Ok((Room { shared }, rx))
    }

    /// Joins the room with this code, as this device (`seed`: the same one it hosts under, so
    /// it's the same member every time it comes back).
    pub async fn join(code: &str, name: String, seed: Option<u64>) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let addrs = parse_code(code)?;
        tracing::info!(host = %addrs[0].id.fmt_short(), "watch: joining room");
        let seed = device_seed(seed);
        let endpoint = bind(seed).await?;
        let (host_addr, conn) = match connect_any(&endpoint, addrs, JOIN_TIMEOUT).await {
            Some(found) => found,
            None => {
                tracing::warn!("watch: couldn't connect to the room");
                endpoint.close().await;
                return Err(Error::Network(
                    "Couldn't reach the room. Check the code, and that the host still has the room open.".into(),
                ));
            }
        };
        Ok(Self::joined(endpoint, seed, name, short_code_of(code), host_addr, conn, None).await)
    }

    /// Gets back into a room this device was in: tries each of its members' codes in turn (the
    /// host first) and joins whoever hosts it now. A room whose host left goes on with one of its
    /// guests hosting, so the old host's own code may no longer answer.
    ///
    /// `so_far` is where the room was when this device was last in it: a room that has just
    /// reopened carries on from there. `hosted`: this device was hosting it, so when nobody else
    /// is, it opens the room again itself (under its own code, where the others look for it).
    pub async fn rejoin(
        codes: &[String],
        name: String,
        seed: Option<u64>,
        so_far: Option<RoomSoFar>,
        hosted: bool,
    ) -> Result<(Room, mpsc::UnboundedReceiver<RoomEvent>)> {
        let seed = device_seed(seed);
        let endpoint = bind(seed).await?;
        // Everyone at once: whoever hosts now answers and keeps the connection; the others (gone,
        // or not hosting) fail or close it. Again, until REJOIN_TRYING: right after the host
        // left, nobody hosts yet until a guest takes over.
        let began = Instant::now();
        while began.elapsed() < REJOIN_TRYING {
            let mut tries = tokio::task::JoinSet::new();
            for code in codes {
                let Ok(addrs) = parse_code(code) else { continue };
                let (endpoint, code) = (endpoint.clone(), short_code_of(code));
                tries.spawn(async move {
                    match ask(&endpoint, addrs, ATTEMPT).await {
                        Reply::Hosting(addr, conn) => Some((code, addr, conn)),
                        _ => None,
                    }
                });
            }
            while let Some(done) = tries.join_next().await {
                if let Ok(Some((code, addr, conn))) = done {
                    tries.abort_all();
                    tracing::info!(host = %addr.id.fmt_short(), "watch: rejoined a room");
                    return Ok(Self::joined(endpoint, seed, name, code, addr, conn, so_far).await);
                }
            }
            // Nobody else hosts it: its host (this device) opens it again, where it was.
            if hosted {
                tracing::info!("watch: nobody else hosts the room; reopening it here");
                endpoint.online().await;
                let (events, rx) = mpsc::unbounded_channel();
                let (to_host, _) = mpsc::unbounded_channel();
                let shared = Shared::new(endpoint, seed, name, events, Role::Guest { to_host, code: String::new() });
                if let Some(r) = so_far {
                    shared.local.lock().unwrap_or_else(|p| p.into_inner()).carry(r);
                }
                shared.become_host(true);
                if let Some(state) = shared.local.lock().unwrap_or_else(|p| p.into_inner()).state.clone() {
                    shared.emit(RoomEvent::State(state));
                }
                tokio::spawn(accept_loop(shared.clone()));
                return Ok((Room { shared }, rx));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        endpoint.close().await;
        Err(Error::Network("That room has ended: nobody in it is hosting any more.".into()))
    }

    async fn joined(
        endpoint: Endpoint,
        seed: Seed,
        name: String,
        code: String,
        host_addr: EndpointAddr,
        conn: Connection,
        so_far: Option<RoomSoFar>,
    ) -> (Room, mpsc::UnboundedReceiver<RoomEvent>) {
        tracing::info!("watch: connected to the room");
        // Its own code goes in the hello (for taking over hosting): wait briefly for the relay.
        let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.online()).await;
        let (events, rx) = mpsc::unbounded_channel();
        let (to_host, outgoing) = mpsc::unbounded_channel();
        let shared = Shared::new(endpoint, seed, name, events, Role::Guest { to_host, code });
        // Where the room was: goes in the hello, for a room that has just reopened.
        if let Some(r) = so_far {
            shared.local.lock().unwrap_or_else(|p| p.into_inner()).carry(r);
        }
        tokio::spawn(accept_loop(shared.clone()));
        tokio::spawn(guest_loop(shared.clone(), host_addr, conn, outgoing));
        (Room { shared }, rx)
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

    /// This device's name in the room.
    pub fn set_name(&self, name: &str) {
        let name = clean_name(name);
        if name.is_empty() {
            return;
        }
        if let Ok(mut n) = self.shared.name.lock() {
            *n = name.clone();
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    h.name = name;
                    h.announce();
                }
            }
            None => self.to_host(Msg::Rename { name }),
        }
    }

    /// The room's name (the host's to change). An empty one goes back to "<host>'s room".
    pub fn set_room_name(&self, name: &str) -> bool {
        let Some(host) = self.shared.host_state() else { return false };
        let Ok(mut h) = host.lock() else { return false };
        let name = clean_name(name);
        h.room_name = if name.is_empty() { default_room_name(&h.name) } else { name };
        h.announce();
        true
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

    /// The host starts the room: everyone opens the chosen episode, held at the beginning until
    /// all have loaded it, then it plays for everyone at once. False when this device isn't the
    /// host or no episode is chosen yet.
    pub fn start(&self) -> bool {
        let Some(host) = self.shared.host_state() else { return false };
        let Ok(mut h) = host.lock() else { return false };
        let Some(mut state) = h.state.clone() else { return false };
        // Started, but held at the beginning: everyone's player opens and loads it, and the host's
        // plays it for everyone once they all have (as after a switch), so nobody starts behind.
        h.started = true;
        state.playing = false;
        state.held = true;
        state.position = 0.0;
        state.by = self.shared.me.to_string();
        let state = h.adopt(state, false);
        h.broadcast_state(&state, None);
        // The host's own player opens it through the same path as everyone else's.
        let _ = h.events.send(RoomEvent::State(state));
        h.announce();
        true
    }

    /// Where this device is, for the member list.
    pub fn send_status(&self, position: f64, buffering: bool, on: Option<(i64, f64)>, length: Option<f64>) {
        if let Ok(mut l) = self.shared.local.lock() {
            l.position = position;
            l.buffering = buffering;
        }
        match self.shared.host_state() {
            Some(host) => {
                if let Ok(mut h) = host.lock() {
                    let changed = h.buffering != buffering || h.on != on || h.length != length;
                    h.position = position;
                    h.buffering = buffering;
                    h.on = on;
                    h.length = length;
                    if changed {
                        h.announce();
                    } else if !h.guests.is_empty() {
                        h.announce_later();
                    }
                }
            }
            None => self.to_host(Msg::Status { position, buffering, media_id: on.map(|o| o.0), episode: on.map(|o| o.1), length }),
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
        } else {
            // Leaving, not lost: the room doesn't wait for this device.
            self.to_host(Msg::Bye);
            tokio::time::sleep(Duration::from_millis(300)).await;
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
                    let new_code = shared
                        .local
                        .lock()
                        .ok()
                        .and_then(|l| l.roster.as_ref()?.members.iter().find(|m| m.id == addr.id.to_string()).map(|m| m.code.clone()))
                        .unwrap_or_default();
                    if let Ok(mut role) = shared.role.lock() {
                        if let Role::Guest { code, .. } = &mut *role {
                            *code = new_code;
                        }
                    }
                    shared.emit(RoomEvent::Info(shared.info()));
                }
                host = addr;
                conn = c;
                shared.emit(RoomEvent::Reconnected);
            }
            // Took over hosting. If nobody comes back to it, the room is over: don't keep hosting
            // an empty room nobody knows about.
            Some(None) => {
                shared.emit(RoomEvent::Reconnected);
                // Alone and not playing for EMPTY_ROOM: over. Someone still watching keeps the room
                // (the others can rejoin it later).
                let shared = shared.clone();
                tokio::spawn(async move {
                    let mut empty_since: Option<Instant> = None;
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        let Some(host) = shared.host_state() else { return };
                        if shared.closing() {
                            return;
                        }
                        let (alone, playing) = host
                            .lock()
                            .map(|h| (h.guests.is_empty(), h.state.as_ref().is_some_and(|s| s.playing)))
                            .unwrap_or((false, false));
                        if !alone || playing {
                            empty_since = None;
                            continue;
                        }
                        if empty_since.get_or_insert_with(Instant::now).elapsed() >= EMPTY_ROOM {
                            tracing::info!("watch: nobody came back after the handover; ending the room");
                            shared.end("The room ended: nobody else came back.".into());
                            shared.endpoint.close().await;
                            return;
                        }
                    }
                });
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
        // Back after a lost connection: where the room was (a brand-new guest knows nothing yet).
        let room = l.state.clone().zip(l.roster.as_ref()).map(|(mut state, r)| {
            if l.position > 0.0 {
                state.position = l.position;
            }
            state.held = false;
            state.by = String::new();
            RoomSoFar { state, started: r.started, name: r.name.clone() }
        });
        Msg::Hello { name: shared.my_name(), code: shared.my_code(), ready: l.ready, check: l.check.clone(), room }
    };
    if write_msg(&mut send, &hello).await.is_err() {
        return Outcome::Dropped;
    }
    shared.linked.store(true, Ordering::SeqCst);
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
    let mut pinger = Pinger::default();
    let mut ping_tick = tokio::time::interval(PING_EVERY);
    let mut heard = Instant::now();
    let outcome = loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                // The host answers every ping: this long without a word, it's gone.
                if heard.elapsed() > GONE_AFTER {
                    tracing::info!("watch: the host went quiet; treating it as gone");
                    break Outcome::Dropped;
                }
                if let Some(ping) = pinger.next() {
                    if write_msg(&mut send, &ping).await.is_err() {
                        break Outcome::Dropped;
                    }
                }
            }
            out = outgoing.recv() => match out {
                Some(m) => {
                    if write_msg(&mut send, &m).await.is_err() {
                        break Outcome::Dropped;
                    }
                }
                None => break Outcome::Closing,
            },
            inc = incoming.recv() => match inc.inspect(|_| heard = Instant::now()) {
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
                Some(Msg::Pong { n }) => {
                    if let Some(ms) = pinger.answered(n) {
                        if write_msg(&mut send, &Msg::Latency { ms }).await.is_err() {
                            break Outcome::Dropped;
                        }
                    }
                }
                Some(_) => {}
                None => break Outcome::Dropped,
            },
        }
    };
    shared.linked.store(false, Ordering::SeqCst);
    reader.abort();
    conn.close(0u32.into(), b"bye");
    outcome
}

/// Gets back into the room after losing the host. The old host gets [`HOST_GRACE`] (none when it
/// handed the room over), then each guest in the order they joined: everyone works the line out
/// from the same roster, so they meet at the same new host.
///
/// Reaching its own turn, a guest asks the others before taking over (taking over on its own
/// word split rooms: a guest whose network dropped became the host of a room of its own):
/// - someone hosts already: it joins them;
/// - someone is still with the host (and the host didn't say it was leaving): this device was
///   the one cut off, and goes back to trying the host;
/// - someone before it in line answers: hosting is theirs, so it goes back to their turn (two
///   guests never both take over);
/// - someone confirms the host is gone for them too, or the host handed the room over: it hosts;
/// - nobody answers: it's offline (or alone), and keeps trying the host.
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
    // Whose turn: `first_turn` at `turns_from`, the next one every HOST_GRACE (0 is the old host).
    let mut turns_from = Instant::now();
    let mut first_turn = usize::from(handed_over);
    loop {
        if shared.closing() || began.elapsed() > REJOIN_FOR {
            return None;
        }
        let turn = first_turn + (turns_from.elapsed().as_secs() / HOST_GRACE.as_secs()) as usize;
        if turn == 0 {
            if let Reply::Hosting(addr, c) = ask(&shared.endpoint, vec![old_host.clone()], ATTEMPT).await {
                return Some(Some((addr, c)));
            }
        } else {
            match line.get(turn - 1) {
                // The end of the line (or no line: the roster never arrived): the old host again.
                None => {
                    (first_turn, turns_from) = (0, Instant::now());
                    continue;
                }
                Some((id, _)) if *id == me => {
                    let mine = turn - 1;
                    let others = line
                        .iter()
                        .enumerate()
                        .filter(|(_, (id, _))| *id != me)
                        .filter_map(|(n, (_, code))| parse_code(code).ok().map(|a| (n, a)))
                        .collect();
                    let (mut with_host, mut lost_host, mut before_me) = (false, false, None::<usize>);
                    for (n, reply) in ask_all(&shared.endpoint, others, ATTEMPT).await {
                        match reply {
                            Reply::Hosting(addr, c) => return Some(Some((addr, c))),
                            Reply::HasHost => with_host = true,
                            Reply::LostHost => lost_host = true,
                            Reply::Unreachable => continue,
                        }
                        if n < mine {
                            before_me = Some(before_me.map_or(n, |b| b.min(n)));
                        }
                    }
                    if with_host && !handed_over {
                        tracing::info!("watch: the others still have the host; this device was cut off, so it keeps trying the host");
                        first_turn = 0;
                    } else if let Some(n) = before_me {
                        tracing::info!("watch: a guest before this one in line is there; hosting is theirs");
                        first_turn = n + 1;
                    } else if lost_host || handed_over {
                        shared.become_host(false);
                        return Some(None);
                    } else {
                        tracing::info!("watch: nobody in the room answers (this device may be offline); not taking over");
                        first_turn = 0;
                    }
                    turns_from = Instant::now();
                    continue;
                }
                Some((_, code)) => {
                    if let Ok(addrs) = parse_code(code) {
                        if let Reply::Hosting(addr, c) = ask(&shared.endpoint, addrs, ATTEMPT).await {
                            return Some(Some((addr, c)));
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// A host whose guests all went quiet together may be the one that lost its connection: they
/// carry on meanwhile, with one of them hosting (which takes two of them: one alone never takes
/// over). While it has nobody connected, it looks for them hosting every few seconds, for
/// [`LOOK_FOR_ROOM`], and joins that room as a guest when it finds it.
async fn look_for_moved_room(shared: Arc<Shared>, host: Weak<Mutex<HostState>>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if shared.closing() {
            return;
        }
        let Some(host) = host.upgrade() else { return };
        // Still hosting this room?
        if !shared.host_state().is_some_and(|h| Arc::ptr_eq(&h, &host)) {
            return;
        }
        let together: Vec<(String, String)> = match host.lock() {
            Ok(mut h) if h.guests.is_empty() => {
                h.lost.retain(|l| l.2.elapsed() < LOOK_FOR_ROOM);
                // The ones that went with the last one to go.
                let last = h.lost.iter().map(|l| l.2).max();
                h.lost
                    .iter()
                    .filter(|l| last.is_some_and(|last| last.duration_since(l.2) <= LOST_TOGETHER))
                    .map(|l| (l.0.to_string(), l.1.clone()))
                    .collect()
            }
            _ => continue,
        };
        drop(host);
        if together.len() < 2 {
            continue;
        }
        let members = together.iter().enumerate().filter_map(|(n, (_, code))| parse_code(code).ok().map(|a| (n, a))).collect();
        for (n, reply) in ask_all(&shared.endpoint, members, ATTEMPT).await {
            if let Reply::Hosting(addr, conn) = reply {
                tracing::info!(host = %addr.id.fmt_short(), "watch: the room went on without this device; joining it");
                shared.become_guest(addr, conn, together[n].1.clone());
                return;
            }
        }
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
                let with_host = shared.linked.load(Ordering::SeqCst);
                connection.close(0u32.into(), if with_host { NOT_HOSTING_WITH_HOST } else { NOT_HOSTING_LOST_HOST });
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
    let Msg::Hello { name, code, ready, check, room: so_far } = read_msg(&mut recv).await? else {
        return Err(Error::Network("a guest didn't say hello".into()));
    };
    let name: String = name.trim().chars().take(32).collect();

    let (tx, mut outgoing) = mpsc::unbounded_channel::<Msg>();
    let admitted = {
        let mut h = host.lock().map_err(|_| Error::Network("room state poisoned".into()))?;
        // Rejoining: the old entry goes (its place in line too: it joins the end).
        h.guests.retain(|g| g.id != id);
        h.dropped.retain(|d| d.id != id);
        h.lost.retain(|l| l.0 != id);
        let room = h.guests.len() < MAX_GUESTS;
        // This room only just opened (its host came back and opened it again) and a guest comes
        // back from it: it carries on from where everyone was, not from the host's own player.
        if room && h.fresh {
            if let Some(so_far) = so_far.filter(|r| r.started) {
                tracing::info!(guest = %id.fmt_short(), "watch: a returning guest brought the room's place; carrying on from it");
                h.started = true;
                if !so_far.name.is_empty() {
                    h.room_name = so_far.name;
                }
                h.position = so_far.state.position;
                h.state = Some(so_far.state.clone());
                let _ = h.events.send(RoomEvent::State(so_far.state));
            }
        }
        if room {
            h.fresh = false;
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
                on: None,
                length: None,
                ready,
                check,
                tx,
                conn: connection.clone(),
                latency: None,
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
    // Said goodbye (left on purpose), rather than the connection being lost.
    let mut left = false;
    loop {
        let next = match tokio::time::timeout(GONE_AFTER, read_msg(&mut recv)).await {
            Ok(next) => next,
            Err(_) => {
                tracing::info!(guest = %id.fmt_short(), "watch: guest went quiet; treating it as gone");
                connection.close(0u32.into(), b"gone quiet");
                break;
            }
        };
        match next {
            Ok(Msg::State(mut asked)) => {
                if let Ok(mut h) = host.lock() {
                    let lobby = !h.started;
                    let wanted_play = asked.playing;
                    // Only the host moves the timeline: a guest's play or pause keeps the room's
                    // position (and the guest is told it, so its player jumps back).
                    let mut refused_seek = false;
                    if let Some(room) = &h.state {
                        let same = room.media_id == asked.media_id && room.episode == asked.episode && room.dub == asked.dub;
                        if same && (asked.position - h.position).abs() > GUEST_SEEK {
                            asked.position = h.position;
                            refused_seek = true;
                        }
                    }
                    let state = h.adopt(asked, true);
                    // A guest's play in the lobby, or a seek, is turned down: the guest hears the
                    // room's state too.
                    let except = if (wanted_play && !state.playing) || refused_seek { None } else { Some(&id) };
                    h.broadcast_state(&state, except);
                    let _ = h.events.send(RoomEvent::State(state));
                    if lobby {
                        h.announce();
                    }
                }
            }
            Ok(Msg::Status { position, buffering, media_id, episode, length }) => {
                if let Ok(mut h) = host.lock() {
                    let mut changed = false;
                    let on = media_id.zip(episode);
                    if let Some(g) = h.guest_mut(&id) {
                        changed = g.buffering != buffering || g.on != on || g.length != length;
                        g.position = position;
                        g.buffering = buffering;
                        g.on = on;
                        g.length = length;
                    }
                    // Loading starts and ends at once (waits depend on them); positions batch.
                    if changed {
                        h.announce();
                    } else {
                        h.announce_later();
                    }
                }
            }
            Ok(Msg::Rename { name }) => {
                let name = clean_name(&name);
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guest_mut(&id) {
                        if !name.is_empty() {
                            g.name = name;
                        }
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
            Ok(Msg::Ping { n }) => {
                if let Ok(h) = host.lock() {
                    if let Some(g) = h.guests.iter().find(|g| g.id == id) {
                        let _ = g.tx.send(Msg::Pong { n });
                    }
                }
            }
            Ok(Msg::Latency { ms }) => {
                if let Ok(mut h) = host.lock() {
                    if let Some(g) = h.guest_mut(&id) {
                        g.latency = Some(ms);
                    }
                    tracing::debug!(guest = %id.fmt_short(), ms, "watch: ping");
                    h.announce_later();
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
            Ok(Msg::Bye) => {
                left = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    writer.abort();
    let mut lost = None;
    if let Ok(mut h) = host.lock() {
        // Only this connection's entry: a rejoin may already have replaced it.
        let gone = h.guests.iter().position(|g| g.id == id && g.conn.stable_id() == connection.stable_id());
        if let Some(i) = gone {
            let g = h.guests.remove(i);
            if !left {
                h.lost.retain(|l| l.0 != id && l.2.elapsed() < LOOK_FOR_ROOM);
                h.lost.push((id, g.code.clone(), Instant::now()));
            }
            // Lost, not left, mid-episode: the room waits for them (the host's player pauses).
            if !left && h.started {
                let since = Instant::now();
                h.dropped.retain(|d| d.id != id);
                h.dropped.push(DroppedGuest { id, name: g.name, code: g.code, position: g.position, on: g.on, since });
                lost = Some(since);
                tracing::info!(guest = %id.fmt_short(), "watch: lost a guest's connection; waiting for it");
            }
            h.announce();
        }
    }
    // Not back in time: gone for good.
    if let Some(since) = lost {
        tokio::spawn(async move {
            tokio::time::sleep(REJOIN_FOR).await;
            if let Ok(mut h) = host.lock() {
                let before = h.dropped.len();
                h.dropped.retain(|d| !(d.id == id && d.since == since));
                if h.dropped.len() != before {
                    h.announce();
                }
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::TransportAddr;

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
            held: false,
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
            media_id: Some(21),
            episode: Some(5.0),
            length: Some(1420.0),
            connected: true,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""playable":true"#) && json.contains(r#""rttMs":30"#), "{json}");
    }



    #[test]
    fn room_codes_name_the_host_through_every_relay() {
        let seed = Seed(0x1F_2E3D_4C5B_6A79 & ((1 << 55) - 1));
        let code = short_code(seed);
        assert_eq!(code.len(), 13, "{code}"); // 11 characters in groups of four
        assert_eq!(code_for_seed(seed.0), code);
        let addrs = parse_code(&code).unwrap();
        assert_eq!(addrs.len(), RELAYS.len());
        assert!(addrs.iter().all(|a| a.id == seed.key().public()));
        assert!(addrs.iter().all(|a| matches!(a.addrs.iter().next(), Some(TransportAddr::Relay(_)))));
        // Typed sloppily: lower case, no dashes, O for 0 and I or L for 1.
        let sloppy = code.replace('-', "").to_lowercase().replace('0', "o").replace('1', "l");
        assert_eq!(parse_code(&sloppy).unwrap()[0].id, seed.key().public());
        assert_eq!(short_code_of(&sloppy.replace('o', "0").replace('l', "1")), code);
        // A kept seed gives the same code every time; new ones differ and stay within 55 bits.
        let (a, b) = (new_seed(), new_seed());
        assert!(a < 1 << 55 && code_for_seed(a) == code_for_seed(a) && code_for_seed(a) != code_for_seed(b));
        assert!(parse_code("not a code").is_err());
        assert!(parse_code("K7QM-4X2P-9FTE").is_err()); // one character too many
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
        PlayState { media_id: 1, episode: 2.0, title: String::new(), dub: false, position, playing, by: String::new(), source: None, subs: None, held: false }
    }

    /// Real network (iroh's relays): `cargo test -p banime-core -- --ignored live_room`.
    #[tokio::test]
    #[ignore]
    async fn live_room_syncs_state_between_two_endpoints() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let (guest, mut guest_events) = Room::join(&host.info().code, "Guest".into(), None).await.unwrap();
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
        // The guest's measured ping reaches the host's list (pings every 2 s).
        let ping = until(&mut host_events, |e| match e {
            RoomEvent::Roster(r) => r.members.iter().find(|m| !m.host).and_then(|m| m.rtt_ms),
            _ => None,
        })
        .await;
        assert!(ping < 5_000, "{ping} ms");
        guest.leave(false).await;
        host.leave(true).await;
    }

    /// Real network: a room its host has just opened again carries on from where a returning
    /// guest says everyone was, not from the host's own player.
    #[tokio::test]
    #[ignore]
    async fn live_room_reopened_carries_on_from_a_returning_guest() {
        // Rejoin on a room it hosted, with nobody else to find: the host reopens it.
        let (host, mut host_events) = Room::rejoin(&[], "Host".into(), None, None, true).await.unwrap();
        assert!(host.is_host());
        let so_far = RoomSoFar { state: play(120.0, true), started: true, name: "Movie night".into() };
        let (guest, _guest_events) =
            Room::rejoin(&[host.info().code], "Guest".into(), None, Some(so_far), false).await.unwrap();
        let s = until(&mut host_events, |e| match e {
            RoomEvent::State(s) => Some(s),
            _ => None,
        })
        .await;
        assert_eq!((s.position, s.playing), (120.0, true));
        until(&mut host_events, |e| match e {
            RoomEvent::Roster(r) => (r.started && r.name == "Movie night" && r.members.len() == 2).then_some(()),
            _ => None,
        })
        .await;
        guest.leave(false).await;
        host.leave(true).await;
    }

    /// Real network: a new room (Start a room) keeps its lobby when a guest comes back from an
    /// earlier room under the same code: it doesn't start playing that room's episode.
    #[tokio::test]
    #[ignore]
    async fn live_room_new_room_ignores_an_old_rooms_place() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let so_far = RoomSoFar { state: play(120.0, true), started: true, name: "Old room".into() };
        let (guest, _guest_events) =
            Room::rejoin(&[host.info().code], "Guest".into(), None, Some(so_far), false).await.unwrap();
        let roster = until(&mut host_events, |e| match e {
            RoomEvent::Roster(r) if r.members.len() == 2 => Some(r),
            RoomEvent::State(s) => panic!("the new room took the old room's state: {s:?}"),
            _ => None,
        })
        .await;
        assert!(!roster.started, "still in the lobby");
        assert_ne!(roster.name, "Old room");
        guest.leave(false).await;
        host.leave(true).await;
    }

    /// Real network: a host that closed the app finds its room again through its guests.
    #[tokio::test]
    #[ignore]
    async fn live_room_rejoin_finds_the_new_host() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let (guest, mut guest_events) = Room::join(&host.info().code, "Guest".into(), None).await.unwrap();
        let codes = until(&mut host_events, |e| match e {
            RoomEvent::Roster(r) if r.members.len() == 2 => Some(r.members.into_iter().map(|m| m.code).collect::<Vec<_>>()),
            _ => None,
        })
        .await;
        until(&mut guest_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        // The host goes (as when the app closes: it handed the room over).
        host.leave(false).await;
        until(&mut guest_events, |e| matches!(e, RoomEvent::Info(i) if i.host).then_some(())).await;
        // Back, with the codes it remembered: the old host's own no longer answers; the guest's does.
        let (back, mut back_events) = Room::rejoin(&codes, "Host again".into(), None, None, false).await.unwrap();
        assert!(!back.is_host());
        until(&mut back_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        back.leave(false).await;
        guest.leave(true).await;
    }

    /// Real network: the host hands over, the first guest hosts, and the second follows it.
    #[tokio::test]
    #[ignore]
    async fn live_room_survives_the_host_leaving() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let code = host.info().code;
        let (first, mut first_events) = Room::join(&code, "First".into(), None).await.unwrap();
        until(&mut host_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        let (second, mut second_events) = Room::join(&code, "Second".into(), None).await.unwrap();
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

    /// Real network: the host vanishes without a word (its app killed, its network gone). The
    /// guests agree it's gone for both of them; the first hosts, paused, and the second joins it.
    #[tokio::test]
    #[ignore]
    async fn live_room_host_vanishes_and_the_guests_agree() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let code = host.info().code;
        let (first, mut first_events) = Room::join(&code, "First".into(), None).await.unwrap();
        until(&mut host_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        let (second, mut second_events) = Room::join(&code, "Second".into(), None).await.unwrap();
        until(&mut second_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 3).then_some(())).await;
        until(&mut first_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 3).then_some(())).await;
        host.send_state(play(30.0, true));
        until(&mut second_events, |e| matches!(e, RoomEvent::State(s) if s.playing).then_some(())).await;

        // Gone without a handover.
        host.shared.closing.store(true, Ordering::SeqCst);
        host.shared.endpoint.close().await;
        until(&mut first_events, |e| match e {
            RoomEvent::Info(i) => i.host.then_some(()),
            RoomEvent::Closed(r) => panic!("{r}"),
            _ => None,
        })
        .await;
        // The second finds it, and the room is paused there until someone plays.
        let s = until(&mut second_events, |e| match e {
            RoomEvent::State(s) => Some(s),
            RoomEvent::Closed(r) => panic!("{r}"),
            _ => None,
        })
        .await;
        assert!(!s.playing, "taken over paused");
        assert!(!second.is_host());
        second.leave(false).await;
        first.leave(true).await;
    }

    /// Real network: a guest alone with a host that vanished can't tell whether the host left or
    /// its own network did, so it doesn't start a room of its own (that split rooms in two).
    #[tokio::test]
    #[ignore]
    async fn live_room_a_lone_guest_does_not_take_over() {
        let (host, mut host_events) = Room::host("Host".into(), None, None).await.unwrap();
        let (guest, mut guest_events) = Room::join(&host.info().code, "Guest".into(), None).await.unwrap();
        until(&mut host_events, |e| matches!(e, RoomEvent::Roster(r) if r.members.len() == 2).then_some(())).await;
        host.shared.closing.store(true, Ordering::SeqCst);
        host.shared.endpoint.close().await;
        // Its turn comes 10 s in; nobody confirms the host is gone, so it keeps looking instead.
        let took_over = tokio::time::timeout(Duration::from_secs(25), async {
            while let Some(e) = guest_events.recv().await {
                if matches!(e, RoomEvent::Info(ref i) if i.host) {
                    return true;
                }
            }
            false
        })
        .await;
        assert!(!matches!(took_over, Ok(true)), "a lone guest took over");
        assert!(!guest.is_host());
        guest.leave(false).await;
    }
}
