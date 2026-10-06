//! Incremental Vanilla world handshake/roster decoder over arbitrary WebSocket
//! byte chunks. The native protocol owns these exact packet bodies and cipher;
//! this module only replaces its blocking TCP framing with a bounded buffer.

use std::collections::VecDeque;

use benilla_protocol::{
    messages::{self, opcode, Character, ServerPacket},
    CharAction, CharCreateReq, SessionEvent, CLIENT_BUILD,
};
use benilla_srp::{
    vanilla_header::{HeaderCrypto, ProofSeed},
    NormalizedString,
};
use serde_json::{json, Value};

use crate::{protocol_error, SessionError};

const MAX_BUFFER: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Challenge,
    AuthResponse,
    Roster,
    Select,
    Action,
    Entering,
    InWorld,
}

/// Typed output of the incremental world codec. The full client consumes this
/// directly; `probe_json` preserves the original N3/N4 browser probe format.
#[derive(Debug, Clone)]
pub enum WorldEvent {
    Challenge,
    Queue {
        position: Option<u32>,
    },
    Authenticated,
    Roster {
        characters: Vec<Character>,
    },
    Entering {
        guid: u64,
    },
    CharacterActionResult {
        action: CharAction,
        code: u8,
    },
    CharacterLoginFailed {
        result: u8,
    },
    WorldEntered {
        map: u32,
        position: [f32; 3],
        orientation: f32,
    },
    /// One ordinary post-entry world packet, decoded through the same pure
    /// `ServerPacket -> SessionEvent` path as the native reader.
    SessionEvents {
        opcode: u16,
        events: Vec<SessionEvent>,
        tail: usize,
    },
    /// A malformed post-entry packet was dropped without losing stream
    /// alignment. This mirrors `WorldReader::poll` rather than terminating an
    /// otherwise healthy browser session on one unsupported packet layout.
    PacketSkipped {
        opcode: u16,
        reason: String,
    },
}

impl WorldEvent {
    pub fn probe_json(&self) -> Value {
        match self {
            Self::Challenge => json!({"kind":"challenge"}),
            Self::Queue { position } => json!({"kind":"queue","position":position}),
            Self::Authenticated => json!({"kind":"auth-ok"}),
            Self::Roster { characters } => {
                let public = characters
                    .iter()
                    .enumerate()
                    .map(|(index, character)| {
                        json!({
                            "index":index,
                            "name":character.name,
                            "race":character.race,
                            "class":character.class,
                            "level":character.level,
                            "map":character.map,
                            "zone":character.zone,
                        })
                    })
                    .collect::<Vec<_>>();
                json!({"kind":"roster","characters":public})
            }
            Self::Entering { guid } => json!({"kind":"entering","guid":guid.to_string()}),
            Self::CharacterActionResult { action, code } => json!({
                "kind":"character-action",
                "action":match action { CharAction::Create => "create", CharAction::Delete => "delete" },
                "code":code,
            }),
            Self::CharacterLoginFailed { result } => {
                json!({"kind":"character-login-failed","result":result})
            }
            Self::WorldEntered {
                map,
                position,
                orientation,
            } => json!({
                "kind":"world-entered",
                "map":map,
                "position":position,
                "orientation":orientation,
            }),
            Self::SessionEvents {
                opcode,
                events,
                tail,
            } => json!({
                "kind":"session-events",
                "opcode":opcode,
                "count":events.len(),
                "tail":tail,
            }),
            Self::PacketSkipped { opcode, reason } => json!({
                "kind":"packet-skipped",
                "opcode":opcode,
                "reason":reason,
            }),
        }
    }
}

pub(crate) struct WorldState {
    username: NormalizedString,
    session_key: [u8; 40],
    phase: Phase,
    crypto: Option<HeaderCrypto>,
    buffer: Vec<u8>,
    pending_header: Option<(u16, usize)>,
    outbound: VecDeque<Vec<u8>>,
    events: VecDeque<WorldEvent>,
    roster: Vec<Character>,
    pending_action: Option<(CharAction, u8)>,
}

impl WorldState {
    pub(crate) fn new(username: NormalizedString, session_key: [u8; 40]) -> Self {
        Self {
            username,
            session_key,
            phase: Phase::Challenge,
            crypto: None,
            buffer: Vec::new(),
            pending_header: None,
            outbound: VecDeque::new(),
            events: VecDeque::new(),
            roster: Vec::new(),
            pending_action: None,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<(), SessionError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFER {
            return Err(protocol_error("world receive buffer exceeds 1 MiB"));
        }
        self.buffer.extend_from_slice(bytes);
        loop {
            if self.pending_header.is_none() {
                if self.buffer.len() < 4 {
                    break;
                }
                let header: [u8; 4] = self.buffer[..4].try_into().expect("four bytes checked");
                self.buffer.drain(..4);
                let (size, opcode) = if self.phase == Phase::Challenge {
                    (
                        u16::from_be_bytes([header[0], header[1]]),
                        u16::from_le_bytes([header[2], header[3]]),
                    )
                } else {
                    let crypto = self
                        .crypto
                        .as_mut()
                        .ok_or_else(|| protocol_error("world cipher not initialized"))?;
                    let header = crypto.decrypter().decrypt_server_header(header);
                    (header.size, header.opcode)
                };
                if size < 2 {
                    return Err(protocol_error("invalid world packet length"));
                }
                self.pending_header = Some((opcode, usize::from(size - 2)));
            }
            let (opcode, body_length) = self.pending_header.expect("header set");
            if self.buffer.len() < body_length {
                break;
            }
            let body: Vec<u8> = self.buffer.drain(..body_length).collect();
            self.pending_header = None;
            self.handle_packet(opcode, &body)?;
        }
        Ok(())
    }

    pub(crate) fn next_outbound(&mut self) -> Vec<u8> {
        self.outbound.pop_front().unwrap_or_default()
    }

    pub(crate) fn next_event(&mut self) -> Option<WorldEvent> {
        self.events.pop_front()
    }

    pub(crate) fn enter_character_index(&mut self, index: usize) -> Result<(), SessionError> {
        if self.phase != Phase::Select {
            return Err(protocol_error("character list not ready"));
        }
        let guid = self
            .roster
            .get(index)
            .ok_or_else(|| protocol_error("invalid character index"))?
            .guid;
        self.enter_character_guid(guid)
    }

    pub(crate) fn enter_character_guid(&mut self, guid: u64) -> Result<(), SessionError> {
        if self.phase != Phase::Select {
            return Err(protocol_error("character list not ready"));
        }
        if !self.roster.iter().any(|character| character.guid == guid) {
            return Err(protocol_error("character is not in the current roster"));
        }
        let body = messages::full_guid(guid);
        let packet = self.client_packet(opcode::CMSG_PLAYER_LOGIN, &body, true)?;
        self.outbound.push_back(packet);
        // The shared app claims the mover once the server's self object is
        // admitted. Sending a STATUS_LOGGEDIN claim beside PLAYER_LOGIN can
        // race asynchronous login and be discarded before world admission.
        self.phase = Phase::Entering;
        self.event(WorldEvent::Entering { guid });
        Ok(())
    }

    /// Queue an ordinary encrypted client packet on the live world socket.
    ///
    /// Browser WebSockets replace only delivery; packet bodies and opcodes continue to be built by
    /// the production app/protocol layers.  This is the narrow transport seam used by the wasm IO
    /// backend once the login handshake has installed the header cipher.
    pub(crate) fn queue_client_packet(
        &mut self,
        opcode: u16,
        body: &[u8],
    ) -> Result<(), SessionError> {
        let packet = self.client_packet(opcode, body, true)?;
        self.outbound.push_back(packet);
        Ok(())
    }

    pub(crate) fn create_character(&mut self, request: &CharCreateReq) -> Result<(), SessionError> {
        self.begin_action(
            CharAction::Create,
            opcode::CMSG_CHAR_CREATE,
            &messages::char_create(request),
        )
    }

    pub(crate) fn delete_character(&mut self, guid: u64) -> Result<(), SessionError> {
        self.begin_action(
            CharAction::Delete,
            opcode::CMSG_CHAR_DELETE,
            &messages::full_guid(guid),
        )
    }

    fn begin_action(
        &mut self,
        action: CharAction,
        opcode: u16,
        body: &[u8],
    ) -> Result<(), SessionError> {
        if self.phase != Phase::Select {
            return Err(protocol_error("character list not ready"));
        }
        let packet = self.client_packet(opcode, body, true)?;
        self.outbound.push_back(packet);
        self.pending_action = Some((action, 0));
        self.phase = Phase::Action;
        Ok(())
    }

    fn handle_packet(&mut self, opcode: u16, body: &[u8]) -> Result<(), SessionError> {
        // Connection-level response, valid at character selection as well as in-world.
        if opcode == opcode::SMSG_PONG && self.phase != Phase::Challenge {
            let (packet, tail) =
                messages::parse_server_with_tail(opcode, body).map_err(protocol_error)?;
            self.event(WorldEvent::SessionEvents {
                opcode,
                events: benilla_protocol::decode(packet),
                tail,
            });
            return Ok(());
        }
        if opcode == opcode::SMSG_WARDEN_DATA {
            return Err(protocol_error(
                "world server requires unsupported Warden anticheat",
            ));
        }
        match self.phase {
            Phase::Challenge => {
                let ServerPacket::AuthChallenge { server_seed } =
                    messages::parse_server(opcode, body).map_err(protocol_error)?
                else {
                    return Err(protocol_error("expected SMSG_AUTH_CHALLENGE"));
                };
                let seed = ProofSeed::new();
                let client_seed = seed.seed();
                let (proof, crypto) =
                    seed.into_client_header_crypto(&self.username, self.session_key, server_seed);
                let body = messages::auth_session(
                    u32::from(CLIENT_BUILD),
                    self.username.as_ref(),
                    client_seed,
                    &proof,
                    &messages::STOCK_SECURE_ADDONS,
                );
                self.crypto = Some(crypto);
                let packet = self.client_packet(opcode::CMSG_AUTH_SESSION, &body, false)?;
                self.outbound.push_back(packet);
                self.phase = Phase::AuthResponse;
                self.event(WorldEvent::Challenge);
            }
            Phase::AuthResponse if opcode == opcode::SMSG_AUTH_RESPONSE => {
                let packet = messages::parse_server(opcode, body).map_err(protocol_error)?;
                if let ServerPacket::AuthResponse {
                    result,
                    queue_position,
                    ..
                } = packet
                {
                    if result == messages::AUTH_WAIT_QUEUE {
                        self.event(WorldEvent::Queue {
                            position: queue_position,
                        });
                    } else if result == messages::AUTH_OK {
                        let packet = self.client_packet(opcode::CMSG_CHAR_ENUM, &[], true)?;
                        self.outbound.push_back(packet);
                        self.phase = Phase::Roster;
                        self.event(WorldEvent::Authenticated);
                    } else {
                        return Err(SessionError::WorldRejected(result));
                    }
                }
            }
            Phase::Roster if opcode == opcode::SMSG_CHAR_ENUM => {
                let packet = messages::parse_server(opcode, body).map_err(protocol_error)?;
                if let ServerPacket::CharEnum { characters } = packet {
                    self.roster.clone_from(&characters);
                    self.phase = Phase::Select;
                    self.event(WorldEvent::Roster { characters });
                    if let Some((action, code)) = self.pending_action.take() {
                        self.event(WorldEvent::CharacterActionResult { action, code });
                    }
                }
            }
            Phase::Action
                if matches!(opcode, opcode::SMSG_CHAR_CREATE | opcode::SMSG_CHAR_DELETE) =>
            {
                let packet = messages::parse_server(opcode, body).map_err(protocol_error)?;
                let (action, result, success) = match packet {
                    ServerPacket::CharCreate { result } => (
                        CharAction::Create,
                        result,
                        result == messages::CHAR_CREATE_SUCCESS,
                    ),
                    ServerPacket::CharDelete { result } => (
                        CharAction::Delete,
                        result,
                        result == messages::CHAR_DELETE_SUCCESS,
                    ),
                    _ => return Ok(()),
                };
                let expected = self
                    .pending_action
                    .map(|(pending, _)| pending)
                    .ok_or_else(|| protocol_error("character action result without request"))?;
                if action != expected {
                    return Err(protocol_error("character action response kind mismatch"));
                }
                if success {
                    self.pending_action = Some((action, result));
                    let packet = self.client_packet(opcode::CMSG_CHAR_ENUM, &[], true)?;
                    self.outbound.push_back(packet);
                    self.phase = Phase::Roster;
                } else {
                    self.pending_action = None;
                    self.phase = Phase::Select;
                    self.event(WorldEvent::CharacterActionResult {
                        action,
                        code: result,
                    });
                }
            }
            // Loading known proficiency spells can send these masks before
            // LOGIN_VERIFY_WORLD. They are player state, not disposable account
            // chatter: the shared tooltip/equip consumer needs both masks.
            Phase::Entering if opcode == opcode::SMSG_SET_PROFICIENCY => {
                let (packet, tail) =
                    messages::parse_server_with_tail(opcode, body).map_err(protocol_error)?;
                self.event(WorldEvent::SessionEvents {
                    opcode,
                    events: benilla_protocol::decode(packet),
                    tail,
                });
            }
            Phase::Entering if opcode == opcode::SMSG_CHARACTER_LOGIN_FAILED => {
                let packet = messages::parse_server(opcode, body).map_err(protocol_error)?;
                if let ServerPacket::CharacterLoginFailed { result } = packet {
                    self.phase = Phase::Select;
                    self.event(WorldEvent::CharacterLoginFailed { result });
                }
            }
            Phase::Entering if opcode == opcode::SMSG_LOGIN_VERIFY_WORLD => {
                let packet = messages::parse_server(opcode, body).map_err(protocol_error)?;
                if let ServerPacket::LoginVerifyWorld {
                    map,
                    position,
                    orientation,
                } = packet
                {
                    self.phase = Phase::InWorld;
                    self.event(WorldEvent::WorldEntered {
                        map,
                        position: [position.x, position.y, position.z],
                        orientation,
                    });
                }
            }
            Phase::InWorld => match messages::parse_server_with_tail(opcode, body) {
                Ok((packet, tail)) => {
                    let events = benilla_protocol::decode(packet);
                    let logged_out = events
                        .iter()
                        .any(|event| matches!(event, benilla_protocol::SessionEvent::LoggedOut));
                    self.event(WorldEvent::SessionEvents {
                        opcode,
                        events,
                        tail,
                    });
                    if logged_out {
                        // The world socket remains authenticated after character logout.
                        // Re-enter the existing roster/select handshake, preserving crypto
                        // and delivering LoggedOut before the fresh character list.
                        self.roster.clear();
                        self.pending_action = None;
                        let packet = self.client_packet(opcode::CMSG_CHAR_ENUM, &[], true)?;
                        self.outbound.push_back(packet);
                        self.phase = Phase::Roster;
                    }
                }
                Err(error) => self.event(WorldEvent::PacketSkipped {
                    opcode,
                    reason: format!(
                        "opcode {opcode:#06x} ({}): {error}",
                        messages::opcode_name(opcode).unwrap_or("?")
                    ),
                }),
            },
            _ => {} // Account data, addon info and tutorial flags may precede world entry.
        }
        Ok(())
    }

    fn client_packet(
        &mut self,
        opcode: u16,
        body: &[u8],
        encrypted: bool,
    ) -> Result<Vec<u8>, SessionError> {
        let size = body
            .len()
            .checked_add(4)
            .ok_or_else(|| protocol_error("client packet overflow"))?;
        let size = u16::try_from(size).map_err(protocol_error)?;
        let mut header = [0u8; 6];
        header[..2].copy_from_slice(&size.to_be_bytes());
        header[2..].copy_from_slice(&u32::from(opcode).to_le_bytes());
        if encrypted {
            self.crypto
                .as_mut()
                .ok_or_else(|| protocol_error("world cipher not initialized"))?
                .encrypter()
                .encrypt(&mut header);
        }
        let mut packet = Vec::with_capacity(header.len() + body.len());
        packet.extend_from_slice(&header);
        packet.extend_from_slice(body);
        Ok(packet)
    }

    fn event(&mut self, event: WorldEvent) {
        self.events.push_back(event);
    }
}
