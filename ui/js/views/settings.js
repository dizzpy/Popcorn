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

/** A titled card of settings rows. */
const group = (title, iconName, ...rows) =>
  h(
    'section',
    { class: 'set-group' },
    h('h2', {}, h('span', { class: 'set-ic' }, icon(iconName)), title),
    h('div', { class: 'set-card' }, rows),
  );

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

/** The split-title switch, with a small preview of what it does. */
function splitTitleSetting() {
  const preview = h(
    'div',
    { class: 'set-preview', 'aria-hidden': 'true' },
    h('span', { class: 'pv-main' }),
    h('span', { class: 'pv-sub' }),
  );
  const paint = (on) => {
    preview.classList.toggle('split', on);
    preview.firstChild.textContent = on ? 'The Lord of the Rings' : 'The Lord of the Rings:';
    preview.lastChild.textContent = on ? 'The Return of the King' : ' The Return of the King';
  };
  paint(pref('splitTitles'));
  return setting(
    'Split long titles',
    h(
      'span',
      { class: 'set-note' },
      'On Home, a title with a colon shows its subtitle on a smaller line underneath.',
      preview,
    ),
    toggle('Split long titles', pref('splitTitles'), 'settings:split-titles', (on) => {
      setPref('splitTitles', on);
      paint(on);
    }),
  );
}

/** A labelled row with its controls on the right. */
function setting(label, note, ...controls) {
  const text = typeof note === 'string' ? h('span', { class: 'set-note', text: note }) : note;
  return h(
    'div',
    { class: 'set-row' },
    h('div', { class: 'set-text' }, h('b', { text: label }), text),
    h('div', { class: 'set-controls' }, controls),
  );
}

function scannedNote() {
  const when = state.lastScan?.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return [
    when && `Last scanned at ${when}.`,
    'New files are added automatically while Popcorn is open.',
  ]
    .filter(Boolean)
    .join(' ');
}

/** The Dark / Light switch. `destroy` stops it following the header button. */
function themePicker() {
  const choices = ['dark', 'light'].map((theme) =>
    h('button', {
      type: 'button',
      text: theme === 'dark' ? 'Dark' : 'Light',
      dataset: { value: theme, focusKey: `settings:theme-${theme}` },
      onClick: () => setTheme(theme),
    }),
  );
  const paint = () => {
    for (const choice of choices) {
      choice.setAttribute('aria-pressed', String(choice.dataset.value === currentTheme()));
    }
  };
  window.addEventListener(THEME_CHANGED, paint);
  paint();
  return {
    el: h('div', { class: 'seg', role: 'group', 'aria-label': 'Theme' }, choices),
    destroy: () => window.removeEventListener(THEME_CHANGED, paint),
  };
}

export default {
  load: () => api.getSettings(),
  build(settings, ctx) {
    const folder = h('span', { class: 'path' });
    const keyNote = h('span', { class: 'set-note' });
    const scanNote = h('span', { class: 'set-note' });
    const theme = themePicker();

    const show = (next) => {
      state.settings = next;
      folder.textContent = next.media_folder ?? 'No folder chosen';
      keyNote.textContent = [
        'Used to download posters, cast and descriptions.',
        next.has_tmdb_key ? 'A key is saved. Paste a new one to replace it.' : 'No key is saved.',
      ].join(' ');
      removeKey.hidden = !next.has_tmdb_key;
      scanNote.textContent = scannedNote();
    };
    const reload = async () => show(await api.getSettings());

    const key = keyField({
      draft: ctx.ui,
      focusPrefix: 'settings',
      hasSavedKey: () => state.settings.has_tmdb_key,
    });
    const save = button('Save key', {
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
          key.clear();
          key.say('Key saved. Fetching anything that is missing.');
          await reload();
        } catch (error) {
          key.say(errorMessage(error), true);
        }
      },
    });
    const removeKey = button('Remove key', {
      variant: 'btn-quiet btn-sm',
      dataset: { focusKey: 'settings:remove-key' },
      onClick: async () => {
        if (await attempt(() => api.setTmdbKey(''), 'TMDB key removed.')) {
          key.say('');
          await reload().catch(() => {});
        }
      },
    });
    key.actions.append(save, removeKey);

    const action = (label, focus, run, success, props = {}) => {
      const el = button(label, {
        variant: 'btn-sm',
        ...props,
        dataset: { focusKey: `settings:${focus}` },
        onClick: async () => {
          el.disabled = true;
          await attempt(run, success);
          el.disabled = false;
        },
      });
      return el;
    };
    const link = (label, target) =>
      h('button', {
        type: 'button',
        class: 'link link-accent',
        text: label,
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
            toast('Media folder changed. Scanning…');
          }
        } catch (error) {
          toast(errorMessage(error), { error: true });
        }
      },
    });

    const el = h(
      'div',
      { class: 'page' },
      h(
        'div',
        { class: 'settings' },
        h(
          'header',
          { class: 'set-head' },
          h('h1', { class: 'page-title', text: 'Settings' }),
          h('p', { class: 'set-lede', text: 'Your library, artwork and how Popcorn looks.' }),
        ),
        group(
          'Appearance',
          'sun',
          setting('Theme', 'The video player always stays dark.', theme.el),
          splitTitleSetting(),
        ),
        group(
          'Library',
          'folder',
          setting('Media folder', folder, changeFolder),
          setting(
            'Rescan library',
            scanNote,
            action('Rescan now', 'rescan', () => api.rescan(), 'Rescan started.', {
              icon: 'refresh',
            }),
          ),
        ),
        group(
          'Metadata',
          'wand',
          h(
            'div',
            { class: 'set-row stacked' },
            h(
              'div',
              { class: 'set-text' },
              h('b', { text: 'TMDB API key' }),
              keyNote,
            ),
            key.el,
            link('Get a free TMDB API key', 'tmdb_api'),
          ),
          setting(
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
          setting(
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
        group(
          'About',
          'info',
          setting(
            `Popcorn ${state.version}`.trim(),
            'Free and open source under GPL-3.0. Playback by mpv.',
            link('GitHub', 'github'),
            button('Show welcome screen', {
              variant: 'btn-quiet btn-sm',
              dataset: { focusKey: 'settings:welcome' },
              onClick: () => ctx.navigate('onboarding'),
            }),
          ),
          h(
            'div',
            { class: 'set-row attribution' },
            tmdbLogo(),
            h('span', { text: ATTRIBUTION }),
            link('themoviedb.org', 'tmdb'),
          ),
        ),
      ),
    );
    show(settings);

    // Updated in place so that a half-typed key survives library refreshes.
    return { el, update: show, destroy: theme.destroy };
  },
};
