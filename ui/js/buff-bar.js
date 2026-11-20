// The buff bar: one line of icons, your buffs then your debuffs on the
// target, each with a countdown. Used by the buff bar window, the Config
// preview and the placer. Icons come from aion2data.com through the app
// (cached on this PC); without one a tile shows the name's initials.
(function (root) {
  'use strict';

  const BUFF = '#4ade80';
  const DEBUFF = '#f87171';
  const icons = new Map();

  /**
   * A cached icon `data:` URL, or null for now. Asks the app once (it
   * downloads from aion2data.com unless `download` is false) and calls `ready`
   * when the icon arrives.
   */
  function iconUrl(icon, ready, download = true) {
    if (!icon) return null;
    const have = icons.get(icon) || { url: null, asked: 0 };
    if (have.url) return have.url;
    const level = download ? 2 : 1;
    if (have.asked < level && root.meterBridge && root.meterBridge.skillIcon) {
      have.asked = level;
      icons.set(icon, have);
      root.meterBridge.skillIcon(icon, download).then((url) => {
        if (url) { have.url = url; if (ready) ready(); }
      }).catch(() => {});
    }
    return null;
  }

  /** "Fire Mark" -> "FM", "Pyroclasm: Fire Damage +5%" -> "PF", unknown ids -> "#047". */
  function initials(row) {
    if (!row.named) return `#${String(row.skill).slice(-3)}`;
    const words = String(row.name).replace(/[^\p{L}\p{N}\s]/gu, ' ').split(/\s+/).filter(Boolean);
    return (words.length > 1 ? words[0][0] + words[1][0] : (words[0] || '?').slice(0, 2)).toUpperCase();
  }

  /** A steady colour per skill for the initials tile. */
  function hue(skill) {
    let h = 0;
    for (const c of String(skill)) h = (h * 31 + c.charCodeAt(0)) % 360;
    return h;
  }

  /** "1m", "12", "3.4" seconds left; "" when it lasts until removed. */
  function timeText(ms) {
    if (ms == null) return '';
    const s = ms / 1000;
    if (s >= 60) return `${Math.floor(s / 60)}m`;
    if (s >= 10) return String(Math.floor(s));
    return s.toFixed(1);
  }

  /** Where the icons sit in the window, from the bar's screen spot. */
  function align(style, layout) {
    const pos = !style || style.position === 'auto' ? 'top' : style.position;
    const side = (pct) => (pct < 34 ? 'start' : pct > 66 ? 'end' : 'center');
    let col = /left$/.test(pos) ? 'start' : /right$/.test(pos) ? 'end' : 'center';
    let row = /^top/.test(pos) ? 'start' : /^bottom/.test(pos) ? 'end' : 'center';
    if (pos === 'custom') { col = side(Number(style.x_pct)); row = side(Number(style.y_pct)); }
    return layout === 'vertical' ? { main: row, cross: col } : { main: col, cross: row };
  }

  function isHex(c) { return typeof c === 'string' && /^#[0-9a-f]{6}$/i.test(c); }

  /**
   * A bar in `host`. `update(view, style, layout)` takes the snapshot's
   * `buffs` ({ player, target }); between snapshots `tick()` counts down.
   */
  function create(host) {
    const wrap = document.createElement('div');
    wrap.className = 'buff-wrap';
    const bar = document.createElement('div');
    bar.className = 'buff-bar';
    wrap.append(bar);
    host.append(wrap);
    const tiles = new Map();
    let rows = [];
    let at = 0;
    let sep = null;

    function tileFor(key) {
      let t = tiles.get(key);
      if (t) return t;
      const el = document.createElement('div');
      el.className = 'buff';
      el.innerHTML = '<span class="buff-face"></span><span class="buff-sweep"></span><span class="buff-time"></span><span class="buff-count"></span>';
      t = { el, face: el.children[0], sweep: el.children[1], time: el.children[2], count: el.children[3], icon: undefined, label: '' };
      tiles.set(key, t);
      return t;
    }

    function paintFace(t, row) {
      const url = iconUrl(row.icon, () => draw());
      const want = url || `text:${initials(row)}`;
      if (t.icon === want) return;
      t.icon = want;
      if (url) {
        t.face.className = 'buff-face';
        t.face.style.removeProperty('--h');
        t.face.innerHTML = `<img alt="" src="${url}">`;
      } else {
        t.face.className = `buff-face text${row.named ? '' : ' unknown'}`;
        t.face.style.setProperty('--h', String(hue(row.skill)));
        t.face.textContent = initials(row);
      }
    }

    function draw() {
      const passed = Date.now() - at;
      const seen = new Set();
      const order = [];
      for (const row of rows) {
        const key = `${row.side}:${row.skill}`;
        seen.add(key);
        const t = tileFor(key);
        paintFace(t, row);
        const left = row.remaining_ms == null ? null : Math.max(0, row.remaining_ms - passed);
        const done = left == null || !row.duration_ms ? 0 : Math.min(1, 1 - left / row.duration_ms);
        t.el.classList.toggle('debuff', row.side === 'target');
        t.el.classList.toggle('fresh', row.age_ms + passed < 350 && row.refreshes > 0);
        t.el.classList.toggle('ending', left != null && left < 3000);
        t.sweep.style.setProperty('--done', `${(done * 100).toFixed(1)}%`);
        t.time.textContent = timeText(left);
        t.count.textContent = row.count > 1 ? String(row.count) : '';
        const label = `${row.name}${left == null ? '' : ` \u00b7 ${timeText(left)} s`}${row.count > 1 ? ` \u00b7 \u00d7${row.count}` : ''}`;
        if (t.label !== label) { t.label = label; t.el.title = label; }
        order.push(t.el);
      }
      for (const [key, t] of tiles) if (!seen.has(key)) { t.el.remove(); tiles.delete(key); }
      const split = rows.findIndex((r) => r.side === 'target');
      if (split > 0) {
        sep = sep || Object.assign(document.createElement('i'), { className: 'buff-sep' });
        order.splice(split, 0, sep);
      } else if (sep) { sep.remove(); sep = null; }
      order.forEach((el, i) => { if (bar.children[i] !== el) bar.insertBefore(el, bar.children[i] || null); });
      bar.hidden = rows.length === 0;
    }

    return {
      update(view, style, layout) {
        const v = view || {};
        rows = [
          ...(v.player || []).map((r) => ({ ...r, side: 'player' })),
          ...(v.target || []).map((r) => ({ ...r, side: 'target' })),
        ];
        at = Date.now();
        const s = style || {};
        const a = align(s, layout);
        wrap.dataset.layout = layout === 'vertical' ? 'vertical' : 'horizontal';
        wrap.dataset.main = a.main;
        wrap.dataset.cross = a.cross;
        const props = bar.style;
        props.setProperty('--buff-tone', isHex(s.accent) ? s.accent : BUFF);
        const alpha = Math.max(0.2, Math.min(1, (Number(s.bg_opacity) || 70) / 100));
        const bg = isHex(s.background) ? s.background : '#0f1115';
        const n = parseInt(bg.slice(1), 16);
        props.background = `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
        if (isHex(s.text)) props.color = s.text; else props.removeProperty('color');
        props.setProperty('--buff-font', `${Number(s.font_px) || 11}px`);
        props.zoom = Number(s.scale) && Number(s.scale) !== 100 ? String(Number(s.scale) / 100) : '';
        draw();
      },
      tick: () => { if (rows.length) draw(); },
      get count() { return rows.length; },
      destroy() { wrap.remove(); tiles.clear(); },
    };
  }

  /** A sample for the preview, the placer and Send test toast. */
  function sample() {
    const row = (skill, name, icon, remaining, duration, extra) => ({ skill, effect: skill * 10 + 1, name, named: true, icon, remaining_ms: remaining, duration_ms: duration, count: 1, refreshes: 0, age_ms: 5000, ...extra });
    return {
      player: [
        row(15310000, 'Wish of Concentration', 'ICON_SO_SKILL_028', 12400, 17500),
        row(15400000, 'Element Enhancement', 'ICON_SO_SKILL_040', 17800, 20000),
        row(15250047, 'Pyroclasm: Fire Damage +5%', null, 6200, 10000, { refreshes: 3 }),
      ],
      target: [
        row(15710000, 'Fire Mark', 'ICON_SO_SKILL_Passive_002', 4100, 5000, { refreshes: 12 }),
        row(15050000, 'Blaze', 'ICON_SO_SKILL_005', 1800, 3000, { count: 2 }),
        row(15320000, 'Delayed Explosion', 'ICON_SO_SKILL_032', 2600, 4000),
      ],
    };
  }

  const api = { create, sample, initials, timeText, align, hue, iconUrl };
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.buffBar = api;
})(typeof window !== 'undefined' ? window : globalThis);
