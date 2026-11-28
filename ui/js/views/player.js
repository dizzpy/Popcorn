// The player: HTML controls drawn over the native mpv view. It is an overlay,
// so the screen underneath keeps its scroll position while a video plays.

import { api, listen } from '../api.js';
import { artwork, button, episodeCode, formatTime, h, icon, iconButton } from '../dom.js';
import { state } from '../state.js';
import { toast, toastError } from '../components/toast.js';

const IDLE_MS = 3000;
/** The countdown starts this close to the end, so nothing is cut off. */
const NEXT_WINDOW_SECS = 10;
const COUNTDOWN_SECS = 10;
/**
 * Right after a switch, events from the previous file may still arrive. A
 * position near the end is only trusted once an earlier one has been seen, or
 * after this long.
 */
const SETTLE_MS = 3000;
const VOLUME_STEP = 5;

let session = null;
let onClosed = () => {};
/** Survives from one video to the next; Rust reports the real values. */
const sound = { volume: 100, muted: false };

export const playerActive = () => session !== null;

const run = (promise) => promise.catch(toastError);

export function initPlayer(options) {
  onClosed = options.onClosed;
  const route = (event, handler) =>
    listen(event, (payload) => {
      if (session) handler(session, payload);
    });
  return Promise.all([
    route('player://state', (s, payload) => {
      s.status = payload.state;
      sound.volume = payload.volume;
      sound.muted = payload.muted;
      if (payload.state === 'playing') s.ended = false;
      s.ui.status();
    }),
    route('player://time', (s, payload) => {
      if (s.loading) return;
      s.position = payload.position;
      s.duration = payload.duration;
      s.ui.time();
      checkCountdown(s);
    }),
    route('player://tracks', (s, payload) => {
      s.tracks = payload;
      s.ui.tracks();
    }),
    route('player://ended', (s, payload) => {
      if (s.loading || payload.file_id !== s.item?.file_id) return;
      s.ended = true;
      if (s.item.next && !s.cancelled) playNext(s);
      else s.ui.wake();
    }),
    route('player://error', (s, payload) => toast(payload.message, { error: true })),
    // Media keys and Control Center; Rust handles play, pause and seeking.
    route('player://remote', (s, payload) => {
      if (payload.action === 'next') playNext(s);
    }),
    route('player://closed', (s, payload) => {
      // Closed from the native side; ignore reports about an earlier file.
      if (!s.loading && payload.file_id === s.item?.file_id) teardown(s);
    }),
  ]);
}

export function openPlayer(fileId, startAt = null) {
  if (session) return;
  document.querySelector('dialog[open]')?.close();
  const s = {
    item: null,
    embedded: null,
    loading: false,
    status: 'buffering',
    returnFocus: document.activeElement,
  };
  resetFile(s);
  session = s;
  state.playing = true;
  s.ui = buildUi(s);
  document.body.append(s.ui.el);
  document.documentElement.classList.add('player-open');
  setBackgroundInert(true);
  s.ui.el.focus({ preventScroll: true });
  load(s, fileId, startAt);
}

/** Forgets everything that belongs to the file being played. */
function resetFile(s) {
  clearInterval(s.countdown?.timer);
  Object.assign(s, {
    position: 0,
    duration: 0,
    tracks: { audio: [], subtitles: [] },
    ended: false,
    armed: false,
    openedAt: 0,
    countdown: null,
    cancelled: false,
  });
}

async function load(s, fileId, startAt) {
  // Set before the first await: it stops a second switch from starting.
  s.loading = true;
  const previous = { tracks: s.tracks, position: s.position, duration: s.duration };
  resetFile(s);
  s.ui.reset();
  let result;
  try {
    result = await api.playerOpen(fileId, startAt);
  } catch (error) {
    toastError(error);
    if (session !== s) return;
    if (!s.item) {
      closePlayer();
      return;
    }
    Object.assign(s, previous, { loading: false, cancelled: true });
    s.ui.reset();
    return;
  }
  if (session !== s) {
    // Closed while opening. Leave a newer player alone.
    if (!session) run(api.playerClose());
    return;
  }
  s.loading = false;
  s.item = result.item;
  s.embedded = result.embedded;
  s.openedAt = performance.now();
  document.documentElement.classList.toggle('playing', s.embedded);
  s.ui.reset();
  s.ui.wake();
}

function playNext(s) {
  if (s.loading || !s.item?.next) return;
  hideCountdown(s);
  load(s, s.item.next.file_id, null);
}

function checkCountdown(s) {
  if (!s.item?.next || s.cancelled || s.loading || !(s.duration > 0)) return;
  if (s.duration - s.position > NEXT_WINDOW_SECS) {
    s.armed = true;
    hideCountdown(s);
    return;
  }
  if (s.countdown) return;
  if (!s.armed && performance.now() - s.openedAt < SETTLE_MS) return;
  let last = performance.now();
  s.countdown = { left: COUNTDOWN_SECS };
  s.countdown.timer = setInterval(() => {
    const now = performance.now();
    // The countdown waits while playback is paused or buffering.
    if (s.status === 'playing') s.countdown.left -= (now - last) / 1000;
    last = now;
    if (s.countdown.left <= 0) playNext(s);
    else s.ui.countdown();
  }, 200);
  s.ui.countdown();
}

function hideCountdown(s) {
  if (!s.countdown) return;
  clearInterval(s.countdown.timer);
  s.countdown = null;
  s.ui.countdown();
}

function setBackgroundInert(inert) {
  for (const id of ['app', 'topbar']) document.getElementById(id).inert = inert;
}

/** Removes the player. `settled` resolves once Rust has saved the progress. */
function teardown(s, settled = Promise.resolve()) {
  if (session !== s) return;
  session = null;
  state.playing = false;
  clearInterval(s.countdown?.timer);
  s.ui.destroy();
  document.documentElement.classList.remove('playing', 'player-open');
  setBackgroundInert(false);
  if (s.returnFocus?.isConnected) s.returnFocus.focus({ preventScroll: true });
  settled.finally(onClosed);
}

export function closePlayer() {
  if (session) teardown(session, run(api.playerClose()));
}

async function escape(s) {
  if (s.ui.closeMenu()) return;
  let wasFullscreen = false;
  try {
    wasFullscreen = await api.playerExitFullscreen();
  } catch (error) {
    toastError(error);
  }
  if (!wasFullscreen && session === s) closePlayer();
}

function togglePause(s) {
  if (s.embedded) s.ui.pulse(s.status === 'paused' || s.ended ? 'play' : 'pause');
  run(api.playerTogglePause());
}
const toggleMute = () => run(api.playerToggleMute());
const toggleFullscreen = () => run(api.playerToggleFullscreen());

function seekBy(s, seconds) {
  run(api.playerSeek(seconds, true));
  // The ten second icons say it all; longer jumps are spelled out.
  if (Math.abs(seconds) === 10) s.ui.pulse(seconds > 0 ? 'skip' : 'rewind');
  else s.ui.flash(`${seconds > 0 ? '+' : '−'}${Math.abs(seconds)} s`);
}

function setVolume(s, volume) {
  sound.volume = Math.min(100, Math.max(0, Math.round(volume)));
  s.ui.status();
  run(api.playerSetVolume(sound.volume));
}

const LANGUAGES = (() => {
  try {
    return new Intl.DisplayNames(['en'], { type: 'language' });
  } catch {
    return null;
  }
})();

function languageName(code) {
  if (!code) return null;
  try {
    return LANGUAGES?.of(code) ?? code;
  } catch {
    return code;
  }
}

function trackLabel(track) {
  const parts = [languageName(track.lang), track.title].filter(Boolean);
  return parts.length ? parts.join(' · ') : `Track ${track.id}`;
}

function selectTrack(s, kind, track) {
  const list = s.tracks[kind];
  // Shown at once so that cycling quickly steps on; Rust sends the real list.
  list.forEach((t) => {
    t.selected = t.id === track?.id;
  });
  s.ui.tracks();
  const audio = kind === 'audio';
  run(audio ? api.playerSetAudio(track.id) : api.playerSetSubtitle(track?.id ?? null));
  s.ui.flash(`${audio ? 'Audio' : 'Subtitles'}: ${track ? trackLabel(track) : 'Off'}`);
}

/** Subtitles cycle through "off" as well; audio does not. */
function cycleTrack(s, kind) {
  const list = s.tracks[kind];
  const choices = kind === 'subtitles' ? [null, ...list] : list;
  if (choices.length < 2) {
    s.ui.flash(kind === 'subtitles' ? 'No subtitles' : 'No other audio tracks');
    return;
  }
  const current = choices.findIndex((t) => (t ? t.selected : !list.some((x) => x.selected)));
  selectTrack(s, kind, choices[(current + 1) % choices.length]);
}

export function playerKey(event) {
  const s = session;
  if (!s) return;
  if (event.metaKey && event.key === '[') {
    event.preventDefault();
    closePlayer();
    return;
  }
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;

  if (key === 'Escape') {
    event.preventDefault();
    if (!event.repeat) escape(s);
    return;
  }
  if (key === 'Tab') {
    s.ui.wake();
    return;
  }
  if (s.ui.menuKey(event)) return;

  const step = event.shiftKey ? 60 : 10;
  const actions = {
    ' ': () => togglePause(s),
    ArrowLeft: () => seekBy(s, -step),
    ArrowRight: () => seekBy(s, step),
    ArrowUp: () => {
      setVolume(s, sound.volume + VOLUME_STEP);
      s.ui.flash(`Volume ${sound.volume}%`);
    },
    ArrowDown: () => {
      setVolume(s, sound.volume - VOLUME_STEP);
      s.ui.flash(`Volume ${sound.volume}%`);
    },
    m: toggleMute,
    f: toggleFullscreen,
    s: () => cycleTrack(s, 'subtitles'),
    a: () => cycleTrack(s, 'audio'),
    n: () => playNext(s),
  };
  const action = actions[key];
  if (!action) return;
  event.preventDefault();
  // Holding a key may keep seeking or changing volume, but not toggle things.
  if (event.repeat && !key.startsWith('Arrow')) return;
  if (!s.loading) action();
}

/** The Playback menu. Returns whether the command was one of its items. */
export function playerCommand(id) {
  const s = session;
  if (!s) return false;
  const actions = {
    play_pause: () => togglePause(s),
    skip_back: () => seekBy(s, -10),
    skip_forward: () => seekBy(s, 10),
    next_episode: () => playNext(s),
    volume_up: () => {
      setVolume(s, sound.volume + VOLUME_STEP);
      s.ui.flash(`Volume ${sound.volume}%`);
    },
    volume_down: () => {
      setVolume(s, sound.volume - VOLUME_STEP);
      s.ui.flash(`Volume ${sound.volume}%`);
    },
    mute: toggleMute,
    next_subtitle: () => cycleTrack(s, 'subtitles'),
    next_audio: () => cycleTrack(s, 'audio'),
    load_subtitle: () => run(api.playerAddSubtitle()),
    close_player: closePlayer,
  };
  const action = actions[id];
  if (!action) return false;
  if (!s.loading || id === 'close_player') action();
  return true;
}

function buildUi(s) {
  let idleTimer;
  let flashTimer;
  let menu = null;
  let scrubbing = null;
  let overControls = false;
  let pointer = '';
  /** What mpv was last told, as a share of the height; null forces a send. */
  let lift = null;
  let shownStatus = '';

  const title = h('span', { class: 'p-title' });
  const subtitle = h('span', { class: 'p-subtitle' });
  const back = iconButton('back', 'Back (Esc)', { onClick: closePlayer });

  const seekFill = h('span', { class: 'seek-played' });
  const seekThumb = h('span', { class: 'seek-thumb' });
  const tooltip = h('span', { class: 'seek-tip', hidden: true });
  const seek = h(
    'div',
    {
      class: 'seek',
      role: 'slider',
      tabIndex: 0,
      'aria-label': 'Seek',
      'aria-valuemin': '0',
    },
    h('span', { class: 'seek-track' }, seekFill),
    seekThumb,
    tooltip,
  );
  const elapsed = h('b');
  const total = h('span');
  const clock = h('span', { class: 'p-time' }, elapsed, ' / ', total);

  const playButton = iconButton('play', 'Play or pause (Space)', {
    class: 'big',
    onClick: () => togglePause(s),
  });
  const rewindButton = iconButton('rewind', 'Back 10 seconds', {
    class: 'p-rew',
    onClick: () => seekBy(s, -10),
  });
  const forwardButton = iconButton('skip', 'Forward 10 seconds', {
    class: 'p-fwd',
    onClick: () => seekBy(s, 10),
  });
  const nextButton = iconButton('next', 'Next episode (N)', { onClick: () => playNext(s) });
  const muteButton = iconButton('volume', 'Mute (M)', { onClick: toggleMute });
  const volume = h('input', {
    type: 'range',
    min: '0',
    max: '100',
    'aria-label': 'Volume',
    onInput: () => setVolume(s, Number(volume.value)),
  });
  const audioButton = iconButton('audio', 'Audio track (A)', {
    'aria-haspopup': 'menu',
    onClick: () => toggleMenu('audio', audioButton),
  });
  const subtitleButton = iconButton('subtitles', 'Subtitles (S)', {
    'aria-haspopup': 'menu',
    onClick: () => toggleMenu('subtitles', subtitleButton),
  });
  const fullscreenButton = iconButton('fullscreen', 'Fullscreen (F)', {
    onClick: toggleFullscreen,
  });
  const menuBox = h('div', { class: 'menu', role: 'menu', hidden: true });

  const top = h(
    'div',
    { class: 'p-ui p-top', 'data-tauri-drag-region': '' },
    back,
    h('div', {}, title, subtitle),
  );
  const controls = h(
    'div',
    { class: 'p-controls' },
      playButton,
      rewindButton,
      forwardButton,
      h('div', { class: 'vol' }, muteButton, volume),
      clock,
      h('span', { class: 'spacer' }),
      nextButton,
      subtitleButton,
      audioButton,
      fullscreenButton,
  );
  const bottom = h('div', { class: 'p-ui p-bottom' }, seek, controls);

  const spinner = h('div', { class: 'p-spinner', role: 'status', 'aria-label': 'Loading' });
  const flashBox = h('div', { class: 'p-flash', role: 'status', hidden: true });
  const pulseBox = h('div', { class: 'p-pulse', 'aria-hidden': 'true' });

  const countdownText = h('small');
  const countdownName = h('b');
  const countdownArt = h('div');
  const countdownCard = h(
    'div',
    { class: 'next-card', hidden: true },
    countdownArt,
    h('div', {}, countdownText, countdownName),
    h(
      'div',
      { class: 'next-buttons' },
      button('Play now', { icon: 'play', variant: 'btn-play btn-sm', onClick: () => playNext(s) }),
      button('Cancel', {
        variant: 'btn-sm',
        onClick: () => {
          s.cancelled = true;
          hideCountdown(s);
          el.focus({ preventScroll: true });
        },
      }),
    ),
  );

  const externalTitle = h('p', { class: 'muted' });
  const external = h(
    'div',
    { class: 'player-external', hidden: true },
    h('h2', { text: 'Playing in a separate window' }),
    externalTitle,
    button('Stop', { variant: 'btn-play', onClick: closePlayer }),
  );

  const surface = h('div', {
    class: 'player-surface',
    onClick: () => {
      if (!closeMenu() && s.embedded) togglePause(s);
    },
    onDblclick: () => {
      if (s.embedded) toggleFullscreen();
    },
  });

  const el = h(
    'div',
    // Controls over video stay dark in either theme.
    { class: 'player theme-dark', tabIndex: -1, role: 'region', 'aria-label': 'Video player' },
    surface,
    // Keeps the window draggable by its top edge.
    h('div', { class: 'player-drag', 'data-tauri-drag-region': '' }),
    external,
    spinner,
    pulseBox,
    flashBox,
    top,
    bottom,
    menuBox,
    countdownCard,
  );

  // The controls stay while they are in use, while loading and at the end.
  // A pause alone does not keep them: like other Mac players, they go once
  // the mouse rests.
  const pinned = () => menu || scrubbing !== null || overControls || s.loading || s.ended;

  /** Keeps subtitles above the controls while they are shown. */
  function liftSubtitles() {
    if (s.embedded !== true || s.loading) return;
    const height = el.clientHeight;
    const shown = !el.classList.contains('idle') && height > 0;
    const covered = shown ? height - seek.getBoundingClientRect().top : 0;
    // Rounded, so that small layout changes do not each cost a call.
    const next = Math.round((Math.max(0, covered) / (height || 1)) * 200) / 200;
    if (next === lift) return;
    lift = next;
    api.playerSetSubtitleLift(next).catch(() => {});
  }

  function hide() {
    clearTimeout(idleTimer);
    if (pinned()) {
      hideLater();
      return;
    }
    el.classList.add('idle');
    liftSubtitles();
    // A hidden control must not keep the focus.
    if (top.contains(document.activeElement) || bottom.contains(document.activeElement)) {
      el.focus({ preventScroll: true });
    }
  }
  function hideLater() {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(hide, IDLE_MS);
  }
  function wake() {
    el.classList.remove('idle');
    liftSubtitles();
    hideLater();
  }
  const onResize = () => {
    lift = null;
    liftSubtitles();
  };
  window.addEventListener('resize', onResize);
  el.addEventListener('pointermove', (event) => {
    // WebKit also reports a move when only the content under the cursor changes.
    const position = `${event.screenX},${event.screenY}`;
    if (position === pointer) return;
    pointer = position;
    wake();
  });
  el.addEventListener('pointerdown', wake);
  // Leaving the window hides the controls at once, as in QuickTime.
  el.addEventListener('pointerleave', () => {
    overControls = false;
    if (scrubbing === null) hide();
  });
  el.addEventListener('focusin', (event) => {
    if (event.target !== el) wake();
  });
  // Only the controls themselves count, not the shaded space around them, or
  // a mouse resting low on the video would keep them forever.
  for (const part of [back, seek, controls, menuBox, countdownCard]) {
    part.addEventListener('pointerenter', () => {
      overControls = true;
    });
    part.addEventListener('pointerleave', () => {
      overControls = false;
    });
  }

  function paintSeek(position) {
    const ratio = s.duration > 0 ? Math.min(1, Math.max(0, position / s.duration)) : 0;
    seekFill.style.transform = `scaleX(${ratio})`;
    seekThumb.style.left = `${ratio * 100}%`;
  }
  function time() {
    const position = scrubbing ?? s.position;
    paintSeek(position);
    elapsed.textContent = formatTime(position);
    total.textContent = formatTime(s.duration);
    seek.setAttribute('aria-valuemax', String(Math.floor(s.duration)));
    seek.setAttribute('aria-valuenow', String(Math.floor(position)));
    seek.setAttribute(
      'aria-valuetext',
      `${formatTime(position)} of ${formatTime(s.duration)}`,
    );
  }

  function timeAt(event) {
    const box = seek.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
    return { ratio, seconds: ratio * s.duration, width: box.width };
  }
  function hover(event) {
    if (!(s.duration > 0)) return null;
    const { ratio, seconds, width } = timeAt(event);
    tooltip.hidden = false;
    tooltip.textContent = formatTime(seconds);
    const half = tooltip.offsetWidth / 2;
    tooltip.style.left = `${Math.min(width - half, Math.max(half, ratio * width))}px`;
    return seconds;
  }
  seek.addEventListener('pointerdown', (event) => {
    const seconds = hover(event);
    if (event.button !== 0 || seconds === null || s.loading) return;
    event.preventDefault();
    seek.setPointerCapture(event.pointerId);
    scrubbing = seconds;
    time();
  });
  seek.addEventListener('pointermove', (event) => {
    const seconds = hover(event);
    if (scrubbing === null || seconds === null) return;
    scrubbing = seconds;
    time();
  });
  seek.addEventListener('pointerup', () => {
    if (scrubbing === null) return;
    s.position = scrubbing;
    scrubbing = null;
    time();
    run(api.playerSeek(s.position, false));
  });
  seek.addEventListener('pointercancel', () => {
    scrubbing = null;
    tooltip.hidden = true;
    time();
  });
  seek.addEventListener('pointerleave', () => {
    if (scrubbing === null) tooltip.hidden = true;
  });
  seek.addEventListener('lostpointercapture', () => {
    if (!seek.matches(':hover')) tooltip.hidden = true;
  });

  function menuItem(label, checked, onClick, key) {
    return h(
      'button',
      {
        type: 'button',
        class: 'menu-item',
        role: checked === null ? 'menuitem' : 'menuitemradio',
        'aria-checked': checked === null ? null : String(checked),
        dataset: { menuKey: key },
        onClick,
      },
      icon(checked === null ? 'plus' : 'check'),
      h('span', { text: label }),
    );
  }
  function renderMenu() {
    if (!menu) return;
    const focused = document.activeElement?.dataset?.menuKey;
    const { kind } = menu;
    const list = s.tracks[kind];
    const items = list.map((track) =>
      menuItem(
        trackLabel(track),
        track.selected,
        () => {
          selectTrack(s, kind, track);
          closeMenu();
        },
        `track-${track.id}`,
      ),
    );
    if (kind === 'subtitles') {
      items.unshift(
        menuItem(
          'Off',
          !list.some((t) => t.selected),
          () => {
            selectTrack(s, kind, null);
            closeMenu();
          },
          'off',
        ),
      );
      items.push(
        h('hr'),
        menuItem(
          'Load subtitle file…',
          null,
          () => {
            closeMenu();
            run(api.playerAddSubtitle());
          },
          'load',
        ),
      );
    }
    menuBox.replaceChildren(
      h('p', { class: 'menu-title', text: kind === 'audio' ? 'Audio' : 'Subtitles' }),
      ...(items.length ? items : [h('p', { class: 'muted menu-title', text: 'No tracks' })]),
    );
    if (focused) menuBox.querySelector(`[data-menu-key="${focused}"]`)?.focus();
  }
  function toggleMenu(kind, opener) {
    const reopen = menu?.kind !== kind;
    closeMenu();
    if (!reopen) return;
    menu = { kind, opener };
    opener.setAttribute('aria-expanded', 'true');
    menuBox.hidden = false;
    renderMenu();
    (
      menuBox.querySelector('[aria-checked="true"]') ?? menuBox.querySelector('button')
    )?.focus();
    wake();
  }
  /** Returns whether a menu was open. */
  function closeMenu() {
    if (!menu) return false;
    const { opener } = menu;
    const hadFocus = menuBox.contains(document.activeElement);
    menu = null;
    menuBox.hidden = true;
    opener.setAttribute('aria-expanded', 'false');
    if (hadFocus) opener.focus({ preventScroll: true });
    return true;
  }
  /** Up and down move through an open menu instead of changing the volume. */
  function menuKey(event) {
    if (!menu || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return false;
    event.preventDefault();
    const items = [...menuBox.querySelectorAll('button')];
    const index = items.indexOf(document.activeElement);
    const step = event.key === 'ArrowDown' ? 1 : -1;
    items[(index + step + items.length) % items.length]?.focus();
    return true;
  }

  let shownPlayIcon = '';
  let shownVolumeIcon = '';
  function status() {
    const playIcon = s.status === 'paused' || s.ended ? 'play' : 'pause';
    if (playIcon !== shownPlayIcon) {
      shownPlayIcon = playIcon;
      playButton.replaceChildren(icon(playIcon));
    }
    const volumeIcon = sound.muted || sound.volume === 0 ? 'muted' : 'volume';
    if (volumeIcon !== shownVolumeIcon) {
      shownVolumeIcon = volumeIcon;
      muteButton.replaceChildren(icon(volumeIcon));
    }
    muteButton.setAttribute('aria-label', sound.muted ? 'Unmute (M)' : 'Mute (M)');
    muteButton.setAttribute('aria-pressed', String(sound.muted));
    volume.value = String(sound.volume);
    volume.style.setProperty('--level', `${sound.volume}%`);
    spinner.hidden = !(s.loading || (s.status === 'buffering' && s.embedded && !s.ended));
    // Pausing shows the controls; they hide again once the mouse rests.
    const shown = s.ended ? 'ended' : s.status;
    if (shown !== shownStatus) {
      shownStatus = shown;
      if (shown === 'paused' || shown === 'ended') wake();
    }
  }

  function countdown() {
    countdownCard.hidden = !s.countdown;
    if (!s.countdown) return;
    const { next } = s.item;
    countdownText.textContent = `Next episode in ${Math.max(1, Math.ceil(s.countdown.left))} s`;
    const code = episodeCode(next.season, next.episode);
    countdownName.textContent = [code, next.episode_name].filter(Boolean).join('  ');
    const seed = `${s.item.title} ${code}`;
    if (countdownArt.dataset.seed !== seed) {
      countdownArt.dataset.seed = seed;
      countdownArt.replaceChildren(
        artwork(null, '', { caption: false, wide: true, seed }),
      );
    }
  }

  /** Redraws everything for a new file, or while one is loading. */
  function reset() {
    closeMenu();
    scrubbing = null;
    tooltip.hidden = true;
    const { item } = s;
    title.textContent = item?.title ?? '';
    const episode =
      item?.season == null
        ? []
        : [episodeCode(item.season, item.episode), item.episode_name].filter(Boolean);
    subtitle.textContent = episode.join(' · ');
    externalTitle.textContent = [item?.title, ...episode].filter(Boolean).join(' · ');
    nextButton.hidden = !item?.next;
    const separate = s.embedded === false;
    el.classList.toggle('external', separate);
    el.classList.toggle('opening', s.embedded === null);
    external.hidden = !separate;
    bottom.hidden = separate;
    // A new file has a new mpv core, with the default margin.
    lift = null;
    liftSubtitles();
    time();
    status();
    countdown();
  }

  function flash(text) {
    if (s.embedded === false) return;
    flashBox.textContent = text;
    flashBox.hidden = false;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => {
      flashBox.hidden = true;
    }, 1200);
  }

  function pulse(name) {
    pulseBox.replaceChildren(icon(name));
    pulseBox.classList.remove('show');
    void pulseBox.offsetWidth;
    pulseBox.classList.add('show');
  }

  return {
    el,
    reset,
    time,
    status,
    tracks: renderMenu,
    countdown,
    wake,
    flash,
    pulse,
    closeMenu,
    menuKey,
    destroy: () => {
      clearTimeout(idleTimer);
      clearTimeout(flashTimer);
      window.removeEventListener('resize', onResize);
      el.remove();
    },
  };
}
