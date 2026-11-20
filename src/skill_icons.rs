//! Skill icons for the buff bar. `data/skill-icons.json` maps a skill family
//! (`15040000`) to its icon name and class, as listed on the aion2data.com
//! class pages. Icons are downloaded from aion2data.com only, once, into
//! `%LOCALAPPDATA%\aion2dps\skill-icons`, and shown as `data:` URLs. Nothing
//! is ever sent to the game.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
pub struct IconInfo {
    pub icon: String,
    pub class: String,
}

#[derive(Debug, Default, Deserialize)]
struct Table {
    icon_url: String,
    skills: BTreeMap<i32, IconInfo>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: Table = serde_json::from_str(include_str!("../data/skill-icons.json")).unwrap_or_default();
        table.skills.retain(|_, info| safe_name(&info.icon));
        table
    })
}

fn safe_name(icon: &str) -> bool {
    !icon.is_empty() && icon.len() <= 80 && icon.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A skill's family: `15250047` is in `15250000`. Short ids stay as they are.
pub fn family(id: i32) -> i32 {
    if id >= 10_000_000 {
        id - id.rem_euclid(10_000)
    } else {
        id
    }
}

/// The icon name for a skill or its family.
pub fn icon_for(id: i32) -> Option<&'static str> {
    let skills = &table().skills;
    skills.get(&id).or_else(|| skills.get(&family(id))).map(|i| i.icon.as_str())
}

/// The class whose skill list has this family, like "Sorcerer".
pub fn class_of(id: i32) -> Option<&'static str> {
    table().skills.get(&family(id)).map(|i| i.class.as_str())
}

/// Every family in the table: `(id, icon, class)`.
pub fn all() -> impl Iterator<Item = (i32, &'static IconInfo)> {
    table().skills.iter().map(|(id, info)| (*id, info))
}

fn known_icon(icon: &str) -> bool {
    safe_name(icon) && table().skills.values().any(|i| i.icon == icon)
}

pub fn icon_dir() -> PathBuf {
    crate::storage::folder().join("skill-icons")
}

/// The cached icon as a `data:` URL; `None` until it was downloaded.
pub fn cached(icon: &str) -> Option<String> {
    if !known_icon(icon) {
        return None;
    }
    let bytes = std::fs::read(icon_dir().join(format!("{icon}.webp"))).ok()?;
    crate::pets::is_webp(&bytes).then(|| format!("data:image/webp;base64,{}", crate::pets::base64(&bytes)))
}

/// Download one icon from aion2data.com (only names in the bundled table).
/// Blocking, up to about 10 s; run it off the capture and UI threads.
pub fn fetch(icon: &str) -> bool {
    if !known_icon(icon) || !table().icon_url.starts_with("https://aion2data.com/") {
        return false;
    }
    let path = icon_dir().join(format!("{icon}.webp"));
    if path.exists() {
        return true;
    }
    crate::pets::download_webp(&table().icon_url.replace("{icon}", icon), &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_find_their_icons() {
        assert_eq!(icon_for(15_710_007), Some("ICON_SO_SKILL_Passive_002"), "Fire Mark");
        assert_eq!(icon_for(15_310_450), Some("ICON_SO_SKILL_028"), "Wish of Concentration");
        assert_eq!(class_of(15_050_350), Some("Sorcerer"));
        assert_eq!(icon_for(15_250_047), None, "Pyroclasm has no icon on aion2data");
        assert_eq!(icon_for(2_011_101), None);
        assert!(!known_icon("../x"));
    }
}
