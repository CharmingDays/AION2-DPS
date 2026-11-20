# Can buffs and debuffs be detected from packets?

Research note for the to-do item "detect buffs/debuffs during combat". Nothing
here is implemented; the meter stays passive and only reads the game traffic
it already captures.

## What the decoder reads today

| Opcode | What it is | Buff/debuff relevance |
| --- | --- | --- |
| `04 38` | Direct damage/heal record (layout after A2Tools DPS Meter) | Carries hit flags (crit, back, block, parry, ...). No buff or status field is decoded. |
| `05 38` | Periodic tick (DoT/HoT). Effect byte `0x02/0x0A/0x06/0x0E` = damage, `0x01/0x09/0x0B` = heal; other effect values are dropped | The strongest lead. A tick proves an effect is *on* a target, with its skill id and caster, but only for effects that deal damage or heal. Pure stat buffs/debuffs (attack up, slow, armor break) produce no tick. The dropped effect values have never been logged and may be other periodic effects. |
| `05 E0` | Boss stagger gauge (`03 01` update, `00 03 02` break) | Shows stagger debuff progress only. |
| `40/41 36` | Spawns; summon kinds `0x5F`, `0x1C`, `0x1F`, `0x1D`, `0x5D` | Kind `0x1C` is a *skill effect attached to the skill's target*, carrying the caster's name. Together with `42 36` (removal) this gives an effect's start and end on a target. Spirits also carry a "buff block" (`SPIRIT_ANCHOR`) that names their owner. Only the caster is read from these today; the effect/skill id inside is not decoded. |
| `42 36` | Despawn/removal (flag 1 despawn, 3 after a kill, 7 cleanup) | Ends a `0x1C` effect entity. |
| `8D`, `4A 36`, `33 36`, ... | HP, "you", names | None. |

A hit into a shield records only the damage that got through; a shield id can
follow the amount in `05 38` but shield-apply packets are not parsed.

## What would be needed

1. **Find the apply/remove opcodes.** Buff and debuff application almost
   certainly has its own server packet (icon, duration, stacks, caster,
   target). It is not among the opcodes above. Finding it needs raw captures:
   extend the existing raw-packet debug logs (`target-debug.jsonl`) to log
   every unknown opcode for a few seconds around a known buff cast (for
   example a Chanter self-buff on a training dummy), then diff casts with and
   without the buff.
2. **Decode `0x1C` effect spawns fully** (effect/skill id, target, duration if
   present) and pair them with `42 36` removals for uptime.
3. **Log the `05 38` effect bytes that are dropped today** to see whether they
   are non-damage periodic effects.
4. **A skill-id table for buffs/debuffs.** `skills_en.json` has ids and names
   only; there is no category or duration data, so a buff list would have to
   be curated by hand or from a data site.
5. **Check the reference meters** (A2Tools DPS Meter, RATmeter), whose
   decoders the spawn layouts came from, for any buff/status packet layout
   before reverse-engineering one from scratch. None is used in this repo yet.

## Recommendation

Feasible in part, not worth building blind. Short term, damage/heal-over-time
effects can already be listed per target from `05 38` ticks, and `0x1C` effect
spawns can give an uptime estimate for ground/target effects once their skill
id is decoded. True buff/debuff tracking (stat buffs, CC, stacks) needs the
apply/remove packet to be identified first. Next step: add an opt-in
"unknown opcodes" capture to the debug logs and collect a few labelled
sessions in game; only then decide whether to light up the greyed-out Buffs,
Debuffs and Shields filters in Ability share.

## Decoded (2026-11-20 capture)
`decode::parse_effects` reads `2A 38` / `2B 38` / `2C 38` and `effects::Effects` tracks them for the
buff bar. Field check against the capture: after the duration comes a **4-byte** field (0 in every
skill effect; -1 on kind 0x11), then the server expiry u64, so the caster starts 20 bytes after the
effect id. Kind 0x11 (flight, rest, hidden states) is skipped. Expiry is local receive time plus the
duration; `2C 38`, a despawn, a new server connection or a fight reset clear effects.
