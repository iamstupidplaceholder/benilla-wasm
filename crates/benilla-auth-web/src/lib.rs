//! Browser-facing auth state machine. TCP/WebSocket stays outside this crate;
//! all WoW packet bytes and SRP math come from the native Benilla crates.

use std::io::Cursor;

use benilla_protocol::{auth, RealmInfo, CLIENT_BUILD};
use benilla_srp::{NormalizedString, PublicKey, SrpClient, SrpClientChallenge};
use serde_json::json;
use wasm_bindgen::prelude::*;

mod world;
pub use world::WorldEvent;
use world::WorldState;

/// A protocol/session failure suitable for both the Bevy adapter and the JS
/// probe wrapper. Authentication refusals retain their wire result so the
/// production login screen can select its authored error string.
#[derive(Debug)]
pub enum SessionError {
    InvalidPassword,
    AuthRejected(u8),
    WorldRejected(u8),
    Protocol(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPassword => f.write_str("Password must contain 1–16 printable ASCII characters (Latin letters, digits or symbols). Please check your keyboard layout."),
            Self::AuthRejected(code) => write!(f, "realmd rejected login: code 0x{code:02x}"),
            Self::WorldRejected(code) => {
                write!(f, "world server rejected login: code 0x{code:02x}")
            }
            Self::Protocol(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SessionError {}

#[wasm_bindgen]
pub struct AuthSession {
    locale: String,
    username: NormalizedString,
    password: Option<NormalizedString>,
    challenge: Option<SrpClientChallenge>,
    client: Option<SrpClient>,
    world: Option<WorldState>,
}

impl AuthSession {
    /// Rust-facing constructor used by the full Bevy client. The wasm-bindgen
    /// constructor below is only a thin compatibility wrapper for N2/N3.
    pub fn create(username: &str, password: &str) -> Result<Self, SessionError> {
        Ok(Self {
            locale: "enUS".into(),
            username: NormalizedString::new(username).map_err(protocol_error)?,
            password: Some(
                NormalizedString::new(password).map_err(|_| SessionError::InvalidPassword)?,
            ),
            challenge: None,
            client: None,
            world: None,
        })
    }

    pub fn challenge_bytes(&self) -> Result<Vec<u8>, SessionError> {
        let mut packet = Vec::new();
        auth::write_logon_challenge_locale(
            &mut packet,
            self.username.as_ref(),
            CLIENT_BUILD,
            &self.locale,
        )
        .map_err(protocol_error)?;
        Ok(packet)
    }

    pub fn set_locale(&mut self, locale: &str) {
        self.locale = locale.to_owned();
    }

    pub fn proof_bytes(&mut self, reply: &[u8]) -> Result<Vec<u8>, SessionError> {
        if self.challenge.is_some() || self.client.is_some() {
            return Err(SessionError::Protocol(
                "SRP challenge was already consumed".to_owned(),
            ));
        }
        let mut cursor = Cursor::new(reply);
        let server = auth::read_challenge_reply(&mut cursor).map_err(auth_error)?;
        require_consumed(&cursor, reply.len())?;
        let server_public_key =
            PublicKey::from_le_bytes(server.server_public_key).map_err(protocol_error)?;
        let password = self
            .password
            .take()
            .ok_or_else(|| SessionError::Protocol("password unavailable".to_owned()))?;
        let challenge = SrpClientChallenge::new(
            self.username.clone(),
            password,
            server.generator,
            server.large_safe_prime,
            server_public_key,
            server.salt,
        );
        let mut packet = Vec::new();
        auth::write_logon_proof(
            &mut packet,
            challenge.client_public_key(),
            challenge.client_proof(),
            &server.crc_salt,
        )
        .map_err(protocol_error)?;
        self.challenge = Some(challenge);
        Ok(packet)
    }

    pub fn finish_proof_bytes(&mut self, reply: &[u8]) -> Result<Vec<u8>, SessionError> {
        let mut cursor = Cursor::new(reply);
        let server_proof = auth::read_proof_reply(&mut cursor).map_err(auth_error)?;
        require_consumed(&cursor, reply.len())?;
        let challenge = self
            .challenge
            .take()
            .ok_or_else(|| SessionError::Protocol("no pending SRP proof".to_owned()))?;
        self.client = Some(
            challenge
                .verify_server_proof(server_proof)
                .map_err(protocol_error)?,
        );
        let mut packet = Vec::new();
        auth::write_realm_list_request(&mut packet).map_err(protocol_error)?;
        Ok(packet)
    }

    pub fn read_realms_typed(&self, reply: &[u8]) -> Result<Vec<RealmInfo>, SessionError> {
        if self.client.is_none() {
            return Err(SessionError::Protocol(
                "SRP proof was not verified".to_owned(),
            ));
        }
        let mut cursor = Cursor::new(reply);
        let realms = auth::read_realm_list(&mut cursor).map_err(auth_error)?;
        require_consumed(&cursor, reply.len())?;
        Ok(realms)
    }

    pub fn start_world_session(&mut self) -> Result<(), SessionError> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| SessionError::Protocol("SRP proof was not verified".to_owned()))?;
        self.world = Some(WorldState::new(
            self.username.clone(),
            *client.session_key(),
        ));
        Ok(())
    }

    pub fn feed_world_bytes(&mut self, bytes: &[u8]) -> Result<(), SessionError> {
        self.world_mut()?.feed(bytes)
    }

    pub fn take_world_outbound(&mut self) -> Vec<u8> {
        self.world
            .as_mut()
            .map(WorldState::next_outbound)
            .unwrap_or_default()
    }

    pub fn take_world_event(&mut self) -> Option<WorldEvent> {
        self.world.as_mut().and_then(WorldState::next_event)
    }

    pub fn enter_character_guid(&mut self, guid: u64) -> Result<(), SessionError> {
        self.world_mut()?.enter_character_guid(guid)
    }

    /// Queue one production client opcode/body on the encrypted world stream.
    pub fn queue_world_packet(&mut self, opcode: u16, body: &[u8]) -> Result<(), SessionError> {
        self.world_mut()?.queue_client_packet(opcode, body)
    }

    pub fn create_character(
        &mut self,
        request: benilla_protocol::CharCreateReq,
    ) -> Result<(), SessionError> {
        self.world_mut()?.create_character(&request)
    }

    pub fn delete_character(&mut self, guid: u64) -> Result<(), SessionError> {
        self.world_mut()?.delete_character(guid)
    }

    fn enter_character_index(&mut self, index: usize) -> Result<(), SessionError> {
        self.world_mut()?.enter_character_index(index)
    }

    fn world_mut(&mut self) -> Result<&mut WorldState, SessionError> {
        self.world
            .as_mut()
            .ok_or_else(|| SessionError::Protocol("world session not started".to_owned()))
    }
}

#[wasm_bindgen]
impl AuthSession {
    #[wasm_bindgen(constructor)]
    pub fn new(username: &str, password: &str) -> Result<AuthSession, JsValue> {
        Self::create(username, password).map_err(js_error)
    }

    pub fn challenge_packet(&self) -> Result<Vec<u8>, JsValue> {
        self.challenge_bytes().map_err(js_error)
    }

    pub fn proof_packet(&mut self, reply: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.proof_bytes(reply).map_err(js_error)
    }

    pub fn finish_proof(&mut self, reply: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.finish_proof_bytes(reply).map_err(js_error)
    }

    /// JSON is returned only for the public realm-list fields. The 40-byte
    /// session key remains inside this WASM instance and is never logged.
    pub fn read_realms(&self, reply: &[u8]) -> Result<String, JsValue> {
        let realms = self.read_realms_typed(reply).map_err(js_error)?;
        let public = realms
            .into_iter()
            .map(|realm| {
                json!({
                    "name": realm.name,
                    "id": realm.id,
                    "characters": realm.characters,
                    "flags": realm.flags,
                    "category": realm.category,
                    "type": realm.realm_type,
                    "population": realm.population,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&public).map_err(js_error)
    }

    pub fn start_world(&mut self) -> Result<(), JsValue> {
        self.start_world_session().map_err(js_error)
    }

    pub fn feed_world(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        self.feed_world_bytes(bytes).map_err(js_error)
    }

    /// Empty means no complete outbound packet is waiting. A real packet is
    /// always at least a six-byte client header.
    pub fn next_world_outbound(&mut self) -> Vec<u8> {
        self.take_world_outbound()
    }

    /// Empty means no event is waiting. Events are small JSON strings; no
    /// session key or proof bytes are included.
    pub fn next_world_event(&mut self) -> String {
        self.take_world_event()
            .map(|event| event.probe_json().to_string())
            .unwrap_or_default()
    }

    pub fn enter_character(&mut self, index: usize) -> Result<(), JsValue> {
        self.enter_character_index(index).map_err(js_error)
    }
}

fn require_consumed(cursor: &Cursor<&[u8]>, length: usize) -> Result<(), SessionError> {
    if cursor.position() as usize != length {
        return Err(SessionError::Protocol(
            "auth packet contains trailing bytes".to_owned(),
        ));
    }
    Ok(())
}

fn auth_error(error: anyhow::Error) -> SessionError {
    if let Some(reject) = error.downcast_ref::<auth::AuthReject>() {
        SessionError::AuthRejected(reject.code)
    } else {
        protocol_error(error)
    }
}

fn protocol_error(error: impl std::fmt::Display) -> SessionError {
    SessionError::Protocol(error.to_string())
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
