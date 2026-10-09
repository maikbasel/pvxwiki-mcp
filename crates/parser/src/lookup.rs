//! Profession and attribute names, ported from the PvXCode extension
//! (github.com/Wikia/pvxcode, gwbbcode/gwbbcode.inc.php at commit afe42e3: `gws_prof_name`,
//! `gws_attr_name`). Same entries, same order, same first-prefix-match rule, so a name resolves
//! exactly as the wiki renders it.

use crate::Profession;

/// The first entry the lowercased input is a prefix of wins.
const PROFESSIONS: [(&str, Profession); 10] = [
    ("elementalist", Profession::Elementalist),
    ("mesmer", Profession::Mesmer),
    ("monk", Profession::Monk),
    ("necromancer", Profession::Necromancer),
    ("ranger", Profession::Ranger),
    ("warrior", Profession::Warrior),
    ("assassin", Profession::Assassin),
    ("ritualist", Profession::Ritualist),
    ("dervish", Profession::Dervish),
    ("paragon", Profession::Paragon),
];

/// Key: lowercase name without spaces. The first key the input is a prefix of wins.
const ATTRIBUTES: [(&str, &str); 50] = [
    ("airmagic", "Air Magic"),
    ("earthmagic", "Earth Magic"),
    ("energystorage", "Energy Storage"),
    ("firemagic", "Fire Magic"),
    ("watermagic", "Water Magic"),
    ("dominationmagic", "Domination Magic"),
    ("fastcasting", "Fast Casting"),
    ("illusionmagic", "Illusion Magic"),
    ("inspirationmagic", "Inspiration Magic"),
    ("divinefavor", "Divine Favor"),
    ("healingprayers", "Healing Prayers"),
    ("protectionprayers", "Protection Prayers"),
    ("smitingprayers", "Smiting Prayers"),
    ("bloodmagic", "Blood Magic"),
    ("curses", "Curses"),
    ("deathmagic", "Death Magic"),
    ("soulreaping", "Soul Reaping"),
    ("beastmastery", "Beast Mastery"),
    ("expertise", "Expertise"),
    ("marksmanship", "Marksmanship"),
    ("wildernesssurvival", "Wilderness Survival"),
    ("axemastery", "Axe Mastery"),
    ("hammermastery", "Hammer Mastery"),
    ("strength", "Strength"),
    ("swordsmanship", "Swordsmanship"),
    ("tactics", "Tactics"),
    ("criticalstrikes", "Critical Strikes"),
    ("daggermastery", "Dagger Mastery"),
    ("deadlyarts", "Deadly Arts"),
    ("shadowarts", "Shadow Arts"),
    ("spawningpower", "Spawning Power"),
    ("channelingmagic", "Channeling Magic"),
    ("communing", "Communing"),
    ("restorationmagic", "Restoration Magic"),
    ("spearmastery", "Spear Mastery"),
    ("command", "Command"),
    ("motivation", "Motivation"),
    ("leadership", "Leadership"),
    ("scythemastery", "Scythe Mastery"),
    ("windprayers", "Wind Prayers"),
    ("earthprayers", "Earth Prayers"),
    ("mysticism", "Mysticism"),
    ("kurzickrank", "Kurzick rank"),
    ("luxonrank", "Luxon rank"),
    ("sunspearrank", "Sunspear rank"),
    ("lightbringerrank", "Lightbringer rank"),
    ("asurarank", "Asura rank"),
    ("deldrimorrank", "Deldrimor rank"),
    ("ebonvanguardrank", "Ebon Vanguard rank"),
    ("nornrank", "Norn rank"),
];

pub(crate) fn profession(input: &str) -> Option<Profession> {
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return None;
    }
    // PvXCode special case: "rt" is the only abbreviation that is not a prefix of the name.
    if input == "rt" {
        return Some(Profession::Ritualist);
    }
    PROFESSIONS
        .iter()
        .find(|(name, _)| name.starts_with(&input))
        .map(|&(_, profession)| profession)
}

pub(crate) fn attribute(input: &str) -> Option<&'static str> {
    let key = input.replace('_', "").to_lowercase();
    if key.is_empty() {
        return None;
    }
    ATTRIBUTES
        .iter()
        .find(|(long, _)| long.starts_with(&key))
        .map(|&(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profession::*;

    #[test]
    fn profession_abbreviations_resolve_like_the_wiki() {
        for (input, expected) in [
            ("A", Assassin),
            ("a", Assassin),
            ("assas", Assassin),
            ("Assassin", Assassin),
            ("Me", Mesmer),
            ("m", Mesmer),
            ("Mo", Monk),
            ("Monk", Monk),
            ("Rt", Ritualist),
            ("rit", Ritualist),
            ("R", Ranger),
            ("E", Elementalist),
            ("N", Necromancer),
            ("W", Warrior),
            ("D", Dervish),
            ("P", Paragon),
        ] {
            assert_eq!(profession(input), Some(expected), "{input}");
        }
    }

    #[test]
    fn any_and_unknown_professions_resolve_to_none() {
        for input in ["any", "Any", "x", "", "?"] {
            assert_eq!(profession(input), None, "{input}");
        }
    }

    #[test]
    fn attribute_prefixes_resolve_in_table_order() {
        for (input, expected) in [
            ("sha", "Shadow Arts"),
            ("Shadow", "Shadow Arts"),
            ("Deadly_Arts", "Deadly Arts"),
            ("deadly", "Deadly Arts"),
            ("dea", "Death Magic"),
            ("CriticalStrikes", "Critical Strikes"),
            ("crit", "Critical Strikes"),
            ("dag", "Dagger Mastery"),
            ("earth", "Earth Magic"),
            ("earthp", "Earth Prayers"),
            ("comm", "Communing"),
            ("comma", "Command"),
            ("Domination", "Domination Magic"),
            ("Illusion_Magic", "Illusion Magic"),
            ("airmagic", "Air Magic"),
            ("nor", "Norn rank"),
        ] {
            assert_eq!(attribute(input), Some(expected), "{input}");
        }
    }

    #[test]
    fn unknown_attributes_resolve_to_none() {
        for input in ["foo", "", "12", "_"] {
            assert_eq!(attribute(input), None, "{input}");
        }
    }
}
