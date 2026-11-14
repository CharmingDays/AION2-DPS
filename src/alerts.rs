//! Event alerts: reminders the user sets for things about to happen in the
//! game, such as a world boss spawn. The game sends no event schedule, so the
//! list is the user's own, kept in `%LOCALAPPDATA%\aion2dps\alerts.json`.
//!
//! The pages run the countdowns and show the toasts; this file only keeps the
//! settings and the list, and refuses anything malformed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage;

pub const SCHEMA_VERSION: u32 = 2;
pub const FILE_NAME: &str = "alerts.json";
/// `--demo` keeps its alerts apart from real ones.
pub const DEMO_FILE_NAME: &str = "alerts-demo.json";
pub const MAX_ALERTS: usize = 50;
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_MESSAGE_CHARS: usize = 120;
pub const MAX_ID_CHARS: usize = 32;
/// The lead times the settings offer, in minutes.
pub const LEAD_CHOICES: [u32; 3] = [1, 5, 10];
/// Longest step for the minute, hour and hours + minutes repeats: a week.
pub const MAX_EVERY_MINS: u32 = 7 * 24 * 60;
pub const MAX_EVERY_HOURS: u32 = 7 * 24;
pub const MAX_EVERY_DAYS: u32 = 365;
/// How long a toast stays, in seconds; 0 means until dismissed.
pub const DEFAULT_DURATION_SECS: u32 = 8;
pub const MAX_DURATION_SECS: u32 = 600;
/// Bumped when built-in timers are added or their hours/days change. A file
/// below this is migrated once: missing built-ins are added and existing
/// built-ins (matched by `default_id`) get the new schedule, keeping the
/// user's enabled flags, lead times, custom messages and custom events.
pub const DEFAULTS_VERSION: u32 = 4;
/// Used only when Region is Custom: a fixed UTC offset. Named regions use
/// an IANA timezone (and follow daylight saving) instead.
pub const DEFAULT_SERVER_OFFSET_MINS: i32 = -8 * 60;
/// UTC-12 .. UTC+14.
pub const MIN_OFFSET_MINS: i32 = -12 * 60;
pub const MAX_OFFSET_MINS: i32 = 14 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Alerts {
    pub schema_version: u32,
    /// The "Event alerts" switch.
    pub enabled: bool,
    /// Minutes before an event to show the first toast. A 30 second toast and
    /// a live toast always follow.
    pub lead_mins: Vec<u32>,
    pub sound: bool,
    pub position: Position,
    pub alerts: Vec<Alert>,
    /// The game server clock as minutes east of UTC, used by events whose
    /// `clock` is `server` when `region` is `Custom` (or missing on very old
    /// files that also have no region, which now use NA West instead).
    pub server_utc_offset_mins: i32,
    /// The player's game region, which picks the timezone for `clock: Server`
    /// events. Missing means they have not chosen yet: countdowns use NA West
    /// until they do, and the Alerts panel shows a warning on the Region button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Region>,
    /// Which set of built-in timers was last added; 0 for files from before
    /// built-in timers. See `seed_defaults`.
    pub defaults_version: u32,
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            enabled: true,
            lead_mins: vec![5],
            sound: false,
            position: Position::Top,
            alerts: Vec::new(),
            server_utc_offset_mins: DEFAULT_SERVER_OFFSET_MINS,
            region: None,
            defaults_version: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    #[default]
    Top,
    Bottom,
}

/// Which clock an event's `time` and `start_date` are read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Clock {
    /// This PC's local time; every event from before clocks.
    #[default]
    Local,
    /// The game server: a named region timezone, or `server_utc_offset_mins` when Region is Custom.
    Server,
    Utc,
}

impl Clock {
    fn is_local(&self) -> bool {
        *self == Clock::Local
    }
}

/// The player's AION 2 Global region. Named regions use an IANA timezone so
/// server-clock events follow daylight saving the same way NC's clock-change
/// maintenances do. `Custom` keeps the manual UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Region {
    NaWest,
    NaEast,
    Europe,
    Custom,
}

impl Region {
    /// IANA timezone for this region, or `None` for a fixed manual offset.
    pub fn time_zone(self) -> Option<&'static str> {
        match self {
            Self::NaWest => Some("America/Los_Angeles"),
            Self::NaEast => Some("America/New_York"),
            Self::Europe => Some("Europe/Berlin"),
            Self::Custom => None,
        }
    }

    /// Hours to add to the canonical NA West wall-clock times so the event
    /// stays at the same UTC instant. NA East is UTC-5 when West is UTC-8,
    /// so +3 all year (both zones change DST together). Europe is assumed
    /// to share NA West's local hours, not the same UTC instant, because a
    /// same-UTC Saturday 17:00 PT siege would land at 02:00 CET Sunday.
    fn west_shift_hours(self) -> i32 {
        match self {
            Self::NaEast => 3,
            Self::NaWest | Self::Europe | Self::Custom => 0,
        }
    }
}

/// Region used to materialize built-in hours when none is set: NA West,
/// which is what the in-game clock confirmed on 2026-11-14.
pub fn effective_region(region: Option<Region>) -> Region {
    region.unwrap_or(Region::NaWest)
}

/// How an event repeats. Daily and weekly happen at the alert's `time`; the
/// steps count from the first occurrence (`start_date` at `time`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Repeat {
    None,
    Minutes { every: u32 },
    Hours { every: u32 },
    Daily { every: u32 },
    /// Weekdays, 0 = Monday .. 6 = Sunday.
    Weekly { days: Vec<u8> },
    HoursMinutes { hours: u32, minutes: u32 },
}

/// One event. `time` is a local time of day, `HH:MM`, and `start_date` the
/// local day of the first occurrence, `YYYY-MM-DD`. Files from before repeat
/// types have no `repeat`: `every_mins` there meant every that many minutes
/// in step with `time`, and no `every_mins` meant once a day. `validate`
/// turns those into a `repeat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub message: String,
    pub time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// The old repeat field, read for compatibility and never written.
    #[serde(default, skip_serializing)]
    pub every_mins: Option<u32>,
    /// Lead times for this event; missing means the global `lead_mins`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_mins: Option<Vec<u32>>,
    /// Seconds each toast stays; 0 is until dismissed, missing is 8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Clock::is_local")]
    pub clock: Clock,
    /// Set on the built-in timers, e.g. `watcher-kaira`, and kept through
    /// edits, so "Restore default timers" knows which ones are missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
}

fn yes() -> bool {
    true
}

pub fn default_path() -> PathBuf {
    storage::folder().join(FILE_NAME)
}

pub fn demo_path() -> PathBuf {
    storage::folder().join(DEMO_FILE_NAME)
}

/// `HH:MM` as minutes after midnight.
pub fn parse_time(text: &str) -> Option<u32> {
    let (h, m) = text.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    if !h.bytes().chain(m.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// `YYYY-MM-DD` as (year, month, day), checked against the calendar.
pub fn parse_date(text: &str) -> Option<(u32, u32, u32)> {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = &text[r];
        part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse().ok()).flatten()
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    ((2000..=2100).contains(&y) && (1..=days).contains(&d)).then_some((y, m, d))
}

/// The repeat, checked and normalized, or what is wrong with it.
fn check_repeat(repeat: Repeat) -> Result<Repeat, &'static str> {
    match repeat {
        Repeat::None => Ok(Repeat::None),
        Repeat::Minutes { every } if (1..=MAX_EVERY_MINS).contains(&every) => Ok(Repeat::Minutes { every }),
        Repeat::Hours { every } if (1..=MAX_EVERY_HOURS).contains(&every) => Ok(Repeat::Hours { every }),
        Repeat::Daily { every } if (1..=MAX_EVERY_DAYS).contains(&every) => Ok(Repeat::Daily { every }),
        Repeat::Minutes { .. } | Repeat::Hours { .. } | Repeat::Daily { .. } => Err("value must be 1 or more (and at most a week or a year of days)"),
        Repeat::Weekly { mut days } => {
            days.sort_unstable();
            days.dedup();
            if days.is_empty() {
                Err("pick at least one day")
            } else if days.iter().any(|d| *d > 6) {
                Err("weekdays must be 0 (Monday) to 6 (Sunday)")
            } else {
                Ok(Repeat::Weekly { days })
            }
        }
        Repeat::HoursMinutes { hours, minutes } => {
            let total = hours.saturating_mul(60).saturating_add(minutes);
            if minutes > 59 || hours > MAX_EVERY_HOURS || !(1..=MAX_EVERY_MINS).contains(&total) {
                Err("interval must be 1 minute or more (minutes 0 to 59, at most a week)")
            } else {
                Ok(Repeat::HoursMinutes { hours, minutes })
            }
        }
    }
}

fn clean_text(text: &str, max: usize, what: &str, required: bool) -> Result<String, String> {
    let text = text.trim();
    if required && text.is_empty() {
        return Err(format!("{what} is empty"));
    }
    if text.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!("{what} has a control character"));
    }
    Ok(text.to_string())
}

/// Checked and trimmed, or a sentence for the user saying what is wrong.
pub fn validate(mut alerts: Alerts) -> Result<Alerts, String> {
    alerts.schema_version = SCHEMA_VERSION;
    alerts.lead_mins.sort_unstable();
    alerts.lead_mins.dedup();
    if alerts.lead_mins.iter().any(|m| !LEAD_CHOICES.contains(m)) {
        return Err("lead time must be 1, 5 or 10 minutes".into());
    }
    if alerts.alerts.len() > MAX_ALERTS {
        return Err(format!("at most {MAX_ALERTS} alerts"));
    }
    if !(MIN_OFFSET_MINS..=MAX_OFFSET_MINS).contains(&alerts.server_utc_offset_mins) {
        return Err("the server clock must be between UTC-12 and UTC+14".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut default_ids = std::collections::HashSet::new();
    for alert in &mut alerts.alerts {
        let id_ok = (1..=MAX_ID_CHARS).contains(&alert.id.len())
            && alert.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !id_ok || !ids.insert(alert.id.clone()) {
            return Err("an alert has a missing or repeated id".into());
        }
        if let Some(did) = &alert.default_id {
            let ok = (1..=MAX_ID_CHARS).contains(&did.len())
                && did.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if !ok || !default_ids.insert(did.clone()) {
                return Err("an alert has a bad or repeated default id".into());
            }
        }
        alert.name = clean_text(&alert.name, MAX_NAME_CHARS, "the alert name", true)?;
        alert.message = clean_text(&alert.message, MAX_MESSAGE_CHARS, "the alert message", false)?;
        let Some(minutes) = parse_time(alert.time.trim()) else {
            return Err(format!("\"{}\" needs a time as HH:MM", alert.name));
        };
        alert.time = format!("{:02}:{:02}", minutes / 60, minutes % 60);
        if let Some(date) = alert.start_date.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            let Some((y, m, d)) = parse_date(date) else {
                return Err(format!("\"{}\" needs a start date as YYYY-MM-DD", alert.name));
            };
            alert.start_date = Some(format!("{y:04}-{m:02}-{d:02}"));
        } else {
            alert.start_date = None;
        }
        // Older files: every_mins, or nothing for once a day.
        let repeat = match (alert.repeat.take(), alert.every_mins.take()) {
            (Some(repeat), _) => repeat,
            (None, Some(every)) => Repeat::Minutes { every },
            (None, None) => Repeat::Daily { every: 1 },
        };
        alert.repeat = Some(check_repeat(repeat).map_err(|why| format!("\"{}\": {why}", alert.name))?);
        if let Some(leads) = &mut alert.lead_mins {
            leads.sort_unstable();
            leads.dedup();
            if leads.iter().any(|m| !LEAD_CHOICES.contains(m)) {
                return Err(format!("\"{}\": lead time must be 1, 5 or 10 minutes", alert.name));
            }
        }
        if let Some(secs) = alert.duration_secs {
            if secs > MAX_DURATION_SECS {
                return Err(format!("\"{}\": notification duration must be 1 to {MAX_DURATION_SECS} seconds", alert.name));
            }
        }
    }
    Ok(alerts)
}

/// What "Restore default timers" did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Restored {
    pub restored: usize,
    pub alerts: Alerts,
}

fn builtin(default_id: &str, name: &str, message: impl Into<String>, time: &str, repeat: Repeat, clock: Clock, enabled: bool) -> Alert {
    Alert {
        id: format!("d-{default_id}"),
        name: name.into(),
        message: message.into(),
        time: time.into(),
        start_date: None,
        repeat: Some(repeat),
        every_mins: None,
        lead_mins: None,
        duration_secs: None,
        enabled,
        clock,
        default_id: Some(default_id.into()),
    }
}

/// Older `default_id` values rewritten to the current ones on migrate.
const DEFAULT_ID_ALIASES: &[(&str, &str)] = &[("beritra-air-raid", "dimensional-invasion")];

fn canonical_default_id(id: &str) -> &str {
    DEFAULT_ID_ALIASES
        .iter()
        .find(|(old, _)| *old == id)
        .map(|(_, new)| *new)
        .unwrap_or(id)
}

fn alert_default_id(alert: &Alert) -> Option<&str> {
    alert.default_id.as_deref().map(canonical_default_id)
}

fn has_builtin(alerts: &Alerts, id: &str) -> bool {
    let want = canonical_default_id(id);
    alerts.alerts.iter().any(|a| alert_default_id(a) == Some(want))
}

/// Rename obsolete built-in ids (e.g. Beritra Air Raid) to the current ones.
/// Returns how many were renamed.
fn canonicalize_default_ids(alerts: &mut Alerts) -> usize {
    let mut n = 0;
    for alert in &mut alerts.alerts {
        if let Some(id) = alert.default_id.as_deref() {
            let canon = canonical_default_id(id);
            if canon != id {
                alert.default_id = Some(canon.to_string());
                n += 1;
            }
        }
    }
    n
}

/// Drop later alerts that share a `default_id` with an earlier one, keeping
/// the first (the user's copy, including its enabled flag and lead times).
/// Returns how many were removed.
fn dedupe_default_ids(alerts: &mut Alerts) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut dropped = 0;
    alerts.alerts.retain(|alert| {
        let Some(id) = alert_default_id(alert) else { return true };
        if seen.insert(id.to_string()) {
            true
        } else {
            dropped += 1;
            false
        }
    });
    dropped
}

/// Idempotent repair: map old ids, drop duplicate built-ins, and migrate a
/// file that is behind `DEFAULTS_VERSION`. True when anything changed.
pub fn prepare_alerts(alerts: &mut Alerts) -> bool {
    let mut changed = false;
    if canonicalize_default_ids(alerts) > 0 {
        changed = true;
    }
    if dedupe_default_ids(alerts) > 0 {
        changed = true;
    }
    if alerts.defaults_version < DEFAULTS_VERSION {
        restore_defaults(alerts);
        sync_builtin_schedule(alerts);
        alerts.defaults_version = DEFAULTS_VERSION;
        changed = true;
    }
    changed
}

/// "HH:MM" plus a number of hours, and how many days the clock rolled.
fn add_hours(time: &str, hours: i32) -> (String, i32) {
    let mut parts = time.split(':');
    let h: i32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let m: i32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let total = h * 60 + m + hours * 60;
    let day = total.div_euclid(24 * 60);
    let rem = total.rem_euclid(24 * 60);
    (format!("{:02}:{:02}", rem / 60, rem % 60), day)
}

fn shift_weekly_days(days: &[u8], delta: i32) -> Vec<u8> {
    let mut out: Vec<u8> = days
        .iter()
        .map(|d| (*d as i32 + delta).rem_euclid(7) as u8)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn shift_repeat(repeat: Repeat, day_delta: i32) -> Repeat {
    match repeat {
        Repeat::Weekly { days } if day_delta != 0 => Repeat::Weekly { days: shift_weekly_days(&days, day_delta) },
        other => other,
    }
}

/// The built-in Aion 2 timers in NA West server time (America/Los_Angeles),
/// except the daily and weekly resets, which are a true 07:00 UTC.
/// Hours for other regions are derived by `default_alerts_for`.
pub fn default_alerts() -> Vec<Alert> {
    default_alerts_for(None)
}

/// Built-in timers with wall-clock hours for `region`. Unset and Europe use
/// the NA West hours; NA East is shifted +3 hours so events stay at the same
/// UTC instant as NA West. Custom also uses the NA West hours, plus the
/// user's manual UTC offset at countdown time.
pub fn default_alerts_for(region: Option<Region>) -> Vec<Alert> {
    use Clock::{Server, Utc};
    use Repeat::{Daily, Hours, Weekly};
    let shift = effective_region(region).west_shift_hours();
    // Hourly events fire at every :00 or :30 either way, so their anchor is
    // left unshifted and stays 00:00 / 00:30 in every region.
    let (kaira, _) = add_hours("01:00", shift);
    let (executors, exec_day) = add_hours("17:30", shift);
    let (nahma, nahma_day) = add_hours("17:00", shift);
    let (argo, _) = add_hours("06:30", shift);
    let (rift, _) = add_hours("02:00", shift);
    let (siege, siege_day) = add_hours("17:00", shift);
    let (arena_day, _) = add_hours("07:00", shift);
    let (arena_day_end, _) = add_hours("10:00", shift);
    let (arena_night, _) = add_hours("15:00", shift);
    let (arena_night_end, _) = add_hours("17:00", shift);
    vec![
        builtin("watcher-kaira", "Watcher Kaira", "Chaotic Lower Reshanta, up for 30 min", &kaira, Hours { every: 3 }, Server, true),
        builtin("executors", "Executors (Argo, Kaira, Tamasa)", "Chaotic Lower Reshanta", &executors, shift_repeat(Weekly { days: vec![3, 5] }, exec_day), Server, true),
        builtin("guardian-lord-nahma", "Guardian Lord Nahma", "", &nahma, shift_repeat(Weekly { days: vec![4, 6] }, nahma_day), Server, true),
        builtin("argo-spirit-king", "Argo, the Spirit King", "Chaotic Lower Reshanta", &argo, Hours { every: 12 }, Server, true),
        builtin("spacetime-rift", "Spacetime Rift", "Open for 3 min", &rift, Hours { every: 3 }, Server, true),
        builtin("dimensional-invasion", "Dimensional Invasion", "Beritra Air Raid, Accursed Sword or Incursion of the Spirits; lasts 10 min", "00:30", Hours { every: 1 }, Server, true),
        builtin("shugo-festival", "Shugo Festival", "", "00:00", Hours { every: 1 }, Server, true),
        builtin("artifact-siege", "Artifact Siege", "", &siege, shift_repeat(Weekly { days: vec![3, 5] }, siege_day), Server, true),
        builtin("arena-of-tactics-day", "Arena of Tactics", format!("Open {arena_day} to {arena_day_end} server time"), &arena_day, Daily { every: 1 }, Server, true),
        builtin("arena-of-tactics-night", "Arena of Tactics", format!("Open {arena_night} to {arena_night_end} server time"), &arena_night, Daily { every: 1 }, Server, true),
        builtin("daily-reset", "Daily reset", "07:00 UTC (16:00 Korea time)", "07:00", Daily { every: 1 }, Utc, true),
        builtin("weekly-reset", "Weekly reset", "Wednesday, at the same moment as the daily reset", "07:00", Weekly { days: vec![2] }, Utc, true),
    ]
}

/// Official message texts a built-in has used. If `Some`, a saved message is
/// only replaced when it is still one of these (a custom message is kept).
/// `None` keeps the older behaviour of always writing the current default.
fn official_messages(default_id: &str) -> Option<&'static [&'static str]> {
    match default_id {
        "spacetime-rift" => Some(&["Open for 10 min", "Open for 3 min"]),
        _ => None,
    }
}

fn should_sync_message(existing: &str, default_id: Option<&str>, current: &str) -> bool {
    match default_id.and_then(official_messages) {
        Some(known) => known.contains(&existing) || existing == current,
        None => true,
    }
}

/// Update existing built-in timers to the current schedule for `alerts.region`,
/// keeping each one's enabled flag, lead times, duration and id. Custom events
/// are left alone. Official built-in messages are updated to the current text
/// unless the user has changed them. Does not add missing built-ins;
/// `restore_defaults` does that. Ids should already be canonical; this also
/// remaps aliases to be safe.
pub fn sync_builtin_schedule(alerts: &mut Alerts) {
    canonicalize_default_ids(alerts);
    for builtin in default_alerts_for(alerts.region) {
        let Some(existing) = alerts
            .alerts
            .iter_mut()
            .find(|a| alert_default_id(a) == builtin.default_id.as_deref())
        else {
            continue;
        };
        existing.name = builtin.name;
        if should_sync_message(&existing.message, builtin.default_id.as_deref(), &builtin.message) {
            existing.message = builtin.message;
        }
        existing.time = builtin.time;
        existing.repeat = builtin.repeat;
        existing.clock = builtin.clock;
    }
}

/// Add the built-in timers that are missing (matched by `default_id`, including
/// obsolete aliases), leaving every other event, edited built-ins included,
/// as it is. Returns how many were added; stops at `MAX_ALERTS`. New ones use
/// the hours for the file's current region.
pub fn restore_defaults(alerts: &mut Alerts) -> usize {
    canonicalize_default_ids(alerts);
    let mut added = 0;
    for mut builtin in default_alerts_for(alerts.region) {
        if alerts.alerts.len() >= MAX_ALERTS {
            break;
        }
        if has_builtin(alerts, builtin.default_id.as_deref().unwrap_or("")) {
            continue;
        }
        let base = builtin.id.clone();
        let mut n = 2;
        while alerts.alerts.iter().any(|a| a.id == builtin.id) {
            builtin.id = format!("{base}-{n}");
            n += 1;
        }
        alerts.alerts.push(builtin);
        added += 1;
    }
    added
}

/// Repair and, when the file is behind `DEFAULTS_VERSION`, migrate built-in
/// timers. Idempotent: safe to call again, and it will not re-add timers the
/// user deleted once the version has been bumped. True when anything changed
/// and the file should be saved.
pub fn seed_defaults(alerts: &mut Alerts) -> bool {
    prepare_alerts(alerts)
}

/// The saved alerts, or the defaults when the file is missing or unreadable.
/// A file that parses but would fail validation (for example duplicate
/// built-in ids left by an older migrate) is repaired in place instead of
/// being thrown away.
pub fn load(path: &Path) -> Alerts {
    load_and_prepare(path).0
}

/// Like `load`, plus whether the alerts were repaired or migrated and should
/// be written back.
pub fn load_and_prepare(path: &Path) -> (Alerts, bool) {
    let mut alerts = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Alerts>(&bytes).ok())
        .unwrap_or_default();
    let changed = prepare_alerts(&mut alerts);
    (validate(alerts.clone()).unwrap_or(alerts), changed)
}

pub fn save(path: &Path, alerts: &Alerts) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(alerts).map_err(std::io::Error::other)?;
    storage::write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(id: &str, name: &str, time: &str, every: Option<u32>) -> Alert {
        Alert {
            id: id.into(),
            name: name.into(),
            message: String::new(),
            time: time.into(),
            start_date: None,
            repeat: None,
            every_mins: every,
            lead_mins: None,
            duration_secs: None,
            enabled: true,
            clock: Clock::Local,
            default_id: None,
        }
    }

    #[test]
    fn times_parse_as_minutes_after_midnight() {
        assert_eq!(parse_time("00:00"), Some(0));
        assert_eq!(parse_time("9:05"), Some(545));
        assert_eq!(parse_time("23:59"), Some(1439));
        for bad in ["24:00", "12:60", "12:5", "1205", "", ":30", "-1:00", "12:3a", "123:00"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_default_list_is_empty() {
        let alerts = Alerts::default();
        assert!(alerts.alerts.is_empty());
        assert!(alerts.enabled);
        assert_eq!(alerts.lead_mins, [5]);
    }

    #[test]
    fn a_partial_file_fills_in_the_defaults() {
        let alerts: Alerts =
            serde_json::from_str(r#"{"alerts":[{"id":"a","name":"Boss","time":"20:00"}]}"#).unwrap();
        let alerts = validate(alerts).unwrap();
        assert_eq!(alerts.position, Position::Top);
        assert!(alerts.alerts[0].enabled);
        assert_eq!(alerts.alerts[0].repeat, Some(Repeat::Daily { every: 1 }));
        assert_eq!(alerts.alerts[0].duration_secs, None);
    }

    #[test]
    fn validation_trims_and_normalizes() {
        let mut alerts = Alerts {
            lead_mins: vec![10, 1, 10],
            ..Alerts::default()
        };
        alerts.alerts.push(alert("a", "  World Boss ", "9:00", Some(120)));
        let alerts = validate(alerts).unwrap();
        assert_eq!(alerts.lead_mins, [1, 10]);
        assert_eq!(alerts.alerts[0].name, "World Boss");
        assert_eq!(alerts.alerts[0].time, "09:00");
    }

    #[test]
    fn validation_refuses_bad_alerts() {
        let with = |a: Alert| Alerts {
            alerts: vec![a],
            ..Alerts::default()
        };
        assert!(validate(with(alert("a", " ", "10:00", None))).is_err());
        assert!(validate(with(alert("a", &"x".repeat(41), "10:00", None))).is_err());
        assert!(validate(with(alert("a", "Bad\u{1b}", "10:00", None))).is_err());
        assert!(validate(with(alert("a", "Boss", "25:00", None))).is_err());
        assert!(validate(with(alert("a", "Boss", "10:00", Some(0)))).is_err());
        assert!(validate(with(alert("a/b", "Boss", "10:00", None))).is_err());
        let two = Alerts {
            alerts: vec![alert("a", "One", "10:00", None), alert("a", "Two", "11:00", None)],
            ..Alerts::default()
        };
        assert!(validate(two).is_err());
        let lead = Alerts {
            lead_mins: vec![3],
            ..Alerts::default()
        };
        assert!(validate(lead).is_err());
    }

    #[test]
    fn alerts_are_saved_and_loaded() {
        let dir = std::env::temp_dir().join(format!("aion2dps-alerts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE_NAME);
        assert_eq!(load(&path), Alerts::default(), "missing file");
        let mut alerts = Alerts {
            sound: true,
            position: Position::Bottom,
            ..Alerts::default()
        };
        alerts.alerts.push(alert("boss", "World Boss", "20:00", Some(180)));
        let alerts = validate(alerts).unwrap();
        save(&path, &alerts).unwrap();
        assert_eq!(load(&path), alerts);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(load(&path), Alerts::default(), "damaged file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_files_migrate_to_repeat_types() {
        let text = r#"{"schema_version":1,"enabled":true,"lead_mins":[5],"alerts":[
            {"id":"a","name":"Daily","time":"20:00","enabled":true},
            {"id":"b","name":"Every","time":"00:15","every_mins":30,"enabled":false}]}"#;
        let alerts = validate(serde_json::from_str(text).unwrap()).unwrap();
        assert_eq!(alerts.schema_version, SCHEMA_VERSION);
        assert_eq!(alerts.alerts[0].repeat, Some(Repeat::Daily { every: 1 }));
        assert_eq!(alerts.alerts[1].repeat, Some(Repeat::Minutes { every: 30 }));
        assert!(!alerts.alerts[1].enabled);
        let saved = serde_json::to_string(&alerts).unwrap();
        assert!(!saved.contains("every_mins"), "{saved}");
        assert!(saved.contains(r#""repeat":{"type":"minutes","every":30}"#), "{saved}");
        // Saved again, it reads back the same.
        assert_eq!(validate(serde_json::from_str(&saved).unwrap()).unwrap(), alerts);
    }

    #[test]
    fn repeat_types_parse() {
        let text = r#"{"alerts":[
            {"id":"n","name":"N","time":"20:00","start_date":"2026-11-03","repeat":{"type":"none"}},
            {"id":"h","name":"H","time":"20:00","repeat":{"type":"hours","every":2}},
            {"id":"d","name":"D","time":"06:00","repeat":{"type":"daily","every":3}},
            {"id":"w","name":"W","time":"19:30","repeat":{"type":"weekly","days":[5,2,5]}},
            {"id":"hm","name":"HM","time":"19:30","repeat":{"type":"hours_minutes","hours":1,"minutes":30},
             "lead_mins":[10,1],"duration_secs":0}]}"#;
        let alerts = validate(serde_json::from_str(text).unwrap()).unwrap();
        let r: Vec<_> = alerts.alerts.iter().map(|a| a.repeat.clone().unwrap()).collect();
        assert_eq!(r[0], Repeat::None);
        assert_eq!(alerts.alerts[0].start_date.as_deref(), Some("2026-11-03"));
        assert_eq!(r[1], Repeat::Hours { every: 2 });
        assert_eq!(r[2], Repeat::Daily { every: 3 });
        assert_eq!(r[3], Repeat::Weekly { days: vec![2, 5] });
        assert_eq!(r[4], Repeat::HoursMinutes { hours: 1, minutes: 30 });
        assert_eq!(alerts.alerts[4].lead_mins, Some(vec![1, 10]));
        assert_eq!(alerts.alerts[4].duration_secs, Some(0));
    }

    #[test]
    fn bad_repeats_durations_and_dates_are_refused() {
        let one = |repeat: Repeat| {
            let mut a = alert("a", "Boss", "10:00", None);
            a.repeat = Some(repeat);
            validate(Alerts { alerts: vec![a], ..Alerts::default() })
        };
        assert!(one(Repeat::Minutes { every: 0 }).is_err());
        assert!(one(Repeat::Hours { every: 0 }).is_err());
        assert!(one(Repeat::Daily { every: 0 }).is_err());
        assert!(one(Repeat::Weekly { days: vec![] }).is_err());
        assert!(one(Repeat::Weekly { days: vec![7] }).is_err());
        assert!(one(Repeat::HoursMinutes { hours: 0, minutes: 0 }).is_err());
        assert!(one(Repeat::HoursMinutes { hours: 1, minutes: 60 }).is_err());
        assert!(one(Repeat::HoursMinutes { hours: 0, minutes: 1 }).is_ok());
        assert!(one(Repeat::HoursMinutes { hours: 2, minutes: 0 }).is_ok());
        let with = |f: &dyn Fn(&mut Alert)| {
            let mut a = alert("a", "Boss", "10:00", None);
            f(&mut a);
            validate(Alerts { alerts: vec![a], ..Alerts::default() })
        };
        assert!(with(&|a| a.duration_secs = Some(601)).is_err());
        assert!(with(&|a| a.duration_secs = Some(600)).is_ok());
        assert!(with(&|a| a.lead_mins = Some(vec![3])).is_err());
        assert!(with(&|a| a.start_date = Some("2026-02-30".into())).is_err());
        assert!(with(&|a| a.start_date = Some("2026-1-05".into())).is_err());
        assert!(with(&|a| a.start_date = Some("2028-02-29".into())).is_ok());
        assert!(with(&|a| a.start_date = Some(" ".into())).unwrap().alerts[0].start_date.is_none());
    }

    fn seeded() -> Alerts {
        let mut a = Alerts::default();
        a.alerts.push(alert("mine", "My boss", "20:00", None));
        assert!(seed_defaults(&mut a));
        validate(a).unwrap()
    }

    #[test]
    fn the_built_in_timers_are_valid() {
        let a = seeded();
        assert_eq!(a.alerts.len(), 1 + default_alerts().len());
        assert_eq!(a.defaults_version, DEFAULTS_VERSION);
        assert!(a.alerts.iter().all(|x| x.enabled), "every built-in timer starts enabled");
        let reset = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("daily-reset")).unwrap();
        assert_eq!(reset.clock, Clock::Utc);
        assert_eq!(a.alerts[0].name, "My boss", "user event kept first and unchanged");
    }

    #[test]
    fn existing_installs_keep_their_disabled_hourly_timers() {
        let mut a = seeded();
        for x in a.alerts.iter_mut().filter(|x| x.default_id.as_deref() == Some("shugo-festival")) {
            x.enabled = false;
        }
        assert!(!seed_defaults(&mut a), "same defaults_version does not re-seed");
        let shugo = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("shugo-festival")).unwrap();
        assert!(!shugo.enabled, "a user's existing timer is not switched on");
        a.alerts.retain(|x| x.default_id.as_deref() != Some("dimensional-invasion"));
        restore_defaults(&mut a);
        let invasion = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("dimensional-invasion")).unwrap();
        assert!(invasion.enabled, "restored missing timer uses the new enabled default");
        assert_eq!(invasion.time, "00:30");
        assert_eq!(invasion.name, "Dimensional Invasion");
    }

    #[test]
    fn seeding_happens_once_and_deletes_stick() {
        let mut a = seeded();
        a.alerts.retain(|x| x.default_id.as_deref() != Some("watcher-kaira"));
        let before = a.clone();
        assert!(!seed_defaults(&mut a));
        assert_eq!(a, before);
        // Saved and loaded again, still not re-added.
        let text = serde_json::to_string(&a).unwrap();
        let mut back = validate(serde_json::from_str(&text).unwrap()).unwrap();
        assert!(!seed_defaults(&mut back));
        assert!(back.alerts.iter().all(|x| x.default_id.as_deref() != Some("watcher-kaira")));
    }

    #[test]
    fn restore_adds_only_missing_and_keeps_edits_and_user_events() {
        let mut a = seeded();
        a.alerts.retain(|x| !matches!(x.default_id.as_deref(), Some("executors" | "daily-reset")));
        let nahma = a.alerts.iter_mut().find(|x| x.default_id.as_deref() == Some("guardian-lord-nahma")).unwrap();
        nahma.time = "20:00".into();
        nahma.name = "Nahma (mine)".into();
        nahma.enabled = false;
        let edited = nahma.clone();
        let user = a.alerts[0].clone();
        assert_eq!(restore_defaults(&mut a), 2);
        let a = validate(a).unwrap();
        assert_eq!(a.alerts.len(), 1 + default_alerts().len());
        assert_eq!(a.alerts[0], user);
        assert!(a.alerts.contains(&edited));
        let mut again = a.clone();
        assert_eq!(restore_defaults(&mut again), 0);
        assert_eq!(again, a);
    }

    #[test]
    fn restore_avoids_id_clashes_and_the_cap() {
        let mut a = Alerts::default();
        a.alerts.push(alert("d-daily-reset", "Not a default", "10:00", None));
        assert_eq!(restore_defaults(&mut a), default_alerts().len());
        let a = validate(a).unwrap();
        assert_eq!(a.alerts[0].default_id, None);
        let reset = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("daily-reset")).unwrap();
        assert_eq!(reset.id, "d-daily-reset-2");
        let mut full = Alerts::default();
        for i in 0..MAX_ALERTS - 1 {
            full.alerts.push(alert(&format!("u{i}"), "U", "10:00", None));
        }
        assert_eq!(restore_defaults(&mut full), 1);
        assert_eq!(full.alerts.len(), MAX_ALERTS);
    }

    #[test]
    fn old_files_load_without_the_new_fields() {
        let text = r#"{"schema_version":2,"enabled":true,"lead_mins":[5],"alerts":[
            {"id":"a","name":"Boss","time":"20:00","repeat":{"type":"daily","every":1}}]}"#;
        let a = validate(serde_json::from_str(text).unwrap()).unwrap();
        assert_eq!(a.defaults_version, 0);
        assert_eq!(a.server_utc_offset_mins, DEFAULT_SERVER_OFFSET_MINS);
        assert_eq!(a.alerts[0].clock, Clock::Local);
        assert_eq!(a.alerts[0].default_id, None);
        let saved = serde_json::to_string(&a).unwrap();
        assert!(!saved.contains("\"clock\""), "local clock is not written: {saved}");
    }

    #[test]
    fn v2_migrates_schedule_keeps_flags_and_renames_beritra() {
        let mut a = Alerts::default();
        a.defaults_version = 1;
        a.alerts = vec![
            alert("mine", "My boss", "20:00", None),
        ];
        // Old v1 built-ins: Monday siege, Beritra name, 02:00 rift.
        let mut siege = builtin("artifact-siege", "Artifact Siege", "", "21:00", Repeat::Weekly { days: vec![0, 3, 5] }, Clock::Server, true);
        siege.enabled = false;
        siege.lead_mins = Some(vec![10]);
        let mut beritra = builtin("beritra-air-raid", "Beritra Air Raid", "Lasts 10 min", "00:30", Repeat::Hours { every: 1 }, Clock::Server, true);
        beritra.enabled = false;
        let rift = builtin("spacetime-rift", "Spacetime Rift", "Open for 10 min", "02:00", Repeat::Hours { every: 3 }, Clock::Server, true);
        a.alerts.extend([siege, beritra, rift]);
        assert!(seed_defaults(&mut a));
        assert_eq!(a.defaults_version, DEFAULTS_VERSION);
        assert_eq!(a.alerts[0].name, "My boss", "custom event is left alone");
        let siege = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("artifact-siege")).unwrap();
        assert_eq!(siege.time, "17:00");
        assert_eq!(siege.repeat, Some(Repeat::Weekly { days: vec![3, 5] }));
        assert!(!siege.enabled, "enabled flag is kept");
        assert_eq!(siege.lead_mins.as_deref(), Some(&[10][..]));
        let invasion_count = a.alerts.iter().filter(|x| x.default_id.as_deref() == Some("dimensional-invasion")).count();
        assert_eq!(invasion_count, 1, "Beritra is renamed, not added beside a new invasion timer");
        let invasion = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("dimensional-invasion")).unwrap();
        assert_eq!(invasion.name, "Dimensional Invasion");
        assert!(!invasion.enabled);
        assert!(a.alerts.iter().all(|x| x.default_id.as_deref() != Some("beritra-air-raid")));
        let rift = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("spacetime-rift")).unwrap();
        assert_eq!(rift.time, "02:00");
        assert_eq!(rift.message, "Open for 3 min");
        assert!(a.alerts.iter().any(|x| x.default_id.as_deref() == Some("weekly-reset")));
        assert!(a.alerts.iter().any(|x| x.default_id.as_deref() == Some("argo-spirit-king")));
        let argo = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("argo-spirit-king")).unwrap();
        assert_eq!(argo.time, "06:30");
        assert_eq!(argo.repeat, Some(Repeat::Hours { every: 12 }));
        assert!(!seed_defaults(&mut a));
    }

    #[test]
    fn v3_updates_stock_rift_message_but_keeps_a_custom_one() {
        let mut stock = Alerts::default();
        stock.defaults_version = 2;
        stock.alerts = vec![builtin(
            "spacetime-rift",
            "Spacetime Rift",
            "Open for 10 min",
            "01:00",
            Repeat::Hours { every: 3 },
            Clock::Server,
            true,
        )];
        assert!(prepare_alerts(&mut stock));
        assert_eq!(stock.defaults_version, DEFAULTS_VERSION);
        assert_eq!(stock.alerts[0].message, "Open for 3 min");
        assert_eq!(stock.alerts[0].time, "02:00");
        assert!(!prepare_alerts(&mut stock), "repair is idempotent");

        let mut custom = Alerts::default();
        custom.defaults_version = 2;
        let mut rift = builtin(
            "spacetime-rift",
            "Spacetime Rift",
            "Open for 10 min",
            "01:00",
            Repeat::Hours { every: 3 },
            Clock::Server,
            true,
        );
        rift.message = "My own note".into();
        rift.enabled = false;
        custom.alerts = vec![rift];
        assert!(prepare_alerts(&mut custom));
        assert_eq!(custom.alerts[0].message, "My own note");
        assert_eq!(custom.alerts[0].time, "02:00");
        assert!(!custom.alerts[0].enabled);
        assert_eq!(custom.defaults_version, DEFAULTS_VERSION);
        assert!(!prepare_alerts(&mut custom), "repair is idempotent");
    }

    #[test]
    fn prepare_dedupes_a_broken_v2_file_and_keeps_the_user_copy() {
        let mut a = Alerts::default();
        a.defaults_version = DEFAULTS_VERSION;
        let mut beritra = builtin("beritra-air-raid", "Beritra Air Raid", "Lasts 10 min", "00:30", Repeat::Hours { every: 1 }, Clock::Server, true);
        beritra.enabled = false;
        beritra.lead_mins = Some(vec![1]);
        let extra = builtin("dimensional-invasion", "Dimensional Invasion", "new copy", "00:30", Repeat::Hours { every: 1 }, Clock::Server, true);
        a.alerts = vec![beritra, extra];
        assert!(prepare_alerts(&mut a));
        assert_eq!(a.alerts.len(), 1);
        assert_eq!(a.alerts[0].default_id.as_deref(), Some("dimensional-invasion"));
        assert!(!a.alerts[0].enabled, "the user's earlier copy is kept");
        assert_eq!(a.alerts[0].lead_mins.as_deref(), Some(&[1][..]));
        assert_eq!(a.defaults_version, DEFAULTS_VERSION);
        assert!(!prepare_alerts(&mut a), "repair is idempotent");
        assert!(validate(a).is_ok());
    }

    #[test]
    fn na_east_hours_are_three_ahead_of_na_west() {
        let west = default_alerts_for(None);
        let east = default_alerts_for(Some(Region::NaEast));
        let time = |rows: &[Alert], id: &str| {
            rows.iter().find(|a| a.default_id.as_deref() == Some(id)).unwrap().time.clone()
        };
        let days = |rows: &[Alert], id: &str| match rows.iter().find(|a| a.default_id.as_deref() == Some(id)).unwrap().repeat.clone() {
            Some(Repeat::Weekly { days }) => days,
            other => panic!("{other:?}"),
        };
        assert_eq!(time(&west, "artifact-siege"), "17:00");
        assert_eq!(time(&east, "artifact-siege"), "20:00");
        assert_eq!(days(&west, "artifact-siege"), vec![3, 5]);
        assert_eq!(days(&east, "artifact-siege"), vec![3, 5]);
        assert_eq!(time(&west, "spacetime-rift"), "02:00");
        assert_eq!(time(&east, "spacetime-rift"), "05:00");
        assert_eq!(time(&west, "watcher-kaira"), "01:00");
        assert_eq!(time(&east, "watcher-kaira"), "04:00");
        assert_eq!(time(&west, "executors"), "17:30");
        assert_eq!(time(&east, "executors"), "20:30");
        assert_eq!(time(&west, "argo-spirit-king"), "06:30");
        assert_eq!(time(&east, "argo-spirit-king"), "09:30");
        assert_eq!(time(&east, "shugo-festival"), "00:00", "hourly anchors are not shifted");
        assert_eq!(time(&west, "daily-reset"), "07:00");
        assert_eq!(time(&east, "daily-reset"), "07:00");
        let eu = default_alerts_for(Some(Region::Europe));
        assert_eq!(time(&eu, "artifact-siege"), "17:00", "Europe uses the same local hours as NA West");
    }

    #[test]
    fn syncing_the_region_rewrites_builtin_hours_only() {
        let mut a = Alerts::default();
        restore_defaults(&mut a);
        let shugo = a.alerts.iter_mut().find(|x| x.default_id.as_deref() == Some("shugo-festival")).unwrap();
        shugo.enabled = false;
        shugo.lead_mins = Some(vec![1]);
        a.region = Some(Region::NaEast);
        sync_builtin_schedule(&mut a);
        let siege = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("artifact-siege")).unwrap();
        assert_eq!(siege.time, "20:00");
        let shugo = a.alerts.iter().find(|x| x.default_id.as_deref() == Some("shugo-festival")).unwrap();
        assert!(!shugo.enabled);
        assert_eq!(shugo.lead_mins.as_deref(), Some(&[1][..]));
    }

    #[test]
    fn bad_offsets_and_default_ids_are_refused() {
        let mut a = Alerts { server_utc_offset_mins: 15 * 60, ..Alerts::default() };
        assert!(validate(a.clone()).is_err());
        a.server_utc_offset_mins = -5 * 60;
        let mut one = alert("a", "A", "10:00", None);
        one.default_id = Some("x".into());
        let mut two = alert("b", "B", "10:00", None);
        two.default_id = Some("x".into());
        a.alerts = vec![one, two];
        assert!(validate(a).is_err());
    }
}
