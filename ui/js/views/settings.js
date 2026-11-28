// Settings: a list of sections on the left, one pane at a time on the right.

import { api, errorMessage } from '../api.js';
import { button, h, icon } from '../dom.js';
import { pref, setPref } from '../prefs.js';
import { state } from '../state.js';
import { currentTheme, setTheme, THEME_CHANGED } from '../theme.js';
import { attempt, toast } from '../components/toast.js';
import { keyField } from '../components/key-field.js';

const ATTRIBUTION = 'This product uses the TMDB API but is not endorsed or certified by TMDB.';

/**
 * TMDB LOGO SLOT.
 * TMDB's attribution rules ask for their official logo. It is not shipped or
 * drawn here: download the approved asset from themoviedb.org (About > Logos
 * and Attribution) and save it as ui/img/tmdb-logo.svg. Until that file loads,
 * a plain text badge is shown.
 */
function tmdbLogo() {
  const badge = h('span', { class: 'tmdb-mark', text: 'TMDB' });
  const logo = h('img', { class: 'tmdb-logo', alt: 'TMDB logo' });
  logo.addEventListener('load', () => badge.replaceWith(logo));
  logo.src = 'img/tmdb-logo.svg';
  return badge;
}

const SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: 'sun' },
  { id: 'library', label: 'Library', icon: 'folder' },
  { id: 'metadata', label: 'Metadata', icon: 'wand' },
  { id: 'shortcuts', label: 'Shortcuts', icon: 'keyboard' },
  { id: 'about', label: 'About', icon: 'info' },
];

/** The keys the app really handles: main.js, views/player.js and menu.rs. */
const SHORTCUTS = [
  [
    'Library',
    [
      ['Search', ['⌘', 'K']],
      ['Home, Movies, TV Shows', ['⌘', '1 2 3']],
      ['Go back', ['Esc']],
      ['Open settings', ['⌘', ',']],
      ['Rescan library', ['⌘', 'R']],
      ['Switch theme', ['⇧', '⌘', 'L']],
      ['Next or previous featured title', ['←', '→'], 'On Home, near the top.'],
    ],
  ],
  [
    'Player',
    [
      ['Play or pause', ['Space']],
      ['Back or forward 10 seconds', ['←', '→']],
      ['Back or forward 1 minute', ['⇧', '←', '→']],
      ['Volume up or down', ['↑', '↓']],
      ['Mute', ['M']],
      ['Full screen', ['F']],
      ['Next subtitle track', ['S']],
      ['Next audio track', ['A']],
      ['Next episode', ['N']],
      ['Close the player', ['Esc']],
    ],
  ],
];

const card = (...rows) => h('div', { class: 'set-card' }, rows);

/** A row: text on the left, controls on the right. */
function row(label, note, ...controls) {
  const text = typeof note === 'string' ? h('p', { class: 'set-note', text: note }) : note;
  return h(
    'div',
    { class: 'set-row' },
    h('div', { class: 'set-text' }, h('b', { text: label }), text),
    controls.length > 0 && h('div', { class: 'set-controls' }, controls),
  );
}

/** Briefly shows "Saved" in a pane's heading. */
function savedBadge() {
  const el = h('span', { class: 'set-saved', 'aria-live': 'polite' }, icon('check'), 'Saved');
  let timer;
  return {
    el,
    flash() {
      el.classList.add('show');
      clearTimeout(timer);
      timer = setTimeout(() => el.classList.remove('show'), 1400);
    },
  };
}

function pane(section, ...children) {
  return h(
    'section',
    {
      class: 'set-pane',
      id: `set-${section.id}`,
      role: 'tabpanel',
      'aria-labelledby': `set-tab-${section.id}`,
    },
    children,
  );
}

const paneHead = (title, extra) => h('div', { class: 'set-pane-head' }, h('h2', { text: title }), extra);

/** The Dark / Light switch. `destroy` stops it following the header button. */
function themePicker(saved) {
  const choices = ['dark', 'light'].map((theme) =>
    h('button', {
      type: 'button',
      role: 'radio',
      text: theme === 'dark' ? 'Dark' : 'Light',
      dataset: { value: theme, focusKey: `settings:theme-${theme}` },
      onClick: () => {
        if (theme === currentTheme()) return;
        setTheme(theme);
        saved.flash();
      },
    }),
  );
  const paint = () => {
    for (const choice of choices) {
      choice.setAttribute('aria-checked', String(choice.dataset.value === currentTheme()));
    }
  };
  window.addEventListener(THEME_CHANGED, paint);
  paint();
  return {
    el: h('div', { class: 'seg', role: 'radiogroup', 'aria-label': 'Theme' }, choices),
    destroy: () => window.removeEventListener(THEME_CHANGED, paint),
  };
}

/** An on/off switch. `onChange` gets the new value. */
function toggle(label, on, focusKey, onChange) {
  const el = h('button', {
    type: 'button',
    class: 'switch',
    role: 'switch',
    'aria-label': label,
    dataset: { focusKey },
    onClick: () => {
      const next = el.getAttribute('aria-checked') !== 'true';
      el.setAttribute('aria-checked', String(next));
      onChange(next);
    },
  });
  el.setAttribute('aria-checked', String(on));
  return el;
}

const SAMPLE = ['The Lord of the Rings', 'The Return of the King'];

function splitTitles(saved) {
  const preview = h('div', { class: 'set-preview-title' });
  const paint = (on) => {
    preview.replaceChildren(
      ...(on ? [SAMPLE[0], h('span', { text: SAMPLE[1] })] : [SAMPLE.join(': ')]),
    );
  };
  paint(pref('splitTitles'));
  return card(
    row(
      'Split long titles',
      'On Home, the part after a colon moves to a smaller line underneath.',
      toggle('Split long titles', pref('splitTitles'), 'settings:split-titles', (on) => {
        setPref('splitTitles', on);
        paint(on);
        saved.flash();
      }),
    ),
    h(
      'div',
      { class: 'set-preview', 'aria-hidden': 'true' },
      h('small', { text: 'Preview' }),
      preview,
    ),
  );
}

function shortcutsPane(section) {
  const keys = (list) =>
    h(
      'span',
      { class: 'set-keys' },
      list.map((key) => h('kbd', { text: key })),
    );
  return pane(
    section,
    paneHead('Shortcuts'),
    SHORTCUTS.map(([group, items]) => [
      h('div', { class: 'set-group-label', text: group }),
      card(items.map(([label, list, note]) => row(label, note ?? null, keys(list)))),
    ]),
  );
}

export default {
  load: () => api.getSettings(),
  build(settings, ctx) {
    const saved = Object.fromEntries(SECTIONS.map((s) => [s.id, savedBadge()]));
    const folder = h('span', { class: 'path' });
    const scanNote = h('p', { class: 'set-note' });
    const keyNote = h('p', { class: 'set-note', role: 'status' });
    const theme = themePicker(saved.appearance);

    // ── TMDB key ──
    const key = keyField({
      draft: ctx.ui,
      focusPrefix: 'settings',
      hasSavedKey: () => state.settings.has_tmdb_key,
    });
    const keyEdit = h('div', { class: 'set-key-edit', id: 'set-key-edit' }, key.el);
    const keyLink = h('button', {
      type: 'button',
      class: 'link link-accent',
      text: 'Get a free TMDB API key',
      dataset: { focusKey: 'settings:link-tmdb_api' },
      onClick: () => attempt(() => api.openExternal('tmdb_api')),
    });
    keyEdit.append(keyLink);
    let editing = false;
    const keyToggle = button('Replace', {
      variant: 'btn-sm',
      dataset: { focusKey: 'settings:key-toggle' },
      'aria-controls': 'set-key-edit',
      onClick: () => {
        openEditor(!editing);
        if (editing) keyEdit.querySelector('input')?.focus();
      },
    });
    function openEditor(open) {
      editing = open;
      keyEdit.classList.toggle('open', open);
      keyToggle.setAttribute('aria-expanded', String(open));
      if (!open) {
        key.clear();
        key.say('');
      }
      paintKey();
    }
    const save = button('Save', {
      variant: 'btn-play btn-sm',
      dataset: { focusKey: 'settings:save-key' },
      onClick: async () => {
        const value = key.value();
        if (!value) {
          key.say('Paste a key first.', true);
          return;
        }
        try {
          await api.setTmdbKey(value);
          await reload();
          openEditor(false);
          toast('Key saved. Fetching anything that is missing.');
        } catch (error) {
          key.say(errorMessage(error), true);
        }
      },
    });
    key.actions.append(save);

    // Remove asks twice, so one stray click cannot drop the key.
    let armed = null;
    const removeKey = button('Remove', {
      variant: 'btn-quiet btn-sm btn-danger',
      dataset: { focusKey: 'settings:remove-key' },
      onClick: async () => {
        if (!armed) {
          removeKey.lastChild.textContent = 'Click again to remove';
          armed = setTimeout(disarm, 4000);
          return;
        }
        disarm();
        if (await attempt(() => api.setTmdbKey(''), 'TMDB key removed.')) {
          await reload().catch(() => {});
        }
      },
    });
    function disarm() {
      clearTimeout(armed);
      armed = null;
      removeKey.lastChild.textContent = 'Remove';
    }
    const removeRow = row(
      'Remove key',
      'Posters and details stop updating until you add a key again.',
      removeKey,
    );

    function paintKey() {
      const has = state.settings.has_tmdb_key;
      keyNote.classList.toggle('ok', has);
      keyNote.classList.toggle('warn', !has);
      keyNote.textContent = has
        ? 'A key is saved. Used to download posters, cast and descriptions.'
        : 'No key. Add one to get posters and details.';
      // Without a key the editor stays open: there is nothing to replace.
      if (!has && !editing) {
        editing = true;
        keyEdit.classList.add('open');
      }
      keyToggle.hidden = !has;
      keyToggle.lastChild.textContent = editing ? 'Cancel' : 'Replace';
      keyToggle.setAttribute('aria-expanded', String(editing));
      removeRow.hidden = !has;
    }

    const show = (next) => {
      state.settings = next;
      folder.textContent = next.media_folder ?? 'No folder chosen';
      folder.title = next.media_folder ?? '';
      const when = state.lastScan?.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      scanNote.textContent = [
        when && `Last scanned at ${when}.`,
        'New files are added automatically while Popcorn is open.',
      ]
        .filter(Boolean)
        .join(' ');
      paintKey();
    };
    const reload = async () => show(await api.getSettings());

    const action = (label, focus, run, success, props = {}) => {
      const el = button(label, {
        variant: 'btn-sm',
        ...props,
        dataset: { focusKey: `settings:${focus}` },
        onClick: async () => {
          el.disabled = true;
          el.setAttribute('aria-busy', 'true');
          await attempt(run, success);
          el.removeAttribute('aria-busy');
          el.disabled = false;
        },
      });
      return el;
    };
    const external = (label, target, props = {}) =>
      button(label, {
        variant: 'btn-sm',
        ...props,
        dataset: { focusKey: `settings:link-${target}` },
        onClick: () => attempt(() => api.openExternal(target)),
      });

    const changeFolder = button('Change folder', {
      icon: 'folder',
      variant: 'btn-sm',
      dataset: { focusKey: 'settings:folder' },
      onClick: async () => {
        try {
          if (await api.pickMediaFolder()) {
            await reload();
            saved.library.flash();
            toast('Media folder changed. Scanning…');
          }
        } catch (error) {
          toast(errorMessage(error), { error: true });
        }
      },
    });

    // ── Panes ──
    const panes = {
      appearance: pane(
        SECTIONS[0],
        paneHead('Appearance', saved.appearance.el),
        card(row('Theme', 'The video player always stays dark.', theme.el)),
        splitTitles(saved.appearance),
      ),
      library: pane(
        SECTIONS[1],
        paneHead('Library', saved.library.el),
        card(
          row('Media folder', folder, changeFolder),
          row(
            'Rescan library',
            scanNote,
            action('Rescan now', 'rescan', () => api.rescan(), 'Rescan started.', {
              icon: 'refresh',
            }),
          ),
        ),
      ),
      metadata: pane(
        SECTIONS[2],
        paneHead('Metadata', saved.metadata.el),
        card(row('TMDB API key', keyNote, keyToggle), keyEdit, removeRow),
        card(
          row(
            'Refresh all metadata',
            'Download details and artwork again for every title. Titles you fixed by hand keep their match.',
            action(
              'Refresh',
              'refresh-all',
              () => api.refreshAllMetadata(),
              'Refreshing metadata in the background.',
              { icon: 'refresh' },
            ),
          ),
          row(
            'Image cache',
            'Posters and backdrops stored on this Mac.',
            action(
              'Clear cache',
              'clear-cache',
              () => api.clearImageCache(),
              'Image cache cleared. Images will download again.',
              { variant: 'btn-quiet btn-sm' },
            ),
          ),
        ),
      ),
      shortcuts: shortcutsPane(SECTIONS[3]),
      about: pane(
        SECTIONS[4],
        paneHead('About'),
        card(
          h(
            'div',
            { class: 'set-about' },
            h('span', { class: 'set-about-logo', 'aria-hidden': 'true' }, appLogo()),
            h(
              'div',
              {},
              h('b', { text: 'Popcorn' }),
              h('p', {
                text: [
                  state.version && `Version ${state.version}.`,
                  'Free and open source under GPL-3.0. Playback by mpv.',
                ]
                  .filter(Boolean)
                  .join(' '),
              }),
            ),
          ),
          row(
            'Source code',
            'Read the code, follow releases or contribute.',
            external('GitHub', 'github'),
          ),
          row(
            'Welcome screen',
            'See the first-launch setup again. Your library isn’t changed.',
            button('Show', {
              variant: 'btn-quiet btn-sm',
              dataset: { focusKey: 'settings:welcome' },
              onClick: () => ctx.navigate('onboarding'),
            }),
          ),
        ),
        card(
          h(
            'div',
            { class: 'set-row attribution' },
            tmdbLogo(),
            h(
              'p',
              { class: 'set-note' },
              `${ATTRIBUTION} `,
              h('button', {
                type: 'button',
                class: 'link link-accent',
                text: 'themoviedb.org',
                dataset: { focusKey: 'settings:link-tmdb' },
                onClick: () => attempt(() => api.openExternal('tmdb')),
              }),
            ),
          ),
        ),
      ),
    };

    // ── Section list ──
    const tabs = SECTIONS.map((section, i) =>
      h(
        'button',
        {
          type: 'button',
          class: 'set-tab',
          role: 'tab',
          id: `set-tab-${section.id}`,
          'aria-controls': `set-${section.id}`,
          dataset: { focusKey: `settings:tab-${section.id}` },
          onClick: () => open(section.id),
          // Up and down move between sections, like a macOS sidebar.
          onKeydown: (event) => {
            const step = { ArrowDown: 1, ArrowUp: -1, ArrowRight: 1, ArrowLeft: -1 }[event.key];
            const to = { Home: 0, End: SECTIONS.length - 1 }[event.key];
            if (step === undefined && to === undefined) return;
            event.preventDefault();
            const next = SECTIONS[to ?? (i + step + SECTIONS.length) % SECTIONS.length];
            open(next.id);
            tabs[SECTIONS.indexOf(next)].focus();
          },
        },
        icon(section.icon),
        section.label,
      ),
    );

    function open(id) {
      if (!panes[id]) id = SECTIONS[0].id;
      ctx.ui.section = id;
      SECTIONS.forEach((section, i) => {
        const on = section.id === id;
        tabs[i].setAttribute('aria-selected', String(on));
        tabs[i].tabIndex = on ? 0 : -1;
        panes[section.id].hidden = !on;
      });
    }

    const el = h(
      'div',
      { class: 'page settings' },
      h(
        'aside',
        { class: 'set-side' },
        h('h1', { class: 'set-title', text: 'Settings' }),
        h(
          'nav',
          { role: 'tablist', 'aria-label': 'Settings sections', 'aria-orientation': 'vertical' },
          tabs,
        ),
      ),
      h('div', { class: 'set-panes' }, Object.values(panes)),
    );
    show(settings);
    open(ctx.ui.section);

    // Updated in place so that a half-typed key survives library refreshes.
    return {
      el,
      update: show,
      destroy: () => {
        theme.destroy();
        clearTimeout(armed);
      },
    };
  },
};

/** The header's logo, reused so there is one drawing of it. */
function appLogo() {
  return document.querySelector('#topbar .logo')?.cloneNode(true) ?? '';
}
