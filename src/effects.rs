//! Buffs and debuffs for the buff bar, from the `2A/2B/2C 38` effect packets
//! (see `decode::parse_effects`). Only what the server already sent: nothing
//! is ever sent to the game. Expiry is local: receive time plus duration.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decode::Event;

/// Names for effect ids that `skills_en.json` has no entry for, or names too
/// plainly (the Pyroclasm buff is only "Pyroclasm" there).
const NAMES: &[(i32, &str)] = &[(15_250_047, "Pyroclasm: Fire Damage +5%")];

/// Instances kept at most; the oldest go first.
const MAX_ACTIVE: usize = 2_048;
/// An instance this far past its expiry without a removal is dropped.
const GRACE_MS: i64 = 3_000;
/// Skills remembered for Config's picker.
const MAX_SEEN: usize = 300;
pub const MAX_PICKED: usize = 500;
pub const LAYOUTS: &[&str] = &["horizontal", "vertical"];

/// Config > Buffs & debuffs, saved in settings.json. Where the bar shows is
/// its `buffs` toast style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuffSettings {
    pub enabled: bool,
    /// One line of icons: `horizontal` or `vertical`.
    pub layout: String,
    /// Off shows only `skills`.
    pub show_all: bool,
    /// Skill ids as [`display_skill`] gives them.
    pub skills: Vec<i32>,
    /// Skills seen on the bar in this and earlier runs, newest last. Not set by Config.
    pub seen: Vec<i32>,
}

impl Default for BuffSettings {
    fn default() -> Self {
        Self { enabled: true, layout: "horizontal".into(), show_all: true, skills: Vec::new(), seen: Vec::new() }
    }
}

impl BuffSettings {
    /// `patch` (any of enabled, layout, show_all, skills) over `self`, checked.
    pub fn merged(&self, patch: &Value) -> Result<Self, String> {
        let Value::Object(patch) = patch else { return Err("expected an object".into()) };
        let mut next = self.clone();
        for (key, value) in patch {
            let bad = || format!("{key} has the wrong type");
            match key.as_str() {
                "enabled" => next.enabled = value.as_bool().ok_or_else(bad)?,
                "show_all" => next.show_all = value.as_bool().ok_or_else(bad)?,
                "layout" => {
                    let layout = value.as_str().ok_or_else(bad)?;
                    if !LAYOUTS.contains(&layout) {
                        return Err("layout is horizontal or vertical".into());
                    }
                    next.layout = layout.into();
                }
                "skills" => {
                    let list: Vec<i32> = serde_json::from_value(value.clone()).map_err(|_| bad())?;
                    if list.len() > MAX_PICKED {
                        return Err(format!("pick up to {MAX_PICKED} skills"));
                    }
                    let mut set = BTreeSet::new();
                    next.skills = list.into_iter().filter(|id| *id > 0 && set.insert(*id)).collect();
                }
                _ => return Err(format!("unknown buff bar setting {key}")),
            }
        }
        Ok(next)
    }

    /// Saved settings, with anything out of range back at its default.
    pub fn load(saved: &Value) -> Self {
        let mut out: Self = serde_json::from_value(saved.clone()).unwrap_or_default();
        if !LAYOUTS.contains(&out.layout.as_str()) {
            out.layout = "horizontal".into();
        }
        out.skills.truncate(MAX_PICKED);
        let keep = out.seen.len().saturating_sub(MAX_SEEN);
        out.seen.drain(..keep);
        out
    }

    fn shows(&self, skill: i32) -> bool {
        self.show_all || self.skills.contains(&skill)
    }
}

/// The skill an effect is shown, picked and iconed as: the effect's own skill
/// (`effect / 10`, for 8- and 9-digit ids) when it has a name, else the skill
/// that applied it, as its named family.
pub fn display_skill(effect: u32, source: u32) -> i32 {
    let own = (effect >= 10_000_000).then(|| (effect / 10) as i32);
    for id in [own, Some(source as i32)].into_iter().flatten() {
        if override_name(id).is_some() {
            return id;
        }
        let canonical = crate::skills::canonical_skill_id(id);
        // A 7-digit id (an NPC's or an item's) must be named itself, not by family.
        if crate::skills::has_name(id) && (id >= 10_000_000 || canonical == id) {
            return canonical;
        }
    }
    own.unwrap_or(source as i32)
}

fn override_name(id: i32) -> Option<&'static str> {
    NAMES.iter().find(|(k, _)| *k == id).map(|(_, n)| *n)
}

/// The name, and whether it is a real one (not just the id).
pub fn skill_label(id: i32) -> (String, bool) {
    if let Some(name) = override_name(id) {
        return (name.into(), true);
    }
    if crate::skills::has_name(id) {
        return (crate::skills::skill_name(id), true);
    }
    (format!("Unknown #{id}"), false)
}

#[derive(Debug, Clone)]
struct Active {
    effect: u32,
    skill: u32,
    caster: i32,
    started_ms: i64,
    refreshed_ms: i64,
    /// `None` lasts until removed.
    expires_ms: Option<i64>,
    duration_ms: i64,
    refreshes: u32,
}

/// One icon on the bar: every live instance of one skill on one entity.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EffectRow {
    pub skill: i32,
    pub effect: u32,
    pub name: String,
    /// False when the name is only the id.
    pub named: bool,
    /// aion2data icon name; the page draws initials without one.
    pub icon: Option<&'static str>,
    /// `None` lasts until removed.
    pub remaining_ms: Option<i64>,
    pub duration_ms: i64,
    /// Instances of this skill at once (shown as a stack count above 1).
    pub count: u32,
    pub refreshes: u32,
    /// Time since it was last applied or refreshed.
    pub age_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EffectsView {
    /// On the local player, from anyone.
    pub player: Vec<EffectRow>,
    /// On the target, from the player (or the player's summons and spell objects).
    pub target: Vec<EffectRow>,
    pub target_id: Option<i32>,
}

#[derive(Debug, Default)]
pub struct Effects {
    active: HashMap<(i32, u32), Active>,
    /// The last entity the player put a debuff on.
    last_target: Option<i32>,
    /// Bumped on each change, so the snapshot can skip unchanged work.
    pub version: u64,
}

impl Effects {
    /// Track the effect events in `events`; despawned entities lose theirs.
    pub fn apply(&mut self, events: &[Event], now: i64) {
        for event in events {
            match *event {
                Event::EffectOn { target, seq, effect, duration_ms, caster, skill, refresh } => {
                    let expires_ms = (duration_ms >= 0).then(|| now + i64::from(duration_ms));
                    let entry = self.active.entry((target, seq)).or_insert(Active {
                        effect,
                        skill,
                        caster,
                        started_ms: now,
                        refreshed_ms: now,
                        expires_ms,
                        duration_ms: i64::from(duration_ms),
                        refreshes: 0,
                    });
                    if refresh || entry.effect != effect {
                        if entry.effect != effect {
                            entry.started_ms = now;
                            entry.refreshes = 0;
                        } else {
                            entry.refreshes = entry.refreshes.saturating_add(1);
                        }
                        entry.effect = effect;
                        entry.skill = skill;
                        entry.caster = caster;
                        entry.refreshed_ms = now;
                        entry.expires_ms = expires_ms;
                        entry.duration_ms = i64::from(duration_ms);
                    }
                    self.version += 1;
                }
                Event::EffectOff { target, seq, .. } => {
                    if self.active.remove(&(target, seq)).is_some() {
                        self.version += 1;
                    }
                }
                Event::Despawn { id } | Event::Removed { id, .. } => self.forget(id),
                _ => {}
            }
        }
        if self.active.len() > MAX_ACTIVE {
            self.prune(now);
            while self.active.len() > MAX_ACTIVE {
                let Some(oldest) = self.active.iter().min_by_key(|(_, a)| a.refreshed_ms).map(|(k, _)| *k) else { break };
                self.active.remove(&oldest);
            }
        }
    }

    /// Drop everything (fight reset, new server connection or zone).
    pub fn clear(&mut self) {
        self.active.clear();
        self.last_target = None;
        self.version += 1;
    }

    /// Drop one entity's effects (it despawned).
    pub fn forget(&mut self, id: i32) {
        let before = self.active.len();
        self.active.retain(|(target, _), _| *target != id);
        if self.last_target == Some(id) {
            self.last_target = None;
        }
        if self.active.len() != before {
            self.version += 1;
        }
    }

    fn prune(&mut self, now: i64) {
        self.active.retain(|_, a| a.expires_ms.is_none_or(|end| end + GRACE_MS > now));
    }

    /// The bar's rows. `you` is the local player, `target` the meter's
    /// target, `owner` maps a caster to who it acts for. Skills that show
    /// are added to `seen` (true when that changed it).
    pub fn view(
        &mut self,
        now: i64,
        you: Option<i32>,
        target: Option<i32>,
        owner: impl Fn(i32) -> i32,
        settings: &BuffSettings,
        seen: &mut Vec<i32>,
    ) -> (EffectsView, bool) {
        self.prune(now);
        let Some(you) = you else {
            return (EffectsView::default(), false);
        };
        let mine = |a: &Active| a.caster == you || owner(a.caster) == you;
        let debuffed = |id: i32| id != you && self.active.iter().any(|((t, _), a)| *t == id && mine(a));
        let target_id = target
            .filter(|id| debuffed(*id))
            .or_else(|| self.last_target.filter(|id| debuffed(*id)))
            .or_else(|| {
                self.active
                    .iter()
                    .filter(|((t, _), a)| *t != you && mine(a))
                    .max_by_key(|(_, a)| a.refreshed_ms)
                    .map(|((t, _), _)| *t)
            });
        self.last_target = target_id.or(self.last_target);
        let rows = |on: i32, only_mine: bool| -> Vec<EffectRow> {
            let mut out: Vec<(i64, EffectRow)> = Vec::new();
            for ((t, _), a) in &self.active {
                if *t != on || (only_mine && !mine(a)) {
                    continue;
                }
                let skill = display_skill(a.effect, a.skill);
                let remaining_ms = a.expires_ms.map(|end| (end - now).max(0));
                if let Some((first, row)) = out.iter_mut().find(|(_, r)| r.skill == skill) {
                    *first = (*first).min(a.started_ms);
                    row.count += 1;
                    row.refreshes += a.refreshes;
                    row.age_ms = row.age_ms.min(now - a.refreshed_ms);
                    if row.remaining_ms.is_some() && remaining_ms.is_none_or(|r| row.remaining_ms.is_some_and(|x| r > x)) {
                        row.remaining_ms = remaining_ms;
                        row.duration_ms = a.duration_ms;
                    }
                    continue;
                }
                let (name, named) = skill_label(skill);
                let icon = crate::skill_icons::icon_for(skill)
                    .or_else(|| crate::skill_icons::icon_for(crate::skills::canonical_skill_id(a.skill as i32)));
                out.push((
                    a.started_ms,
                    EffectRow {
                        skill,
                        effect: a.effect,
                        name,
                        named,
                        icon,
                        remaining_ms,
                        duration_ms: a.duration_ms,
                        count: 1,
                        refreshes: a.refreshes,
                        age_ms: now - a.refreshed_ms,
                    },
                ));
            }
            // First applied first, so icons do not jump around.
            out.sort_by_key(|(first, row)| (*first, row.skill));
            out.into_iter().map(|(_, row)| row).collect()
        };
        let player = rows(you, false);
        let on_target = target_id.map(|id| rows(id, true)).unwrap_or_default();
        let mut changed = false;
        for row in player.iter().chain(&on_target) {
            if !seen.contains(&row.skill) {
                seen.push(row.skill);
                changed = true;
            }
        }
        if seen.len() > MAX_SEEN {
            let extra = seen.len() - MAX_SEEN;
            seen.drain(..extra);
        }
        let view = EffectsView {
            player: player.into_iter().filter(|r| settings.shows(r.skill)).collect(),
            target: on_target.into_iter().filter(|r| settings.shows(r.skill)).collect(),
            target_id,
        };
        (view, changed)
    }
}

/// One choice in Config's skill picker.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillOption {
    pub skill: i32,
    pub name: String,
    pub named: bool,
    pub icon: Option<&'static str>,
    pub class: Option<&'static str>,
    /// Shown on the bar in this or an earlier run.
    pub seen: bool,
}

/// Seen and picked skills first, then the named overrides and every class
/// skill aion2data lists.
pub fn skill_options(settings: &BuffSettings) -> Vec<SkillOption> {
    let mut ids: Vec<i32> = Vec::new();
    let mut push = |id: i32| {
        if !ids.contains(&id) {
            ids.push(id);
        }
    };
    settings.seen.iter().rev().for_each(|id| push(*id));
    settings.skills.iter().for_each(|id| push(*id));
    NAMES.iter().for_each(|(id, _)| push(*id));
    crate::skill_icons::all().for_each(|(id, _)| push(id));
    let mut out: Vec<SkillOption> = ids
        .into_iter()
        .map(|skill| {
            let (name, named) = skill_label(skill);
            SkillOption {
                skill,
                name,
                named,
                icon: crate::skill_icons::icon_for(skill),
                class: crate::skill_icons::class_of(skill),
                seen: settings.seen.contains(&skill),
            }
        })
        .collect();
    out.sort_by(|a, b| b.seen.cmp(&a.seen).then(a.class.cmp(&b.class)).then(a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(target: i32, seq: u32, effect: u32, ms: i32, caster: i32, skill: u32, refresh: bool) -> Event {
        Event::EffectOn { target, seq, effect, duration_ms: ms, caster, skill, refresh }
    }

    #[test]
    fn display_skills_and_names() {
        assert_eq!(display_skill(157_100_071, 15_710_007), 15_710_000);
        assert_eq!(skill_label(15_710_000).0, "Fire Mark");
        assert_eq!(display_skill(152_500_471, 15_250_047), 15_250_047);
        assert_eq!(skill_label(15_250_047), ("Pyroclasm: Fire Damage +5%".into(), true));
        assert_eq!(skill_label(display_skill(3015, 15_000_100)).0, "Dodge");
        assert_eq!(skill_label(display_skill(20_111_011, 2_011_101)).0, "Life Potion");
        assert_eq!(skill_label(display_skill(150_900_001, 15_110_340)).0, "Ice Chain");
        assert_eq!(skill_label(99_999_999), ("Unknown #99999999".into(), false));
        // An NPC's debuff on the player is not named after a family it only looks like.
        assert_eq!(skill_label(display_skill(12_000_171, 1_607_415)).0, "Unknown #1200017");
    }

    #[test]
    fn tracks_apply_refresh_and_remove() {
        let mut fx = Effects::default();
        let all = BuffSettings::default();
        let mut seen = Vec::new();
        fx.apply(&[on(1966, 156, 152_500_471, 10_000, 1966, 15_250_047, false), on(41546, 6, 157_100_071, 5000, 1966, 15_710_007, false)], 0);
        fx.apply(&[on(41546, 6, 157_100_071, 5000, 1966, 15_710_007, true)], 4_000);
        let (view, changed) = fx.view(6_000, Some(1966), None, |id| id, &all, &mut seen);
        assert!(changed);
        assert_eq!(view.player.len(), 1);
        assert_eq!(view.player[0].remaining_ms, Some(4_000));
        assert_eq!(view.target_id, Some(41546));
        assert_eq!(view.target[0].remaining_ms, Some(3_000));
        assert_eq!(view.target[0].refreshes, 1);
        assert_eq!(view.target[0].icon, Some("ICON_SO_SKILL_Passive_002"));
        fx.apply(&[Event::EffectOff { target: 41546, seq: 6, reason: 1 }], 6_500);
        let (view, _) = fx.view(6_500, Some(1966), Some(41546), |id| id, &all, &mut seen);
        assert!(view.target.is_empty());
        // Past expiry plus grace, a lost removal still clears it.
        let (view, _) = fx.view(14_000, Some(1966), None, |id| id, &all, &mut seen);
        assert!(view.player.is_empty());
    }

    #[test]
    fn debuffs_are_only_the_players_and_filters_apply() {
        let mut fx = Effects::default();
        let mut seen = Vec::new();
        fx.apply(&[
            on(41546, 1, 150_500_301, 3000, 1966, 15_050_350, false),
            on(41546, 2, 150_500_301, 3000, 1966, 15_050_350, false),
            on(41546, 3, 152_000_121, 10_000, 40596, 15_200_012, false),
            on(41546, 4, 153_200_201, 4000, 777, 15_320_020, false),
        ], 0);
        let owner = |id: i32| if id == 40596 { 1966 } else { id };
        let (view, _) = fx.view(100, Some(1966), Some(41546), owner, &BuffSettings::default(), &mut seen);
        let names: Vec<_> = view.target.iter().map(|r| (r.name.as_str(), r.count)).collect();
        assert_eq!(names, vec![("Blaze", 2), ("Cold Storm", 1)]);
        let picked = BuffSettings { show_all: false, skills: vec![15_200_000], ..BuffSettings::default() };
        let (view, _) = fx.view(100, Some(1966), Some(41546), owner, &picked, &mut seen);
        assert_eq!(view.target.len(), 1);
        fx.apply(&[Event::Despawn { id: 41546 }], 200);
        let (view, _) = fx.view(200, Some(1966), Some(41546), owner, &picked, &mut seen);
        assert!(view.target.is_empty() && view.target_id.is_none());
    }

    #[test]
    fn settings_patch_is_checked() {
        let base = BuffSettings::default();
        let next = base.merged(&serde_json::json!({"layout": "vertical", "show_all": false, "skills": [3, 3, 15_250_047]})).unwrap();
        assert_eq!((next.layout.as_str(), next.show_all, next.skills.clone()), ("vertical", false, vec![3, 15_250_047]));
        assert!(base.merged(&serde_json::json!({"layout": "diagonal"})).is_err());
        assert!(base.merged(&serde_json::json!({"seen": [1]})).is_err());
        assert_eq!(BuffSettings::load(&serde_json::json!({"layout": "x"})).layout, "horizontal");
        assert_eq!(BuffSettings::load(&Value::Null), base);
    }

    #[test]
    fn picker_lists_seen_and_class_skills() {
        let s = BuffSettings { seen: vec![15_250_047], ..BuffSettings::default() };
        let list = skill_options(&s);
        assert_eq!(list[0].skill, 15_250_047);
        assert!(list.iter().any(|o| o.name == "Fire Mark" && o.class == Some("Sorcerer")));
    }
}
