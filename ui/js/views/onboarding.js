// First launch: choose the media folder, optionally add a TMDB key.

import { api, errorMessage, listen } from '../api.js';
import { button, h, icon } from '../dom.js';
import { state } from '../state.js';
import { keyField } from '../components/key-field.js';
import { attempt, toastError } from '../components/toast.js';

const plural = (count, word) => `${count} ${word}${count === 1 ? '' : 's'}`;
const folderName = (path) => path.split('/').filter(Boolean).pop() || path;

export default {
  load: async () => null,
  build(_, ctx) {
    /** `idle` before a folder is chosen here, then `scanning`, `ready` or `failed`. */
    let scan = state.settings.media_folder ? 'ready' : 'idle';
    let found = null;
    const unlisteners = [];

    // ── Folder picker: a big target that also takes a dropped folder ──
    const drop = h(
      'button',
      {
        type: 'button',
        class: 'wl-drop',
        dataset: { focusKey: 'onboarding:folder', autofocus: '' },
        onClick: () => pick(),
      },
      h('span', { class: 'wl-ic' }, icon('folderAdd')),
      h('span', {}, h('b', { text: 'Choose media folder' }), h('span', { text: 'or drag a folder here' })),
    );

    const name = h('b');
    const path = h('span');
    const bar = h('div', {
      class: 'wl-progress',
      role: 'progressbar',
      'aria-label': 'Scanning folder',
    });
    const counts = h('span');
    const ready = h('span', { class: 'wl-ready' }, icon('check'), 'Ready');
    const chosen = h(
      'div',
      { class: 'wl-chosen', 'aria-live': 'polite' },
      h(
        'div',
        { class: 'wl-chosen-top' },
        h('span', { class: 'wl-ic wl-ic-accent' }, icon('folder')),
        h('div', { class: 'wl-path' }, name, path),
        h('button', {
          type: 'button',
          class: 'wl-link',
          text: 'Change',
          dataset: { focusKey: 'onboarding:change' },
          onClick: () => pick(),
        }),
      ),
      bar,
      h('div', { class: 'wl-status' }, counts, ready),
    );

    const go = button('Open my library', {
      variant: 'btn-play wl-go',
      dataset: { focusKey: 'onboarding:done' },
      onClick: () => finish(),
    });
    const hint = h('p', { class: 'wl-hint', role: 'status' });

    // ── Optional TMDB key ──
    const key = keyField({
      draft: ctx.ui,
      focusPrefix: 'onboarding',
      hasSavedKey: () => state.settings.has_tmdb_key,
    });
    const keyBox = h(
      'details',
      { class: 'wl-key', open: Boolean(ctx.ui.key) },
      h(
        'summary',
        { dataset: { focusKey: 'onboarding:key-toggle' } },
        icon('forward'),
        'Add a TMDB key for posters and details (optional)',
      ),
      h(
        'div',
        { class: 'wl-key-body' },
        h(
          'p',
          {},
          'TMDB is a free movie database. Your key stays on this Mac. ',
          h('button', {
            type: 'button',
            class: 'link link-accent',
            text: 'Get a free key',
            dataset: { focusKey: 'onboarding:help' },
            onClick: () => attempt(() => api.openExternal('tmdb_api')),
          }),
        ),
        key.el,
      ),
    );

    function paint() {
      const folder = state.settings.media_folder;
      drop.hidden = Boolean(folder);
      chosen.hidden = !folder;
      if (folder) {
        name.textContent = folderName(folder);
        path.textContent = folder;
      }
      bar.hidden = scan !== 'scanning';
      ready.hidden = scan !== 'ready';
      if (scan === 'scanning') {
        counts.textContent = found ? `Found ${plural(found, 'video')}…` : 'Looking for videos…';
      } else if (scan === 'ready' && found !== null) {
        counts.textContent = found
          ? `Found ${plural(found, 'video')}`
          : 'No videos yet. Add some to this folder anytime.';
      } else if (scan === 'ready') {
        counts.textContent = 'Popcorn keeps this folder in sync.';
      }
      // The library fills in as the scan goes, so there is no need to wait.
      go.disabled = !folder;
    }

    function useFolder(folder) {
      state.settings.media_folder = folder;
      hint.textContent = '';
      scan = 'scanning';
      found = 0;
      paint();
    }

    async function pick() {
      drop.disabled = true;
      hint.textContent = '';
      try {
        const picked = await api.pickMediaFolder();
        if (picked) useFolder(picked);
      } catch (failure) {
        hint.textContent = errorMessage(failure);
      }
      drop.disabled = false;
    }

    async function finish() {
      if (!state.settings.media_folder) {
        hint.textContent = 'Choose a folder first.';
        drop.focus();
        return;
      }
      go.disabled = true;
      try {
        if (key.value()) await api.setTmdbKey(key.value());
        await api.finishOnboarding();
        state.settings = await api.getSettings();
        ctx.start();
      } catch (failure) {
        toastError(failure);
        paint();
      }
    }

    unlisteners.push(
      listen('scan://progress', ({ found: total, done, error }) => {
        if (scan !== 'scanning') return;
        found = total;
        if (error) {
          scan = 'failed';
          counts.textContent = '';
          hint.textContent = error;
        } else if (done) scan = 'ready';
        paint();
      }),
      // Rust takes the folder from the native drop and reports it here.
      listen('onboarding://drop', ({ over, path: dropped, error }) => {
        drop.classList.toggle('over', over);
        if (error) hint.textContent = error;
        else if (dropped) useFolder(dropped);
      }),
    );

    paint();
    const logo = document.querySelector('#topbar .logo').cloneNode(true);
    return {
      el: h(
        'div',
        { class: 'welcome' },
        h(
          'div',
          { class: 'wl-col' },
          h('div', { class: 'wl-mark', 'aria-hidden': 'true' }, logo),
          h('h1', { text: 'Welcome to Popcorn' }),
          h('p', {
            class: 'wl-lede',
            text: 'Choose the folder where you keep your movies and shows. Popcorn only reads it; nothing is moved or uploaded.',
          }),
          drop,
          chosen,
          go,
          hint,
          keyBox,
        ),
        h('p', { class: 'wl-foot', text: 'You can change all of this later in Settings.' }),
      ),
      // Nothing here depends on the library.
      update: () => {},
      destroy: () => {
        for (const unlisten of unlisteners) unlisten.then((stop) => stop());
      },
    };
  },
};
