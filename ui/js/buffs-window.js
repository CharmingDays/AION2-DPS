// The buff bar window: shows itself only while there are icons. Its size,
// place and colours follow Config > Toasts & pop-ups > Buff bar; which
// skills show, the layout and on/off follow Config > Buffs & debuffs.
// Click-through: it never takes the mouse or focus from the game.
(function () {
  const tauri = window.__TAURI__;
  const styles = window.toastStyle;
  const bar = window.buffBar.create(document.body);
  const TEST_MS = 6000;
  let test = 0;
  let last = null;
  let visible = false;

  const sync = (want) => {
    if (want === visible || !tauri) return;
    visible = want;
    tauri.core.invoke('buffs_visible', { show: want }).catch(() => {});
  };

  function render() {
    const data = last || {};
    const settings = data.buff_settings || {};
    const style = styles.of(data.toast_styles, 'buffs');
    const testing = Date.now() < test;
    const view = testing ? window.buffBar.sample() : (settings.enabled === false ? null : data.buffs);
    bar.update(view, style, settings.layout);
    sync(bar.count > 0);
  }

  if (tauri) {
    tauri.event.listen('buffs-test', () => {
      test = Date.now() + TEST_MS;
      render();
      setTimeout(render, TEST_MS + 50);
    });
  }
  window.meterBridge.subscribe((data) => { last = data; if (Date.now() >= test) render(); });
  setInterval(() => bar.tick(), 100);
})();
