// Router and bootstrap.

import { api, errorMessage, listen } from './api.js';
import { button, h, icon } from './dom.js';
import { state } from './state.js';
import { currentTheme, syncWindowTheme, THEME_CHANGED, toggleTheme } from './theme.js';
import { moveFocus } from './components/keyboard-nav.js';
import { scanStatus } from './components/scan-status.js';
import { toast, toastError } from './components/toast.js';
import detailsMovie from './views/details-movie.js';
import detailsShow from './views/details-show.js';
import home from './views/home.js';
import movies from './views/movies.js';
import onboarding from './views/onboarding.js';
import {
  closePlayer,
  initPlayer,
  openPlayer,
  playerActive,
  playerCommand,
  playerKey,
} from './views/player.js';
import search from './views/search.js';
import settings from './views/settings.js';
import shows from './views/shows.js';

// A view is `{ load(params, ui), build(data, ctx) }`. `build` returns
// `{ el, ready?, update?, destroy? }`; without `update`, a reload rebuilds it.
const VIEWS = {
  onboarding,
  home,
  movies,
  shows,
  search,
  settings,
  movie: detailsMovie,
  show: detailsShow,
};

const HISTORY_LIMIT = 50;
/** Reloads wait for a quiet moment, but never longer than the maximum. */
const DEBOUNCE_MS = 400;
const DEBOUNCE_MAX_MS = 2000;

const app = document.getElementById('app');
const topbar = document.getElementById('topbar');
const searchInput = document.getElementById('search');

/** The entry whose screen is in the page, which lags behind while loading. */
let shown = null;
let renderToken = 0;

const current = () => state.history.at(-1);
const entry = (name, params = {}) => ({ name, params, ui: {}, snapshot: null });

const context = (ui) => ({
  ui,
  navigate,
  back,
  openTitle: ({ id, kind }) => navigate(kind === 'show' ? 'show' : 'movie', { id }),
  play: openPlayer,
  refresh,
  start,
});

function navigate(name, params = {}, ui = {}) {
  const top = current();
  if (top?.name === name && top.params.id === params.id) {
    app.scrollTo({ top: 0, behavior: 'smooth' });
    app.querySelector('[data-autofocus]')?.focus({ preventScroll: true });
    return;
  }
  // Remember the screen being left before anything changes.
  if (shown) shown.entry.snapshot = takeSnapshot();
  state.history.push({ ...entry(name, params), ui });
  if (state.history.length > HISTORY_LIMIT) state.history.shift();
  render();
}

function back() {
  if (state.history.length < 2) return;
  state.history.pop();
  render();
}

function start() {
  const { onboarded, media_folder: folder } = state.settings;
  state.history = [entry(onboarded && folder ? 'home' : 'onboarding')];
  render();
}

function takeSnapshot() {
  const active = document.activeElement;
  return {
    scroll: app.scrollTop,
    focusKey: app.contains(active) ? (active.dataset.focusKey ?? null) : null,
    rows: [...app.querySelectorAll('[data-scroll-key]')].map((el) => [
      el.dataset.scrollKey,
      el.scrollLeft,
    ]),
  };
}

function restore({ scroll, focusKey, rows }) {
  for (const [key, left] of rows) {
    const el = app.querySelector(`[data-scroll-key="${CSS.escape(key)}"]`);
    if (el) el.scrollLeft = left;
  }
  app.scrollTop = scroll;
  const typing = document.activeElement === searchInput;
  if (focusKey && !typing && !document.querySelector('dialog[open]') && !playerActive()) {
    app.querySelector(`[data-focus-key="${CSS.escape(focusKey)}"]`)?.focus({
      preventScroll: true,
    });
  }
  onScroll();
}

async function mount(target, data, saved) {
  shown?.view.destroy?.();
  const view = VIEWS[target.name].build(data, context(target.ui));
  app.replaceChildren(view.el);
  shown = { entry: target, view };
  updateChrome();
  await view.ready?.();
  if (shown.view !== view) return;
  if (saved) {
    restore(saved);
  } else {
    app.scrollTop = 0;
    onScroll();
    // Typing in the header search must not lose the focus.
    if (playerActive() || document.activeElement === searchInput) return;
    const autofocus = app.querySelector('[data-autofocus]');
    (autofocus ?? app).focus({ preventScroll: true });
  }
}

/** Loads and shows the newest history entry. */
async function render() {
  const target = current();
  const token = ++renderToken;
  updateChrome();
  let data;
  try {
    data = await VIEWS[target.name].load(target.params, target.ui);
  } catch (error) {
    if (token !== renderToken) return;
    toastError(error);
    if (state.history.length < 2) {
      showFailure(error, render);
      return;
    }
    // Drop the screen that failed. After a failed step forward the one before
    // it is still in the page; after a failed step back, load the one earlier.
    state.history.pop();
    if (shown?.entry === current()) updateChrome();
    else await render();
    return;
  }
  if (token !== renderToken) return;
  // A screen seen before comes back as it was left.
  await mount(target, data, target.snapshot);
}

function showFailure(error, retry) {
  shown?.view.destroy?.();
  shown = null;
  app.replaceChildren(
    h(
      'div',
      { class: 'page' },
      h(
        'div',
        { class: 'empty empty-large' },
        h('h1', { text: 'Could not load this screen' }),
        h('p', { text: errorMessage(error) }),
        button('Try again', { variant: 'btn-play', onClick: retry }),
      ),
    ),
  );
}

let refreshing = null;
let refreshAgain = false;

/** Reloads the current screen in place. Calls made meanwhile run once more. */
function refresh() {
  if (refreshing) {
    refreshAgain = true;
    return refreshing;
  }
  refreshing = (async () => {
    do {
      refreshAgain = false;
      await reloadShown();
    } while (refreshAgain);
    refreshing = null;
  })();
  return refreshing;
}

async function reloadShown() {
  // While a navigation is loading, it will bring fresh data itself.
  if (!shown || shown.entry !== current()) return;
  const { entry: target, view } = shown;
  const token = renderToken;
  const stale = () => token !== renderToken || shown?.view !== view;
  try {
    const data = await VIEWS[target.name].load(target.params, target.ui);
    if (stale()) return;
    if (view.update) await view.update(data);
    else await mount(target, data, takeSnapshot());
  } catch (error) {
    if (stale()) return;
    toastError(error);
    // The title may have been removed from the library.
    if (target.params.id !== undefined) back();
  }
}

let debounceTimer = null;
let debounceStarted = 0;

function libraryUpdated() {
  const now = performance.now();
  if (debounceTimer === null) debounceStarted = now;
  clearTimeout(debounceTimer);
  const wait = Math.min(DEBOUNCE_MS, Math.max(0, debounceStarted + DEBOUNCE_MAX_MS - now));
  debounceTimer = setTimeout(() => {
    debounceTimer = null;
    // Closing the player reloads, so nothing is redrawn under the video.
    if (!state.playing) refresh();
  }, wait);
}

/** Screens whose artwork runs underneath a see-through header. */
const IMMERSIVE = new Set(['home', 'movie', 'show']);

function updateChrome() {
  const name = current()?.name;
  topbar.classList.toggle('minimal', !name || name === 'onboarding');
  if (name === 'search') searchInput.value = current().ui.query ?? '';
  else if (document.activeElement !== searchInput) searchInput.value = '';
  onScroll();
  for (const tab of topbar.querySelectorAll('[data-route]:not(.brand)')) {
    if (tab.dataset.route === name) tab.setAttribute('aria-current', 'page');
    else tab.removeAttribute('aria-current');
  }
}

function onScroll() {
  const name = current()?.name;
  const bare = !name || name === 'onboarding';
  topbar.classList.toggle('solid', !bare && (!IMMERSIVE.has(name) || app.scrollTop > 40));
}

function focusSearch() {
  searchInput.focus();
  searchInput.select();
}

function onSearchInput() {
  const query = searchInput.value;
  if (current()?.name !== 'search') {
    if (query.trim()) navigate('search', {}, { query });
    return;
  }
  if (!query) {
    back();
    return;
  }
  current().ui.query = query;
  // While the screen is still loading, it will read the query itself.
  if (shown?.entry === current()) shown.view.search(query);
}

function onSearchKey(event) {
  if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    searchInput.value = '';
    searchInput.blur();
    if (current()?.name === 'search') back();
  } else if (event.key === 'ArrowDown' || event.key === 'Enter') {
    const first = app.querySelector('[data-nav-item]');
    if (first && current()?.name === 'search') {
      event.preventDefault();
      first.focus();
    }
  }
}

/** Left and right turn the featured carousel when no row card has the focus. */
function stepHero(event) {
  const step = { ArrowLeft: -1, ArrowRight: 1 }[event.key];
  if (!step || !shown?.view.heroStep || app.scrollTop > 300) return false;
  if (event.target.closest('.scroller, .header')) return false;
  event.preventDefault();
  shown.view.heroStep(step);
  return true;
}

function onKeyDown(event) {
  if (playerActive()) {
    playerKey(event);
    return;
  }
  // An open dialog handles its own keys.
  if (event.defaultPrevented || document.querySelector('dialog[open]')) return;
  const onboardingScreen = current()?.name === 'onboarding';
  if (event.metaKey && event.key === '[') {
    event.preventDefault();
    back();
  } else if (
    !onboardingScreen &&
    (((event.metaKey || event.ctrlKey) && ['k', 'f'].includes(event.key.toLowerCase())) ||
      (event.key === '/' && !event.target.matches('input, select')))
  ) {
    event.preventDefault();
    focusSearch();
  } else if (event.metaKey || event.ctrlKey || event.altKey) {
    return;
  } else if (event.key === 'Escape') {
    event.preventDefault();
    if (!event.repeat) back();
  } else if (!event.target.matches('input, select') && !stepHero(event)) {
    moveFocus(event);
  }
}

// Space also clicks buttons on key up in some engines.
function onKeyUp(event) {
  if (playerActive() && event.key === ' ') event.preventDefault();
}

/** Items of the native menu bar (see src-tauri/src/menu.rs). */
function onMenuCommand(id) {
  if (id === 'theme') {
    toggleTheme();
    return;
  }
  // While a video plays, only the Playback menu applies.
  if (playerActive()) {
    if (id === 'back') closePlayer();
    else playerCommand(id);
    return;
  }
  if (document.querySelector('dialog[open]') || current()?.name === 'onboarding') return;
  if (['home', 'movies', 'shows', 'settings'].includes(id)) navigate(id);
  else if (id === 'search') focusSearch();
  else if (id === 'back') back();
  else if (id === 'rescan') {
    api
      .rescan()
      .then(() => toast('Rescan started.'))
      .catch(toastError);
  }
}

/** The header button shows the theme it switches to. */
function paintThemeToggle() {
  const dark = currentTheme() === 'dark';
  const toggle = document.getElementById('theme-toggle');
  const label = `Switch to ${dark ? 'light' : 'dark'} theme`;
  toggle.replaceChildren(icon(dark ? 'sun' : 'moon'));
  toggle.setAttribute('aria-label', label);
  toggle.title = label;
}

function initChrome() {
  document.getElementById('theme-toggle').addEventListener('click', toggleTheme);
  window.addEventListener(THEME_CHANGED, paintThemeToggle);
  paintThemeToggle();
  for (const control of topbar.querySelectorAll('[data-route]')) {
    if (control.dataset.icon) control.prepend(icon(control.dataset.icon));
    control.addEventListener('click', () => navigate(control.dataset.route));
  }
  document.getElementById('search-box').prepend(icon('search'));
  searchInput.addEventListener('input', onSearchInput);
  searchInput.addEventListener('keydown', onSearchKey);
  app.addEventListener('scroll', onScroll, { passive: true });
  window.addEventListener('keydown', onKeyDown);
  window.addEventListener('keyup', onKeyUp);
}

async function boot() {
  initChrome();
  if (!window.__TAURI__) {
    app.replaceChildren(
      h('p', { class: 'empty empty-large', text: 'Popcorn needs to run inside its app.' }),
    );
    return;
  }
  document.documentElement.classList.add('tauri');
  syncWindowTheme();
  try {
    const onScan = scanStatus();
    await Promise.all([
      listen('library://updated', libraryUpdated),
      listen('scan://progress', onScan),
      initPlayer({ onClosed: refresh }),
      listen('menu://command', onMenuCommand),
    ]);
    const [saved, info] = await Promise.all([api.getSettings(), api.appInfo()]);
    state.settings = saved;
    state.version = info.version;
    start();
  } catch (error) {
    showFailure(error, () => window.location.reload());
  }
  window.addEventListener('pagehide', closePlayer);
}

boot();
