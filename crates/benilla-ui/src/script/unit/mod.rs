//! The game-state `Unit*` bindings (decision 0068 §3) — the first slice of the addon-facing API that
//! reads *live game state*, and the seam that keeps this crate engine-free while doing so.
//!
//! The engine must not touch the ECS/net (decisions 0006/0061: the codec is stateless, the ECS owns
//! state). So instead of reaching outward, the app pushes a per-frame **unit snapshot** into the VM
//! via [`UiScript::set_unit`], and the `Unit*` globals here read that plain data. A frame's
//! `UnitHealth("player")` therefore resolves against a [`UnitState`] the Bevy side deposited that
//! frame — the same shape a real addon sees, with none of the coupling.
//!
//! Return shapes follow the reference: `UnitName` returns the name *or nil*;
//! `UnitHealth`/`UnitHealthMax`/`UnitLevel` return numbers (`UnitLevel` returns **−1** for a
//! unit whose level "can't be told" — [`level_reads_unknown`], the target frame's skull gate);
//! a unit that doesn't exist reports `UnitExists` nil and the numbers `0` (nil for the name),
//! exactly as the client does for an absent unit token. Tokens (`"player"`, `"target"`, …) are opaque strings the host never
//! interprets — the app decides what each maps to.
//!
//! **Every predicate in this family answers the number `1` or `nil`, never a Lua boolean**, and it
//! answers it through the one helper — [`unit_predicate`], over
//! [`super::binding_abi::neutral_flag`]. Decision 2043; the law is 1830's, which settled the same
//! question for the widget predicates a family earlier.
//!
//! v1 gaps, stated not hidden: the snapshot carries only the *active* power slot (see the
//! `UnitMana` note below); `UnitIsConnected`'s backing
//! field defaults `false`, and no app-side feed sets it `true` for live units yet (decision 0434
//! §2's party-frame predicates land the binding first, the feed follows — see the field doc).

use benilla_lua::{LuaContext, LuaString};

use super::Model;

pub use crate::script_data::power_token;
pub use crate::script_data::SelectionRequest;

pub use crate::script_game_data::{PlayerRecord, UnitState};

/// `FrameScript_GetText("UNKNOWNOBJECT")` as the client's name resolvers call it — the VM's own
/// GlobalString, read out of `_G` exactly as `0x703bf0` reads it (so a translated
/// `GlobalStrings.lua` translates this too; enUS `"Unknown"`, `GlobalStrings.lua:4444`), falling
/// back to the binary's own literal `"Unknown Being"` (`0x860fa4`) when the global is missing or
/// empty. Always a string — this is the "name not yet known" state, never a nil.
///
/// **ONE home, two callers**, the same reason [`is_civilian_kill`] is a function. `UnitName`
/// `0x517020` reaches it at `0x517220`, and `CGUnit_C::GetUnitName 0x609210` — which the UNIT
/// TOOLTIP builder `0x529fe0` calls for its title line (`0x52a187`) — reaches the *same* tail at
/// `0x609324`, from every one of its misses:
///
/// ```text
/// 609353  je 0x609324        ; creature: [CGUnit+0xb30] (creaturecache.wdb) still null
/// 6092ce  je 0x609324        ; pet: petnamecache.wdb row absent, or its stamp stale
/// 609265  je 0x609324        ; player: namecache.wdb has not answered
/// 609324  push 0x0 / or edx,-1 / mov ecx,0x850ed0 ("UNKNOWNOBJECT") / call 0x703bf0
/// 60933c  mov eax,0x860fa4   ; the global missing or empty -> "Unknown Being"
/// ```
///
/// One resolver, so the verb and the plate can never disagree about what a unit whose name is
/// still in flight is called (decisions 2002, 2040).
///
/// **Both halves of that tail are here, and both are load-bearing.** The lookup is the rule
/// (decision 2045: the sentence is the install's, not ours); the literal beside it is the
/// reference's own `0x860fa4`, which is what "a literal is legitimate only as a fallback beside a
/// lookup" means. Spelled `get_global::<LuaString>` — the shape `benilla-app`'s
/// `reference_strings` tripwire recognises as a resolver — so the fallback reads as the fallback
/// it is rather than as undeclared drift. It answers the same three cases the value read
/// it replaces did: a non-empty string is returned, and both a missing global and a present-but-
/// EMPTY one take `0x860fa4` (the binary tests the pointer and then its first byte).
pub fn unknownobject(lua: &mut impl LuaContext) -> benilla_lua::Result<LuaString> {
    // One expression, deliberately: `get_global::<LuaString>` is what the tripwire matches on,
    // and a line break inside it would hide this resolver from the walk again.
    let global = lua.get_global::<LuaString>("UNKNOWNOBJECT").ok();
    Ok(global
        .filter(|s| !s.as_bytes().is_empty())
        .unwrap_or_else(|| "Unknown Being".into()))
}

/// The grey-band table `0x80ae98` (a byte-identical twin at `0x81dda8` drives the nameplate's
/// grey level colour), indexed by `playerLevel / 5` and clamped to 12 past the table. ONE home
/// for its three consumers: [`unit_is_grey`] (tooltip CIVILIAN gate + the plate's skull/grey
/// legs), the `GetQuestGreenRange` binding (the FrameXML `GetDifficultyColor` green→grey
/// boundary), and — through both — every level con the UI shows.
const GREY_BAND: [u32; 20] = [
    4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 12, 12, 12, 12, 12, 12, 12,
];

/// The band value for a player level — `GREY_BAND[min(playerLevel/5, 19)]` (the
/// `GetQuestGreenRange` return, §5-VERIFIED 2026-07-17: the binding `0x4e17d0` reads the
/// byte-identical `0x8076c0` twin with exactly this index clamp).
pub fn grey_band(player_level: u32) -> u32 {
    GREY_BAND
        .get((player_level / 5) as usize)
        .copied()
        .unwrap_or(12)
}

/// The trivial/GREY check (`0x5f0700`, §5-VERIFIED 2026-07-17 — was INFERRED): a unit enough
/// levels *below* the player that it cons grey — `playerLevel − unitLevel >
/// GREY_BAND[playerLevel/5]`, **strict >** (diff == band is still green). The binary's full
/// predicate also requires not-player-controlled; every consumer here applies this to creatures.
pub fn unit_is_grey(player_level: u32, unit_level: u32) -> bool {
    player_level > unit_level && player_level - unit_level > grey_band(player_level)
}

/// The **CIVILIAN predicate `0x612550`** — "killing this would be a dishonorable kill": the unit's
/// PvP bit **and** the creature-query civilian flag **and** the unit is HOSTILE to the player
/// (`UnitReaction` ≤ 2) **and** the kill would be GREY/trivial ([`unit_is_grey`]). A friendly
/// civilian, or one that still cons, is not one.
///
/// **ONE home, two callers**, which is the whole reason it is a function: the unit tooltip's green
/// CIVILIAN line ([`super::tooltip_unit`], decision 0276) and `UnitPVPName`'s civilian arm
/// ([`super::pvp`] — `0x609370` leg B, which prefixes `PVP_RANK_CIVILIAN` onto a non-player's
/// name). The engine gates both on the same call; a second copy here would let the tooltip and the
/// name disagree about whether the same mob is a civilian.
///
/// **Scope of the verification.** wow-re's honor carve leaves `0x612550` itself uncarved (its §12,
/// "named by its use here"); the four terms are decision 0276's tooltip-line law, which is where
/// this composition was verified. So this is the tooltip's gate reused at the second call site the
/// carve proves shares it — not an independently byte-checked predicate.
pub fn is_civilian_kill(u: &UnitState, player_level: u32) -> bool {
    u.civilian && u.pvp && u.reaction != 0 && u.reaction <= 2 && unit_is_grey(player_level, u.level)
}

/// The "level reads ??" gate — the engine tooltip's byte-pinned leg set (`0x529fe0` §2-LEVEL):
/// never players; level 0 (unstreamed), world-boss rank 3, or a much-higher hostile (reaction
/// ≤ 2 on the 1..8 API scale, unit ≥ player + 10 — **inclusive**, byte-confirmed). The Lua
/// `UnitLevel`'s −1 law was §5-VERIFIED against this (wow-re `ui/scratch/
/// level-assess-bindings.md`, 2026-07-17): its two −1 legs are exactly the boss + hostile-≥10
/// legs here (no trivial-gray term exists in `0x517fc0`, and none is possible at ≥ +10), with
/// ONE divergence the binding handles itself — a raw level ≤ 0 returns VERBATIM, not −1
/// (same skull outcome in FrameXML; the tooltip's "??" keeps the level-0 leg per its own law).
pub fn level_reads_unknown(u: &UnitState, player_level: u32) -> bool {
    let much_higher_hostile =
        u.reaction != 0 && u.reaction <= 2 && u.level >= player_level.saturating_add(10);
    !u.is_player && (u.level == 0 || u.rank == 3 || much_higher_hostile)
}

/// `UnitClassification`'s return — the classification-word table, **byte-verified** at `0x850424`
/// (decision 0782): the binding `0x516d90` is nothing but `TABLE[rank(unit)]`, indexed by the same
/// gated rank [`UnitState::rank`] carries. The table is exactly five entries long (the sixth dword
/// at `0x850438` is an unrelated `"UnitExists"` literal), and rank comes pre-clamped to 0..4 by
/// `CreatureInfo+0x20`, so an out-of-range value can only be our own bug — it reads `"normal"`,
/// which is also what the binding's unresolved-token path pushes (`0x516dc4` loads index 0).
///
/// Note `"rareelite"` is a real return here even though 1.12 has no rare-elite *art* and no
/// rare-elite tooltip *word* — the reference's `TargetFrame_CheckClassification` sends it to the
/// same Elite border as `"elite"`, and the tooltip's `{"", Elite, Elite, Boss, ""}` prints ELITE for
/// it. The distinction exists in the API and nowhere in the pixels.
pub fn classification_word(rank: u32) -> &'static str {
    match rank {
        1 => "elite",
        2 => "rareelite",
        3 => "worldboss",
        4 => "rare",
        _ => "normal",
    }
}

impl super::UiScript {
    /// Push (or clear) a unit token's snapshot. `Some(state)` stores it under `token`; `None` removes
    /// it (so `UnitExists(token)` reports false and the numeric getters return `0`). The app's
    /// per-frame feed (decision 0068 §3) calls this for `"player"`/`"target"`/… before the VM's
    /// event dispatch, so a frame's `OnEvent` sees the current values.
    pub fn set_unit(&mut self, token: &str, state: Option<UnitState>) {
        {
            let mut model = self.model_mut();
            match state {
                // Folded on the way IN as well as on the way out — the map's key type is the
                // canonical lowercase token, so a feed that ever pushed `"Target"` could not create
                // a second, shadowing entry.
                Some(s) => {
                    // **A `"player"` push keeps the record in step, field by field** (2261,
                    // widened by 2263). The reference writes `0xc27d80` once per Enter World
                    // commit and never clears it; our VM is rebuilt per login, so the entry load
                    // seeds it — and this keeps it true for any later push that KNOWS a field,
                    // without ever letting one that doesn't blank it.
                    //
                    // That asymmetry is the whole mechanism: 2260's nameless player snapshot is
                    // exactly the push that must not be able to reach these four verbs. It is also
                    // what keeps every fixture in the workspace correct without touching them — a
                    // test that seats a full player is, by this rule, also seating the record.
                    if token.eq_ignore_ascii_case("player") {
                        let rec = &mut model.player_record;
                        if let Some(name) = s.name.as_deref().filter(|n| !n.is_empty()) {
                            if rec.name != name {
                                rec.name = name.to_string();
                            }
                        }
                        if let Some(race) = s.race.clone().zip(s.race_file.clone()) {
                            rec.race = Some(race);
                        }
                        if let Some(class) = s.class.clone().zip(s.class_file.clone()) {
                            rec.class = Some(class);
                        }
                        if s.sex != 0 {
                            rec.sex = s.sex;
                        }
                    }
                    model.units_by_lower.insert(token.to_ascii_lowercase(), s);
                }
                None => {
                    model.units_by_lower.remove(&token.to_ascii_lowercase());
                }
            }
        }
        // A push for a tooltip's LIVE unit token re-drives its health bar (the byte law's
        // HEALTH watcher — decision 0276; no line rebuild).
        super::tooltip_unit::on_unit_push(&mut self.lua, token);
    }

    /// Push the player's purse in copper (`PLAYER_FIELD_COINAGE`), read by the `GetMoney` global. A
    /// player-level snapshot (like the unit tokens) — the app calls this each frame the coinage
    /// field changes (decision 0081 phase 4).
    pub fn set_money(&mut self, copper: u64) {
        self.model_mut().money = copper;
    }

    /// Push the player's experience within the level (`PLAYER_XP`) and the level's requirement
    /// (`PLAYER_NEXT_LEVEL_XP`), read by the `UnitXP`/`UnitXPMax` globals. A player-level pair (like
    /// [`Self::set_money`]) — the app calls this each frame the XP fields change (the MainMenuBar XP
    /// bar feed). Both PRIVATE descriptor fields, so this only ever carries our own avatar's values.
    pub fn set_player_xp(&mut self, xp: u32, next_level_xp: u32) {
        let mut model = self.model_mut();
        model.player_xp = xp;
        model.player_next_level_xp = next_level_xp;
    }

    /// Push the player's rest snapshot — the `PLAYER_BYTES_2` byte 3 rest state, the
    /// `PLAYER_REST_STATE_EXPERIENCE` pool (raw wire value, base kill-XP units) and the
    /// `PLAYER_FLAGS_RESTING` bit — taken together so the `GetRestState`/`GetXPExhaustion`/
    /// `IsResting` trio can never read a half-updated rest picture. Player-level fields, same
    /// shape as [`Self::set_money`]; the app calls this each frame any of the three moves
    /// (decision 1082).
    pub fn set_rest_state(&mut self, state: u8, pool: u32, resting: bool) {
        let mut model = self.model_mut();
        model.rest_state = state;
        model.rest_pool = pool;
        model.resting = resting;
    }

    /// Push the two **play-time** bits of `PLAYER_FLAGS` — 12 (`PartialPlayTime`) and 13
    /// (`NoPlayTime`), decision 1746. Taken together for the same reason the rest trio is:
    /// stock `PlayerFrame_UpdatePlaytime` (PlayerFrame.lua:244) tests them as an if/elseif pair
    /// and a half-updated pair would paint the wrong one of the two icons.
    pub fn set_play_time(&mut self, partial: bool, none: bool) {
        let mut model = self.model_mut();
        model.partial_play_time = partial;
        model.no_play_time = none;
    }

    /// Push the account's **rested billing minutes**, from the `SMSG_AUTH_RESPONSE` that admitted
    /// the session — what `GetBillingTimeRested()` returns (decision 1820). Set once at login: the
    /// client parks it in a process-lifetime global and nothing else on the wire ever writes it.
    pub fn set_billing_time_rested(&mut self, minutes: u32) {
        self.model_mut().billing_time_rested = minutes;
    }

    /// Enter the **UI-load sound-suppression scope** — the reference's `0x458f50`, an `inc` on the
    /// counted depth every name-keyed `PlaySound` reads. Counted, not a flag, exactly as the
    /// client has it: nesting is legal and only the outermost exit re-enables sound.
    pub fn push_sound_suppression(&mut self) {
        let mut model = self.model_mut();
        model.sound_suppression = model.sound_suppression.saturating_add(1);
    }

    /// Leave it — `0x458f60`'s `dec`. Saturating on the way down too: an unbalanced pop is a bug
    /// in the caller, and silently wrapping to a permanently-muted UI would be the worse failure.
    pub fn pop_sound_suppression(&mut self) {
        let mut model = self.model_mut();
        model.sound_suppression = model.sound_suppression.saturating_sub(1);
    }

    /// Push whether a cinematic is playing — what `InCinematic()` answers.
    pub fn set_in_cinematic(&mut self, playing: bool) {
        self.model_mut().in_cinematic = playing;
    }

    /// Push Exhaustion.dbc — `(rest-state byte, localized name, factor)` rows for the
    /// `GetRestState`/`GetXPExhaustion` bindings (they read the table exactly as `0x48d350` /
    /// `0x48d3f0` read the client's own copy; decision 1087). Called once at startup off the
    /// patch chain; an empty push is ignored so a failed DBC read keeps the shipped-table
    /// fallback the model seeds.
    pub fn set_exhaustion_rows(&mut self, rows: Vec<(u8, String, f64)>) {
        if rows.is_empty() {
            return;
        }
        let mut model = self.model_mut();
        model.exhaustion = rows.into_iter().map(|(id, n, f)| (id, (n, f))).collect();
    }

    /// Push the player's banked combo points (`PLAYER_FIELD_BYTES` byte 1) and the unit they sit
    /// on (`PLAYER_FIELD_COMBO_TARGET`) — the pair `Player::SetComboPoints` writes together, taken
    /// together so they can never be read half-updated. Player-level PRIVATE fields, same shape as
    /// [`Self::set_money`]; the app calls this each frame either moves, including the drop back to
    /// zero (decisions 0869, 0875).
    ///
    /// **Raw wire values** — `GetComboPoints`'s class and current-target gates live in the binding,
    /// where the binary puts them.
    pub fn set_combo_points(&mut self, points: u8, target: u64) {
        let mut model = self.model_mut();
        model.combo_points = points;
        model.combo_target = target;
    }

    /// Drain the selection asks queued since the last call — the app resolves each to a guid and
    /// commits it through the one SetSelection tail. See [`SelectionRequest`] for why the three
    /// verbs share one ordered queue.
    pub fn take_selection_requests(&mut self) -> Vec<SelectionRequest> {
        std::mem::take(&mut self.model_mut().selection_requests)
    }

    /// Drain the `TargetNearestFriend([reverse])` presses queued since the last call — one `bool`
    /// per call, the cycle direction (`false` forward, `true` backward). Its own queue rather than
    /// a [`SelectionRequest`] variant because it names no unit: the reference runs it through the
    /// TAB cycler `0x493f60(reverse, mode 2)` and straight into `SetSelection`, never through the
    /// select-if-resolves helper the other three share.
    pub fn take_target_nearest_friend_requests(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.model_mut().target_nearest_friend_requests)
    }

    /// Drain the `(name, exactMatch)` pairs `TargetByName` queued since the last call — the app
    /// runs the shared by-name resolver (decision 0886) and commits the selection.
    pub fn take_target_by_name_requests(&mut self) -> Vec<(String, bool)> {
        std::mem::take(&mut self.model_mut().target_by_name_requests)
    }

    /// Drain the `ClearTarget()` trigger: `true` if it fired with a live target since the last
    /// call — the ESC chain's last leg; the app commits the deselect (SetSelection guid 0).
    pub fn take_target_clear(&mut self) -> bool {
        std::mem::take(&mut self.model_mut().target_clear)
    }

    /// Drain the unit tokens `DropItemOnUnit` queued since the last call — the app runs every gate
    /// and, on the pet leg, casts the learned Feed Pet spell at the held item (`0x48d960`).
    pub fn take_drop_item_on_unit(&mut self) -> Vec<String> {
        std::mem::take(&mut self.model_mut().drop_item_on_unit)
    }
}

/// Pick the interesting token from a directional two-unit call (`UnitIsEnemy(a, b)`): whichever
/// arg isn't `"player"` — our snapshot stores the relationship on the non-player unit (target),
/// and the ref calls both argument orders.
///
/// **The `"player"` test folds case**, like every other token comparison here and like the client's
/// own resolver. It is the kind of site the map's fold does not cover on its own: an addon passing
/// `UnitIsEnemy("Player", "target")` would otherwise be read as naming a non-player first arg, and
/// this would answer about the wrong unit — a wrong ANSWER rather than a missing one, which is the
/// worse failure of the two.
fn pick_unit_token(a: &Option<String>, b: &Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), _) if !x.eq_ignore_ascii_case("player") => Some(x.clone()),
        (_, Some(y)) => Some(y.clone()),
        (x, _) => x.clone(),
    }
}

/// The token prefixes 1.12's resolver `0x515970` recognises, in its own order.
///
/// The order is the binary's and is preserved even though our recogniser is order-insensitive:
/// `partypet` must be tested before `party` and `raidpet` before `raid` for a *resolver* to pick the
/// right unit, and the day this grows from a boolean into a resolver, the list is already right.
///
/// **These are PREFIX tests.** The compare length is the literal's, and `_strnicmp` stops at either
/// string's NUL, so `"playerfoo"` matches `player` — which is why it is a quiet nil rather than a
/// raise. `npc` is the sole FULL-STRING compare in the resolver, so `"npctarget"` matches nothing
/// and raises; it is deliberately absent from this list and handled separately below.
const UNIT_TOKEN_PREFIXES: [&str; 8] = [
    "player",
    "pet",
    "target",
    "partypet",
    "party",
    "raidpet",
    "raid",
    "mouseover",
];

/// Does 1.12's resolver RECOGNISE this token — whether or not it names a live unit?
///
/// This is the whole raise/nil distinction, so it is worth being exact about what it is not: it does
/// not ask whether a unit exists. `"party5"` on a solo character, `"pet"` with no pet, and
/// `"playerfoo"` are all *recognised* and answer nil; only a token the resolver's nine compares
/// never match raises `Unknown unit name`.
pub(crate) fn token_recognised(token: &str) -> bool {
    // `npc` full-string, everything else a prefix — and folded, like every compare in the resolver
    // (`SStrCmpI` -> `_strnicmp`, ASCII only; 1247).
    token.eq_ignore_ascii_case("npc")
        || UNIT_TOKEN_PREFIXES
            .iter()
            // BYTES, not a string slice. `token[..p.len()]` panics when the token is multibyte
            // UTF-8 and the prefix length lands mid-character ("byte index N is not a char
            // boundary") — an addon passing a non-ASCII token would take the client down. The
            // client's own compare is `_strnicmp` over bytes with an ASCII-only fold, so byte
            // comparison is both the safe form and the faithful one.
            .any(|p| {
                token.len() >= p.len()
                    && token.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
            })
}

/// The gate every `Unit*` binding puts an addon-supplied token through.
///
/// **An unrecognised token RAISES and does not return** — `0x515970` falls off the end of its nine
/// compares into `luaL_error(L, "Unknown unit name: %s")`, which longjmps (wow-re
/// `system/ui/scratch/unit-token-grammar.md`). Ours answered nil for everything unknown, which is
/// 1203's shape pointed the other way: a failure the client reports, silently swallowed.
///
/// The split is THREE-way, not two, and the two quiet legs are as carved:
///   * **absent argument** — quiet nil *here*, because this helper is only the resolver's half.
///     Whether a nil ever reaches it is the BINDING's question, and it is settled per binding:
///     wow-re's census of all 83 entries at `0x850438` found 53 that gate the token position with
///     `lua_isstring` and raise `Usage:`, against 13 unit-token bindings with no gate at all
///     `binding_abi::neutral_string_arg` before they get here, so a
///     nil never arrives; the quiet 13 pass it straight through. This comment used to say the
///     gates were "NOT uniform … only those two poles are verified" and decline to guess, which
///     was the right call at the time — the table now exists, so it is applied rather than feared;
///   * **`""`** — quiet nil;
///   * a **recognised** token naming nothing (`"party5"` solo, `"playerfoo"`) — quiet nil.
pub(crate) fn check_unit_token(token: &Option<String>) -> benilla_lua::Result<()> {
    if let Some(error) = unit_token_error(token) {
        return Err(benilla_lua::Error::runtime(error));
    }
    Ok(())
}

fn unit_token_error(token: &Option<String>) -> Option<String> {
    token
        .as_ref()
        .filter(|token| !token.is_empty() && !token_recognised(token))
        .map(|token| format!("Unknown unit name: {token}"))
}

/// Read a unit token's snapshot under a short model borrow, mapping it through `f`; `default` when the
/// token is absent (the "unit doesn't exist" path).
///
/// Raises for an unrecognised token ([`check_unit_token`]) before it looks anything up.
fn with_unit<T>(
    lua: &mut impl LuaContext,
    token: &Option<String>,
    default: T,
    f: impl FnOnce(&UnitState) -> T,
) -> benilla_lua::Result<T> {
    check_unit_token(token)?;
    let model = lua.app_data_ref::<Model>().expect("model app_data");
    Ok(match token.as_ref().and_then(|t| model.unit(t)) {
        Some(u) => f(u),
        None => default,
    })
}

/// A unit **predicate** binding's whole body: resolve the token, map its snapshot to a bool, and
/// push the family's one return shape — the number `1` or `nil`, never a Lua boolean.
///
/// [`with_unit`] answers whatever `f` answers, so a predicate written on it *directly* hands the
/// VM host a Rust `bool`, and the host pushes tag 1. That is the one shape 1.12 cannot produce: `UnitExists`
/// (`0x515fb0`) pushes `lua_pushnumber` (`0x6f3810`, tag 3, the double `1.0`) or `lua_pushnil`
/// (`0x6f37f0`, tag 0) and nothing else, and so does every other predicate in the family. So the
/// whole family goes through [`super::binding_abi::neutral_flag`] — 1830's widget law, which is the
/// *same* law, finally applied to the unit surface (decision 2043). A predicate that hand-computes
/// its own bool calls that helper directly; one that reads a snapshot field calls this. Both end at
/// the one push site, which is what stops the family drifting apart again.
///
/// **Not every unit binding is a predicate.** `UnitReaction` answers the reaction *number*,
/// `UnitLevel` a level (with `-1` for "can't be told"), `UnitPowerType` a power index whose miss is
/// the number `0` and never nil. Those keep their own shapes and must not be routed here.
fn unit_predicate(
    lua: &mut impl LuaContext,
    token: &Option<String>,
    f: impl FnOnce(&UnitState) -> bool,
) -> benilla_lua::Result<benilla_lua::Value> {
    Ok(super::binding_abi::neutral_flag(with_unit(
        lua, token, false, f,
    )?))
}

/// The `Unit*`/`GetQuestGreenRange` Lua binding registrations — split from this module's
/// state+laws half purely for size (same seam as the other script modules' install fns).
mod bindings;
#[cfg(test)]
mod tests;

pub(super) use bindings::install;
