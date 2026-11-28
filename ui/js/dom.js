// Small DOM, image and formatting helpers.
// Library text (file names, TMDB text) is untrusted: it only ever reaches the
// page through textContent and element properties, never through markup.

import { fileUrl } from './api.js';
import { motif } from './components/motif.js';

/** The number of `.art-pN` and `.face-pN` classes in tokens.css. */
const PALETTES = 12;
export const FACES = 6;

export function h(tag, props = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === null || value === undefined) continue;
    if (key === 'class') el.className = value;
    else if (key === 'text') el.textContent = value;
    else if (key === 'dataset') Object.assign(el.dataset, value);
    else if (key.startsWith('on')) el.addEventListener(key.slice(2).toLowerCase(), value);
    else if (key.includes('-') || key === 'role' || key === 'for') el.setAttribute(key, value);
    else el[key] = value;
  }
  append(el, ...children);
  return el;
}

export function append(el, ...children) {
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false || child === '') continue;
    el.append(child);
  }
  return el;
}

// Each icon is a list of shapes: a path string, or [tag, attributes, text].
const solid = (d) => ['path', { d, class: 'solid' }];
const ring = (cx, cy, r) => ['circle', { cx, cy, r }];
const ten = (x) => ['text', { x, y: '15.3' }, '10'];
const SPEAKER = solid('M4 9.5h3.5L12 6v12l-4.5-3.5H4z');

const ICONS = {
  play: [solid('M7.5 5.2v13.6L19 12z')],
  pause: [solid('M7 5h3.4v14H7zM13.6 5H17v14h-3.4z')],
  back: ['M15 5l-7 7 7 7'],
  forward: ['M9 5l7 7-7 7'],
  check: ['M5 12.5l4.5 4.5L19 7.5'],
  info: [ring(12, 12, 8.5), 'M12 11v5.5M12 7.8v.2'],
  search: [ring(11, 11, 6.5), 'M16 16l4.5 4.5'],
  settings: ['M4 7h9M17 7h3M4 17h3M11 17h9', ring(15, 7, 2.2), ring(9, 17, 2.2)],
  subtitles: [
    ['rect', { x: 3, y: 5.5, width: 18, height: 13, rx: 2.5 }],
    'M10.3 10.3a2.4 2.4 0 1 0 0 3.4M16.8 10.3a2.4 2.4 0 1 0 0 3.4',
  ],
  audio: ['M4 10v4M8 7v10M12 4.5v15M16 8v8M20 10.5v3'],
  volume: [SPEAKER, 'M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11'],
  muted: [SPEAKER, 'M16 9.5l5 5M21 9.5l-5 5'],
  fullscreen: ['M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5'],
  rewind: ['M5.2 12a7 7 0 1 0 2.2-5.1', 'M4.5 3.8v3.6h3.6', ten('12.4')],
  skip: ['M18.8 12a7 7 0 1 1-2.2-5.1', 'M19.5 3.8v3.6h-3.6', ten('11.6')],
  next: [solid('M6 5.5v13l9.5-6.5z'), 'M18.5 5v14'],
  folder: ['M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z'],
  folderAdd: [
    'M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z',
    'M12 11v5M9.5 13.5h5',
  ],
  refresh: ['M19 12a7 7 0 1 1-2.1-5', 'M19.5 4.5V8H16'],
  restart: ['M5 12a7 7 0 1 0 2.1-5', 'M4.5 4.5V8H8'],
  star: [solid('M12 3.8l2.5 5.1 5.6.8-4 3.9.9 5.6-5-2.6-5 2.6.9-5.6-4-3.9 5.6-.8z')],
  wand: ['M4 20L14 10', solid('M15.5 3.5l.9 2.3 2.3.9-2.3.9-.9 2.3-.9-2.3-2.3-.9 2.3-.9z')],
  eye: ['M2.5 12s3.5-6.5 9.5-6.5 9.5 6.5 9.5 6.5-3.5 6.5-9.5 6.5S2.5 12 2.5 12z', ring(12, 12, 2.8)],
  plus: ['M12 5v14M5 12h14'],
  close: ['M6 6l12 12M18 6L6 18'],
  sun: [ring(12, 12, 4), 'M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4'],
  moon: ['M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z'],
};

export function svg(tag, attrs = {}, ...children) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
  el.append(...children);
  return el;
}

export function icon(name) {
  return svg(
    'svg',
    { viewBox: '0 0 24 24', class: 'icon', 'aria-hidden': 'true' },
    ...ICONS[name].map((shape) => {
      if (typeof shape === 'string') return svg('path', { d: shape });
      const [tag, attrs, text] = shape;
      return svg(tag, attrs, ...(text ? [text] : []));
    }),
  );
}

export function button(label, { icon: iconName, variant = '', ...props } = {}) {
  return h(
    'button',
    { type: 'button', ...props, class: `btn ${variant} ${props.class ?? ''}`.trim() },
    iconName && icon(iconName),
    label && h('span', { text: label }),
  );
}

export function iconButton(iconName, label, props = {}) {
  return h(
    'button',
    {
      type: 'button',
      title: label,
      'aria-label': label,
      ...props,
      class: `icon-btn ${props.class ?? ''}`.trim(),
    },
    icon(iconName),
  );
}

export function hashOf(text) {
  let hash = 0;
  for (const char of String(text)) hash = (hash * 31 + char.codePointAt(0)) >>> 0;
  return hash;
}

// Images that have loaded once are shown at once when a view is rebuilt, so a
// library refresh does not make the artwork fade in again.
const seenImages = new Set();

/**
 * A box showing a cached image over generated artwork, which stays when there
 * is no image or it fails to load. The artwork's colors and motif follow from
 * `seed`, so a title always looks the same.
 */
export function artwork(source, label, options = {}) {
  const { className = '', caption = true, sub = null, wide = false, seed = label, remote = false } =
    options;
  const hash = hashOf(seed);
  const url = remote ? source : fileUrl(source);
  const box = h('span', {
    class: ['art', `art-p${(hash % PALETTES) + 1}`, wide && 'art-wide', className]
      .filter(Boolean)
      .join(' '),
  });
  if (!url) box.append(motif(hash, wide));
  if (caption) {
    box.append(
      h('span', { class: 'art-label' }, label, sub && h('small', { text: String(sub) })),
    );
  }
  if (url) {
    const img = h('img', { alt: '', loading: 'lazy', decoding: 'async', draggable: false });
    const loaded = () => {
      seenImages.add(url);
      box.classList.add('has-image');
    };
    img.addEventListener('load', loaded);
    img.addEventListener('error', () => {
      img.remove();
      box.classList.remove('has-image', 'instant');
    });
    if (seenImages.has(url)) box.classList.add('instant', 'has-image');
    img.src = url;
    box.append(img);
  }
  return box;
}

export function progressBar(ratio, className = '') {
  const fill = h('span', { class: 'progress-fill' });
  fill.style.transform = `scaleX(${Math.min(1, Math.max(0, ratio))})`;
  return h('span', { class: `progress ${className}`.trim() }, fill);
}

export function progressRatio(position, duration) {
  return duration > 0 ? position / duration : 0;
}

// Mirrors `progress::resume_position` in Rust.
export function resumeAt(position, duration) {
  return duration > 0 && position > 60 && position < duration * 0.9 ? position : null;
}

export function formatTime(seconds) {
  const total = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = String(total % 60).padStart(2, '0');
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${secs}` : `${minutes}:${secs}`;
}

export function formatRuntime(minutes) {
  if (!minutes) return null;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (!hours) return `${rest} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

export function formatSize(bytes) {
  if (!(bytes > 0)) return null;
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const exponent = Math.min(units.length - 1, Math.floor(Math.log10(bytes) / 3));
  const value = bytes / 1000 ** exponent;
  return `${value.toFixed(exponent >= 3 ? 1 : 0)} ${units[exponent]}`;
}

export function formatResolution(width, height) {
  return width && height ? `${width} × ${height}` : null;
}

export function episodeCode(season, episode) {
  return `S${season} E${episode}`;
}

export function formatRating(rating) {
  return rating > 0 ? rating.toFixed(1) : null;
}

export function formatLeft(position, duration) {
  return `${formatRuntime(Math.max(1, Math.round((duration - position) / 60)))} left`;
}

/** Year, length, certificate and score. `extra` entries are plain text. */
export function factList({ year, certification, rating }, ...extra) {
  const items = [year, ...extra].filter(Boolean).map((text) => h('li', { text: String(text) }));
  if (certification) items.push(h('li', { class: 'cert', text: certification }));
  const score = formatRating(rating);
  if (score) {
    items.push(h('li', { class: 'score', title: 'TMDB rating' }, icon('star'), score));
  }
  return h('ul', { class: 'facts' }, items);
}
