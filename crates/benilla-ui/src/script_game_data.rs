//! Gameplay snapshots and intents shared by the native and browser FrameScript hosts.

use std::collections::HashMap;

/// `GetPetitionInfo`'s first return — so a charter reads *"Guild Name:"* / *"Guild Master:"* and a
/// plain petition reads *"Petition:"* / *"Created by"*.
///
/// **The third line, the signature count, is deliberately NOT here.** The law names it, but neither
/// the carve nor the packet says where the number comes from: the petition record carries no count,
/// and the only candidate — the item's `ITEM_FIELD_ENCHANTMENT` slot-0 *charges* dword, which
/// vmangos's own source documents as "the on-item signature count" — has its write **commented
/// out** server-side, so it is always zero and the guess could never be falsified here. Omitted
/// rather than invented; the line is one field away the day someone pins the source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PetitionSlotView {
    /// The record's charter bit: `true` picks the `GUILD_CHARTER_*` keys, `false` the `PETITION_*`
    /// ones.
    pub is_charter: bool,
    /// The proposed guild's (or petition's) name.
    pub title: String,
    /// The owner's name, app-resolved through the name cache. `None` while that query is in
    /// flight — the line is withheld, as the creator line is, rather than printed with a hole.
    pub owner: Option<String>,
}

/// One occupied bag slot, resolved by the app (icon from ItemDisplayInfo, count/quality from the
/// item object + template). Plain data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerSlot {
    /// Icon texture path (`Interface\Icons\…`); `None` while the template answer is in flight.
    pub texture: Option<String>,
    pub count: u32,
    /// Item quality 0..6; `None` while unresolved (the API reports `nil`).
    pub quality: Option<u32>,
    /// The item's template entry (`containerInfo.itemID`); 0 while unresolved.
    pub item_id: u32,
    /// An `|Hitem:…|h[Name]|h` link once the name is known; Bagnon's search reads it.
    pub link: Option<String>,
    pub locked: bool,
    /// The 1-based live-API inventory slot ids this item could be EQUIPPED into (empty = not
    /// equippable) — the app-resolved "fit rule" decision 0208 phase 1b's cursor arc needs
    /// (`cursor::CursorItem::equip_slots` captures it at pickup). Derived from the item
    /// template's `inventoryType` via `ui_items::find_equip_slot`; the engine holds no item
    /// knowledge of its own.
    pub equip_slots: Vec<u8>,
    /// Whether this item may be placed on an ACTION-BAR slot — app-resolved from the template
    /// exactly like [`Self::equip_slots`] (the engine holds no item knowledge of its own).
    /// `PlaceAction`'s only item filter, byte-read: an on-use spell OR equippable
    /// (`ItemInfo::placeable_on_action_bar`, wow-re `action-item-slot.md` §5 — decision 0666).
    pub bar_placeable: bool,
    /// The instance's live durability `(current, max)` — `ITEM_FIELD_DURABILITY`/`MAXDURABILITY`
    /// off the streamed item object; `None` for indestructible items (max 0) or while the create
    /// hasn't landed. The real-instance tooltip's "Durability X / Y" line reads it (a template
    /// hover keeps the template's full max/max).
    pub durability: Option<(u32, u32)>,
    /// The item's running use-cooldown as `(start_ms on the GetTime clock, duration_ms,
    /// enabled)` — the same app-computed triple [`super::ActionState::cooldown`] carries;
    /// `None` = cold. Stored at push ([`super::UiScript::set_container`]) so
    /// `GetContainerItemCooldown` answers the reference's `(start, duration, enable)`.
    pub cooldown: Option<(i64, u32, bool)>,
    /// Right-clicking reads this item (an instance carrying `ITEM_FIELD_ITEM_TEXT_ID` — a mail
    /// permanent copy). `GetContainerItemInfo`'s `isReadable`; the bag hover shows the Inspect
    /// magnifier off it (ref ContainerFrame.lua l.638 `this.readable → ShowInspectCursor()`),
    /// and the tooltip's WRITTEN_BY/READABLE gates key on it.
    pub readable: bool,
    /// The RESOLVED `ITEM_FIELD_CREATOR` name (app: ask-once name cache) — the tooltip's
    /// "Written by %s" (a letter) / "<Made by %s>" (anything crafted) line. `None` = authorless
    /// or the name query is in flight (no line; the re-push repaints the hover when it lands).
    pub creator: Option<String>,
    /// The instance's `ITEM_FIELD_FLAGS` — the tooltip's openable lock sub-gate reads
    /// UNLOCKED `0x4`, the wrapped-gift arm WRAPPED `0x8` (see
    /// [`super::char_stats::InvSlotView::flags`], the doll twin).
    pub flags: u32,
    /// `0x5da2c0` — **the instance is runtime-bound**: `ITEM_FIELD_FLAGS & 1` (soulbound), or a
    /// live enchant slot naming a `SpellItemEnchantment` row that binds. App-resolved off the raw
    /// descriptor ([`crate::items::already_bound`] in benilla-app — the same predicate the enchant
    /// cursor's bind question asks), because the binding half needs a DBC join and the engine
    /// holds no item knowledge of its own. The tooltip's §6 bind line overrides to **Soulbound**
    /// on it (B310); `false` on any source with no streamed item object.
    pub already_bound: bool,
    /// The instance's enchant slots, resolved by the app ([`super::EnchantView`]) and in
    /// enchant-slot order: it joins the id through `SpellItemEnchantment.dbc`'s name column and
    /// hands over the row's name plus the three facts the line law needs to place and paint it.
    /// Empty = unenchanted, or the enchant DBC never loaded. Decisions 0915/0920.
    pub enchants: Vec<EnchantView>,
    /// **The instance's remaining LIFETIME in milliseconds** — a duration-limited item counting
    /// down to its own destruction (a conjured stone, a holiday gift, a timed quest item). Its
    /// only feed is `SMSG_ITEM_TIME_UPDATE`, parked as an absolute deadline and recomputed on
    /// read ([`crate::items::Items::duration_remaining_display_ms`] in benilla-app), floored to
    /// the whole second so the snapshot moves once a second rather than every frame.
    /// `None` = no timer, which is nearly every item. Decision 1933.
    pub duration_ms: Option<u64>,
    /// The petition this item names, when it is a signable charter — **line 3 of the tooltip's
    /// emission law**, between the NAME and the green `ITEM_SIGNABLE` line.
    ///
    /// `None` for every ordinary item, and also for a charter whose petition record has not
    /// arrived: the hover is what ISSUES that query, so the first hover of an unopened charter
    /// shows the name and the green line, and the two guild lines appear on the repaint. Exactly
    /// [`Self::creator`]'s shape one field up, for exactly its reason.
    pub petition: Option<PetitionSlotView>,
}

/// One enchant slot as the tooltip renders it (wow-re `ui/scratch/tooltip-content-law.md` §E3,
/// byte-verified; decision 0920). The app resolves the DBC row; every rule below is the engine's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnchantView {
    /// `ITEM_FIELD_ENCHANTMENT` slot: **0** permanent · **1** temporary · **2..6** the
    /// random-property suffix. It decides the colour band — only 0 and 1 are ever coloured.
    pub slot: u8,
    /// The enchant id was NEGATIVE. `abs(id)` names the row either way — the sign's only effect is
    /// to paint slots 0/1 pure-red instead of green (`0x52ca29–0x52ca49`).
    pub negative: bool,
    /// The `SpellItemEnchantment` row's name, verbatim (`"Agility +15"`, `"Crusader"`) — the
    /// reference copies the string with no format at all (`0x52ca8b–0x52caa1`).
    pub name: String,
    /// The slot's `ITEM_FIELD_ENCHANTMENT` charges dword — nonzero appends " (N Charges)".
    pub charges: u32,
    /// Milliseconds left on a TEMPORARY enchant, from `SMSG_ITEM_ENCHANT_TIME_UPDATE` (the item's
    /// own duration field is never read for this). `Some` replaces the plain name with the
    /// countdown phrasing; `None` = no timer, or expired.
    pub remaining_ms: Option<u64>,
}

/// One `ItemRandomProperties` row as the engine renders it — the **random-suffix roll** a dropped,
/// linked, auctioned or mailed item carries (decision 1547). The app resolves the DBC (both of
/// them: the roll's table and the `SpellItemEnchantment` names its five ids land on) and pushes the
/// whole table once at load; a tooltip source supplies only the id, exactly as in the reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RandomPropertyView {
    /// The roll's suffix — `"of the Monkey"`. The reference joins it onto the item name through
    /// `ITEM_SUFFIX_TEMPLATE` (`"%s %s"`) in its one name formatter `0x5d8b00`; benilla does that
    /// join app-side, where every display name is composed, so this is carried for reference and
    /// for the surfaces that name an item the app never composed.
    pub suffix: String,
    /// The enchant lines the roll grants, already named and seated in enchant slots **2..6** —
    /// the five dwords the builder copies out of the suffix row (`0x52b7e0–0x52b7fb`). Slots 2..6
    /// are always white, whatever the sign.
    pub enchants: Vec<EnchantView>,
}

/// One bag's snapshot: its name, capacity, and occupied slots (1-based).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContainerState {
    /// `GetBagName` (the bag item's own name; the backpack is the client-local "Backpack").
    pub name: Option<String>,
    pub num_slots: u32,
    pub slots: std::collections::HashMap<u32, ContainerSlot>,
}

/// One queued backpack pick/place/swap/split: move the item from `(src_bag, src_slot)` to
/// `(dst_bag, dst_slot)` (live-API space, 1-based slots). The app maps a backpack-internal move
/// (both bags 0) onto `CMSG_SWAP_INV_ITEM` player-array slots. `count`: `None` = a whole-stack
/// move/swap (including a same-item merge — the wire tops the stack up itself); `Some(n)` = a
/// split placement (`SplitContainerItem`) the app maps onto `CMSG_SPLIT_ITEM` instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMove {
    pub src_bag: i64,
    pub src_slot: u32,
    pub dst_bag: i64,
    pub dst_slot: u32,
    pub count: Option<u32>,
}

/// One queued **auto-store into a bag** — `PutItemInBag`'s third leg and the whole of
/// `PutItemInBackpack` (`crate::script::cursor::bag_verbs`): take the item at `(src_bag, src_slot)`
/// and put it anywhere inside container `dst_bag`.
///
/// **There is no destination slot, and that is the finding, not an omission**: the reference's
/// `CMSG_AUTOSTORE_BAG_ITEM 0x10B` carries `(srcbag, srcslot, dstbag)` and nothing else, and its
/// split sibling `CMSG_SPLIT_ITEM 0x10E` carries the literal `0xFF` where a slot would go. The
/// server picks. `count`: `None` = the whole stack (AUTOSTORE); `Some(n)` = a split carry, which
/// the drain sends as SPLIT instead — the same fork the reference makes on `[0xb4b40c]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BagAutoStore {
    pub src_bag: i64,
    pub src_slot: u32,
    /// Live-API container id (0 = backpack, 1..=4 an equipped bag, 5..=10 a bank bag).
    pub dst_bag: i64,
    pub count: Option<u32>,
}

/// One skill-line tab (`GetSpellTabInfo`'s own Era tuple shape). `offset` is the tab's 0-based
/// START index into [`SpellBookState::slots`] (module docs' book-id seam) — pushed by the app,
/// trusted here (the engine holds no spell knowledge to derive it from itself).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellTabView {
    pub name: String,
    pub texture: Option<String>,
    pub offset: u32,
    pub num_spells: u32,
}

/// One spell in the flat book (0-based [`SpellBookState::slots`] index; module docs' book-id
/// seam). Every field is pre-resolved by the app — the engine draws whatever it's given.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellSlotView {
    pub spell_id: u32,
    pub name: String,
    /// The rank/subtext line (`Spell.dbc`'s `NameSubtext`, `benilla-formats`' own pin); `None`
    /// shows no second line.
    pub rank: Option<String>,
    pub texture: Option<String>,
    /// `SPELL_ATTR_PASSIVE` (`benilla-formats`' `SpellDisplay::passive`) — grays the name in the
    /// transcribed XML and refuses both [`CastSpell`]-family casts (this module) and, faithfully,
    /// nothing else: a passive can still be picked up/placed on a bar (the ref never blocks that).
    pub passive: bool,
    /// The `IsCurrentCast` verdict for this slot — the checked ring (`SpellButton_UpdateSelection`'s
    /// gold glow). The delegate `0x4b3600` has exactly two arms (wow-re
    /// `spellbook-checked-predicate.md`): a shapeshift spell whose form is the player's current
    /// form byte, or the open trade-skill window's own spell — never an ordinary in-flight cast.
    /// App-resolved (`benilla::ui_spellbook`), pushed with the book; the app fires
    /// `CURRENT_SPELL_CAST_CHANGED` on its edges.
    pub current: bool,
    /// The spell's running cooldown as `(start_ms on the GetTime clock, duration_ms, enabled)` —
    /// the same app-computed triple [`super::ActionState::cooldown`] and the container slots
    /// carry, resolved by the ONE cooldown store (`benilla::cooldowns::Cooldowns::info` — id,
    /// category and GCD reads alike); `GetSpellCooldown` answers the reference's
    /// `(start, duration, enable)`. `None` = cold. Frame-stable per arm (the absolute start), so
    /// a running cooldown never churns the book diff.
    pub cooldown: Option<(i64, u32, bool)>,
    /// `GetSpellAutocast`'s `(allowed, enabled)` pair — **pet-book only**, and `None` is not
    /// "neither": it is the reference's own player-book answer, `(nil, nil)`, because `0x4b4180`
    /// short-circuits on the book flag (`0x4b41cb`/`0x4b41d6`) before it looks a spell up at all.
    /// Read off the pet's **raw** word (`0x4bd160` → bits 31/30), never off the filtered book.
    pub autocast: Option<(bool, bool)>,
    /// The pet slot's packed word **verbatim** — what `PickupSpell(id, "pet")` puts on the cursor.
    /// `0x4b3260`'s pet arm hands `0x494e20` a *pointer* to this very word, so the payload is the
    /// server's own dword, type byte and autocast bits included, not a synthesized one. `0` for a
    /// player-book slot, which has no word.
    pub packed: u32,
}

/// The **pet's** book — the reference's second flat array (`0xb6f098`, count `0xb71174`), which is
/// a genuinely different object from the player's rather than a variant of it:
///
/// - **no tabs.** `GetNumSpellTabs`/`GetSpellTabInfo` take no `bookType` and only ever answer the
///   player's skill lines; `SpellBookFrame_Update` hides every skill-line tab while the pet book is
///   up (`SpellBookFrame.lua:124`), and `SpellBook_GetSpellID` returns the button's own 1..12 id
///   with no tab offset at all (`l.460-462`).
/// - **a different add-gate.** `0x4b2f90` admits a spell iff it resolves in `Spell.dbc` **and**
///   `Attributes & 0x80` (DO_NOT_DISPLAY) is clear — `0x4b2fa8 mov dl,[rec+0x18]; test dl,dl; js`.
///   That is *one* of the three tests the player book's own gate makes: no `IS_TRADESKILL` leg and
///   no `castUI == 0` leg. Reusing the player book's gate here would be the same class of mistake
///   as reusing its tab routing.
/// - **the same order.** `0x4b2fd0(ecx = 0, edx = 1)` sorts it with `0x4b30c0`, the player book's
///   own comparator (name, then parsed rank), and tail-jumps `SPELLS_CHANGED`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PetBookState {
    /// `HasPetSpells`'s second return: `ChrClasses.dbc` field 4 for the player's class — `"PET"`
    /// or `"DEMON"` (`benilla_formats::PetNameTokens`). A **key**, not display text: FrameXML does
    /// `getglobal("PET_TYPE_"..token)`. `None` only while there is no book at all.
    pub token: Option<String>,
    pub slots: Vec<SpellSlotView>,
}

/// The player's known-spell book: tabs (skill lines) + the flat slot list every tab indexes into
/// (module docs). Durable player state, not a session window (like [`super::action`]'s
/// `actions` map) — never `Option`; "no known spells yet" is simply empty vectors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpellBookState {
    pub tabs: Vec<SpellTabView>,
    pub slots: Vec<SpellSlotView>,
}

/// Resolve a non-passive spell by whole name and optional `(subtext)`, preferring the highest
/// known rank when the query does not select one explicitly.
pub fn resolve_spell_by_name<'a>(
    book: &'a SpellBookState,
    query: &str,
) -> Option<&'a SpellSlotView> {
    let (name, subtext) = split_subtext(query);
    if name.is_empty() {
        return None;
    }
    book.slots
        .iter()
        .filter(|slot| !slot.passive && slot.name.eq_ignore_ascii_case(name))
        .filter(|slot| match subtext {
            Some(want) => slot
                .rank
                .as_deref()
                .is_some_and(|rank| rank.eq_ignore_ascii_case(want)),
            None => true,
        })
        .enumerate()
        .max_by_key(|(index, slot)| (rank_number(slot.rank.as_deref()), *index))
        .map(|(_, slot)| slot)
}

fn split_subtext(query: &str) -> (&str, Option<&str>) {
    let query = query.trim();
    let Some(open) = query.find('(') else {
        return (query, None);
    };
    let Some(close) = query.rfind(')') else {
        return (query, None);
    };
    if close < open {
        return (query, None);
    }
    (query[..open].trim(), Some(query[open + 1..close].trim()))
}

fn rank_number(subtext: Option<&str>) -> u32 {
    subtext
        .unwrap_or("")
        .chars()
        .skip_while(|character| !character.is_ascii_digit())
        .take_while(|character| character.is_ascii_digit())
        .fold(0, |rank, character| {
            rank * 10 + character.to_digit(10).unwrap_or(0)
        })
}

/// An item template's tooltip view (decision 0274 P1) — the fields the line law consumes, in
/// wire terms (vmangos `SMSG_ITEM_QUERY_SINGLE_RESPONSE`), plus app-resolved display strings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemTemplateView {
    /// The name line (quality-colored).
    pub name: String,
    pub quality: u32,
    /// Item class/subclass (the slot line's right column) + the equip slot (its left).
    pub class: u32,
    pub subclass: u32,
    pub inventory_type: u32,
    /// `GetItemInfo`'s `itemType`: what [`Self::class`] is **called** — "Weapon", "Container",
    /// "Trade Goods" — app-resolved from `ItemClass.dbc` (byte-verified: `0x48e070` pushes
    /// `[classRow + 4*locale + 0xc]`). `None` for a class with no row, which the reference
    /// renders as the empty string.
    pub item_type: Option<String>,
    /// `GetItemInfo`'s `itemSubType`: what `(class, subclass)` is **called** — app-resolved from
    /// `ItemSubClass.dbc`, **VerboseName first with a DisplayName fallback**
    /// ([`benilla_formats::ItemSubClassCatalog::name`]), which is the binding's own two-step at
    /// `0x48e311`: read `[row + 4*locale + 0x4c]` and use it unless its first byte is 0, else
    /// `[row + 4*locale + 0x28]`.
    ///
    /// **This is a different spelling from the tooltip's type cell**, which reads `+0x28`
    /// (DisplayName) only — a one-handed sword is "One-Handed Swords" here and "Sword" there.
    pub item_sub_type: Option<String>,
    /// What `(class, subclass)` is called on the TOOLTIP's own type cell — `ItemSubClass.dbc`'s
    /// **DisplayName** alone (`row + 4*locale + 0x28`, column 10: the read the builder makes at
    /// `0x52c0xx` off the `0xc0db90` row cache), which is the SINGULAR spelling: "Sword", not
    /// [`Self::item_sub_type`]'s "One-Handed Swords". App-resolved, because the engine reads no
    /// DBCs — and it is the same string the bag line's `CONTAINER_SLOTS` second hole takes.
    ///
    /// This used to be a hand-typed `(class, subclass) → &str` table in the renderer, which is
    /// the shape decision 2080 caught one file over: a stale copy of data the catalog already
    /// held, missing every container family outright.
    pub sub_class_display: Option<String>,
    /// The alternate subclass whose proficiency also permits use (ItemSubClass.dbc
    /// prerequisite/postrequisite, app-resolved with the builder's sentinel walk: prerequisite
    /// wins, postrequisite only when prerequisite is −1). A weapon missing its own mask bit but
    /// holding the alternate's reds the SLOT cell instead of the type cell.
    pub proficiency_alt: Option<u32>,
    /// ItemSubClass displayFlags bit 0 (app-resolved): the type cell never prints — the
    /// "Miscellaneous" family (rings, trinkets, shirts, consumables, recipes).
    pub hide_subclass: bool,
    /// Template flags — bit `0x2` = conjured ("Conjured Item").
    pub flags: u32,
    /// Bonding: 1 = binds on pickup, 2 = on equip, 3 = on use, 4/5 = quest item.
    pub bonding: u32,
    /// `MaxCount` — 1 = "Unique", N > 1 = "Unique (N)", 0 = no line. **Not the stack size**; see
    /// [`Self::stackable`].
    pub max_count: u32,
    /// `Stackable` — the biggest stack one slot can hold (1 = doesn't stack). `GetItemInfo`'s
    /// `itemStackCount`, read from the cache record's `+0x60` at `0x48e28b` — the dword *after*
    /// `MaxCount` at `+0x5c`. Linen Cloth is `maxcount 0, stackable 20`, so the two are not
    /// interchangeable in either direction.
    pub stackable: u32,
    /// Nonzero = "This Item Begins a Quest".
    pub start_quest: u32,
    /// Container size — "N Slot Bag".
    pub container_slots: u32,
    /// Stat mods `(type, value)` in wire order (types: 0 mana, 1 health, 3 agi, 4 str, 5 int,
    /// 6 spi, 7 stam — the `ITEM_MOD_*` GlobalStrings family).
    pub stats: Vec<(u32, i32)>,
    /// Damage blocks `(min, max, school)` in wire order; school 0 physical, 1..6 =
    /// Holy/Fire/Nature/Frost/Shadow/Arcane.
    pub damages: Vec<(f32, f32, u32)>,
    pub delay_ms: u32,
    pub armor: u32,
    pub block: u32,
    /// Holy..Arcane (armor is its own field), the "+N X Resistance" lines.
    pub resistances: [i32; 6],
    /// "Durability N / N" (a template hover shows full).
    pub max_durability: u32,
    /// "Requires Level N" — printed only for N > 1 (the real builder's `0x52d2cf` gate); red
    /// when the player is lower.
    pub required_level: u32,
    /// Class/race masks; `<= 0` = everyone (no line). Red when the player's bit is absent.
    pub allowable_class: i32,
    pub allowable_race: i32,
    /// Skill requirement: the SkillLine id + rank, with the display name app-resolved
    /// (`SkillLine.dbc`). `required_skill_name = None` = no skill line.
    pub required_skill: u32,
    pub required_skill_rank: u32,
    pub required_skill_name: Option<String>,
    /// Spell requirement (nonzero = "Requires <name>") — red when the spellbook doesn't know it.
    pub required_spell: u32,
    pub required_spell_name: Option<String>,
    /// `RequiredHonorRank` — no tooltip line in 1.12, but the item-usable gate (`0x5ea930`)
    /// compares it against the player's highest honor rank; the merchant list reds on it.
    pub required_honor_rank: u32,
    /// `RequiredCityRank` — usable-gate only, like the honor rank. Vanilla data ships no nonzero
    /// value and vmangos never writes the `PVP_MEDALS` bits the client would test, so a nonzero
    /// requirement can only fail (see [`item_usable`]).
    pub required_city_rank: u32,
    /// Reputation requirement, app-resolved to "Requires <Faction> - <Standing>"; the raw
    /// faction id + rank ride along for the red check against [`PlayerReqState::rep_ranks`].
    pub required_rep_line: Option<String>,
    pub required_rep_faction: u32,
    pub required_rep_rank: u32,
    /// Trigger-spell lines `(trigger, spell id, display text)` in wire order: trigger 0/5 =
    /// "Use:", 1 = "Equip:", 2 = "Chance on hit:", 6 = a taught spell (the "Already known" red
    /// check, no green line). The text is app-resolved (the spell's name in P1; its substituted
    /// description in P2) — green lines.
    pub spell_triggers: Vec<(u32, u32, String)>,
    /// `LockID` — nonzero prints the red "Locked" line (the key-item sub-line joins with the
    /// Lock.dbc resolve, the GO-locks follow-up).
    pub lock_id: u32,
    /// "N Charge(s)" — the app-resolved count for the first spell slot that survives the real
    /// builder's charge gate (`0x52db51`: a slot whose value is 0 or the `-1` consume-on-use
    /// sentinel prints nothing; else `abs`). 0 = no line.
    pub charges: i32,
    /// The yellow quoted flavor text (wrapped).
    pub description: String,
    /// Nonzero = "<Right Click to Read>" (green).
    pub page_text: u32,
    /// Copper — the merchant-open money row (the engine fires `OnTooltipAddMoney`), and
    /// `ITEM_UNSELLABLE` when 0 in a sell context.
    pub sell_price: u32,
    /// `itemset` — nonzero renders the SET block ([`ItemSetView`], asked once by set id).
    pub item_set: u32,
    /// The inventory icon as a ready `Interface\Icons\…` path — app-resolved through
    /// `ItemDisplayInfo.dbc` off the template's `display_info_id`, the same lookup the bag slots
    /// use. `GetItemInfo`'s `itemTexture`; the reference builds the identical string at
    /// `0x48e2dd` (`"%s%s%s"` over the icon directory, `"\\"`, and the row's icon name).
    /// `None` on the ~26% of display rows that carry no icon.
    pub icon: Option<String>,
    /// `RandomProperty` (template `+0x1b8`) — the item CAN roll a "… of the Bear" suffix. Its one
    /// consumer is the enchant family's third arm: with no instance to read a roll from, the
    /// tooltip prints the `<Random enchantment>` placeholder instead of any per-slot line (wow-re
    /// §1-ENCHANT §E5). Decision 0920.
    pub random_property: u32,
}

/// The player state the red-line law compares against (decision 0274 P1): pushed by the app
/// whenever it changes. Level and class also ride the `"player"` unit feed; this carries the
/// pieces that don't (the class/race IDS as mask bits, and the skill ranks).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerReqState {
    pub level: u32,
    /// Class id (1 warrior … 11 druid) — the `allowable_class` mask bit is `1 << (id-1)`.
    pub class_id: u32,
    /// Race id (1 human … 8 troll) — the `allowable_race` mask bit is `1 << (id-1)`.
    pub race_id: u32,
    /// SkillLine id → current rank, for "Requires <skill> (N)" checks.
    pub skills: HashMap<u32, u32>,
    /// Item class (2 weapons / 4 armor) → allowed-subclass bitmask (`SMSG_SET_PROFICIENCY`,
    /// the client's `0xc4d4a0[class]` store). The slot-line proficiency red: a class WITH an
    /// entry here reds when the item's `1 << subclass` bit is absent; a class with no entry
    /// (consumables etc.) never reds.
    pub proficiency: HashMap<u32, u32>,
    /// Faction id → the player's reputation rank (0 hated … 7 exalted; DBC base + wire
    /// standing, app-ranked) — the "Requires <Faction> - <Standing>" red check.
    pub rep_ranks: HashMap<u32, u8>,
    /// Whether the spellbook holds an effect-40 (SPELL_EFFECT_DUAL_WIELD) spell — the client's
    /// `0xc4d770` global (stored on learn, cleared on unlearn; reader `0x5eab70`). An off-hand
    /// weapon (InventoryType 22) reds its SLOT cell without it.
    pub can_dual_wield: bool,
    /// The player's **highest lifetime honor rank** (`PLAYER_FIELD_BYTES` byte 3) — the
    /// usable-gate's `RequiredHonorRank` comparand.
    pub honor_rank: u8,
}

/// [`item_usable`] for an item the caller knows only by **entry** — the shape every window seam
/// wants, since a row carries an id and the engine holds the templates.
///
/// Two conventions are folded in here rather than re-derived per caller: entry `0` (a row with no
/// item at all) is usable, and a template the app has not answered for yet is usable, which is the
/// getter's own null-record skip. It was four identical private copies across the merchant, mail,
/// trade and auction seams before it was one — three had the `item_id == 0` leg and the fourth

/// An item set's tooltip view (the §22 SET block), app-resolved: the ItemSet.dbc row with
/// member item NAMES joined from the template cache (a `None` name = the member's template is
/// still in flight — its line waits; the app re-pushes as answers land) and the threshold
/// bonuses' TEXT ($-substituted spell descriptions). The engine supplies the live half: the
/// owned/equipped counts off its own inventory slots.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemSetView {
    pub name: String,
    /// `(item id, resolved name)` per member, DBC order.
    pub members: Vec<(u32, Option<String>)>,
    /// `(required equipped count, bonus text)` per bonus, in the DBC's stored slot order —
    /// the renderer sorts threshold-ascending at print time, like the builder's qsort
    /// (`0x52e5c0`; The Gladiator ships 3,2,5,4 and prints 2,3,4,5).
    pub bonuses: Vec<(u32, String)>,
    /// The set-level skill requirement ("Requires <skill> (N)"), app-named; 0/None = no line.
    pub required_skill: u32,
    pub required_skill_rank: u32,
    pub required_skill_name: Option<String>,
}

///
/// **Resolved strings, not the raw bytes**, because this crate has no DBC: the reference resolves
/// `+0x100`/`+0x101` through `ChrRaces`/`ChrClasses` at call time, and the app does that same
/// resolution once when it seeds the record (it is the identical lookup
/// `ui_unit::race_names`/`class_names` already does for the unit snapshot, so the two cannot
/// disagree about what a race byte means).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerRecord {
    /// `+0x08` — the NUL-terminated name buffer at `0xc27d88`. Empty is the record's only unset
    /// state, and the reference's only `UnitName("player")` nil: `lua_pushstring(NULL)` at
    /// `0x517095` tail-jumps into `lua_pushnil`. Unreachable from Lua in the real client, because
    /// the verb is not registered until `UI_Init` and only an Enter World commit gets there.
    pub name: String,
    /// `+0x100` resolved — `UnitRace`'s `(localized, file token)` pair, e.g.
    /// `("Night Elf", "NightElf")`. `None` is the all-zero record: race 0 has no `ChrRaces` row,
    /// and the reference's bound/NULL-row arms push `nil, nil` rather than reaching the resolver.
    pub race: Option<(String, String)>,
    /// `+0x101` resolved — `UnitClass`'s `(localized, UPPERCASE token)` pair, e.g.
    /// `("Warrior", "WARRIOR")`. `None` for the all-zero record, same shape as [`Self::race`].
    pub class: Option<(String, String)>,
    /// `+0x102` on `UnitSex`'s scale (2 male, 3 female) — **not** the wire's 0/1.
    ///
    /// `0` is our unset marker and reads as `2`, which is not a fudge: `0x517ef9`'s accessor feeds
    /// `fild [4*eax+0x808be4]` over `{2,3,1,6}` with **no bounds check at all**, so an all-zero
    /// record answers `2` ("male") in the reference too. It is the one of the four with no
    /// validity guard.
    pub sex: u8,
}

/// A unit's resolved guild membership as returned by `GetGuildInfo`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitGuild {
    pub name: String,
    pub rank_name: String,
    pub rank_index: u32,
}

/// One unit-token's game-state snapshot, pushed by the app each frame and read by the `Unit*`
/// bindings. Plain data (no mlua handles, no ECS types) — the engine-free seam decision 0068 §3
/// draws between the app's net/ECS feed and the Lua API.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitState {
    /// Whether the unit exists (`UnitExists`). A token with no snapshot is treated as `false` too.
    ///
    /// **Stays TRUE for an out-of-range party member** — that is the whole point of the roster
    /// fallback, and it is what separates this field from [`Self::has_object`].
    ///
    /// *Named gap, second order:* 1.12's `UnitExists 0x515fb0` is a **conjunction** — the object
    /// lookup AND a virtual `[edx+0x58]` that resolves to `IsSelectable 0x60be60`
    /// (`!(UNIT_FIELD_FLAGS & 0x02000000) || UNIT_FIELD_CREATEDBY == you`) — falling back to the
    /// GUID-only roster test on a miss. We model the fallback and not the conjunct, so a
    /// not-selectable unit created by someone else reads `exists` here and `nil` there. No corpus
    /// caller reaches it; recorded rather than folded in (wow-re
    /// `ui/scratch/unitisvisible-object-presence.md`).
    pub exists: bool,
    /// **`UnitIsVisible` — the client holds a live object for this token, and nothing else.**
    ///
    /// The reference is 57 bytes with one branch (`0x516030`):
    /// `ClntObjMgrObjectPtr(resolve(token), TYPEMASK_UNIT) != NULL`. Its body reads **zero** object
    /// fields, makes **zero** comparisons beyond `test eax,eax`, and contains **zero** float
    /// opcodes — so there is no distance term, no radius, no alpha/fade/interpolation flag and no
    /// "known to the client" bit in it. Addons read it as a range gate because **the range test is
    /// the server's**: `SMSG_DESTROY_OBJECT` and the OUT_OF_RANGE demotion unlink the object from
    /// the manager index this query searches, so the client answers with the presence of the
    /// object the server chose to send.
    ///
    /// Distinct from [`Self::exists`], and neither implies the other. The one that matters here:
    /// an out-of-range party member is `exists == true`, `has_object == false` — which is exactly
    /// pfUI's `if not UnitIsVisible(unitstr) or not UnitIsConnected(unitstr)` portrait branch.
    ///
    /// The app already computed this and did not publish it: `ui_party::feed`'s
    /// `member_unit_state` takes `store: Option<&ObjectStore>` and its own comment calls that
    /// argument "the reference's `0x468460`". Every snapshot built from a live store sets it
    /// (`ui_unit::snapshot`); the roster-record leg and a token with no snapshot leave it `false`.
    ///
    /// **One presence field serves three verbs** — `UnitIsVisible`, and the object-presence
    /// conjunct of both tapped predicates below.
    pub has_object: bool,
    /// `UnitIsTapped` — `UNIT_DYNAMIC_FLAGS` (UpdateField **143**) bit `0x4`: this unit is
    /// someone's kill credit. `0x519c90` is `test BYTE PTR [eax+0x224],0x4` behind the same
    /// object-presence check [`Self::has_object`] carries, and **nothing else** — no ownership, no
    /// GUID compare, no party/raid or health conjunct anywhere in the body (wow-re
    /// `ui/scratch/tapped-bits-and-unit-faction.md`).
    ///
    /// Set only on the live-descriptor leg, so an out-of-range unit reads `false` — which is the
    /// object-presence conjunct, for free, by the same route `has_object` gets it.
    pub tapped: bool,
    /// **`UnitIsPartyLeader`'s FIRST leg** — the server's `PLAYER_FLAGS & 0x1` on this unit's own
    /// descriptor, restricted to players (`TYPEMASK_PLAYER`, `0x10` — **not** this family's usual
    /// `8`, which is the one difference the predicate makes to the object lookup).
    ///
    /// Not derived client-side, and not the same question as "leads MY group": it is true for a
    /// stranger who leads their own party, which no comparison against our group's leader GUID can
    /// express. The second leg is that comparison, in `script::party` — the two cover disjoint
    /// failures and the reference ORs them (wow-re
    /// `ui/scratch/party-leader-and-nameplate-verbs.md`).
    pub group_leader: bool,
    /// `UnitIsTappedByPlayer` — the same field's bit `0x8` (`0x519d00`, a masked-byte clone of its
    /// sibling: 108 bytes each, differing only in the mask and the `Usage:` string).
    ///
    /// Read as a **pair**: `tapped && !tapped_by_player` is "someone else's kill", which is the
    /// grey-bar condition unit-frame addons draw (pfUI `api/unitframes.lua:2012`).
    pub tapped_by_player: bool,
    /// The unit's name (`UnitName` → the string, or nil). `None` while the app's name-query cache
    /// hasn't answered (or the server doesn't know the unit).
    pub name: Option<String>,
    /// Current health (`UnitHealth`).
    pub health: u32,
    /// Maximum health (`UnitHealthMax`).
    pub max_health: u32,
    /// Level (`UnitLevel`).
    pub level: u32,
    /// The active power type (`UnitPowerType`): `0` mana, `1` rage, `2` focus, `3` energy,
    /// `4` happiness — the descriptor's `UNIT_FIELD_BYTES_0` byte 3.
    pub power_type: u8,
    /// Current power of the active type (`UnitMana`).
    pub power: u32,
    /// Maximum power of the active type (`UnitManaMax`).
    pub max_power: u32,
    /// Whether the unit is dead (`UnitIsDead`). NB a released ghost is NOT dead by this
    /// predicate — its wire health is 1 (decision 0308 §1); the trio is dead / [`Self::ghost`] /
    /// dead-or-ghost, the real client's three tests.
    pub dead: bool,
    /// Whether somebody is charming this unit (`UnitIsCharmed`).
    ///
    /// **`UNIT_FIELD_CHARMEDBY != 0`, and the asymmetry is the fact worth keeping.** The binding
    /// `0x516cf0` reads fields 10/11 as one 64-bit value and tests it non-zero — not a
    /// `UNIT_FIELD_FLAGS` bit, and not a comparison against the player (its body contains no `cmp`
    /// at all). The field means *"who charms me"*; its mirror `UNIT_FIELD_CHARM` (*"whom I charm"*)
    /// is never read here. So a **charmer** answers nil, a **charmed** pet answers 1, and an
    /// ordinary **summoned** pet answers nil — `"pet"` is itself charm-else-summon in the resolver.
    /// (wow-re `system/ui/scratch/unit-verbs-controlled-charmed-creaturetype.md`.)
    pub charmed: bool,
    /// Whether the unit is a released ghost (`UnitIsGhost` — `PLAYER_FLAGS` bit 0x10, decision
    /// 0308 §1). Only meaningful for player tokens; creatures stay `false`.
    pub ghost: bool,
    /// The unit's reaction toward the player on the `UnitReaction` scale (`1` hated … `4` neutral …
    /// `8` exalted; `0` = unknown → `UnitReaction` returns nil). The app resolves it from the
    /// faction-template / reputation decode (the same one the selection ring uses) for the `"target"`
    /// token; other tokens leave it `0`. Read by `UnitReaction`, which the target frame tints its
    /// name plate by (`TargetFrame_CheckFaction`).
    pub reaction: u8,
    /// The unit's localized race display name (`UnitRace`'s first return, e.g. "Night Elf").
    /// `None` = unknown (the race byte hasn't streamed, or a raceless creature) → `UnitRace`
    /// returns nil, nil. The app resolves it from `UNIT_FIELD_BYTES_0` byte 0 (decision 0208 §3).
    pub race: Option<String>,
    /// The race's file/token name (`UnitRace`'s second return, e.g. "NightElf" — 1.12's 8 races),
    /// the shape texture-path splices key on.
    pub race_file: Option<String>,
    /// The unit's localized class display name (`UnitClass`'s first return, e.g. "Warrior").
    pub class: Option<String>,
    /// The class's file name (`UnitClass`'s second return): the **uppercase** token ("WARRIOR")
    /// the ref's stat-tooltip lookups key on (`getglobal(strupper(class).."_STAT_TOOLTIP")` —
    /// the ref uppercases it again defensively; we store it already uppercase).
    pub class_file: Option<String>,
    /// This unit's INVSLOT 17 is a **relic** slot rather than a ranged-weapon slot —
    /// `UnitHasRelicSlot`'s whole answer (`0x519e50`).
    ///
    /// Resolved app-side off `ChrClasses.dbc` field 16, for the same reason [`Self::class_file`]
    /// is resolved app-side: it is a static per-class column of a table this crate cannot read.
    /// The reference gates on TYPEMASK_PLAYER before it looks at the class byte, so the feed sets
    /// this only for a player — a creature, a pet and an unstreamed unit all leave it `false`,
    /// which is the nil leg.
    ///
    /// True for exactly Paladin, Shaman and Druid in 1.12 (Libram, Totem, Idol). It was believed
    /// here for a long time that no 1.12 class had one; see decision 1796 for how that survived.
    pub has_relic_slot: bool,
    /// The unit's sex on the `UnitSex` scale: `2` male, `3` female (`1` = neuter/unknown). `0`
    /// (the unfilled default) reports as nil — the API's "can't tell", like [`Self::reaction`].
    /// The app maps `UNIT_FIELD_BYTES_0` byte 2 (0 male / 1 female) onto this shape.
    pub sex: u8,
    /// This token is a PLAYER character (guid family) — the unit tooltip's level line renders
    /// "Race Class (Player)" instead of the creature type (decision 0276's verified law).
    pub is_player: bool,
    /// This token is **player-CONTROLLED** — `UNIT_FIELD_FLAGS` bit 3
    /// (`UNIT_FLAG_PVP_ATTACKABLE 0x8`, behaviourally "player-controlled"; the same bit
    /// `target::relations::ring_reaction`'s duel leg selects on). Wider than
    /// [`Self::is_player`]: a player's pet and a charmed creature are player-controlled without
    /// being players, which is exactly the distinction stock `UnitFrame_OnEnter`
    /// (UnitFrame.lua:58) needs when it decides whether the hovered plate gets the
    /// player-options newbie tip. `UnitIsPlayer`'s note used to call this reach a gap we did not
    /// carry; the unit-frame migration is what closed it.
    pub player_controlled: bool,
    /// `UNIT_FIELD_FLAGS`, raw — what `UNIT_FLAGS` fires on (the app's `fire_transitions`); the
    /// three readings above are bits of it.
    pub flags: u32,
    /// `PLAYER_FLAGS` (descriptor index 190), raw — what **`PLAYER_FLAGS_CHANGED`** fires on, the
    /// [`Self::flags`] pattern one field over. `0` on a creature, which has no PLAYER block.
    ///
    /// The reference watches this dword type-wide over TYPEID_PLAYER (`0x468070(ecx=4, edx=8,
    /// width 4)` registered at `0x5e25d7`), and its handler `0x5ee990` fires the event from
    /// `0x5eea35`-`0x5eea3d` — **unguarded by any bit test, and above the local-player GUID gate
    /// at `0x5eea93`** — so *any* bit moving on *any* player announces itself, remote players
    /// included. Decision 2078.
    ///
    /// Raw, not a decoded subset, and that is the point: [`Self::group_leader`] (`0x1`) and
    /// [`Self::ghost`] (`0x10`) are the only bits this struct decodes, while `0x2`/`0x4` (the
    /// chat AFK/DND flags), `0x8` (GM), `0x200` (PvP-desired), `0x400`/`0x800` (hide helm/cloak)
    /// and `0x1000`/`0x2000` (the play-time regimes) all move without touching either. Firing off
    /// the decoded pair would silently under-announce every one of them.
    pub player_flags: u32,
    /// `UNIT_DYNAMIC_FLAGS` (descriptor index 143), raw — what **`UNIT_DYNAMIC_FLAGS`** the
    /// *event* fires on, the [`Self::flags`] pattern a hundred fields over.
    ///
    /// The event is id **137**, and the id is not arbitrary: the reference's generic bridge
    /// `0x51bbb0` registers one watch per unit-window field whose name-table slot is non-NULL, and
    /// the id it dispatches is that field's own window index — `137 + 6 = 143`, this field.
    /// Byte-verified here rather than taken from the note: `0xbe1198 + 137*4 = 0xbe13bc` has
    /// exactly one writer image-wide (`0x51ad8b`), and the pointer it stores resolves to
    /// `"UNIT_DYNAMIC_FLAGS"`. The watch **length** is 4 — selector byte `[0x51bc98 + 137] = 4`,
    /// jump-table entry `0x51bbf9 mov eax,4` — so the gate is a `repe cmpsb` over exactly this one
    /// dword against the object's shadow copy (`0x4655bb`), and **any** bit moving fires it.
    ///
    /// Raw for the same reason [`Self::player_flags`] is, and the cost of getting it wrong is
    /// larger here: this struct decodes four of its bits ([`Self::tapped`],
    /// [`Self::tapped_by_player`], and — through the store — lootable `0x1` and dead-looking
    /// `0x20`), while `0x2` (tracked, Hunter's Mark) and `0x10` moves without touching any of
    /// them. Firing off the decoded subset would silently under-announce those.
    pub dynamic_flags: u32,
    /// The unit's owner — `UNIT_FIELD_SUMMONEDBY`, else its charmer, else its creator; `0` for
    /// nobody's. What `UnitPlayerOrPetInParty`/`InRaid` read for the "or pet" half (1958).
    pub owner: u64,
    /// The unit's guild membership (`GetGuildInfo(unit)`, decision 1257). `None` = guildless, or
    /// a creature, or a player whose `PLAYER_GUILDID` has not streamed yet. Filled from the
    /// PUBLIC descriptor fields 191/192 joined against the app's guild-identity cache — see
    /// [`super::guild::UnitGuild`].
    pub guild: Option<UnitGuild>,
    /// The NPC subtitle line ("Stable Master") — the creature template's subname; the unit
    /// tooltip's second line. `None` for players/untitled creatures.
    pub subtitle: Option<String>,
    /// The creature TYPE display word ("Beast", "Humanoid", …) — app-resolved from
    /// `CreatureType.dbc`; the level line's class slot for hostile/neutral creatures.
    pub creature_type_name: Option<String>,
    /// Elite rank 0..4 — the creature classification every rank reader in the client shares, via
    /// the one getter `0x605620`: the level line's rank word `{"", Elite, Elite, Boss, ""}` (0276),
    /// `UnitLevel`'s world-boss −1 ([`level_reads_unknown`]), and `UnitClassification`
    /// ([`classification_word`], decision 0782). **Already gated** by the app when it fills this:
    /// the getter answers `0` unless the unit has a cached creature template *and* a zero
    /// `UNIT_FIELD_PETNUMBER`, so a player, an un-queried creature and an enslaved elite all read
    /// `0` here — the gate belongs at the one write, not in each of the three readers.
    pub rank: u32,
    /// The creature civilian flag — feeds the tooltip's green CIVILIAN line (shown only for a
    /// PvP-flagged HOSTILE unit that is also GREY/trivial to the player — the dishonorable-kill
    /// warning, the client's `0x612550` gate; a friendly civilian never shows it).
    pub civilian: bool,
    /// The creature racial-leader flag — the tooltip's white LEADER line (`0x6125c0`, gated on
    /// the same PvP bit as CIVILIAN).
    pub racial_leader: bool,
    /// PvP-flagged (`UNIT_FIELD_FLAGS` PvP bit) — the tooltip's "PvP" line, and also `UnitIsPVP`
    /// (decision 0434 §2: the party-frame PVP icon reads the same flag the tooltip line does, so
    /// there's one field, not two).
    pub pvp: bool,
    /// Skinnable (`UNIT_FIELD_FLAGS` skinnable bit) — the tooltip's RED "Skinnable" line.
    pub skinnable: bool,
    /// The unit's faction display name ("Stormwind") — the tooltip's white line between the
    /// level line and "PvP". App-resolved (Faction.dbc through the unit's faction template,
    /// every gate applied: reputation faction, race/class slot, hidden flag, the creature
    /// HIDE_FACTION_TOOLTIP type flag); `None` = no line.
    pub faction_name: Option<String>,
    /// Connection to the server is live (`UnitIsConnected`) — an offline party member's portrait
    /// desaturates (decision 0434 §2/§3: the `GROUP_LIST` status byte's `0x01` online bit).
    /// **The zero value is `false`**, so a snapshot built via `..Default::default()` reports a unit
    /// DISCONNECTED — which matters more than it reads: stock `UnitFrameManaBar_Update`
    /// (UnitFrame.lua:213) greys a disconnected unit's bar and never reaches the power colour at
    /// all. A synthetic unit that means to be live must say so.
    ///
    /// The app's own feed does (`ui_unit::snapshot`, "the stated `is_connected` gap, closed"), and
    /// the party feed reads the roster status byte's `0x01` for its tokens. This doc used to call
    /// that "a pending app-side change, not yet wired"; it has been wired since, and the note
    /// outlived it. A token absent from the store (`UnitExists` false) still reports nil
    /// regardless, via the same "no snapshot" path every other predicate here uses.
    pub is_connected: bool,
    /// Away-from-keyboard (`UnitIsAFK` — decision 0434 §2's status-byte `0x40` bit). The party
    /// frame's name line and chat both key off it. App-resolved; `false` until the feed lands.
    pub is_afk: bool,
    /// Do-not-disturb (`UnitIsDND` — decision 0434 §2's status-byte `0x80` bit), the AFK flag's
    /// sibling.
    pub is_dnd: bool,
    /// Free-for-all PvP-flagged (`UnitIsPVPFreeForAll` — decision 0434 §2's status-byte `0x10`
    /// bit) — distinct from [`Self::pvp`]'s ordinary faction PvP flag; a unit can carry either, both,
    /// or neither independently (an FFA zone flags this without the ordinary flag ever setting).
    pub is_pvp_ffa: bool,
    /// The unit's **current** PvP rank on the internal `0..=18` scale, `0` = no rank
    /// (`UnitPVPRank`; decision 1512). `PLAYER_BYTES_3` **byte 3** — a **PUBLIC** descriptor field,
    /// which is the whole reason a foreign player's rank is knowable and why the reference's
    /// inspect pane can call `UnitPVPRank("target")`. Not to be confused with the HIGHEST LIFETIME
    /// rank (`PLAYER_FIELD_BYTES` byte 3), which is private, rides
    /// [`HonorState`](super::HonorState) instead, and is what `GetPVPLifetimeStats` reports.
    ///
    /// Internal, not visual: the badge texture and `GetPVPRankInfo`'s second return are the
    /// `-4..=14` visual scale, converted in [`super::pvp`] and nowhere else.
    pub pvp_rank: u8,
    /// The unit's **city-protector title** index — `PLAYER_BYTES_3` **byte 2**, the byte next door
    /// to [`Self::pvp_rank`] (vmangos `PLAYER_BYTES_3_OFFSET_CITY_PROTECTOR_TITLE`), PUBLIC like
    /// it. `0` = none. Read by exactly one thing: `UnitPVPName`'s medal line, which appends
    /// `"\n"` + the `PVP_MEDAL<n>` GlobalString when it is non-zero (`0x6093ef`).
    ///
    /// **v1 feed gap, stated not hidden:** no app-side feed fills this yet — it rides the same
    /// dword the rank byte is decoded from, so wiring it is one line there, and until that line
    /// exists every unit reads `0` and `UnitPVPName` renders no medal. The binding leg is built
    /// against the field rather than left out, so the seam is a feed line rather than a missing
    /// arm (same shape as [`Self::is_connected`]'s stated gap).
    pub pvp_medal: u8,
    /// The unit's PvP faction group — `"Alliance"` or `"Horde"`, else `None` (`UnitFactionGroup`;
    /// decision 0646 §1). App-resolved from `UNIT_FIELD_FACTIONTEMPLATE` → `FactionTemplate.dbc`'s
    /// group mask, with the Player bit skipped: every playable race's template carries
    /// Player|<side> (mask 3 or 5), and so do the PvP-flagged city guards, so the side bit is the
    /// only one that can name a shipped `UI-PVP-<group>` texture. `None` for Monster/neutral
    /// templates is a state the icon callers explicitly handle (`if ( factionGroup and … )`).
    ///
    /// This is the **English** half — FactionGroup.dbc's `InternalName` — because that is what
    /// `UnitFactionGroup`'s first return must carry: every stock consumer concatenates it into a
    /// texture path (`"…\UI-PVP-"..factionGroup` at `PlayerFrame.lua:68`, `TargetFrame.lua:198`,
    /// `PartyMemberFrame.lua:125`; `"…\Battleground-"..` at `BattlefieldFrame.lua:195`), and
    /// `HonorFrame.lua:68` compares it against the literal `"Alliance"`.
    ///
    /// This field's doc used to say the binding "returns it twice … enUS ships the same word for
    /// both". That is true of enUS and of nothing else: `Name0` is localized, so on any other
    /// client the texture path would name a file that does not exist. The localized half lives in
    /// [`Self::faction_group_localized`] now.
    pub faction_group: Option<String>,
    /// The **localized** group name — FactionGroup.dbc's `Name0`, and `UnitFactionGroup`'s SECOND
    /// return, which stock uses as display text (`PlayerFrame.lua`'s PvP hit-area tooltip title).
    /// Never interchangeable with the English half above.
    pub faction_group_localized: Option<String>,
    /// The unit's **PvP team digit** — `0x5efe00`'s tri-state: `0` Horde, `1` Alliance, `-1` a
    /// unit with no side. The second `%d` of `PVP_RANK_<rank>_<team>`, and **not**
    /// [`Self::faction_group`] restated.
    ///
    /// The two answer different questions and the difference is a shipped bug's whole cause
    /// (report B378, decision 2227): `UnitFactionGroup` reads the unit's LIVE
    /// `UNIT_FIELD_FACTIONTEMPLATE` (`0x5166b8`/`0x5166be`), while every rank-title surface reads
    /// the unit's **RACE** and walks `ChrRaces` → `FactionTemplate` → factionGroupMask
    /// (`0x5efe00`, `[obj+0x110]+0x78`). A vmangos GM is forced to template 35 and so genuinely
    /// loses the PvP flag icon — and keeps his rank title, because his race did not move. Wiring
    /// this to the faction group instead read `NONE` at every rank for a Grand Marshal.
    ///
    /// **`Default` is `0`, and that is the reference's answer too, not a placeholder.** A literal
    /// snapshot is one the object manager could not resolve (the out-of-range roster leg), and
    /// `GetPVPRankInfo`'s team register is left at its initial `0` on exactly that edge
    /// (`0x51a9af`'s lookup failing) — so a roster-only unit is named off the Horde list on both
    /// clients. Every snapshot built from a live descriptor fills it (`ui_unit::snapshot`).
    pub pvp_team: i8,
    /// The unit's GUID (`OBJECT_FIELD_GUID`) — the identity the cross-token predicates compare
    /// (`UnitIsUnit`, `UnitInParty`; decision 0434 §5's popup gating). `0` = the app's feed didn't
    /// resolve one; two zero guids never compare equal.
    pub guid: u64,
    /// The raid-target mark on this unit (`GetRaidTargetIndex`): `0` none, `1..=8` the slot.
    /// App-fed from the `GroupState` raid-target board (decision 0434 §6's verified 8-slot table
    /// law) — the phase-5 submenu reads it; the phase-6 world renders draw from the same board.
    pub raid_target: u8,
    /// The player can attack this unit (`UnitCanAttack("player", unit)`) — app-fed from the
    /// byte-confirmed `CanAttack 0x606980` predicate (decision 0172; the flag disqualifiers +
    /// reaction ≤ neutral legs TAB and the combat flash share; the Lua binding `0x516c50` is
    /// pure delegation to it, §5-VERIFIED). Like [`Self::reaction`], the feed resolves it for
    /// the `"target"` token; other tokens leave the default `false` (→ nil), which is right for
    /// every current caller (`TargetFrame_CheckLevel`'s difficulty-color gate).
    pub can_attack: bool,
    /// This token resolves to a **TYPEID_CORPSE world object** — `UnitIsCorpse`'s whole
    /// predicate (`0x5161c0`, §5-VERIFIED: a pure object-type check, no health test; a dead
    /// mob/player is NOT a corpse). No feed sets it yet — corpse objects (a released player's
    /// remains) aren't selectable in benilla — a stated gap, not an oversight.
    pub corpse_object: bool,
    /// In combat (`UnitAffectingCombat`) — `UNIT_FIELD_FLAGS` (descriptor index 46) **bit 19**,
    /// mask `0x00080000` (`0x517e10`, wow-re `bag-language-combat-action-bindings.md` §3,
    /// §5-cross-checked: `mov ecx,[eax+0xa0]; shr ecx,0x13; test cl,1`).
    ///
    /// **There is no player-specific combat latch**, stated as the explicit negative because it
    /// is precisely what a client is tempted to invent: a whole-image census of the
    /// `shr reg,0x13` + `test rl,1` idiom returns 7 hits, six of them this same field+bit — and
    /// two of those six are hardcoded *local-player* readers taking no token at all. `"player"`
    /// takes a fast path to the cached local-player GUID inside `0x515970` and then reads the
    /// identical bit. One wire flag answers this for every token, which is what this one field
    /// is; `benilla::ui_action::usable` already reads the same bit off the same descriptor.
    pub in_combat: bool,
}
