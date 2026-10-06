//! Browser IO backend for the production [`super::NetPlugin`].
//!
//! The protocol and SRP state live in `benilla-auth-web`, shared with the
//! already verified N2/N3 probe. This module owns only browser WebSockets and feeds the exact
//! [`SessionEvent`] channel consumed by the native ECS bridge. There is no browser-side shadow net
//! contract: login, object creation, movement and world state all cross the same drain as native.

use std::collections::{HashSet, VecDeque};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use benilla_auth_web::{AuthSession, SessionError, WorldEvent};
use benilla_protocol::{CharAction, LoginRefusal, LoginStage, RealmInfo, SessionEnd, SessionEvent};
use bevy::prelude::*;
use crossbeam_channel::{Receiver, Sender};
use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{BinaryType, CloseEvent, ErrorEvent, Event, MessageEvent, WebSocket};

const MAX_AUTH_BUFFER: usize = 64 * 1024;
const MAX_SRP_ATTEMPTS: u8 = 8;
// A WebSocket can stay OPEN while its upstream stops answering. Require a
// correlated game-server pong, not merely successful browser send/relay traffic.
const WORLD_PONG_TIMEOUT: benilla_time::Duration = benilla_time::Duration::from_secs(90);

use super::{CharRequest, ClientCommand, RealmRequest};

/// One submitted login attempt; identical to the native IO backend's parked request.
#[derive(Clone)]
pub(crate) struct LoginRequest {
    pub(crate) user: String,
    pub(crate) pass: String,
    pub(crate) host: String,
    pub(crate) generation: u64,
}

pub(crate) use super::ping::PingClock;
use super::ping::PING_INTERVAL;
use benilla_time::Instant;

pub(crate) struct NetConfig;

impl NetConfig {
    pub(crate) fn from_env() -> Self {
        Self
    }
}

pub(super) struct NetHandles {
    pub(super) events: Receiver<SessionEvent>,
    pub(super) commands: Sender<ClientCommand>,
    pub(super) pick: Sender<CharRequest>,
    pub(super) realm: Sender<RealmRequest>,
    pub(super) login: Sender<LoginRequest>,
    pub(super) login_abandon: Arc<AtomicU64>,
    pub(super) ping: Arc<std::sync::Mutex<PingClock>>,
}

/// Install the main-thread WebSocket pump and return the same channels as the native thread pair.
pub(super) fn spawn_net(app: &mut App, _cfg: NetConfig, connect: bool) -> NetHandles {
    let (events_tx, events_rx) = crossbeam_channel::unbounded();
    let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
    let (pick_tx, pick_rx) = crossbeam_channel::unbounded();
    let (realm_tx, realm_rx) = crossbeam_channel::unbounded();
    let (login_tx, login_rx) = crossbeam_channel::unbounded();
    let abandon = Arc::new(AtomicU64::new(0));
    let ping = Arc::new(std::sync::Mutex::new(PingClock::default()));

    if connect {
        app.insert_non_send_resource(WebNetState::new(
            login_rx,
            realm_rx,
            pick_rx,
            cmd_rx,
            abandon.clone(),
            events_tx,
            ping.clone(),
        ))
        .add_systems(
            Update,
            drive_web_net.in_set(benilla_world::schedule::WorldStage::Net),
        );
    }

    NetHandles {
        events: events_rx,
        commands: cmd_tx,
        pick: pick_tx,
        realm: realm_tx,
        login: login_tx,
        login_abandon: abandon,
        ping,
    }
}

/// The native runaway probe reads this from its socket thread. WebSocket delivery is main-thread
/// driven, so the age counter is not meaningful yet; zero/unknown is the honest adapter value.
pub(crate) fn inbound_census() -> (u64, Option<u64>) {
    (0, None)
}

#[derive(Clone)]
struct Credentials {
    user: String,
    pass: String,
    host: String,
    generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthStage {
    Challenge,
    Proof,
    Realms,
}

struct AuthFlow {
    socket: SocketBinding,
    session: Option<AuthSession>,
    credentials: Credentials,
    attempt: u8,
    stage: AuthStage,
    buffer: Vec<u8>,
}

struct RealmFlow {
    session: AuthSession,
    realms: Vec<RealmInfo>,
    generation: u64,
}

struct WorldFlow {
    socket: SocketBinding,
    session: AuthSession,
    realm: RealmInfo,
    generation: u64,
    roster_seen: bool,
    roster: Vec<benilla_protocol::Character>,
    chat_language: u32,
    ping: Arc<std::sync::Mutex<PingClock>>,
    next_ping: Option<Instant>,
    pending_pings: VecDeque<(u32, Instant)>,
}

enum Active {
    Idle,
    Auth(AuthFlow),
    Realm(RealmFlow),
    World(WorldFlow),
}

impl Active {
    fn socket_id(&self) -> Option<u64> {
        match self {
            Self::Auth(flow) => Some(flow.socket.id),
            Self::World(flow) => Some(flow.socket.id),
            Self::Idle | Self::Realm(_) => None,
        }
    }

    fn generation(&self) -> Option<u64> {
        match self {
            Self::Auth(flow) => Some(flow.credentials.generation),
            Self::Realm(flow) => Some(flow.generation),
            Self::World(flow) => Some(flow.generation),
            Self::Idle => None,
        }
    }
}

enum SocketSignal {
    Open { id: u64 },
    Binary { id: u64, bytes: Vec<u8> },
    Error { id: u64, reason: String },
    Closed { id: u64, code: u16, reason: String },
}

impl SocketSignal {
    fn id(&self) -> u64 {
        match self {
            Self::Open { id }
            | Self::Binary { id, .. }
            | Self::Error { id, .. }
            | Self::Closed { id, .. } => *id,
        }
    }
}

enum Output {
    Stage(LoginStage),
    Realms(Vec<RealmInfo>),
    Roster {
        characters: Vec<benilla_protocol::Character>,
        realm: RealmInfo,
    },
    Queue {
        position: Option<u32>,
        realm: String,
    },
    CharAction {
        action: CharAction,
        code: u8,
    },
    CharacterLoginFailed(u8),
    Connected {
        guid: u64,
        name: String,
    },
    EnteredWorld {
        map_id: u32,
        position: [f32; 3],
        orientation: f32,
    },
    SessionEvents {
        opcode: u16,
        events: Vec<SessionEvent>,
        tail: usize,
    },
    PacketSkipped {
        opcode: u16,
        reason: String,
    },
    LoginFailed(SessionError),
    Disconnected(String),
}

struct WebNetState {
    login_rx: Receiver<LoginRequest>,
    realm_rx: Receiver<RealmRequest>,
    char_rx: Receiver<CharRequest>,
    command_rx: Receiver<ClientCommand>,
    abandon: Arc<AtomicU64>,
    events_tx: Sender<SessionEvent>,
    socket_tx: Sender<SocketSignal>,
    socket_rx: Receiver<SocketSignal>,
    next_socket_id: u64,
    active: Active,
    output: VecDeque<Output>,
    ping: Arc<std::sync::Mutex<PingClock>>,
}

impl WebNetState {
    fn new(
        login_rx: Receiver<LoginRequest>,
        realm_rx: Receiver<RealmRequest>,
        char_rx: Receiver<CharRequest>,
        command_rx: Receiver<ClientCommand>,
        abandon: Arc<AtomicU64>,
        events_tx: Sender<SessionEvent>,
        ping: Arc<std::sync::Mutex<PingClock>>,
    ) -> Self {
        let (socket_tx, socket_rx) = crossbeam_channel::unbounded();
        Self {
            login_rx,
            realm_rx,
            char_rx,
            command_rx,
            abandon,
            events_tx,
            socket_tx,
            socket_rx,
            next_socket_id: 1,
            active: Active::Idle,
            output: VecDeque::new(),
            ping,
        }
    }

    fn tick(&mut self) {
        while let Ok(request) = self.login_rx.try_recv() {
            if request.generation != self.abandon.load(Ordering::SeqCst) {
                continue;
            }
            let credentials = Credentials {
                user: request.user,
                pass: request.pass,
                host: request.host,
                generation: request.generation,
            };
            self.output.push_back(Output::Stage(LoginStage::Connecting));
            self.restart_auth(credentials, 1);
        }

        if self
            .active
            .generation()
            .is_some_and(|generation| generation != self.abandon.load(Ordering::SeqCst))
        {
            self.active = Active::Idle;
        }

        while let Ok(signal) = self.socket_rx.try_recv() {
            if self.active.socket_id() == Some(signal.id()) {
                self.handle_socket_signal(signal);
            }
        }
        while let Ok(request) = self.realm_rx.try_recv() {
            self.handle_realm_request(request);
        }
        while let Ok(request) = self.char_rx.try_recv() {
            self.handle_char_request(request);
        }
        while let Ok(command) = self.command_rx.try_recv() {
            self.handle_client_command(command);
        }
        let result = if let Active::World(flow) = &mut self.active {
            send_keepalive(flow, &mut self.output)
        } else {
            Ok(())
        };
        if let Err(error) = result {
            self.output
                .push_back(Output::Disconnected(error.to_string()));
            self.active = Active::Idle;
        }
    }

    fn restart_auth(&mut self, credentials: Credentials, attempt: u8) {
        let result = (|| {
            let mut session = AuthSession::create(&credentials.user, &credentials.pass)?;
            session.set_locale(&crate::web_config::active_locale());
            let endpoint = crate::web_config::endpoint("auth");
            let socket = self.open_socket(&endpoint)?;
            Ok::<_, SessionError>(AuthFlow {
                socket,
                session: Some(session),
                credentials,
                attempt,
                stage: AuthStage::Challenge,
                buffer: Vec::new(),
            })
        })();
        match result {
            Ok(flow) => self.active = Active::Auth(flow),
            Err(error) => self.fail_login(error),
        }
    }

    fn start_world(&mut self, mut session: AuthSession, realm: RealmInfo, generation: u64) {
        let result = (|| {
            session.start_world_session()?;
            let endpoint = crate::web_config::endpoint("world");
            let socket = self.open_socket(&endpoint)?;
            Ok::<_, SessionError>(WorldFlow {
                socket,
                session,
                realm,
                generation,
                roster_seen: false,
                roster: Vec::new(),
                chat_language: benilla_protocol::messages::LANGUAGE_COMMON,
                ping: self.ping.clone(),
                next_ping: None,
                pending_pings: VecDeque::new(),
            })
        })();
        match result {
            Ok(flow) => {
                self.ping.lock().unwrap_or_else(|e| e.into_inner()).clear();
                self.output
                    .push_back(Output::Stage(LoginStage::Handshaking));
                self.active = Active::World(flow);
            }
            Err(error) => self.fail_login(error),
        }
    }

    fn open_socket(&mut self, endpoint: &str) -> Result<SocketBinding, SessionError> {
        let id = self.next_socket_id;
        self.next_socket_id = self.next_socket_id.wrapping_add(1);
        SocketBinding::open(id, endpoint, self.socket_tx.clone())
    }

    fn handle_socket_signal(&mut self, signal: SocketSignal) {
        let mut transition = None;
        let result = match &mut self.active {
            Active::Auth(flow) => handle_auth_signal(flow, signal, &mut self.output),
            Active::World(flow) => handle_world_signal(flow, signal, &mut self.output),
            Active::Idle | Active::Realm(_) => Ok(None),
        };
        match result {
            Ok(next) => transition = next,
            Err(error) => {
                let roster_seen = matches!(&self.active, Active::World(flow) if flow.roster_seen);
                if roster_seen {
                    self.output
                        .push_back(Output::Disconnected(error.to_string()));
                    self.active = Active::Idle;
                } else {
                    self.fail_login(error);
                }
            }
        }
        let Some(transition) = transition else {
            return;
        };
        match transition {
            Transition::RetryAuth {
                credentials,
                attempt,
            } => self.restart_auth(credentials, attempt),
            Transition::ChooseRealm {
                session,
                realms,
                generation,
            } => {
                self.output.push_back(Output::Realms(realms.clone()));
                if let Some(realm) = realms.first().cloned() {
                    // Start with the first realm; later UI choices use that realm's
                    // advertised realm metadata; the gateway owns the fixed TCP destination.
                    self.start_world(session, realm, generation);
                } else {
                    self.fail_login(protocol_error("realmd returned no realms"));
                }
            }
        }
    }

    fn handle_realm_request(&mut self, request: RealmRequest) {
        let active = std::mem::replace(&mut self.active, Active::Idle);
        let Active::Realm(flow) = active else {
            self.active = active;
            return;
        };
        match request {
            RealmRequest::Enter(name) => {
                if let Some(realm) = flow.realms.iter().find(|realm| realm.name == name).cloned() {
                    self.start_world(flow.session, realm, flow.generation);
                } else {
                    self.active = Active::Realm(flow);
                }
            }
            RealmRequest::Refresh => {
                self.output.push_back(Output::Realms(flow.realms.clone()));
                self.active = Active::Realm(flow);
            }
            RealmRequest::Abandon => {
                self.output.push_back(Output::LoginFailed(protocol_error(
                    "realm selection cancelled",
                )));
            }
        }
    }

    fn handle_char_request(&mut self, request: CharRequest) {
        if matches!(&request, CharRequest::Abandon) {
            self.active = Active::Idle;
            return;
        }
        let (result, roster_seen) = {
            let Active::World(flow) = &mut self.active else {
                return;
            };
            let result = match request {
                CharRequest::Enter(guid) => {
                    flow.chat_language = flow
                        .roster
                        .iter()
                        .find(|c| c.guid == guid)
                        .map_or(benilla_protocol::messages::LANGUAGE_COMMON, |c| {
                            benilla_protocol::messages::faction_language(c.race)
                        });
                    flow.session.enter_character_guid(guid)
                }
                CharRequest::Create(request) => flow.session.create_character(request),
                CharRequest::Delete(guid) => flow.session.delete_character(guid),
                CharRequest::Abandon => unreachable!("handled above"),
            };
            if result.is_ok() {
                drain_world(flow, &mut self.output);
            }
            (result, flow.roster_seen)
        };
        if let Err(error) = result {
            if roster_seen {
                self.output
                    .push_back(Output::Disconnected(error.to_string()));
                self.active = Active::Idle;
            } else {
                self.fail_login(error);
            }
        }
    }

    fn handle_client_command(&mut self, command: ClientCommand) {
        if let ClientCommand::Disconnect { reason } = command {
            self.active = Active::Idle; // Socket::drop closes the actual WebSocket.
            self.output.push_back(Output::Disconnected(reason));
            return;
        }
        use benilla_protocol::messages::{self, opcode};
        use benilla_protocol::world::movement::{
            client_uptime_ms, movement_info, movement_snapshot,
        };

        let packet = match command {
            ClientCommand::AutoStoreBagItem {
                src_bag,
                src_slot,
                dst_bag,
            } => Some((
                opcode::CMSG_AUTOSTORE_BAG_ITEM,
                messages::auto_store_bag_item(src_bag, src_slot, dst_bag),
            )),
            ClientCommand::SplitItem {
                src_bag,
                src_slot,
                dst_bag,
                dst_slot,
                count,
            } => Some((
                opcode::CMSG_SPLIT_ITEM,
                messages::split_item(src_bag, src_slot, dst_bag, dst_slot, count),
            )),
            ClientCommand::SetAmmo { entry } => {
                Some((opcode::CMSG_SET_AMMO, messages::set_ammo(entry)))
            }
            ClientCommand::MinimapPing { x, y } => {
                Some((opcode::MSG_MINIMAP_PING, messages::minimap_ping(x, y)))
            }
            ClientCommand::UseItem {
                bag_index,
                slot,
                spell_index,
                target,
            } => Some((
                opcode::CMSG_USE_ITEM,
                messages::use_item(bag_index, slot, spell_index, target),
            )),
            ClientCommand::AutoEquipItem { bag_index, slot } => Some((
                opcode::CMSG_AUTOEQUIP_ITEM,
                messages::auto_equip_item(bag_index, slot),
            )),
            ClientCommand::SwapInvItem { src_slot, dst_slot } => Some((
                opcode::CMSG_SWAP_INV_ITEM,
                messages::swap_inv_item(src_slot, dst_slot),
            )),
            ClientCommand::SwapItem {
                dst_bag,
                dst_slot,
                src_bag,
                src_slot,
            } => Some((
                opcode::CMSG_SWAP_ITEM,
                messages::swap_item(dst_bag, dst_slot, src_bag, src_slot),
            )),
            ClientCommand::SetSheathed { state } => {
                Some((opcode::CMSG_SETSHEATHED, messages::set_sheathed(state)))
            }
            ClientCommand::StandStateChange { state } => Some((
                opcode::CMSG_STANDSTATECHANGE,
                messages::stand_state_change(state),
            )),
            ClientCommand::TextEmote { text_id, target } => Some((
                opcode::CMSG_TEXT_EMOTE,
                messages::text_emote(text_id, target),
            )),
            ClientCommand::BuyItem {
                vendor,
                entry,
                count,
            } => Some((
                opcode::CMSG_BUY_ITEM,
                messages::buy_item(vendor, entry, count),
            )),
            ClientCommand::SellItem {
                vendor,
                item_guid,
                count,
            } => Some((
                opcode::CMSG_SELL_ITEM,
                messages::sell_item(vendor, item_guid, count),
            )),
            ClientCommand::BuybackItem { vendor, slot } => Some((
                opcode::CMSG_BUYBACK_ITEM,
                messages::buyback_item(vendor, slot),
            )),
            ClientCommand::BankerActivate { guid } => Some((
                opcode::CMSG_BANKER_ACTIVATE,
                messages::banker_activate(guid),
            )),
            ClientCommand::TrainerList { trainer } => {
                Some((opcode::CMSG_TRAINER_LIST, messages::trainer_list(trainer)))
            }
            ClientCommand::TrainerBuySpell { trainer, spell_id } => Some((
                opcode::CMSG_TRAINER_BUY_SPELL,
                messages::trainer_buy_spell(trainer, spell_id),
            )),
            ClientCommand::LearnTalent { talent_id, rank } => Some((
                opcode::CMSG_LEARN_TALENT,
                messages::learn_talent(talent_id, rank),
            )),
            ClientCommand::UnlearnSkill { skill_id } => Some((
                opcode::CMSG_UNLEARN_SKILL,
                messages::unlearn_skill(skill_id),
            )),
            ClientCommand::SetFactionAtWar {
                rep_list_id,
                at_war,
            } => Some((
                opcode::CMSG_SET_FACTION_ATWAR,
                messages::set_faction_at_war(rep_list_id, at_war),
            )),
            ClientCommand::SetFactionInactive {
                rep_list_id,
                inactive,
            } => Some((
                opcode::CMSG_SET_FACTION_INACTIVE,
                messages::set_faction_inactive(rep_list_id, inactive),
            )),
            ClientCommand::SetWatchedFaction { rep_list_id } => Some((
                opcode::CMSG_SET_WATCHED_FACTION,
                messages::set_watched_faction(rep_list_id),
            )),
            ClientCommand::Loot { guid } => Some((opcode::CMSG_LOOT, messages::loot(guid))),
            ClientCommand::AutostoreLootItem { slot } => Some((
                opcode::CMSG_AUTOSTORE_LOOT_ITEM,
                messages::autostore_loot_item(slot),
            )),
            ClientCommand::LootMoney => Some((opcode::CMSG_LOOT_MONEY, messages::loot_money())),
            ClientCommand::LootRelease { guid } => {
                Some((opcode::CMSG_LOOT_RELEASE, messages::loot_release(guid)))
            }
            ClientCommand::LootRoll {
                looted_target,
                item_slot,
                roll_type,
            } => Some((
                opcode::CMSG_LOOT_ROLL,
                messages::loot_roll(looted_target, item_slot, roll_type),
            )),
            ClientCommand::PushQuestToParty { quest } => Some((
                opcode::CMSG_PUSHQUESTTOPARTY,
                messages::push_quest_to_party(quest),
            )),
            ClientCommand::QuestConfirmAccept { quest } => Some((
                opcode::CMSG_QUEST_CONFIRM_ACCEPT,
                messages::quest_confirm_accept(quest),
            )),
            ClientCommand::QuestPushResult { sharer, msg } => Some((
                opcode::MSG_QUEST_PUSH_RESULT,
                messages::quest_push_result(sharer, msg),
            )),
            ClientCommand::ItemTextQuery { text_id, mail_id } => Some((
                opcode::CMSG_ITEM_TEXT_QUERY,
                messages::item_text_query(text_id, mail_id),
            )),
            ClientCommand::Inspect { target } => {
                Some((opcode::CMSG_INSPECT, messages::full_guid(target)))
            }
            ClientCommand::CompleteCinematic => Some((opcode::CMSG_COMPLETE_CINEMATIC, Vec::new())),
            ClientCommand::RequestRaidInfo => Some((
                opcode::CMSG_REQUEST_RAID_INFO,
                messages::request_raid_info(),
            )),
            ClientCommand::TogglePvp => Some((opcode::CMSG_TOGGLE_PVP, Vec::new())),
            ClientCommand::PlayedTime => Some((opcode::CMSG_PLAYED_TIME, messages::played_time())),
            ClientCommand::SetSelection { guid } => {
                Some((opcode::CMSG_SET_SELECTION, messages::full_guid(guid)))
            }
            ClientCommand::AttackSwing { guid } => {
                Some((opcode::CMSG_ATTACKSWING, messages::attack_swing(guid)))
            }
            ClientCommand::AttackStop => Some((opcode::CMSG_ATTACKSTOP, Vec::new())),
            ClientCommand::CancelAutoRepeat => {
                Some((opcode::CMSG_CANCEL_AUTO_REPEAT_SPELL, Vec::new()))
            }
            ClientCommand::CancelCast { spell_id } => {
                Some((opcode::CMSG_CANCEL_CAST, spell_id.to_le_bytes().to_vec()))
            }
            ClientCommand::CancelChannelling { spell_id } => Some((
                opcode::CMSG_CANCEL_CHANNELLING,
                spell_id.to_le_bytes().to_vec(),
            )),
            ClientCommand::CancelAura { spell_id } => {
                Some((opcode::CMSG_CANCEL_AURA, messages::cancel_aura(spell_id)))
            }
            ClientCommand::CastSpell { spell_id, target } => Some((
                opcode::CMSG_CAST_SPELL,
                messages::cast_spell(spell_id, target),
            )),
            ClientCommand::CastSpellGameObject { spell_id, go_guid } => Some((
                opcode::CMSG_CAST_SPELL,
                messages::cast_spell_gameobject(spell_id, go_guid),
            )),
            ClientCommand::CastSpellItem {
                spell_id,
                item_guid,
            } => Some((
                opcode::CMSG_CAST_SPELL,
                messages::cast_spell_item(spell_id, item_guid),
            )),
            ClientCommand::CastSpellAtDest { spell_id, dest } => Some((
                opcode::CMSG_CAST_SPELL,
                messages::cast_spell_at_dest(spell_id, dest),
            )),
            ClientCommand::CastSpellAtSource { spell_id, src } => Some((
                opcode::CMSG_CAST_SPELL,
                messages::cast_spell_at_source(spell_id, src),
            )),
            ClientCommand::SetActionButton { button, packed } => Some((
                opcode::CMSG_SET_ACTION_BUTTON,
                messages::set_action_button(button, packed),
            )),
            ClientCommand::GossipHello { guid } => {
                Some((opcode::CMSG_GOSSIP_HELLO, messages::gossip_hello(guid)))
            }
            ClientCommand::GossipSelectOption { guid, option } => Some((
                opcode::CMSG_GOSSIP_SELECT_OPTION,
                messages::gossip_select_option(guid, option, None),
            )),
            ClientCommand::NpcTextQuery { text_id, guid } => Some((
                opcode::CMSG_NPC_TEXT_QUERY,
                messages::npc_text_query(text_id, guid),
            )),
            ClientCommand::ListInventory { guid } => {
                Some((opcode::CMSG_LIST_INVENTORY, messages::list_inventory(guid)))
            }
            ClientCommand::QuestQuery { quest } => {
                Some((opcode::CMSG_QUEST_QUERY, messages::quest_query(quest)))
            }
            ClientCommand::QuestgiverHello { npc } => Some((
                opcode::CMSG_QUESTGIVER_HELLO,
                messages::questgiver_hello(npc),
            )),
            ClientCommand::QuestgiverStatusQuery { npc } => Some((
                opcode::CMSG_QUESTGIVER_STATUS_QUERY,
                messages::questgiver_status_query(npc),
            )),
            ClientCommand::QuestgiverQuery { npc, quest } => Some((
                opcode::CMSG_QUESTGIVER_QUERY_QUEST,
                messages::questgiver_query_quest(npc, quest),
            )),
            ClientCommand::QuestgiverAccept { npc, quest } => Some((
                opcode::CMSG_QUESTGIVER_ACCEPT_QUEST,
                messages::questgiver_accept_quest(npc, quest),
            )),
            ClientCommand::QuestgiverComplete { npc, quest } => Some((
                opcode::CMSG_QUESTGIVER_COMPLETE_QUEST,
                messages::questgiver_complete_quest(npc, quest),
            )),
            ClientCommand::QuestgiverRequestReward { npc, quest } => Some((
                opcode::CMSG_QUESTGIVER_REQUEST_REWARD,
                messages::questgiver_request_reward(npc, quest),
            )),
            ClientCommand::QuestgiverChooseReward { npc, quest, choice } => Some((
                opcode::CMSG_QUESTGIVER_CHOOSE_REWARD,
                messages::questgiver_choose_reward(npc, quest, choice),
            )),
            ClientCommand::QuestlogRemove { slot } => Some((
                opcode::CMSG_QUESTLOG_REMOVE_QUEST,
                messages::questlog_remove_quest(slot),
            )),
            ClientCommand::JoinChannel { name, password } => Some((
                opcode::CMSG_JOIN_CHANNEL,
                messages::join_channel(&name, &password),
            )),
            ClientCommand::LeaveChannel { name } => {
                Some((opcode::CMSG_LEAVE_CHANNEL, messages::leave_channel(&name)))
            }
            ClientCommand::AddonMessage { distribution, text } => Some((
                opcode::CMSG_MESSAGECHAT,
                messages::messagechat(
                    super::addon_wire_chat_type(distribution),
                    messages::LANGUAGE_ADDON,
                    &text,
                ),
            )),
            ClientCommand::Chat { kind, target, text } => {
                use super::ChatKind;
                let Active::World(flow) = &self.active else {
                    return;
                };
                let language = flow.chat_language;
                let body = match kind {
                    ChatKind::Whisper => messages::messagechat_whisper(
                        language,
                        target.as_deref().unwrap_or_default(),
                        &text,
                    ),
                    ChatKind::Channel => messages::messagechat_channel(
                        language,
                        target.as_deref().unwrap_or_default(),
                        &text,
                    ),
                    other => {
                        let kind = match other {
                            ChatKind::Say => messages::CHAT_TYPE_SAY,
                            ChatKind::Yell => messages::CHAT_TYPE_YELL,
                            ChatKind::Emote => messages::CHAT_TYPE_EMOTE,
                            ChatKind::Party => messages::CHAT_TYPE_PARTY,
                            ChatKind::Raid => messages::CHAT_TYPE_RAID,
                            ChatKind::RaidLeader => messages::CHAT_TYPE_RAID_LEADER,
                            ChatKind::RaidWarning => messages::CHAT_TYPE_RAID_WARNING,
                            ChatKind::Guild => messages::CHAT_TYPE_GUILD,
                            ChatKind::Officer => messages::CHAT_TYPE_OFFICER,
                            ChatKind::Battleground => messages::CHAT_TYPE_BATTLEGROUND,
                            ChatKind::BattlegroundLeader => messages::CHAT_TYPE_BATTLEGROUND_LEADER,
                            ChatKind::Afk => messages::CHAT_TYPE_AFK,
                            ChatKind::Dnd => messages::CHAT_TYPE_DND,
                            ChatKind::Whisper | ChatKind::Channel => unreachable!("handled above"),
                        };
                        messages::messagechat(kind, language, &text)
                    }
                };
                Some((opcode::CMSG_MESSAGECHAT, body))
            }
            ClientCommand::ItemQuery { entry, guid } => Some((
                opcode::CMSG_ITEM_QUERY_SINGLE,
                messages::item_query(entry, guid),
            )),
            ClientCommand::NameQuery { guid } => {
                Some((opcode::CMSG_NAME_QUERY, messages::full_guid(guid)))
            }
            ClientCommand::CreatureQuery { entry, guid } => Some((
                opcode::CMSG_CREATURE_QUERY,
                messages::creature_query(entry, guid),
            )),
            ClientCommand::PetNameQuery { pet_number, guid } => Some((
                opcode::CMSG_PET_NAME_QUERY,
                messages::pet_name_query(pet_number, guid),
            )),
            ClientCommand::GameObjectQuery { entry, guid } => Some((
                opcode::CMSG_GAMEOBJECT_QUERY,
                messages::gameobject_query(entry, guid),
            )),
            ClientCommand::GameObjUse { guid } => {
                Some((opcode::CMSG_GAMEOBJ_USE, messages::gameobj_use(guid)))
            }
            // The production writer's lifecycle packets have empty bodies.
            // Only encrypted WebSocket transport differs from WorldWriter's TCP send.
            ClientCommand::Logout => Some((opcode::CMSG_LOGOUT_REQUEST, Vec::new())),
            ClientCommand::LogoutCancel => Some((opcode::CMSG_LOGOUT_CANCEL, Vec::new())),
            ClientCommand::ForceLogout => Some((opcode::CMSG_PLAYER_LOGOUT, Vec::new())),
            ClientCommand::QueryTime => Some((opcode::CMSG_QUERY_TIME, messages::query_time())),
            ClientCommand::WorldportAck => Some((opcode::MSG_MOVE_WORLDPORT_ACK, Vec::new())),
            ClientCommand::RepopRequest => Some((opcode::CMSG_REPOP_REQUEST, Vec::new())),
            ClientCommand::ChannelList { name } => {
                Some((opcode::CMSG_CHANNEL_LIST, messages::channel_list(&name)))
            }
            ClientCommand::RandomRoll { min, max } => {
                Some((opcode::MSG_RANDOM_ROLL, messages::random_roll(min, max)))
            }
            ClientCommand::ChannelOwner { name } => {
                Some((opcode::CMSG_CHANNEL_OWNER, messages::channel_owner(&name)))
            }
            ClientCommand::ChannelSetOwner { name, player } => Some((
                opcode::CMSG_CHANNEL_SET_OWNER,
                messages::channel_set_owner(&name, &player),
            )),
            ClientCommand::ChannelPassword { name, password } => Some((
                opcode::CMSG_CHANNEL_PASSWORD,
                messages::channel_password(&name, &password),
            )),
            ClientCommand::ChannelModerator { name, player } => Some((
                opcode::CMSG_CHANNEL_MODERATOR,
                messages::channel_moderator(&name, &player),
            )),
            ClientCommand::ChannelUnmoderator { name, player } => Some((
                opcode::CMSG_CHANNEL_UNMODERATOR,
                messages::channel_unmoderator(&name, &player),
            )),
            ClientCommand::ChannelMute { name, player } => Some((
                opcode::CMSG_CHANNEL_MUTE,
                messages::channel_mute(&name, &player),
            )),
            ClientCommand::ChannelUnmute { name, player } => Some((
                opcode::CMSG_CHANNEL_UNMUTE,
                messages::channel_unmute(&name, &player),
            )),
            ClientCommand::ChannelInvite { name, player } => Some((
                opcode::CMSG_CHANNEL_INVITE,
                messages::channel_invite(&name, &player),
            )),
            ClientCommand::ChannelKick { name, player } => Some((
                opcode::CMSG_CHANNEL_KICK,
                messages::channel_kick(&name, &player),
            )),
            ClientCommand::ChannelBan { name, player } => Some((
                opcode::CMSG_CHANNEL_BAN,
                messages::channel_ban(&name, &player),
            )),
            ClientCommand::ChannelUnban { name, player } => Some((
                opcode::CMSG_CHANNEL_UNBAN,
                messages::channel_unban(&name, &player),
            )),
            ClientCommand::ChannelAnnouncements { name } => Some((
                opcode::CMSG_CHANNEL_ANNOUNCEMENTS,
                messages::channel_announcements(&name),
            )),
            ClientCommand::ChannelModerate { name } => Some((
                opcode::CMSG_CHANNEL_MODERATE,
                messages::channel_moderate(&name),
            )),
            ClientCommand::OpenItem { bag_index, slot } => {
                Some((opcode::CMSG_OPEN_ITEM, messages::open_item(bag_index, slot)))
            }
            ClientCommand::WrapItem {
                gift_bag,
                gift_slot,
                item_bag,
                item_slot,
            } => Some((
                opcode::CMSG_WRAP_ITEM,
                messages::wrap_item(gift_bag, gift_slot, item_bag, item_slot),
            )),
            ClientCommand::DestroyItem {
                bag_index,
                slot,
                count,
            } => Some((
                opcode::CMSG_DESTROYITEM,
                messages::destroy_item(bag_index, slot, count),
            )),
            ClientCommand::SetActionBarToggles { toggles } => Some((
                opcode::CMSG_SET_ACTIONBAR_TOGGLES,
                messages::set_actionbar_toggles(toggles),
            )),
            ClientCommand::PetAction {
                pet_guid,
                packed,
                target_guid,
            } => Some((
                opcode::CMSG_PET_ACTION,
                messages::pet_action(pet_guid, packed, target_guid),
            )),
            ClientCommand::PetSetAction { pet_guid, entries } => Some((
                opcode::CMSG_PET_SET_ACTION,
                messages::pet_set_action(pet_guid, &entries),
            )),
            ClientCommand::PetStopAttack { pet_guid } => Some((
                opcode::CMSG_PET_STOP_ATTACK,
                messages::pet_stop_attack(pet_guid),
            )),
            ClientCommand::PetCancelAura { pet_guid, spell_id } => Some((
                opcode::CMSG_PET_CANCEL_AURA,
                messages::pet_cancel_aura(pet_guid, spell_id),
            )),
            ClientCommand::PetSpellAutocast {
                pet_guid,
                spell_id,
                enabled,
            } => Some((
                opcode::CMSG_PET_SPELL_AUTOCAST,
                messages::pet_spell_autocast(pet_guid, spell_id, enabled),
            )),
            ClientCommand::PetAbandon { pet_guid } => {
                Some((opcode::CMSG_PET_ABANDON, messages::pet_abandon(pet_guid)))
            }
            ClientCommand::PetRename { pet_guid, name } => Some((
                opcode::CMSG_PET_RENAME,
                messages::pet_rename(pet_guid, &name),
            )),
            ClientCommand::MountSpecial => Some((opcode::CMSG_MOUNTSPECIAL_ANIM, Vec::new())),
            ClientCommand::BuyItemInSlot {
                vendor,
                entry,
                bag_guid,
                bag_slot,
                count,
            } => Some((
                opcode::CMSG_BUY_ITEM_IN_SLOT,
                messages::buy_item_in_slot(vendor, entry, bag_guid, bag_slot, count),
            )),
            ClientCommand::RepairItem { vendor, item_guid } => Some((
                opcode::CMSG_REPAIR_ITEM,
                messages::repair_item(vendor, item_guid),
            )),
            ClientCommand::SummonResponse { summoner } => Some((
                opcode::CMSG_SUMMON_RESPONSE,
                messages::summon_response(summoner),
            )),
            ClientCommand::TalentWipeConfirm { trainer } => Some((
                opcode::MSG_TALENT_WIPE_CONFIRM,
                messages::talent_wipe_confirm(trainer),
            )),
            ClientCommand::BattlefieldPort { map_id, accept } => Some((
                opcode::CMSG_BATTLEFIELD_PORT,
                messages::battlefield_port(map_id, accept),
            )),
            ClientCommand::RequestBattlefieldScoreData => {
                Some((opcode::MSG_PVP_LOG_DATA, Vec::new()))
            }
            ClientCommand::LeaveBattlefield { map_id } => Some((
                opcode::CMSG_LEAVE_BATTLEFIELD,
                messages::leave_battlefield(map_id),
            )),
            ClientCommand::MeetingStoneJoin { go_guid } => Some((
                opcode::CMSG_MEETINGSTONE_JOIN,
                messages::meeting_stone_join(go_guid),
            )),
            ClientCommand::MeetingStoneLeave => Some((
                opcode::CMSG_MEETINGSTONE_LEAVE,
                messages::meeting_stone_leave(),
            )),
            ClientCommand::TutorialFlag { id } => {
                Some((opcode::CMSG_TUTORIAL_FLAG, messages::tutorial_flag(id)))
            }
            ClientCommand::TutorialClear => Some((opcode::CMSG_TUTORIAL_CLEAR, Vec::new())),
            ClientCommand::TutorialReset => Some((opcode::CMSG_TUTORIAL_RESET, Vec::new())),
            ClientCommand::BattlefieldList { map_id } => Some((
                opcode::CMSG_BATTLEFIELD_LIST,
                messages::battlefield_list(map_id),
            )),
            ClientCommand::RequestBattlefieldPositions => {
                Some((opcode::MSG_BATTLEGROUND_PLAYER_POSITIONS, Vec::new()))
            }
            ClientCommand::TabardVendorActivate { npc } => Some((
                opcode::MSG_TABARDVENDOR_ACTIVATE,
                messages::tabard_vendor_activate(npc),
            )),
            ClientCommand::SaveGuildEmblem { vendor, design } => Some((
                opcode::MSG_SAVE_GUILD_EMBLEM,
                messages::save_guild_emblem(vendor, design),
            )),
            ClientCommand::BattlemasterHello { npc } => Some((
                opcode::CMSG_BATTLEMASTER_HELLO,
                messages::battlemaster_hello(npc),
            )),
            ClientCommand::BattlemasterJoin {
                battlemaster,
                map_id,
                instance_id,
                as_group,
            } => Some((
                opcode::CMSG_BATTLEMASTER_JOIN,
                messages::battlemaster_join(battlemaster, map_id, instance_id, as_group),
            )),
            ClientCommand::BattlefieldJoin {
                map_id,
                instance_id,
                as_group,
            } => Some((
                opcode::CMSG_BATTLEFIELD_JOIN,
                messages::battlefield_join(map_id, instance_id, as_group),
            )),
            ClientCommand::BattlefieldStatusRequest => {
                Some((opcode::CMSG_BATTLEFIELD_STATUS, Vec::new()))
            }
            ClientCommand::PetUnlearn { trainer } => {
                Some((opcode::CMSG_PET_UNLEARN, messages::pet_unlearn(trainer)))
            }
            ClientCommand::BuyBankSlot { guid } => {
                Some((opcode::CMSG_BUY_BANK_SLOT, messages::buy_bank_slot(guid)))
            }
            ClientCommand::AutoBankItem { bag, slot } => Some((
                opcode::CMSG_AUTOBANK_ITEM,
                messages::autobank_item(bag, slot),
            )),
            ClientCommand::AutoStoreBankItem { bag, slot } => Some((
                opcode::CMSG_AUTOSTORE_BANK_ITEM,
                messages::autostore_bank_item(bag, slot),
            )),
            ClientCommand::ListStabledPets { npc } => Some((
                opcode::MSG_LIST_STABLED_PETS,
                messages::list_stabled_pets(npc),
            )),
            ClientCommand::StablePet { npc } => {
                Some((opcode::CMSG_STABLE_PET, messages::stable_pet(npc)))
            }
            ClientCommand::UnstablePet { npc, pet_number } => Some((
                opcode::CMSG_UNSTABLE_PET,
                messages::unstable_pet(npc, pet_number),
            )),
            ClientCommand::StableSwapPet { npc, pet_number } => Some((
                opcode::CMSG_STABLE_SWAP_PET,
                messages::stable_swap_pet(npc, pet_number),
            )),
            ClientCommand::BuyStableSlot { npc } => {
                Some((opcode::CMSG_BUY_STABLE_SLOT, messages::buy_stable_slot(npc)))
            }
            ClientCommand::PageTextQuery { page_id, guid } => Some((
                opcode::CMSG_PAGE_TEXT_QUERY,
                messages::page_text_query(page_id, guid),
            )),
            ClientCommand::LootMasterGive { guid, slot, target } => Some((
                opcode::CMSG_LOOT_MASTER_GIVE,
                messages::loot_master_give(guid, slot, target),
            )),
            ClientCommand::GetMailList { mailbox } => {
                Some((opcode::CMSG_GET_MAIL_LIST, messages::get_mail_list(mailbox)))
            }
            ClientCommand::SendMail {
                mailbox,
                receiver,
                subject,
                body,
                stationery,
                item_guid,
                money,
                cod,
            } => Some((
                opcode::CMSG_SEND_MAIL,
                messages::send_mail(
                    mailbox, &receiver, &subject, &body, stationery, 0, item_guid, money, cod,
                ),
            )),
            ClientCommand::MailTakeMoney { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_TAKE_MONEY,
                messages::mail_take_money(mailbox, mail_id),
            )),
            ClientCommand::MailTakeItem { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_TAKE_ITEM,
                messages::mail_take_item(mailbox, mail_id),
            )),
            ClientCommand::MailMarkAsRead { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_MARK_AS_READ,
                messages::mail_mark_as_read(mailbox, mail_id),
            )),
            ClientCommand::MailReturnToSender { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_RETURN_TO_SENDER,
                messages::mail_return_to_sender(mailbox, mail_id),
            )),
            ClientCommand::MailDelete { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_DELETE,
                messages::mail_delete(mailbox, mail_id),
            )),
            ClientCommand::MailCreateTextItem { mailbox, mail_id } => Some((
                opcode::CMSG_MAIL_CREATE_TEXT_ITEM,
                messages::mail_create_text_item(mailbox, mail_id),
            )),
            ClientCommand::QueryNextMailTime => {
                Some((opcode::MSG_QUERY_NEXT_MAIL_TIME, Vec::new()))
            }
            ClientCommand::AuctionHello { auctioneer } => Some((
                opcode::MSG_AUCTION_HELLO,
                messages::auction_hello(auctioneer),
            )),
            ClientCommand::AuctionListItems {
                auctioneer,
                list_from,
                searched_name,
                level_min,
                level_max,
                slot_id,
                main_category,
                sub_category,
                quality,
                usable,
            } => Some((
                opcode::CMSG_AUCTION_LIST_ITEMS,
                messages::auction_list_items(
                    auctioneer,
                    list_from,
                    &searched_name,
                    level_min,
                    level_max,
                    slot_id,
                    main_category,
                    sub_category,
                    quality,
                    usable,
                ),
            )),
            ClientCommand::AuctionListOwnerItems {
                auctioneer,
                list_from,
            } => Some((
                opcode::CMSG_AUCTION_LIST_OWNER_ITEMS,
                messages::auction_list_owner_items(auctioneer, list_from),
            )),
            ClientCommand::AuctionListBidderItems {
                auctioneer,
                list_from,
                auction_ids,
            } => Some((
                opcode::CMSG_AUCTION_LIST_BIDDER_ITEMS,
                messages::auction_list_bidder_items(auctioneer, list_from, &auction_ids),
            )),
            ClientCommand::AuctionSellItem {
                auctioneer,
                item_guid,
                bid,
                buyout,
                etime_minutes,
            } => Some((
                opcode::CMSG_AUCTION_SELL_ITEM,
                messages::auction_sell_item(auctioneer, item_guid, bid, buyout, etime_minutes),
            )),
            ClientCommand::AuctionPlaceBid {
                auctioneer,
                auction_id,
                price,
            } => Some((
                opcode::CMSG_AUCTION_PLACE_BID,
                messages::auction_place_bid(auctioneer, auction_id, price),
            )),
            ClientCommand::AuctionRemoveItem {
                auctioneer,
                auction_id,
            } => Some((
                opcode::CMSG_AUCTION_REMOVE_ITEM,
                messages::auction_remove_item(auctioneer, auction_id),
            )),
            ClientCommand::InspectHonorStats { target } => Some((
                opcode::MSG_INSPECT_HONOR_STATS,
                messages::inspect_honor_stats(target),
            )),
            ClientCommand::InitiateTrade { target } => Some((
                opcode::CMSG_INITIATE_TRADE,
                messages::initiate_trade(target),
            )),
            ClientCommand::BeginTrade => Some((opcode::CMSG_BEGIN_TRADE, Vec::new())),
            ClientCommand::BusyTrade => Some((opcode::CMSG_BUSY_TRADE, Vec::new())),
            ClientCommand::IgnoreTrade => Some((opcode::CMSG_IGNORE_TRADE, Vec::new())),
            ClientCommand::AcceptTrade => {
                Some((opcode::CMSG_ACCEPT_TRADE, messages::accept_trade()))
            }
            ClientCommand::UnacceptTrade => Some((opcode::CMSG_UNACCEPT_TRADE, Vec::new())),
            ClientCommand::CancelTrade => Some((opcode::CMSG_CANCEL_TRADE, Vec::new())),
            ClientCommand::SetTradeGold { copper } => Some((
                opcode::CMSG_SET_TRADE_GOLD,
                messages::set_trade_gold(copper),
            )),
            ClientCommand::SetTradeItem {
                trade_slot,
                bag,
                slot,
            } => Some((
                opcode::CMSG_SET_TRADE_ITEM,
                messages::set_trade_item(trade_slot, bag, slot),
            )),
            ClientCommand::ClearTradeItem { trade_slot } => Some((
                opcode::CMSG_CLEAR_TRADE_ITEM,
                messages::clear_trade_item(trade_slot),
            )),
            ClientCommand::NextCinematicCamera => {
                Some((opcode::CMSG_NEXT_CINEMATIC_CAMERA, Vec::new()))
            }
            ClientCommand::GroupInvite { name } => {
                Some((opcode::CMSG_GROUP_INVITE, messages::group_invite(&name)))
            }
            ClientCommand::GroupAccept => {
                Some((opcode::CMSG_GROUP_ACCEPT, messages::group_accept()))
            }
            ClientCommand::GroupDecline => {
                Some((opcode::CMSG_GROUP_DECLINE, messages::group_decline()))
            }
            ClientCommand::GroupUninvite { name } => {
                Some((opcode::CMSG_GROUP_UNINVITE, messages::group_uninvite(&name)))
            }
            ClientCommand::GroupSetLeader { guid } => Some((
                opcode::CMSG_GROUP_SET_LEADER,
                messages::group_set_leader(guid),
            )),
            ClientCommand::GroupLeave => {
                Some((opcode::CMSG_GROUP_DISBAND, messages::group_disband()))
            }
            ClientCommand::GroupRaidConvert => Some((
                opcode::CMSG_GROUP_RAID_CONVERT,
                messages::group_raid_convert(),
            )),
            ClientCommand::RequestPartyMemberStats { guid } => Some((
                opcode::CMSG_REQUEST_PARTY_MEMBER_STATS,
                messages::request_party_member_stats(guid),
            )),
            ClientCommand::LootMethod {
                method,
                master,
                threshold,
            } => Some((
                opcode::CMSG_LOOT_METHOD,
                messages::loot_method(method, master, threshold),
            )),
            ClientCommand::SetRaidTarget { icon, guid } => Some((
                opcode::MSG_RAID_TARGET_UPDATE,
                messages::raid_target_set(icon, guid),
            )),
            ClientCommand::GroupChangeSubGroup { name, group } => Some((
                opcode::CMSG_GROUP_CHANGE_SUB_GROUP,
                messages::group_change_sub_group(&name, group),
            )),
            ClientCommand::GroupSwapSubGroup { name, other } => Some((
                opcode::CMSG_GROUP_SWAP_SUB_GROUP,
                messages::group_swap_sub_group(&name, &other),
            )),
            ClientCommand::GroupAssistantLeader { guid, grant } => Some((
                opcode::CMSG_GROUP_ASSISTANT_LEADER,
                messages::group_assistant_leader(guid, grant),
            )),
            ClientCommand::ReadyCheckStart => {
                Some((opcode::MSG_RAID_READY_CHECK, messages::ready_check_start()))
            }
            ClientCommand::ReadyCheckAnswer { ready } => Some((
                opcode::MSG_RAID_READY_CHECK,
                messages::ready_check_answer(ready),
            )),
            ClientCommand::ResetInstances => {
                Some((opcode::CMSG_RESET_INSTANCES, messages::reset_instances()))
            }
            ClientCommand::DuelAccepted { arbiter } => {
                Some((opcode::CMSG_DUEL_ACCEPTED, messages::duel_accepted(arbiter)))
            }
            ClientCommand::DuelCancelled { arbiter } => Some((
                opcode::CMSG_DUEL_CANCELLED,
                messages::duel_cancelled(arbiter),
            )),
            ClientCommand::ToggleHelm => Some((opcode::CMSG_TOGGLE_HELM, Vec::new())),
            ClientCommand::ToggleCloak => Some((opcode::CMSG_TOGGLE_CLOAK, Vec::new())),
            ClientCommand::FriendListRequest => {
                Some((opcode::CMSG_FRIEND_LIST, messages::friend_list()))
            }
            ClientCommand::AddFriend { name } => {
                Some((opcode::CMSG_ADD_FRIEND, messages::add_friend(&name)))
            }
            ClientCommand::SetLookingForGroup { slots, comment } => Some((
                opcode::CMSG_SET_LOOKING_FOR_GROUP,
                messages::set_looking_for_group(slots, &comment),
            )),
            ClientCommand::DelFriend { guid } => {
                Some((opcode::CMSG_DEL_FRIEND, messages::del_friend(guid)))
            }
            ClientCommand::AddIgnore { name } => {
                Some((opcode::CMSG_ADD_IGNORE, messages::add_ignore(&name)))
            }
            ClientCommand::DelIgnore { guid } => {
                Some((opcode::CMSG_DEL_IGNORE, messages::del_ignore(guid)))
            }
            ClientCommand::Who { request } => Some((opcode::CMSG_WHO, messages::who(&request))),
            ClientCommand::ChatIgnored { guid } => {
                Some((opcode::CMSG_CHAT_IGNORED, messages::full_guid(guid)))
            }
            ClientCommand::GuildQuery { guild_id } => {
                Some((opcode::CMSG_GUILD_QUERY, messages::guild_query(guild_id)))
            }
            ClientCommand::GuildCreate { name } => {
                Some((opcode::CMSG_GUILD_CREATE, messages::guild_create(&name)))
            }
            ClientCommand::GuildInvite { name } => {
                Some((opcode::CMSG_GUILD_INVITE, messages::guild_invite(&name)))
            }
            ClientCommand::GuildAccept => {
                Some((opcode::CMSG_GUILD_ACCEPT, messages::guild_accept()))
            }
            ClientCommand::GuildDecline => {
                Some((opcode::CMSG_GUILD_DECLINE, messages::guild_decline()))
            }
            ClientCommand::GuildInfoRequest => {
                Some((opcode::CMSG_GUILD_INFO, messages::guild_info()))
            }
            ClientCommand::GuildRosterRequest => {
                Some((opcode::CMSG_GUILD_ROSTER, messages::guild_roster()))
            }
            ClientCommand::GuildPromote { name } => {
                Some((opcode::CMSG_GUILD_PROMOTE, messages::guild_promote(&name)))
            }
            ClientCommand::GuildDemote { name } => {
                Some((opcode::CMSG_GUILD_DEMOTE, messages::guild_demote(&name)))
            }
            ClientCommand::GuildLeave => Some((opcode::CMSG_GUILD_LEAVE, messages::guild_leave())),
            ClientCommand::GuildRemove { name } => {
                Some((opcode::CMSG_GUILD_REMOVE, messages::guild_remove(&name)))
            }
            ClientCommand::GuildDisband => {
                Some((opcode::CMSG_GUILD_DISBAND, messages::guild_disband()))
            }
            ClientCommand::GuildLeader { name } => {
                Some((opcode::CMSG_GUILD_LEADER, messages::guild_leader(&name)))
            }
            ClientCommand::GuildMotd { motd } => {
                Some((opcode::CMSG_GUILD_MOTD, messages::guild_motd(&motd)))
            }
            ClientCommand::GuildRank {
                rank_id,
                rights,
                name,
            } => Some((
                opcode::CMSG_GUILD_RANK,
                messages::guild_rank(rank_id, rights, &name),
            )),
            ClientCommand::GuildAddRank { name } => {
                Some((opcode::CMSG_GUILD_ADD_RANK, messages::guild_add_rank(&name)))
            }
            ClientCommand::GuildDelRank => {
                Some((opcode::CMSG_GUILD_DEL_RANK, messages::guild_del_rank()))
            }
            ClientCommand::GuildSetPublicNote { name, note } => Some((
                opcode::CMSG_GUILD_SET_PUBLIC_NOTE,
                messages::guild_set_public_note(&name, &note),
            )),
            ClientCommand::GuildSetOfficerNote { name, note } => Some((
                opcode::CMSG_GUILD_SET_OFFICER_NOTE,
                messages::guild_set_officer_note(&name, &note),
            )),
            ClientCommand::GuildInfoText { text } => Some((
                opcode::CMSG_GUILD_INFO_TEXT,
                messages::guild_info_text(&text),
            )),
            ClientCommand::PetitionShowList { npc } => Some((
                opcode::CMSG_PETITION_SHOWLIST,
                messages::petition_show_list(npc),
            )),
            ClientCommand::PetitionBuy { npc, name } => Some((
                opcode::CMSG_PETITION_BUY,
                messages::petition_buy(npc, &name),
            )),
            ClientCommand::PetitionShowSignatures { item } => Some((
                opcode::CMSG_PETITION_SHOW_SIGNATURES,
                messages::petition_show_signatures(item),
            )),
            ClientCommand::PetitionSign { item, byte } => Some((
                opcode::CMSG_PETITION_SIGN,
                messages::petition_sign(item, byte),
            )),
            ClientCommand::OfferPetition { item, player } => Some((
                opcode::CMSG_OFFER_PETITION,
                messages::offer_petition(item, player),
            )),
            ClientCommand::TurnInPetition { item } => Some((
                opcode::CMSG_TURN_IN_PETITION,
                messages::turn_in_petition(item),
            )),
            ClientCommand::PetitionQuery { petition_id, item } => Some((
                opcode::CMSG_PETITION_QUERY,
                messages::petition_query(petition_id, item),
            )),
            ClientCommand::PetitionRename { item, name } => Some((
                opcode::MSG_PETITION_RENAME,
                messages::petition_rename(item, &name),
            )),
            ClientCommand::PetitionDecline { item } => Some((
                opcode::MSG_PETITION_DECLINE,
                messages::petition_decline(item),
            )),
            ClientCommand::MeetingStoneStatusQuery => {
                Some((opcode::CMSG_MEETINGSTONE_STATUS_QUERY, Vec::new()))
            }
            ClientCommand::GmTicketGet => Some((opcode::CMSG_GMTICKET_GETTICKET, Vec::new())),
            ClientCommand::GmTicketDelete => Some((opcode::CMSG_GMTICKET_DELETETICKET, Vec::new())),
            ClientCommand::GmTicketSystemStatus => {
                Some((opcode::CMSG_GMTICKET_SYSTEMSTATUS, Vec::new()))
            }
            ClientCommand::GmTicketCreate {
                category,
                map,
                pos,
                text,
            } => Some((
                opcode::CMSG_GMTICKET_CREATE,
                messages::gm_ticket_create(category, map, pos, &text),
            )),
            ClientCommand::GmTicketUpdate { category, text } => Some((
                opcode::CMSG_GMTICKET_UPDATETEXT,
                messages::gm_ticket_updatetext(category, &text),
            )),
            ClientCommand::BinderActivate { binder } => Some((
                opcode::CMSG_BINDER_ACTIVATE,
                messages::binder_activate(binder),
            )),
            ClientCommand::CorpseQuery => Some((opcode::MSG_CORPSE_QUERY, Vec::new())),
            ClientCommand::ReclaimCorpse { corpse } => Some((
                opcode::CMSG_RECLAIM_CORPSE,
                messages::reclaim_corpse(corpse),
            )),
            ClientCommand::SelfRes => Some((opcode::CMSG_SELF_RES, Vec::new())),
            ClientCommand::SpiritHealerActivate { npc } => Some((
                opcode::CMSG_SPIRIT_HEALER_ACTIVATE,
                messages::spirit_healer_activate(npc),
            )),
            ClientCommand::ResurrectResponse { caster, accept } => Some((
                opcode::CMSG_RESURRECT_RESPONSE,
                messages::resurrect_response(caster, accept),
            )),
            ClientCommand::AreaSpiritHealerQuery { healer } => Some((
                opcode::CMSG_AREA_SPIRIT_HEALER_QUERY,
                messages::area_spirit_healer(healer),
            )),
            ClientCommand::AreaSpiritHealerQueue { healer } => Some((
                opcode::CMSG_AREA_SPIRIT_HEALER_QUEUE,
                messages::area_spirit_healer(healer),
            )),
            ClientCommand::AreaTrigger { trigger_id } => {
                Some((opcode::CMSG_AREATRIGGER, messages::area_trigger(trigger_id)))
            }
            ClientCommand::TaxiNodeStatusQuery { guid } => Some((
                opcode::CMSG_TAXINODE_STATUS_QUERY,
                messages::taxi_node_status_query(guid),
            )),
            ClientCommand::TaxiQueryNodes { guid } => Some((
                opcode::CMSG_TAXIQUERYAVAILABLENODES,
                messages::taxi_query_available_nodes(guid),
            )),
            ClientCommand::ActivateTaxi {
                guid,
                source_node,
                dest_node,
            } => Some((
                opcode::CMSG_ACTIVATETAXI,
                messages::activate_taxi(guid, source_node, dest_node),
            )),
            ClientCommand::ActivateTaxiExpress {
                guid,
                total_cost,
                nodes,
            } => Some((
                opcode::CMSG_ACTIVATETAXIEXPRESS,
                messages::activate_taxi_express(guid, total_cost, &nodes),
            )),
            ClientCommand::SetActiveMover { guid } => {
                Some((opcode::CMSG_SET_ACTIVE_MOVER, messages::full_guid(guid)))
            }
            ClientCommand::Move {
                kind,
                flags,
                pos,
                orientation,
                pitch,
                fall_time,
                jump,
                transport,
            } => {
                let info =
                    movement_snapshot(pos, orientation, flags, pitch, fall_time, jump, transport);
                Some((kind.opcode(), messages::movement(&info)))
            }
            ClientCommand::ForceSpeedAck {
                kind,
                guid,
                counter,
                speed,
                flags,
                pos,
                orientation,
                pitch,
                fall_time,
                jump,
                transport,
            } => {
                let info =
                    movement_snapshot(pos, orientation, flags, pitch, fall_time, jump, transport);
                Some((
                    kind.ack_opcode(),
                    messages::force_speed_ack(guid, counter, &info, speed),
                ))
            }
            ClientCommand::TeleportAck { guid, counter } => Some((
                opcode::MSG_MOVE_TELEPORT_ACK,
                messages::teleport_ack(guid, counter, client_uptime_ms()),
            )),
            ClientCommand::MoveTimeSkipped { guid, lag_ms } => Some((
                opcode::CMSG_MOVE_TIME_SKIPPED,
                messages::move_time_skipped(guid, lag_ms),
            )),
            ClientCommand::MoveSplineDone {
                flags,
                pos,
                orientation,
                spline_id,
            } => Some((
                opcode::CMSG_MOVE_SPLINE_DONE,
                messages::move_spline_done(&movement_info(pos, orientation, flags), spline_id),
            )),
            ClientCommand::NotActiveMover {
                guid,
                flags,
                pos,
                orientation,
                fall_time,
            } => {
                let mut info = movement_info(pos, orientation, flags);
                info.fall_time = fall_time;
                let mut body = messages::full_guid(guid);
                body.extend_from_slice(&messages::movement(&info));
                Some((opcode::CMSG_MOVE_NOT_ACTIVE_MOVER, body))
            }
            ClientCommand::FarSight { engage } => {
                Some((opcode::CMSG_FAR_SIGHT, vec![u8::from(engage)]))
            }
            ClientCommand::MoveModeAck {
                guid,
                counter,
                mode,
                apply,
                flags,
                pos,
                orientation,
            } => Some((
                mode.ack_opcode(apply),
                messages::move_flag_ack(
                    guid,
                    counter,
                    &movement_info(pos, orientation, flags),
                    mode.ack_carries_apply().then_some(apply),
                ),
            )),
            ClientCommand::KnockBackAck {
                guid,
                counter,
                launch,
                flags,
                pos,
                orientation,
                transport,
            } => Some((
                opcode::CMSG_MOVE_KNOCK_BACK_ACK,
                messages::knock_back_ack(
                    guid,
                    counter,
                    &movement_snapshot(pos, orientation, flags, 0.0, 0, Some(launch), transport),
                ),
            )),
            // Dispatched above; explicit arm keeps this match exhaustive so
            // newly added production commands cannot silently go unported.
            ClientCommand::Disconnect { .. } => None,
        };
        let Some((opcode, body)) = packet else {
            return;
        };
        let result = {
            let Active::World(flow) = &mut self.active else {
                return;
            };
            flow.session
                .queue_world_packet(opcode, &body)
                .map(|()| drain_world(flow, &mut self.output))
        };
        if let Err(error) = result {
            self.output
                .push_back(Output::Disconnected(error.to_string()));
            self.active = Active::Idle;
        }
    }

    fn fail_login(&mut self, error: SessionError) {
        self.output.push_back(Output::LoginFailed(error));
        self.active = Active::Idle;
    }
}

enum Transition {
    RetryAuth {
        credentials: Credentials,
        attempt: u8,
    },
    ChooseRealm {
        session: AuthSession,
        realms: Vec<RealmInfo>,
        generation: u64,
    },
}

fn handle_auth_signal(
    flow: &mut AuthFlow,
    signal: SocketSignal,
    output: &mut VecDeque<Output>,
) -> Result<Option<Transition>, SessionError> {
    match signal {
        SocketSignal::Open { .. } => {
            let session = flow
                .session
                .as_ref()
                .ok_or_else(|| protocol_error("auth session unavailable"))?;
            flow.socket.send(&session.challenge_bytes()?)?;
            output.push_back(Output::Stage(LoginStage::Authenticating));
            Ok(None)
        }
        SocketSignal::Binary { bytes, .. } => {
            if flow.buffer.len().saturating_add(bytes.len()) > MAX_AUTH_BUFFER {
                return Err(protocol_error("auth response exceeds 64 KiB"));
            }
            flow.buffer.extend_from_slice(&bytes);
            loop {
                let packet_len = match flow.stage {
                    AuthStage::Challenge => challenge_packet_len(&flow.buffer)?,
                    AuthStage::Proof => proof_packet_len(&flow.buffer)?,
                    AuthStage::Realms => realm_packet_len(&flow.buffer)?,
                };
                let Some(packet_len) = packet_len else {
                    return Ok(None);
                };
                if flow.stage == AuthStage::Challenge
                    && flow.buffer.get(34) == Some(&0)
                    && flow.attempt < MAX_SRP_ATTEMPTS
                {
                    return Ok(Some(Transition::RetryAuth {
                        credentials: flow.credentials.clone(),
                        attempt: flow.attempt + 1,
                    }));
                }
                let packet: Vec<u8> = flow.buffer.drain(..packet_len).collect();
                match flow.stage {
                    AuthStage::Challenge => {
                        let proof = flow
                            .session
                            .as_mut()
                            .ok_or_else(|| protocol_error("auth session unavailable"))?
                            .proof_bytes(&packet)?;
                        flow.socket.send(&proof)?;
                        flow.stage = AuthStage::Proof;
                    }
                    AuthStage::Proof => {
                        let request = flow
                            .session
                            .as_mut()
                            .ok_or_else(|| protocol_error("auth session unavailable"))?
                            .finish_proof_bytes(&packet)?;
                        flow.socket.send(&request)?;
                        flow.stage = AuthStage::Realms;
                    }
                    AuthStage::Realms => {
                        let realms = flow
                            .session
                            .as_ref()
                            .ok_or_else(|| protocol_error("auth session unavailable"))?
                            .read_realms_typed(&packet)?;
                        let generation = flow.credentials.generation;
                        // Move the session out without ever exposing its key.
                        let session = flow
                            .session
                            .take()
                            .ok_or_else(|| protocol_error("auth session unavailable"))?;
                        return Ok(Some(Transition::ChooseRealm {
                            session,
                            realms,
                            generation,
                        }));
                    }
                }
            }
        }
        SocketSignal::Error { reason, .. } => Err(protocol_error(reason)),
        SocketSignal::Closed { code, reason, .. } => Err(protocol_error(format!(
            "auth relay closed ({code}){}",
            close_reason(&reason)
        ))),
    }
}

fn handle_world_signal(
    flow: &mut WorldFlow,
    signal: SocketSignal,
    output: &mut VecDeque<Output>,
) -> Result<Option<Transition>, SessionError> {
    match signal {
        SocketSignal::Open { .. } => Ok(None),
        SocketSignal::Binary { bytes, .. } => {
            flow.session.feed_world_bytes(&bytes)?;
            drain_world(flow, output);
            Ok(None)
        }
        SocketSignal::Error { reason, .. } => Err(protocol_error(reason)),
        SocketSignal::Closed { code, reason, .. } => Err(protocol_error(format!(
            "world relay closed ({code}){}",
            close_reason(&reason)
        ))),
    }
}

fn send_keepalive(flow: &mut WorldFlow, output: &mut VecDeque<Output>) -> Result<(), SessionError> {
    let now = Instant::now();
    // Measure from the oldest unanswered send; sending the next ping must not
    // reset the deadline. Socket events are drained before this check, so pongs
    // already queued after a suspended browser frame are consumed first.
    if let Some(&(sequence, sent)) = flow.pending_pings.front() {
        let elapsed = now.saturating_duration_since(sent);
        if elapsed >= WORLD_PONG_TIMEOUT {
            warn!(
                "net: world pong timeout — seq={sequence}, unanswered={} elapsed={:.1}s websocket_state={} buffered_bytes={}",
                flow.pending_pings.len(),
                elapsed.as_secs_f64(),
                flow.socket.socket.ready_state(),
                flow.socket.socket.buffered_amount()
            );
            return Err(protocol_error(format!(
                "world server did not answer ping {sequence} for {:.0} seconds",
                elapsed.as_secs_f64()
            )));
        }
    }
    if !flow.next_ping.is_some_and(|deadline| now >= deadline) {
        return Ok(());
    }
    // Rearm from this send; never catch up missed intervals with a burst.
    flow.next_ping = Some(now + PING_INTERVAL);
    let (sequence, last_rtt) = {
        let mut clock = flow.ping.lock().unwrap_or_else(|e| e.into_inner());
        clock.sequence = clock.sequence.wrapping_add(1);
        clock.sent_at = Some(now);
        (clock.sequence, clock.last_rtt_ms.unwrap_or(0))
    };
    flow.session.queue_world_packet(
        benilla_protocol::messages::opcode::CMSG_PING,
        &benilla_protocol::messages::ping(sequence, last_rtt),
    )?;
    flow.pending_pings.push_back((sequence, now));
    debug!("net: web ping seq={sequence} last_rtt={last_rtt}ms");
    drain_world(flow, output);
    Ok(())
}

fn drain_world(flow: &mut WorldFlow, output: &mut VecDeque<Output>) {
    loop {
        let packet = flow.session.take_world_outbound();
        if packet.is_empty() {
            break;
        }
        if let Err(error) = flow.socket.send(&packet) {
            output.push_back(Output::Disconnected(error.to_string()));
            break;
        }
    }
    while let Some(event) = flow.session.take_world_event() {
        match event {
            WorldEvent::Challenge => {}
            WorldEvent::Authenticated => {
                flow.next_ping = Some(Instant::now() + PING_INTERVAL);
                info!("net: web keepalive armed: 30s cadence, including character selection");
            }
            WorldEvent::Entering { guid } => {
                let name = flow
                    .roster
                    .iter()
                    .find(|character| character.guid == guid)
                    .map(|character| character.name.clone())
                    .unwrap_or_default();
                output.push_back(Output::Connected { guid, name });
            }
            WorldEvent::Queue { position } => output.push_back(Output::Queue {
                position,
                realm: flow.realm.name.clone(),
            }),
            WorldEvent::Roster { characters } => {
                info!(
                    "net: web roster received — {} character(s)",
                    characters.len()
                );
                flow.roster_seen = true;
                flow.roster = characters.clone();
                output.push_back(Output::Roster {
                    characters,
                    realm: flow.realm.clone(),
                });
            }
            WorldEvent::CharacterActionResult { action, code } => {
                output.push_back(Output::CharAction { action, code });
            }
            WorldEvent::CharacterLoginFailed { result } => {
                output.push_back(Output::CharacterLoginFailed(result));
            }
            WorldEvent::WorldEntered {
                map,
                position,
                orientation,
            } => output.push_back(Output::EnteredWorld {
                map_id: map,
                position,
                orientation,
            }),
            WorldEvent::SessionEvents {
                opcode,
                events,
                tail,
            } => {
                if events
                    .iter()
                    .any(|event| matches!(event, SessionEvent::LoggedOut))
                {
                    info!("net: web logout complete — refreshing roster on authenticated socket");
                    flow.roster.clear();
                }
                let events = events
                    .into_iter()
                    .filter(|event| {
                        if let SessionEvent::Pong { sequence } = event {
                            // Accept a delayed reply to any outstanding ping,
                            // not just the latest RTT sample. A correlated reply
                            // proves round-trip progress through that send.
                            if let Some(at) = flow
                                .pending_pings
                                .iter()
                                .position(|(sent, _)| sent == sequence)
                            {
                                flow.pending_pings.drain(..=at);
                            }
                            if let Some(rtt) = flow
                                .ping
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .record_pong(*sequence)
                            {
                                debug!("net: web pong seq={sequence} rtt={rtt}ms");
                            }
                            false
                        } else {
                            true
                        }
                    })
                    .collect::<Vec<_>>();
                if !events.is_empty() || tail != 0 {
                    output.push_back(Output::SessionEvents {
                        opcode,
                        events,
                        tail,
                    });
                }
            }
            WorldEvent::PacketSkipped { opcode, reason } => {
                output.push_back(Output::PacketSkipped { opcode, reason });
            }
        }
    }
}

fn challenge_packet_len(bytes: &[u8]) -> Result<Option<usize>, SessionError> {
    if bytes.len() < 3 {
        return Ok(None);
    }
    if bytes[0] != 0 {
        return Err(protocol_error(format!(
            "unexpected realmd challenge opcode 0x{:02x}",
            bytes[0]
        )));
    }
    if bytes[2] != 0 {
        return Ok(Some(3));
    }
    if bytes.len() < 36 {
        return Ok(None);
    }
    let generator_len = usize::from(bytes[35]);
    if !(1..=32).contains(&generator_len) {
        return Err(protocol_error("invalid SRP generator length"));
    }
    let prime_len_index = 36 + generator_len;
    let Some(&prime_len) = bytes.get(prime_len_index) else {
        return Ok(None);
    };
    if prime_len != 32 {
        return Err(protocol_error(format!(
            "invalid SRP prime length {prime_len}"
        )));
    }
    let security_index = prime_len_index + 1 + usize::from(prime_len) + 32 + 16;
    let Some(&security) = bytes.get(security_index) else {
        return Ok(None);
    };
    let total = security_index + 1 + usize::from(security & 1 != 0) * 20;
    Ok((bytes.len() >= total).then_some(total))
}

fn proof_packet_len(bytes: &[u8]) -> Result<Option<usize>, SessionError> {
    if bytes.len() < 2 {
        return Ok(None);
    }
    if bytes[0] != 1 {
        return Err(protocol_error(format!(
            "unexpected realmd proof opcode 0x{:02x}",
            bytes[0]
        )));
    }
    let length = if bytes[1] == 0 { 26 } else { 2 };
    Ok((bytes.len() >= length).then_some(length))
}

fn realm_packet_len(bytes: &[u8]) -> Result<Option<usize>, SessionError> {
    if bytes.len() < 8 {
        return Ok(None);
    }
    if bytes[0] != 0x10 {
        return Err(protocol_error(format!(
            "unexpected realm-list opcode 0x{:02x}",
            bytes[0]
        )));
    }
    let mut cursor = 8;
    for _ in 0..bytes[7] {
        if bytes.len() < cursor + 5 {
            return Ok(None);
        }
        cursor += 5;
        for _ in 0..2 {
            let Some(end) = bytes[cursor..].iter().position(|byte| *byte == 0) else {
                return Ok(None);
            };
            cursor += end + 1;
        }
        if bytes.len() < cursor + 7 {
            return Ok(None);
        }
        cursor += 7;
    }
    let total = cursor + 2;
    Ok((bytes.len() >= total).then_some(total))
}

fn same_id(sender: &Sender<SocketSignal>, signal: SocketSignal) {
    let _ = sender.send(signal);
}

struct SocketBinding {
    id: u64,
    socket: WebSocket,
    on_open: Closure<dyn FnMut(Event)>,
    on_message: Closure<dyn FnMut(MessageEvent)>,
    on_error: Closure<dyn FnMut(ErrorEvent)>,
    on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl SocketBinding {
    fn open(id: u64, url: &str, sender: Sender<SocketSignal>) -> Result<Self, SessionError> {
        let socket = WebSocket::new(url).map_err(js_error)?;
        socket.set_binary_type(BinaryType::Arraybuffer);

        let open_tx = sender.clone();
        let on_open = Closure::wrap(Box::new(move |_event: Event| {
            same_id(&open_tx, SocketSignal::Open { id });
        }) as Box<dyn FnMut(_)>);
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let message_tx = sender.clone();
        let on_message = Closure::wrap(Box::new(move |event: MessageEvent| {
            let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() else {
                same_id(
                    &message_tx,
                    SocketSignal::Error {
                        id,
                        reason: "game relay sent a non-binary WebSocket message".to_owned(),
                    },
                );
                return;
            };
            same_id(
                &message_tx,
                SocketSignal::Binary {
                    id,
                    bytes: Uint8Array::new(&buffer).to_vec(),
                },
            );
        }) as Box<dyn FnMut(_)>);
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let error_tx = sender.clone();
        let on_error = Closure::wrap(Box::new(move |event: ErrorEvent| {
            let message = event.message();
            same_id(
                &error_tx,
                SocketSignal::Error {
                    id,
                    reason: if message.is_empty() {
                        "game WebSocket failed".to_owned()
                    } else {
                        message
                    },
                },
            );
        }) as Box<dyn FnMut(_)>);
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        let close_tx = sender;
        let on_close = Closure::wrap(Box::new(move |event: CloseEvent| {
            same_id(
                &close_tx,
                SocketSignal::Closed {
                    id,
                    code: event.code(),
                    reason: event.reason(),
                },
            );
        }) as Box<dyn FnMut(_)>);
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        Ok(Self {
            id,
            socket,
            on_open,
            on_message,
            on_error,
            on_close,
        })
    }

    fn send(&self, bytes: &[u8]) -> Result<(), SessionError> {
        self.socket.send_with_u8_array(bytes).map_err(js_error)
    }
}

impl Drop for SocketBinding {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onerror(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
        let _ = (
            &self.on_open,
            &self.on_message,
            &self.on_error,
            &self.on_close,
        );
    }
}

fn drive_web_net(mut state: NonSendMut<WebNetState>, mut trace: Local<WorldStreamTrace>) {
    state.tick();
    while let Some(output) = state.output.pop_front() {
        let event = match output {
            Output::Stage(stage) => Some(SessionEvent::LoginStage { stage }),
            Output::Realms(realms) => Some(SessionEvent::RealmList { realms }),
            Output::Roster { characters, realm } => Some(SessionEvent::CharacterList {
                characters,
                realm: Some(realm),
            }),
            Output::Queue { position, realm } => Some(SessionEvent::LoginQueued {
                position,
                realm: Some(realm),
            }),
            Output::CharAction { action, code } => {
                Some(SessionEvent::CharActionResult { action, code })
            }
            Output::CharacterLoginFailed(result) => {
                Some(SessionEvent::CharacterLoginFailed { result })
            }
            Output::Connected { guid, name } => Some(SessionEvent::Connected {
                self_guid: guid,
                name,
                billing_time_rested: 0,
                tutorial_flags: None,
                addon_info: None,
            }),
            Output::EnteredWorld {
                map_id,
                position,
                orientation,
            } => Some(SessionEvent::Worldport {
                map_id,
                position,
                orientation,
                needs_ack: false,
            }),
            Output::SessionEvents {
                opcode,
                events,
                tail,
            } => {
                if tail != 0 && trace.tailed_opcodes.insert(opcode) {
                    warn!(
                        "browser world stream: opcode {opcode:#06x} left {tail} undecoded byte(s)"
                    );
                }
                let object_creates = events
                    .iter()
                    .filter(|event| matches!(event, SessionEvent::ObjectCreate { .. }))
                    .count();
                trace.object_creates += object_creates;
                if !trace.events_announced && !events.is_empty() {
                    trace.events_announced = true;
                    info!(
                        "browser world stream: production SessionEvent feed active (opcode {opcode:#06x}, {} event(s))",
                        events.len()
                    );
                }
                if !trace.objects_announced && object_creates != 0 {
                    trace.objects_announced = true;
                    info!(
                        "browser world stream: object feed active (opcode {opcode:#06x}, {object_creates} create(s) in batch, {} total)",
                        trace.object_creates
                    );
                }
                for event in events {
                    let _ = state.events_tx.send(event);
                }
                None
            }
            Output::PacketSkipped { opcode, reason } => {
                if trace.skipped_opcodes.insert(opcode) {
                    warn!("browser world stream: skipped opcode {opcode:#06x}: {reason}");
                }
                Some(SessionEvent::PacketDropped {
                    opcode,
                    unparseable: true,
                })
            }
            Output::LoginFailed(error) => {
                let refusal = match &error {
                    SessionError::AuthRejected(code) => Some(LoginRefusal::Logon(*code)),
                    SessionError::WorldRejected(code) => Some(LoginRefusal::World(*code)),
                    SessionError::Protocol(_) | SessionError::InvalidPassword => None,
                };
                Some(SessionEvent::LoginFailed {
                    refusal,
                    reason: error.to_string(),
                    terminal: false,
                    dial: None,
                })
            }
            Output::Disconnected(reason) => Some(SessionEvent::Disconnected {
                reason,
                end: SessionEnd::Lost,
            }),
        };
        if let Some(event) = event {
            let _ = state.events_tx.send(event);
        }
    }
}

#[derive(Default)]
struct WorldStreamTrace {
    events_announced: bool,
    objects_announced: bool,
    object_creates: usize,
    tailed_opcodes: HashSet<u16>,
    skipped_opcodes: HashSet<u16>,
}

fn close_reason(reason: &str) -> String {
    if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    }
}

fn protocol_error(message: impl Into<String>) -> SessionError {
    SessionError::Protocol(message.into())
}

fn js_error(value: wasm_bindgen::JsValue) -> SessionError {
    protocol_error(
        value
            .as_string()
            .unwrap_or_else(|| "browser WebSocket API failed".to_owned()),
    )
}
