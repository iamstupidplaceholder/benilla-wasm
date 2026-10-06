//! Backend-neutral data exchanged between the game and FrameScript.
//!
//! The native host executes these values through `mlua`; the browser host executes them through
//! `benilla-lua`.  Keeping the wire/state shapes here prevents either VM adapter from inventing a
//! second version of the gameplay contract.

/// What one action slot displays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionSlot {
    pub texture: Option<String>,
    pub kind: u8,
    pub action: u32,
    pub count: u32,
    pub consumable: bool,
}

/// One action's dynamic state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionState {
    pub usable: bool,
    pub not_enough_mana: bool,
    pub in_range: Option<bool>,
    pub has_range: bool,
    pub current: bool,
    pub auto_repeat: bool,
    pub is_attack: bool,
    pub equipped: bool,
    pub cooldown: Option<(i64, u32, bool)>,
}

/// One queued `UseAction` press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionUse {
    pub action: u32,
    pub on_self: bool,
}

/// One host-registered key-binding command.
#[derive(Clone, Copy, Debug)]
pub struct KeybindCommand {
    pub name: &'static str,
    pub category: &'static str,
    pub run_on_up: bool,
    pub default1: Option<&'static str>,
    pub default2: Option<&'static str>,
}

/// Runnable half of an addon-declared binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddonBindingBody {
    pub name: String,
    pub run_on_up: bool,
    pub body: String,
}

/// A host request queued by the key-binding Lua API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeybindRequest {
    Save(u32),
    Run(String),
}

pub const MAX_MACROS: usize = 18;
pub const MAX_MACRO_NAME: usize = 16;
pub const MAX_MACRO_BODY: usize = 255;

/// One player macro, as `GetMacroInfo` reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroView {
    pub name: String,
    pub texture: Option<String>,
    pub body: String,
    pub local_only: bool,
}

/// The account and per-character macro lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacroState {
    pub account: Vec<MacroView>,
    pub character: Vec<MacroView>,
}

/// One queued selection request from FrameScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionRequest {
    Unit(String),
    Assist(String),
    AssistByName(String),
    LastEnemy,
}

/// The sentinel bag id used for equipped inventory slots.
pub const EQUIPMENT_BAG: i64 = -100;

/// What the cursor carries.
#[derive(Clone, Debug, PartialEq)]
pub enum CursorPayload {
    Item(CursorItem),
    Spell(CursorSpell),
    Action(CursorAction),
    Macro(CursorMacro),
    PetAction(CursorPetAction),
    Merchant(CursorMerchantItem),
    StablePet(CursorStablePet),
    Money(CursorMoney),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorMoney {
    pub copper: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorMerchantItem {
    pub item_id: u32,
    pub row: u32,
    pub texture: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorStablePet {
    pub slot: u8,
    pub texture: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorItem {
    pub bag: i64,
    pub slot: u32,
    pub item_id: u32,
    pub texture: Option<String>,
    pub link: Option<String>,
    pub count: Option<u32>,
    pub quality: Option<u32>,
    pub equip_slots: Vec<u8>,
    pub bar_placeable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorSpell {
    pub book_slot: u32,
    pub book_type: String,
    pub spell_id: u32,
    pub texture: Option<String>,
    pub passive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorAction {
    pub src_slot: u32,
    pub kind: u8,
    pub action: u32,
    pub texture: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorPetAction {
    pub src_slot: u32,
    pub packed: u32,
    pub passive: bool,
    pub texture: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorMacro {
    pub index: u32,
    pub texture: Option<String>,
}

/// A positive answer to one of the enchant confirmation dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnchantConfirm {
    Bind,
    Replace,
}

/// What the app's world pick resolved under the cursor this frame.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum WorldPick {
    /// No world hit at all (sky or beyond the world).
    #[default]
    Nothing,
    /// Terrain, WMO or doodad, but no unit or game object.
    Terrain,
    /// A unit or game object.
    Object,
}

/// One zone-sensitive row from `ChatChannels.dbc` as exposed to FrameScript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneChannelRow {
    pub id: u32,
    pub shortcut: String,
    pub resolved: Option<String>,
    pub listed: bool,
}

/// A channel verb queued by FrameScript for the app/network layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelCommand {
    Join { name: String, password: String },
    Leave { name: String },
    List { name: String },
    ListAll,
    DisplayOwner { name: String },
    SetOwner { name: String, player: String },
    SetPassword { name: String, password: String },
    Ban { name: String, player: String },
    Invite { name: String, player: String },
    Kick { name: String, player: String },
    Moderator { name: String, player: String },
    Unmoderator { name: String, player: String },
    Mute { name: String, player: String },
    Unmute { name: String, player: String },
    Unban { name: String, player: String },
    Moderate { name: String },
    ToggleAnnouncements { name: String },
}

/// One live entry in the chat colour registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatTypeColor {
    pub name: String,
    pub rgb: [u8; 3],
}

/// `CHATMSGGROUP`, in the reference client's load-bearing table order.
pub const MESSAGE_GROUPS: [(&str, bool, u8); 68] = [
    ("SYSTEM", true, 0),
    ("SAY", true, 0),
    ("YELL", true, 0),
    ("WHISPER", true, 0),
    ("PARTY", true, 0),
    ("GUILD", true, 0),
    ("CREATURE", true, 0),
    ("CHANNEL", true, 0),
    ("SKILL", true, 0),
    ("LOOT", true, 0),
    ("COMBAT_MISC_INFO", true, 0),
    ("COMBAT_SELF_HITS", true, 0),
    ("COMBAT_SELF_MISSES", true, 0),
    ("COMBAT_PET_HITS", true, 0),
    ("COMBAT_PET_MISSES", true, 0),
    ("COMBAT_PARTY_HITS", false, 0),
    ("COMBAT_PARTY_MISSES", false, 0),
    ("COMBAT_FRIENDLYPLAYER_HITS", false, 0),
    ("COMBAT_FRIENDLYPLAYER_MISSES", false, 0),
    ("COMBAT_HOSTILEPLAYER_HITS", true, 0),
    ("COMBAT_HOSTILEPLAYER_MISSES", true, 0),
    ("COMBAT_CREATURE_VS_SELF_HITS", true, 0),
    ("COMBAT_CREATURE_VS_SELF_MISSES", true, 0),
    ("COMBAT_CREATURE_VS_PARTY_HITS", false, 0),
    ("COMBAT_CREATURE_VS_PARTY_MISSES", false, 0),
    ("COMBAT_CREATURE_VS_CREATURE_HITS", false, 0),
    ("COMBAT_CREATURE_VS_CREATURE_MISSES", false, 0),
    ("COMBAT_FRIENDLY_DEATH", true, 0),
    ("COMBAT_HOSTILE_DEATH", true, 0),
    ("COMBAT_XP_GAIN", true, 0),
    ("SPELL_SELF_DAMAGE", true, 0),
    ("SPELL_SELF_BUFF", true, 0),
    ("SPELL_PET_DAMAGE", true, 0),
    ("SPELL_PET_BUFF", true, 0),
    ("SPELL_PARTY_DAMAGE", false, 0),
    ("SPELL_PARTY_BUFF", false, 0),
    ("SPELL_FRIENDLYPLAYER_DAMAGE", false, 0),
    ("SPELL_FRIENDLYPLAYER_BUFF", false, 0),
    ("SPELL_HOSTILEPLAYER_DAMAGE", true, 0),
    ("SPELL_HOSTILEPLAYER_BUFF", true, 0),
    ("SPELL_CREATURE_VS_SELF_DAMAGE", true, 0),
    ("SPELL_CREATURE_VS_SELF_BUFF", true, 0),
    ("SPELL_CREATURE_VS_PARTY_DAMAGE", false, 0),
    ("SPELL_CREATURE_VS_PARTY_BUFF", false, 0),
    ("SPELL_CREATURE_VS_CREATURE_DAMAGE", false, 0),
    ("SPELL_CREATURE_VS_CREATURE_BUFF", false, 0),
    ("SPELL_TRADESKILLS", true, 0),
    ("SPELL_DAMAGESHIELDS_ON_SELF", true, 0),
    ("SPELL_DAMAGESHIELDS_ON_OTHERS", false, 0),
    ("SPELL_AURA_GONE_SELF", true, 0),
    ("SPELL_AURA_GONE_PARTY", false, 0),
    ("SPELL_AURA_GONE_OTHER", false, 0),
    ("SPELL_ITEM_ENCHANTMENTS", true, 0),
    ("SPELL_BREAK_AURA", true, 0),
    ("SPELL_PERIODIC_SELF_DAMAGE", true, 0),
    ("SPELL_PERIODIC_SELF_BUFFS", true, 0),
    ("SPELL_PERIODIC_PARTY_DAMAGE", false, 0),
    ("SPELL_PERIODIC_PARTY_BUFFS", false, 0),
    ("SPELL_PERIODIC_FRIENDLYPLAYER_DAMAGE", false, 0),
    ("SPELL_PERIODIC_FRIENDLYPLAYER_BUFFS", false, 0),
    ("SPELL_PERIODIC_HOSTILEPLAYER_DAMAGE", true, 0),
    ("SPELL_PERIODIC_HOSTILEPLAYER_BUFFS", true, 0),
    ("SPELL_PERIODIC_CREATURE_DAMAGE", true, 0),
    ("SPELL_PERIODIC_CREATURE_BUFFS", true, 0),
    ("SPELL_FAILED_LOCALPLAYER", false, 0),
    ("COMBAT_HONOR_GAIN", true, 0),
    ("COMBAT_FACTION_CHANGE", true, 1),
    ("MONEY", true, 2),
];

pub fn message_group_index(name: &str) -> Option<usize> {
    MESSAGE_GROUPS
        .iter()
        .position(|(candidate, _, _)| candidate.eq_ignore_ascii_case(name))
}

/// Mutable, persisted presentation state for one chat window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChatWindowLook {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    pub font_size: i32,
    pub locked: bool,
    pub docked: Option<u8>,
    pub name: String,
    pub shown: bool,
    pub messages: Vec<String>,
    pub channels: Vec<(String, u32)>,
}

impl Default for ChatWindowLook {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl ChatWindowLook {
    pub(crate) const DEFAULT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
        font_size: 0,
        locked: true,
        docked: None,
        name: String::new(),
        shown: false,
        messages: Vec::new(),
        channels: Vec::new(),
    };

    pub fn stock(index: usize) -> Self {
        let groups = |range: std::ops::Range<usize>, all: bool| -> Vec<String> {
            MESSAGE_GROUPS[range]
                .iter()
                .filter(|(_, on, _)| all || *on)
                .map(|(name, _, _)| (*name).to_string())
                .collect()
        };
        match index {
            0 => Self {
                shown: true,
                messages: groups(0..10, true),
                ..Self::DEFAULT
            },
            1 => Self {
                shown: true,
                docked: Some(1),
                messages: groups(10..68, false),
                ..Self::DEFAULT
            },
            _ => Self::DEFAULT,
        }
    }

    pub fn normalize_messages(&mut self) {
        let mut flags = [false; MESSAGE_GROUPS.len()];
        for message in &self.messages {
            if let Some(index) = message_group_index(message) {
                flags[index] = true;
            }
        }
        self.messages = MESSAGE_GROUPS
            .iter()
            .zip(flags)
            .filter(|(_, enabled)| *enabled)
            .map(|((name, _, _), _)| (*name).to_string())
            .collect();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuelRequest {
    Accept,
    Cancel,
    StartByName(String),
    StartByUnit(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRequest {
    Logout,
    Quit,
    CancelLogout,
    ForceQuit,
    ForceLogout,
    ReloadUi,
    StopCinematic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WornDisplay {
    Helm,
    Cloak,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitReach {
    pub dist_sq: f64,
    pub inspectable: bool,
}

pub fn power_token(ty: u8) -> &'static str {
    match ty {
        1 => "RAGE",
        2 => "FOCUS",
        3 => "ENERGY",
        4 => "HAPPINESS",
        _ => "MANA",
    }
}
