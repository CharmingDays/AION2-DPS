// Groups the Ability share tab's rows. A skill's DoT ticks and direct hits
// are one row, and variants of one skill (ids in the same 10000 block whose
// name, up to " - ", is the same, e.g. "Dimensional Control" and
// "Dimensional Control - Delayed Damage") fold into the plain skill's row.
// Different skills that share an id block (spirit attacks) stay apart.
//
// Type filters: Damage, Heals and Stagger come from recorded events (hits,
// heal ticks, estimated stagger shares). Buffs, Debuffs and Shields have no
// packet or skill-table source in this app, so those filters stay unavailable.
(function (root) {
  'use strict';

  /** Rows past this many fold into one "Other (N)" row. */
  const SHARE_ROWS = 12;

  const UNAVAILABLE = {
    buffs: 'The meter does not record buff casts or applications. The skill table has only English names, not categories.',
    debuffs: 'The meter does not record debuff casts or applications. The skill table has only English names, not categories.',
    shields: 'Shields and absorbs are not counted. A hit into a shield records only the damage that got through, and shield-apply packets are not parsed.',
  };

  const TYPES = [
    { id: 'all', label: 'All', available: true },
    { id: 'damage', label: 'Damage', available: true, metric: 'damage' },
    { id: 'heals', label: 'Heals', available: true, metric: 'heal' },
    { id: 'stagger', label: 'Stagger', available: true, metric: 'stagger' },
    { id: 'buffs', label: 'Buffs', available: false, reason: UNAVAILABLE.buffs },
    { id: 'debuffs', label: 'Debuffs', available: false, reason: UNAVAILABLE.debuffs },
    { id: 'shields', label: 'Shields', available: false, reason: UNAVAILABLE.shields },
  ];

  const typeInfo = (id) => TYPES.find((t) => t.id === id) || TYPES[0];

  const stem = (name) => {
    const i = name.indexOf(' - ');
    return i > 0 ? name.slice(0, i) : name;
  };

  /** The row a skill id belongs to: block + name stem, or the id when unnamed. */
  function groupKey(skill, names) {
    const name = names && names[skill];
    return name ? `${Math.floor(skill / 10000)}|${stem(name)}` : `#${skill}`;
  }

  function blank(key, skill, name) {
    return {
      key, skill, name,
      damage: 0, hits: 0, crits: 0, dotDamage: 0,
      heal: 0, healHits: 0, hotHeal: 0,
      stagger: 0, staggerHits: 0,
      ids: new Set(),
      per: new Map(), perHeal: new Map(), perStagger: new Map(),
    };
  }

  /**
   * skillMaps: per actor, a Map of { skill, dot, damage, hits, crits } plus
   * optional heal, healHits, hot, stagger, staggerHits.
   * actors: the actor indexes to include. Returns rows with
   * { key, skill, name, damage, hits, crits, dotDamage, heal, healHits, hotHeal,
   *   stagger, staggerHits, variants, by, byHeal, byStagger }.
   */
  function merge(skillMaps, actors, names) {
    const rows = new Map();
    for (const a of actors) {
      const m = skillMaps[a];
      if (!m) continue;
      for (const r of m.values()) {
        const key = groupKey(r.skill, names);
        let g = rows.get(key);
        if (!g) {
          const n = names && names[r.skill];
          rows.set(key, (g = blank(key, r.skill, n ? stem(n) : null)));
        }
        if (r.skill < g.skill) g.skill = r.skill;
        g.ids.add(r.skill);
        const damage = r.damage || 0;
        const heal = r.heal || 0;
        const stagger = r.stagger || 0;
        g.damage += damage;
        g.hits += r.hits || 0;
        g.crits += r.crits || 0;
        if (r.dot) g.dotDamage += damage;
        g.heal += heal;
        g.healHits += r.healHits || 0;
        if (r.hot || (r.dot && heal)) g.hotHeal += heal;
        g.stagger += stagger;
        g.staggerHits += r.staggerHits || 0;
        if (damage) g.per.set(a, (g.per.get(a) || 0) + damage);
        if (heal) g.perHeal.set(a, (g.perHeal.get(a) || 0) + heal);
        if (stagger) g.perStagger.set(a, (g.perStagger.get(a) || 0) + stagger);
      }
    }
    return [...rows.values()].map((g) => ({
      key: g.key, skill: g.skill, name: g.name,
      damage: g.damage, hits: g.hits, crits: g.crits, dotDamage: g.dotDamage,
      heal: g.heal, healHits: g.healHits, hotHeal: g.hotHeal,
      stagger: g.stagger, staggerHits: g.staggerHits,
      variants: g.ids.size,
      by: [...g.per], byHeal: [...g.perHeal], byStagger: [...g.perStagger],
    }));
  }

  /** Heal maps used by history (heal amount stored as `damage`, HoT as `dot`). */
  function fromHeals(skillMaps) {
    return (skillMaps || []).map((m) => {
      const out = new Map();
      if (!m) return out;
      for (const [k, r] of m) {
        const heal = r.heal != null ? r.heal : r.damage;
        if (!(heal > 0)) continue;
        out.set(k, {
          skill: r.skill,
          heal,
          healHits: r.healHits || r.hits || 0,
          hot: r.hot || r.dot,
          damage: 0, hits: 0, crits: 0, dot: false, stagger: 0, staggerHits: 0,
        });
      }
      return out;
    });
  }

  /** Stagger maps used by history (points stored as `damage`). */
  function fromStagger(skillMaps) {
    return (skillMaps || []).map((m) => {
      const out = new Map();
      if (!m) return out;
      for (const [k, r] of m) {
        const stagger = r.stagger != null ? r.stagger : r.damage;
        if (!(stagger > 0)) continue;
        out.set(k, {
          skill: r.skill,
          stagger,
          staggerHits: r.staggerHits || r.hits || 0,
          damage: 0, hits: 0, crits: 0, dot: false, heal: 0, healHits: 0,
        });
      }
      return out;
    });
  }

  /** One row set from damage, heal and stagger maps (each already per actor). */
  function combine(damageMaps, healSkillMaps, staggerSkillMaps, actors, names) {
    const heals = fromHeals(healSkillMaps);
    const staggers = fromStagger(staggerSkillMaps);
    const n = Math.max(damageMaps && damageMaps.length || 0, heals.length, staggers.length);
    const maps = Array.from({ length: n }, () => new Map());
    const add = (src, tag) => {
      (src || []).forEach((m, i) => {
        if (!m || !maps[i]) return;
        for (const [k, r] of m) maps[i].set(`${tag}:${k}`, r);
      });
    };
    add(damageMaps, 'd');
    add(heals, 'h');
    add(staggers, 's');
    return merge(maps, actors, names);
  }

  const critRate = (r) => (r.hits ? r.crits / r.hits : 0);
  const hitCount = (r, type) => {
    if (type === 'heals') return r.healHits || 0;
    if (type === 'stagger') return r.staggerHits || 0;
    if (type === 'all') return (r.hits || 0) + (r.healHits || 0) + (r.staggerHits || 0);
    return r.hits || 0;
  };
  const amount = (r, type) => {
    if (type === 'heals') return r.heal || 0;
    if (type === 'stagger') return r.stagger || 0;
    return r.damage || 0;
  };
  const matches = (r, type) => {
    if (type === 'heals') return r.heal > 0;
    if (type === 'stagger') return r.stagger > 0;
    if (type === 'all') return r.damage > 0 || r.heal > 0 || r.stagger > 0;
    if (type === 'damage') return r.damage > 0;
    return false;
  };

  const SORTS = {
    damage: (a, b) => b.damage - a.damage || b.hits - a.hits,
    hits: (a, b, type) => hitCount(b, type) - hitCount(a, type) || amount(b, type) - amount(a, type),
    crit: (a, b) => critRate(b) - critRate(a) || b.damage - a.damage,
    heal: (a, b) => b.heal - a.heal || b.healHits - a.healHits,
    stagger: (a, b) => b.stagger - a.stagger || b.staggerHits - a.staggerHits,
  };

  /** Sorts allowed for a type; `hasHeals` / `hasStagger` hide unused All sorts. */
  function sortsFor(type, opts) {
    const hasHeals = !opts || opts.hasHeals !== false;
    const hasStagger = !opts || opts.hasStagger !== false;
    if (type === 'heals') return ['heal', 'hits'];
    if (type === 'stagger') return ['stagger', 'hits'];
    if (type === 'all') {
      const sorts = ['damage'];
      if (hasHeals) sorts.push('heal');
      if (hasStagger) sorts.push('stagger');
      sorts.push('hits');
      return sorts;
    }
    return ['damage', 'hits', 'crit'];
  }

  const defaultSort = (type) => (type === 'heals' ? 'heal' : type === 'stagger' ? 'stagger' : 'damage');

  /** Sorted rows of one type, split into the shown ones and the rest ("Other"). */
  function split(rows, sort = 'damage', cutoff = SHARE_ROWS, type = 'damage') {
    const cmp = SORTS[sort] || SORTS[defaultSort(type)] || SORTS.damage;
    const all = rows.filter((r) => matches(r, type)).sort((a, b) => cmp(a, b, type));
    const n = all.length > cutoff + 1 ? cutoff : all.length;
    return { shown: all.slice(0, n), rest: all.slice(n) };
  }

  const api = {
    SHARE_ROWS, TYPES, UNAVAILABLE, typeInfo, stem, groupKey, merge, fromHeals,
    fromStagger, combine, matches, amount, hitCount, sortsFor, defaultSort, split,
  };
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.abilityShare = api;
})(typeof window !== 'undefined' ? window : globalThis);
