// "Check for updates" (the version pill in the detailed meter) and the
// automatic background check (Config > General, on by default).
// The app asks GitHub for the latest release of aion-tallymark; on "Update now"
// it downloads the installer, checks its SHA-256 against the release's
// SHA256SUMS.txt, starts it and closes so the files can be replaced. Nothing is
// downloaded or installed unless the user clicks "Update now". App only.
//
// Automatic checks only mark the pill, keep live.update_available, and fire
// `update-available` once per new version. There is no overlay indicator yet.
(function () {
  const SKIP_KEY = 'aion2dps.update.skip';
  const tauri = window.__TAURI__;
  const inApp = Boolean(tauri && tauri.core);
  const invoke = (cmd, args) => tauri.core.invoke(cmd, args);

  const errText = (err) => String((err && err.message) || err || 'Something went wrong.').trim();

  /** The latest release; `fresh` asks GitHub even if it was asked lately. */
  const check = (fresh) => invoke('check_update', { fresh: Boolean(fresh) });

  const mb = (n) => `${(n / 1048576).toFixed(1)} MB`;

  /** Release notes as plain text, without markdown marks. */
  const plainNotes = (text) => String(text || '').replace(/\r/g, '')
    .replace(/^#{1,6}\s*/gm, '').replace(/\*\*(.+?)\*\*/g, '$1').replace(/`([^`]+)`/g, '$1').trim();

  let pill = null;
  let skipped = '';

  function mark(latest) {
    if (!pill) return;
    const on = Boolean(latest);
    pill.classList.toggle('on', on);
    pill.title = on ? `Version ${latest} is available - click to see what's new` : '';
  }

  /** Apply live.update_available (string or null) from a meter snapshot. */
  function sync(live) {
    if (!live) return;
    const latest = live.update_available || '';
    if (latest) skipped = '';
    mark(latest || '');
  }

  function migrateSkip() {
    const old = localStorage.getItem(SKIP_KEY) || '';
    if (!old || !inApp) return Promise.resolve();
    return invoke('set_update_skipped', { version: old }).then(() => {
      skipped = old;
      localStorage.removeItem(SKIP_KEY);
    }).catch(() => {});
  }

  /** The meter's button and dialog. */
  function attach(button, versionLabel) {
    if (!button) return;
    pill = button;
    if (!inApp) { button.hidden = true; return; }
    invoke('app_version').then((v) => { if (versionLabel) versionLabel.textContent = `v${v}`; }).catch(() => {});

    const back = document.createElement('div');
    back.className = 'ae-back up-back';
    back.hidden = true;
    back.innerHTML = `
      <div class="ae up" role="dialog" aria-modal="true" aria-labelledby="up-title">
        <div class="ae-head"><h2 id="up-title">Check for updates</h2><button type="button" class="t-close" data-close title="Close"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg></button></div>
        <p class="up-msg" data-msg></p>
        <pre class="up-notes" data-notes hidden></pre>
        <div class="up-bar" data-bar hidden><i></i></div>
        <p class="up-step" data-step hidden></p>
        <div class="ap-btns">
          <a class="up-link" data-link href="#" hidden>Release page</a>
          <button type="button" class="btn" data-skip hidden>Skip this version</button>
          <button type="button" class="btn" data-later>Close</button>
          <button type="button" class="btn primary" data-update hidden>Update now</button>
          <button type="button" class="btn" data-again hidden>Check again</button>
        </div>
      </div>`;
    document.body.appendChild(back);
    const q = (s) => back.querySelector(`[data-${s}]`);
    let current = null;
    let installing = false;

    function view({ msg, notes, update, skip, again, link, later = 'Close', bad }) {
      q('msg').textContent = msg;
      q('msg').classList.toggle('bad', Boolean(bad));
      q('notes').hidden = !notes;
      q('notes').textContent = notes || '';
      q('update').hidden = !update;
      q('skip').hidden = !skip;
      q('again').hidden = !again;
      q('link').hidden = !link;
      q('later').textContent = later;
      q('bar').hidden = true;
      q('step').hidden = true;
    }

    function close() {
      if (installing) return;
      back.hidden = true;
    }

    async function run(fresh) {
      back.hidden = false;
      view({ msg: 'Checking GitHub for a newer version\u2026' });
      try {
        current = await check(fresh);
      } catch (err) {
        view({ msg: errText(err), again: true, bad: true });
        return;
      }
      const on = Boolean(current.available);
      mark(on ? current.latest : '');
      if (!current.newer) {
        view({ msg: `You're up to date (v${current.current} is the latest).`, link: Boolean(current.url) });
        return;
      }
      const size = current.size ? ` (${mb(current.size)})` : '';
      const canInstall = Boolean(current.asset);
      view({
        msg: canInstall
          ? `Version ${current.latest} is available. You have v${current.current}. "Update now" downloads the installer${size}, checks it, installs it and restarts aion2dps.`
          : `Version ${current.latest} is available, but it has no Windows installer attached. Get it from the release page.`,
        notes: plainNotes(current.notes) || 'No release notes.',
        update: canInstall,
        skip: !current.skipped,
        link: Boolean(current.url),
        later: 'Later',
      });
    }

    const STEPS = {
      sums: 'Getting SHA256SUMS.txt\u2026',
      download: 'Downloading\u2026',
      verify: 'Checking the SHA-256\u2026',
      install: 'Starting the installer. aion2dps closes now and opens again when it is done.',
    };
    if (tauri.event) {
      tauri.event.listen('update-progress', ({ payload: p }) => {
        if (!installing || !p) return;
        const step = q('step');
        step.hidden = false;
        let text = STEPS[p.stage] || '';
        if (p.stage === 'download') {
          const bar = q('bar');
          bar.hidden = false;
          const pct = p.total ? Math.min(100, Math.round((p.done / p.total) * 100)) : 0;
          bar.firstElementChild.style.width = `${pct}%`;
          text = `Downloading\u2026 ${mb(p.done)}${p.total ? ` of ${mb(p.total)} (${pct}%)` : ''}`;
        }
        step.textContent = text;
      }).catch(() => {});
      tauri.event.listen('update-available', ({ payload: c }) => {
        if (c && (c.available || (c.newer && !c.skipped))) mark(c.latest);
      }).catch(() => {});
    }

    async function update() {
      if (!current || installing) return;
      installing = true;
      ['update', 'skip', 'later', 'again'].forEach((s) => { q(s).disabled = true; });
      q('step').hidden = false;
      q('step').textContent = 'Preparing\u2026';
      q('msg').textContent = `Updating to ${current.latest}\u2026`;
      try {
        await invoke('install_update', { version: current.latest });
        q('step').textContent = STEPS.install;
      } catch (err) {
        installing = false;
        ['update', 'skip', 'later', 'again'].forEach((s) => { q(s).disabled = false; });
        q('bar').hidden = true;
        q('step').hidden = true;
        q('msg').textContent = errText(err);
        q('msg').classList.add('bad');
      }
    }

    button.addEventListener('click', () => run(true));
    q('close').addEventListener('click', close);
    q('later').addEventListener('click', close);
    q('again').addEventListener('click', () => run(true));
    q('update').addEventListener('click', update);
    q('skip').addEventListener('click', () => {
      if (!current) { close(); return; }
      skipped = current.latest;
      invoke('set_update_skipped', { version: current.latest }).catch(() => {});
      mark('');
      close();
    });
    q('link').addEventListener('click', (event) => {
      event.preventDefault();
      invoke('open_release_page').catch(() => {});
    });
    back.addEventListener('click', (event) => { if (event.target === back) close(); });
    document.addEventListener('keydown', (event) => { if (event.key === 'Escape' && !back.hidden) close(); });

    migrateSkip();

    function openDialog() {
      run(false);
    }
    if (tauri.event) {
      tauri.event.listen('open-update', () => { openDialog(); }).catch(() => {});
    }
    invoke('take_open_update_dialog').then((open) => { if (open) openDialog(); }).catch(() => {});
    window.updates.openDialog = openDialog;
  }

  window.updates = { attach, sync };
})();