// The featured carousel on Home, with a film strip to pick a slide.

import { artwork, button, episodeCode, factList, formatRuntime, formatTime, h } from '../dom.js';
import { pref } from '../prefs.js';

function playLabel(play) {
  const code = play.season == null ? null : episodeCode(play.season, play.episode);
  if (play.resume_position) {
    return code ? `Resume ${code}` : `Resume from ${formatTime(play.resume_position)}`;
  }
  return code ? `Play ${code}` : 'Play';
}

/** Splits "Main: Subtitle" at the first colon, when the setting is on. */
function titleParts(text) {
  const at = text.indexOf(': ');
  if (!pref('splitTitles') || at < 1) return { main: text, sub: null };
  return { main: text.slice(0, at), sub: text.slice(at + 2).trim() || null };
}

/** Long titles get a smaller size, so they fit in three lines. */
const sizeClass = (text) => (text.length > 28 ? 'xlong' : text.length > 16 ? 'long' : '');

/** A single long word, like "Thunderbolts*", cannot wrap: shrink the line
 * until it fits the width instead of cutting it off. */
function fitWidth(el) {
  el.style.fontSize = '';
  // The clamped box hides the overflow from scrollWidth; measure the text.
  const range = document.createRange();
  range.selectNodeContents(el);
  const widest = Math.max(0, ...[...range.getClientRects()].map((line) => line.width));
  const over = widest / el.clientWidth;
  if (over > 1.001) {
    const size = parseFloat(getComputedStyle(el).fontSize);
    el.style.fontSize = `${Math.floor((size / over) * 0.98)}px`;
  }
}

function heroTitle(text) {
  const { main, sub } = titleParts(text);
  return h(
    'h1',
    { class: 'hero-title', 'aria-label': text },
    h('span', { class: `ht-main ${sizeClass(main)}`.trim(), text: main }),
    sub && h('span', { class: 'ht-sub', text: sub }),
  );
}

function slide(title, details, ctx) {
  const show = title.kind === 'show';
  const length = show
    ? details && `${details.seasons.length} season${details.seasons.length === 1 ? '' : 's'}`
    : formatRuntime(title.runtime);
  const play = details?.play;
  return h(
    'article',
    { class: 'slide', 'aria-roledescription': 'slide', 'aria-label': title.title },
    artwork(title.backdrop_path, title.title, { caption: false, wide: true }),
    h(
      'div',
      { class: 'hero-copy' },
      h(
        'div',
        { class: 'hero-kicker' },
        h('span', { class: 'kind', text: show ? 'Series' : 'Film' }),
        h('span', { text: title.genres.slice(0, 3).join(', ') }),
      ),
      heroTitle(title.title),
      factList({ ...title, certification: details?.certification }, length),
      title.overview && h('p', { class: 'hero-overview', text: title.overview }),
      h(
        'div',
        { class: 'hero-actions' },
        play &&
          button(playLabel(play), {
            icon: 'play',
            variant: 'btn-play',
            dataset: { focusKey: `hero-play:${title.id}` },
            onClick: () => ctx.play(play.file_id, play.resume_position),
          }),
        button('Details', {
          icon: 'info',
          dataset: { focusKey: `hero-info:${title.id}` },
          onClick: () => ctx.openTitle(title),
        }),
      ),
    ),
  );
}

/** `featured` is a list of `{ title, details }`. Returns `{ el, step, destroy }`. */
export function hero(featured, ctx) {
  let index = Math.min(ctx.ui.slide ?? 0, featured.length - 1);
  const slides = featured.map(({ title, details }) => slide(title, details, ctx));
  const pickers = (build) =>
    featured.length > 1 ? featured.map(({ title }, i) => build(title, i)) : [];
  const picker = (title, i, props, ...children) =>
    h(
      'button',
      { type: 'button', 'aria-label': `Show ${title.title}`, onClick: () => go(i), ...props },
      ...children,
    );
  const thumbs = pickers((title, i) =>
    picker(
      title,
      i,
      { class: 'strip-item' },
      artwork(title.backdrop_path, title.title, { className: 'thumb', caption: false, wide: true }),
      h('span', { class: 'track' }, h('span', { class: 'fill' })),
      h('span', { class: 'name', text: title.title }),
    ),
  );
  const dots = pickers((title, i) => picker(title, i, {}));
  // Its animation ending is what moves the carousel on, so pausing the
  // animation pauses the carousel.
  const timer = h('span', { class: 'hero-timer' });

  const el = h(
    'section',
    { class: 'hero', 'aria-roledescription': 'carousel', 'aria-label': 'Featured' },
    slides,
    h('div', { class: 'grain' }),
    thumbs.length > 0 && [
      timer,
      h('ol', { class: 'strip' }, thumbs.map((thumb) => h('li', {}, thumb))),
      h('div', { class: 'dots' }, dots),
    ],
  );

  function go(next) {
    const hadFocus = slides[index].contains(document.activeElement);
    index = (next + slides.length) % slides.length;
    ctx.ui.slide = index;
    slides.forEach((item, i) => {
      item.classList.toggle('active', i === index);
      item.inert = i !== index;
    });
    for (const set of [thumbs, dots]) {
      set.forEach((item, i) => {
        item.classList.toggle('active', i === index);
        item.setAttribute('aria-current', String(i === index));
      });
    }
    // Restart both animations.
    for (const target of [timer, thumbs[index]?.querySelector('.fill')]) {
      if (!target) continue;
      target.style.animation = 'none';
      void target.offsetWidth;
      target.style.animation = '';
    }
    timer.classList.add('run');
    if (hadFocus) slides[index].querySelector('button')?.focus({ preventScroll: true });
  }

  // The copy's width follows the window, so fit again when it changes.
  const fitter = new ResizeObserver(() => {
    for (const title of el.querySelectorAll('.ht-main, .ht-sub')) fitWidth(title);
  });
  fitter.observe(el);

  timer.addEventListener('animationend', () => go(index + 1));
  const pause = (on) => el.classList.toggle('paused', on);
  el.addEventListener('mouseenter', () => pause(true));
  el.addEventListener('mouseleave', () => pause(false));
  el.addEventListener('focusin', (event) => pause(event.target.matches(':focus-visible')));
  el.addEventListener('focusout', () => pause(false));
  const onVisibility = () => pause(document.hidden);
  document.addEventListener('visibilitychange', onVisibility);
  go(index);

  return {
    el,
    step: (by) => go(index + by),
    destroy: () => {
      document.removeEventListener('visibilitychange', onVisibility);
      fitter.disconnect();
    },
  };
}
